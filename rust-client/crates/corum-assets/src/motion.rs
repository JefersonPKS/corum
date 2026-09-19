use std::fmt;

const HEADER_SIZE: usize = 0xA0;
const RECORD_TAG: u32 = 0x0000_F000;

#[derive(Debug, Clone, PartialEq)]
pub struct MotionFile {
    pub version: u32,
    pub ticks_per_frame: u32,
    pub first_frame: u32,
    pub last_frame: u32,
    pub frame_speed: u32,
    pub field_14: u32,
    pub field_18: u32,
    pub duration_ticks: u32,
    pub name: String,
    pub records: Vec<MotionRecord>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MotionRecord {
    pub offset: usize,
    pub payload_size: usize,
    pub counters: [u32; 5],
    pub candidate_name: String,
    pub track_24: Vec<Keyframe24>,
    pub track_20: Vec<Keyframe20>,
    pub track_36: Vec<Keyframe36>,
    pub morph_key_count: u32,
    pub morph_bytes: usize,
}

impl MotionFile {
    /// Number of frames, `first_frame..=last_frame`.
    #[must_use]
    pub fn frame_count(&self) -> u32 {
        self.last_frame.saturating_sub(self.first_frame) + 1
    }

    /// Length of the motion in seconds (`frame_speed` is frames per second).
    #[must_use]
    pub fn duration_seconds(&self) -> f32 {
        self.frame_count() as f32 / self.frame_speed.max(1) as f32
    }

    /// Looping frame position for a time in seconds.
    #[must_use]
    pub fn frame_at(&self, seconds: f32) -> f32 {
        let frames = self.frame_count() as f32;
        (seconds * self.frame_speed.max(1) as f32).rem_euclid(frames) + self.first_frame as f32
    }

    /// Frame position for a motion played **once**: it stops on the last frame instead of looping
    /// (attacks, being hit, dying).
    #[must_use]
    pub fn frame_clamped_at(&self, seconds: f32) -> f32 {
        let last = (self.frame_count() - 1) as f32;
        (seconds * self.frame_speed.max(1) as f32).clamp(0.0, last) + self.first_frame as f32
    }

    /// Time in seconds at which the motion reaches `frame` (a frame number as the `.cdt` tables give
    /// them); frames before the first one give 0.
    #[must_use]
    pub fn seconds_at_frame(&self, frame: u32) -> f32 {
        frame.saturating_sub(self.first_frame) as f32 / self.frame_speed.max(1) as f32
    }
}

impl MotionRecord {
    /// Whether the record animates its node at all.
    #[must_use]
    pub fn has_tracks(&self) -> bool {
        !self.track_24.is_empty() || !self.track_20.is_empty() || !self.track_36.is_empty()
    }

    /// Rotation quaternion `(x, y, z, w)` at `frame`, interpolated between keyframes. Comes
    /// from the 24-byte track; the 36-byte track (position followed by a quaternion, an
    /// unverified reading) is the fallback. Frames outside the keys hold the nearest key.
    #[must_use]
    pub fn rotation_at(&self, frame: f32) -> Option<[f32; 4]> {
        if !self.track_24.is_empty() {
            let keys: Vec<(u32, [f32; 4])> = self
                .track_24
                .iter()
                .map(|key| (key.frame_index, key.value))
                .collect();
            return Some(sample(&keys, frame, slerp));
        }
        if !self.track_36.is_empty() {
            let keys: Vec<(u32, [f32; 4])> = self
                .track_36
                .iter()
                .map(|key| {
                    (
                        key.frame_index,
                        [key.value[3], key.value[4], key.value[5], key.value[6]],
                    )
                })
                .collect();
            return Some(sample(&keys, frame, slerp));
        }
        None
    }

