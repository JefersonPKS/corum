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
            let track_24 = parse_track_24(bytes, &mut track_offset, counters[1])?;
            let track_20 = parse_track_20(bytes, &mut track_offset, counters[2])?;
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