    /// Position at `frame` from the 20-byte track (or the first three values of the 36-byte one).
    #[must_use]
    pub fn position_at(&self, frame: f32) -> Option<[f32; 3]> {
        let lerp = |a: [f32; 3], b: [f32; 3], t: f32| {
            [
                a[0] + (b[0] - a[0]) * t,
                a[1] + (b[1] - a[1]) * t,
                a[2] + (b[2] - a[2]) * t,
            ]
        };
        if !self.track_20.is_empty() {
            let keys: Vec<(u32, [f32; 3])> = self
                .track_20
                .iter()
                .map(|key| (key.frame_index, key.value))
                .collect();
            return Some(sample(&keys, frame, lerp));
        }
        if !self.track_36.is_empty() {
            let keys: Vec<(u32, [f32; 3])> = self
                .track_36
                .iter()
                .map(|key| (key.frame_index, [key.value[0], key.value[1], key.value[2]]))
                .collect();
            return Some(sample(&keys, frame, lerp));
        }
        None
    }
}

/// Value at `frame` between the two surrounding keys (`frame_index` order), holding the
/// first or last key outside the range.
fn sample<T: Copy>(keys: &[(u32, T)], frame: f32, blend: impl Fn(T, T, f32) -> T) -> T {
    let (first, last) = (keys[0], keys[keys.len() - 1]);
    if frame <= first.0 as f32 {
        return first.1;
    }
    if frame >= last.0 as f32 {
        return last.1;
    }
    let after = keys.partition_point(|key| (key.0 as f32) <= frame);
    let (before, after) = (keys[after - 1], keys[after]);
    let span = (after.0 - before.0) as f32;
    blend(before.1, after.1, (frame - before.0 as f32) / span.max(1.0))
}

/// Shortest-path spherical interpolation of two quaternions.
fn slerp(a: [f32; 4], mut b: [f32; 4], t: f32) -> [f32; 4] {
    let mut dot: f32 = a.iter().zip(b).map(|(a, b)| a * b).sum();
    if dot < 0.0 {
        b = b.map(|v| -v);
        dot = -dot;
    }
    let (wa, wb) = if dot > 0.9995 {
        (1.0 - t, t)
    } else {
        let angle = dot.acos();
        let sine = angle.sin();
        (((1.0 - t) * angle).sin() / sine, (t * angle).sin() / sine)
    };
    let mixed = [
        a[0] * wa + b[0] * wb,
        a[1] * wa + b[1] * wb,
        a[2] * wa + b[2] * wb,
        a[3] * wa + b[3] * wb,
    ];
    let length = mixed.iter().map(|v| v * v).sum::<f32>().sqrt().max(1e-6);
    mixed.map(|v| v / length)
}

#[derive(Debug, Clone, PartialEq)]
pub struct Keyframe20 {
    pub tick: u32,
    pub frame_index: u32,
    pub value: [f32; 3],
}

#[derive(Debug, Clone, PartialEq)]
pub struct Keyframe24 {
    pub tick: u32,
    pub frame_index: u32,
    pub value: [f32; 4],
}

#[derive(Debug, Clone, PartialEq)]
pub struct Keyframe36 {
    pub tick: u32,
    pub frame_index: u32,
    pub value: [f32; 7],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MotionError {
    pub offset: usize,
    pub message: String,
}

impl fmt::Display for MotionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid ANM at offset 0x{:X}: {}",
            self.offset, self.message
        )
    }
}

impl std::error::Error for MotionError {}

impl MotionFile {
    pub fn parse(bytes: &[u8]) -> Result<Self, MotionError> {
        require(bytes, 0, HEADER_SIZE)?;
        let version = u32_at(bytes, 0)?;
        if version != 1 {
            return Err(error(0, format!("unsupported version {version}")));
        }

        let mut records = Vec::new();
        let mut offset = HEADER_SIZE;
        while offset < bytes.len() {
            require(bytes, offset, 8)?;
            let tag = u32_at(bytes, offset)?;
            if tag != RECORD_TAG {
                return Err(error(
                    offset,
                    format!("expected record tag 0x{RECORD_TAG:08X}, found 0x{tag:08X}"),
                ));
            }
            let payload_size = usize_at(bytes, offset + 4)?;
            let payload = checked_add(offset, 8, offset)?;
            let end = checked_add(payload, payload_size, offset + 4)?;
            require(bytes, payload, payload_size)?;
            require(bytes, payload, 0x98)?;

            let counters = [
                u32_at(bytes, payload)?,
                u32_at(bytes, payload + 4)?,
                u32_at(bytes, payload + 8)?,
                u32_at(bytes, payload + 12)?,
                u32_at(bytes, payload + 16)?,
            ];
            if counters[0] != 0 {
                return Err(error(
                    payload,
                    format!("unknown track counter 0 is nonzero: {}", counters[0]),
                ));
            }
            let raw_name = c_string(bytes, payload + 0x14, 128)?;
            let candidate_name = if raw_name.len() >= 2
                && raw_name.chars().all(|character| !character.is_control())
            {
                raw_name
            } else {
                String::new()
            };

            let mut track_offset = payload + 0x98;
            // The 20-byte (position) keys come first in the file, then the 24-byte (rotation)
            // keys, even though the counters list the rotation count first. Reading them the
            // other way round keeps every byte total valid and misaligns the second array.
            let track_20 = parse_track_20(bytes, &mut track_offset, counters[2])?;
            let track_24 = parse_track_24(bytes, &mut track_offset, counters[1])?;
            let track_36 = parse_track_36(bytes, &mut track_offset, counters[3])?;
            if track_offset > end {
                return Err(error(track_offset, "keyframe tracks exceed the record"));
            }
            let morph_bytes = end - track_offset;
            if counters[4] == 0 && morph_bytes != 0 {
                return Err(error(
                    track_offset,
                    format!("{morph_bytes} unexplained bytes after known keyframe tracks"),
                ));
            }
            records.push(MotionRecord {
                offset,
                payload_size,
                counters,
                candidate_name,
                track_24,
                track_20,
                track_36,
                morph_key_count: counters[4],
                morph_bytes,
            });
            offset = end;
        }

        Ok(Self {
            version,
            ticks_per_frame: u32_at(bytes, 4)?,
            first_frame: u32_at(bytes, 8)?,
            last_frame: u32_at(bytes, 12)?,
            frame_speed: u32_at(bytes, 16)?,
            field_14: u32_at(bytes, 20)?,
            field_18: u32_at(bytes, 24)?,
            duration_ticks: u32_at(bytes, 28)?,
            name: c_string(bytes, 32, 128)?,
            records,
        })
    }
}

fn parse_track_20(
    bytes: &[u8],
    offset: &mut usize,
    count: u32,
) -> Result<Vec<Keyframe20>, MotionError> {
    let mut values = Vec::with_capacity(count as usize);
    for _ in 0..count {
        require(bytes, *offset, 20)?;
        values.push(Keyframe20 {
            tick: u32_at(bytes, *offset)?,
            frame_index: u32_at(bytes, *offset + 4)?,
            value: [
                f32_at(bytes, *offset + 8)?,
                f32_at(bytes, *offset + 12)?,
                f32_at(bytes, *offset + 16)?,
            ],
        });
        *offset += 20;
    }
    Ok(values)
}

fn parse_track_24(
    bytes: &[u8],
    offset: &mut usize,
    count: u32,
) -> Result<Vec<Keyframe24>, MotionError> {
    let mut values = Vec::with_capacity(count as usize);
    for _ in 0..count {
        require(bytes, *offset, 24)?;
        values.push(Keyframe24 {
            tick: u32_at(bytes, *offset)?,
            frame_index: u32_at(bytes, *offset + 4)?,
            value: [
                f32_at(bytes, *offset + 8)?,
                f32_at(bytes, *offset + 12)?,
                f32_at(bytes, *offset + 16)?,
                f32_at(bytes, *offset + 20)?,
            ],
        });
        *offset += 24;
    }
    Ok(values)
}

fn parse_track_36(
    bytes: &[u8],
    offset: &mut usize,
    count: u32,
) -> Result<Vec<Keyframe36>, MotionError> {
    let mut values = Vec::with_capacity(count as usize);
    for _ in 0..count {
        require(bytes, *offset, 36)?;
        values.push(Keyframe36 {
            tick: u32_at(bytes, *offset)?,
            frame_index: u32_at(bytes, *offset + 4)?,
            value: [
                f32_at(bytes, *offset + 8)?,
                f32_at(bytes, *offset + 12)?,
                f32_at(bytes, *offset + 16)?,
                f32_at(bytes, *offset + 20)?,
                f32_at(bytes, *offset + 24)?,
                f32_at(bytes, *offset + 28)?,
                f32_at(bytes, *offset + 32)?,
            ],
        });
        *offset += 36;
    }
    Ok(values)
}

fn c_string(bytes: &[u8], offset: usize, size: usize) -> Result<String, MotionError> {
    require(bytes, offset, size)?;
    let field = &bytes[offset..offset + size];
    let end = field.iter().position(|byte| *byte == 0).unwrap_or(size);
    Ok(String::from_utf8_lossy(&field[..end]).into_owned())
}

fn u32_at(bytes: &[u8], offset: usize) -> Result<u32, MotionError> {
    require(bytes, offset, 4)?;
    Ok(u32::from_le_bytes(
        bytes[offset..offset + 4].try_into().unwrap(),
    ))
}

fn f32_at(bytes: &[u8], offset: usize) -> Result<f32, MotionError> {
    Ok(f32::from_bits(u32_at(bytes, offset)?))
}

fn usize_at(bytes: &[u8], offset: usize) -> Result<usize, MotionError> {
    usize::try_from(u32_at(bytes, offset)?)
        .map_err(|_| error(offset, "value does not fit in memory"))
}

fn checked_add(left: usize, right: usize, offset: usize) -> Result<usize, MotionError> {
    left.checked_add(right)
        .ok_or_else(|| error(offset, "offset overflow"))
}

fn require(bytes: &[u8], offset: usize, size: usize) -> Result<(), MotionError> {
    if offset.checked_add(size).is_none_or(|end| end > bytes.len()) {
        return Err(error(offset, format!("truncated {size}-byte field")));
    }
    Ok(())
}

fn error(offset: usize, message: impl Into<String>) -> MotionError {
    MotionError {
        offset,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn motion(first: u32, last: u32, speed: u32) -> MotionFile {
        MotionFile {
            version: 1,
            ticks_per_frame: 160,
            first_frame: first,
            last_frame: last,
            frame_speed: speed,
            field_14: 0,
            field_18: 0,
            duration_ticks: 0,
            name: String::new(),
            records: Vec::new(),
        }
    }

    #[test]
    fn a_motion_played_once_stops_on_its_last_frame() {
        let attack = motion(10, 29, 20); // 20 quadros a 20 por segundo: 1 s
        assert_eq!(attack.frame_clamped_at(0.0), 10.0);
        assert_eq!(attack.frame_clamped_at(0.5), 20.0);
        assert_eq!(attack.frame_clamped_at(1.0), 29.0);
        assert_eq!(
            attack.frame_clamped_at(9.0),
            29.0,
            "holds instead of looping"
        );
        assert_eq!(attack.frame_at(1.5), 20.0, "the looping variant wraps");
    }

    #[test]
    fn frame_numbers_convert_to_seconds() {
        let attack = motion(10, 29, 20);
        assert_eq!(attack.seconds_at_frame(20), 0.5);
        assert_eq!(attack.seconds_at_frame(10), 0.0);
        assert_eq!(attack.seconds_at_frame(3), 0.0, "before the first frame");
    }

    #[test]
    fn parses_an_empty_motion_record() {
        let mut bytes = vec![0_u8; HEADER_SIZE];
        bytes[0..4].copy_from_slice(&1_u32.to_le_bytes());
        bytes[4..8].copy_from_slice(&160_u32.to_le_bytes());
        bytes[12..16].copy_from_slice(&4_u32.to_le_bytes());
        bytes[32..38].copy_from_slice(b"motion");
        bytes.extend_from_slice(&RECORD_TAG.to_le_bytes());
        bytes.extend_from_slice(&0x98_u32.to_le_bytes());
        bytes.extend_from_slice(&[0_u8; 0x98]);

        let motion = MotionFile::parse(&bytes).unwrap();
        assert_eq!(motion.name, "motion");
        assert_eq!(motion.records.len(), 1);
    }
}

#[cfg(test)]
mod sampling_tests {
    use super::*;

    fn record_with(track_24: Vec<Keyframe24>, track_20: Vec<Keyframe20>) -> MotionRecord {
        MotionRecord {
            offset: 0,
            payload_size: 0,
            counters: [0; 5],
            candidate_name: String::new(),
            track_24,
            track_20,
            track_36: Vec::new(),
            morph_key_count: 0,
            morph_bytes: 0,
        }
    }

    #[test]
    fn positions_interpolate_and_hold_at_the_ends() {
        let record = record_with(
            Vec::new(),
            vec![
                Keyframe20 {
                    tick: 0,
                    frame_index: 0,
                    value: [0.0, 0.0, 0.0],
                },
                Keyframe20 {
                    tick: 0,
                    frame_index: 10,
                    value: [10.0, 20.0, 0.0],
                },
            ],
        );
        assert_eq!(record.position_at(5.0), Some([5.0, 10.0, 0.0]));
        assert_eq!(record.position_at(-3.0), Some([0.0, 0.0, 0.0]));
        assert_eq!(record.position_at(99.0), Some([10.0, 20.0, 0.0]));
    }

    #[test]
    fn rotations_take_the_short_way_and_stay_unit_length() {
        let half = std::f32::consts::FRAC_1_SQRT_2;
        let record = record_with(
            vec![
                Keyframe24 {
                    tick: 0,
                    frame_index: 0,
                    value: [0.0, 0.0, 0.0, 1.0],
                },
                // The same rotation as a quarter turn about Y, written with the opposite sign.
                Keyframe24 {
                    tick: 0,
                    frame_index: 2,
                    value: [0.0, -half, 0.0, -half],
                },
            ],
            Vec::new(),
        );
        let middle = record.rotation_at(1.0).expect("track has keys");
        let length: f32 = middle.iter().map(|v| v * v).sum::<f32>().sqrt();
        assert!((length - 1.0).abs() < 1e-5);
        // Halfway between identity and a 90 degree turn is a 45 degree turn (w = cos 22.5).
        assert!((middle[3].abs() - 0.9239).abs() < 1e-3, "{middle:?}");
    }

    #[test]
    fn a_motion_loops_by_time() {
        let file = MotionFile {
            version: 1,
            ticks_per_frame: 200,
            first_frame: 0,
            last_frame: 59,
            frame_speed: 24,
            field_14: 0,
            field_18: 0,
            duration_ticks: 12_000,
            name: String::new(),
            records: Vec::new(),
        };
        assert_eq!(file.frame_count(), 60);
        assert!((file.duration_seconds() - 2.5).abs() < 1e-6);
        assert!((file.frame_at(0.5) - 12.0).abs() < 1e-4);
        assert!((file.frame_at(2.5 + 0.5) - 12.0).abs() < 1e-3);
    }
}

#[cfg(test)]
mod layout_tests {
    use super::*;

    fn record(track_20_count: u32, track_24_count: u32, body: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0_u8; 8];
        bytes[..4].copy_from_slice(&RECORD_TAG.to_le_bytes());
        let mut payload = vec![0_u8; 0x98];
        payload[4..8].copy_from_slice(&track_24_count.to_le_bytes());
        payload[8..12].copy_from_slice(&track_20_count.to_le_bytes());
        payload[0x14..0x14 + 4].copy_from_slice(b"Bone");
        payload.extend_from_slice(body);
        bytes[4..8].copy_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes.extend(payload);
        bytes
    }

    #[test]
    fn position_keys_precede_rotation_keys() {
        let mut body = Vec::new();
        // one 20-byte position key, then one 24-byte rotation key
        for value in [200_u32, 0] {
            body.extend_from_slice(&value.to_le_bytes());
        }
        for value in [1.0_f32, 2.0, 3.0] {
            body.extend_from_slice(&value.to_le_bytes());
        }
        for value in [200_u32, 0] {
            body.extend_from_slice(&value.to_le_bytes());
        }
        for value in [0.0_f32, 0.0, 0.0, 1.0] {
            body.extend_from_slice(&value.to_le_bytes());
        }
        let mut file = vec![0_u8; HEADER_SIZE];
        file[..4].copy_from_slice(&1_u32.to_le_bytes());
        file.extend(record(1, 1, &body));
        let motion = MotionFile::parse(&file).expect("motion should parse");
        let record = &motion.records[0];
        assert_eq!(record.track_20[0].value, [1.0, 2.0, 3.0]);
        assert_eq!(record.track_24[0].value, [0.0, 0.0, 0.0, 1.0]);
    }
}
