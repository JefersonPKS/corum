use std::fmt;

const RECORD_HEADER_SIZE: usize = 12;
const MAX_DIMENSION: u32 = 4_096;

/// Baked lightmaps (`.lm`): one texture per lightmapped (type 3) object of the matching
/// `.stm`, in the same order as those objects appear in the scene file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LightmapFile {
    pub maps: Vec<Lightmap>,
}

/// One lightmap. The `.stm` object repeats `(first_field, width, height)` in its trailing
/// data, which lets the pairing be checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lightmap {
    /// Leading `u32` of the record header: 1 for most maps, 6 and 38 for the two atlases
    /// of the `1100` scene. Its meaning is unknown.
    pub first_field: u32,
    pub width: u32,
    pub height: u32,
    /// `width * height` RGBA texels, alpha always 255.
    pub rgba: Vec<u8>,
}

impl LightmapFile {
    /// Decodes the whole file. It must end exactly after the last record; an empty file
    /// (some maps ship a zero-byte `.lm`) is valid and holds no lightmaps.
    ///
    /// Each record is `first_field: u32, width: u32, height: u32` followed by
    /// `width * height` little-endian RGB565 texels.
    pub fn parse(bytes: &[u8]) -> Result<Self, LightmapError> {
        let mut maps = Vec::new();
        let mut offset = 0_usize;
        while offset < bytes.len() {
            let header_end = offset
                .checked_add(RECORD_HEADER_SIZE)
                .filter(|end| *end <= bytes.len())
                .ok_or_else(|| LightmapError::new(offset, "truncated record header"))?;
            let first_field = u32_at(bytes, offset);
            let width = u32_at(bytes, offset + 4);
            let height = u32_at(bytes, offset + 8);
            if width == 0 || height == 0 || width > MAX_DIMENSION || height > MAX_DIMENSION {
                return Err(LightmapError::new(
                    offset,
                    format!("unsupported lightmap size {width}x{height}"),
                ));
            }
            let texel_count = width as usize * height as usize;
            let end = texel_count
                .checked_mul(2)
                .and_then(|size| header_end.checked_add(size))
                .filter(|end| *end <= bytes.len())
                .ok_or_else(|| {
                    LightmapError::new(header_end, "pixel data runs past the end of the file")
                })?;

            let mut rgba = Vec::with_capacity(texel_count * 4);
            for texel in bytes[header_end..end].as_chunks::<2>().0 {
                let [red, green, blue] = rgb565(u16::from_le_bytes(*texel));
                rgba.extend_from_slice(&[red, green, blue, 255]);
            }
            maps.push(Lightmap {
                first_field,
                width,
                height,
                rgba,
            });
            offset = end;
        }
        Ok(Self { maps })
    }
}

fn rgb565(value: u16) -> [u8; 3] {
    let red = u32::from((value >> 11) & 0x1f);
    let green = u32::from((value >> 5) & 0x3f);
    let blue = u32::from(value & 0x1f);
    [
        ((red * 255 + 15) / 31) as u8,
        ((green * 255 + 31) / 63) as u8,
        ((blue * 255 + 15) / 31) as u8,
    ]
}

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LightmapError {
    pub offset: usize,
    pub message: String,
}

impl LightmapError {
    fn new(offset: usize, message: impl Into<String>) -> Self {
        Self {
            offset,
            message: message.into(),
        }
    }
}

impl fmt::Display for LightmapError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid LM at offset 0x{:X}: {}",
            self.offset, self.message
        )
    }
}

impl std::error::Error for LightmapError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(first: u32, width: u32, height: u32, texels: &[u16]) -> Vec<u8> {
        let mut bytes = Vec::new();
        for value in [first, width, height] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        for texel in texels {
            bytes.extend_from_slice(&texel.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn decodes_consecutive_records_of_different_sizes() {
        let mut bytes = record(1, 2, 1, &[0xf800, 0x001f]);
        bytes.extend(record(6, 1, 2, &[0x07e0, 0xffff]));
        let file = LightmapFile::parse(&bytes).expect("two records should parse");
        assert_eq!(file.maps.len(), 2);
        assert_eq!(file.maps[0].first_field, 1);
        assert_eq!(
            file.maps[0].rgba,
            vec![255, 0, 0, 255, 0, 0, 255, 255],
            "RGB565 red then blue"
        );
        assert_eq!((file.maps[1].width, file.maps[1].height), (1, 2));
        assert_eq!(&file.maps[1].rgba[4..], &[255, 255, 255, 255]);
    }

    #[test]
    fn maps_the_common_grey_texel_to_the_baseline_grey() {
        assert_eq!(rgb565(0x4228), [66, 69, 66]);
    }

    #[test]
    fn empty_files_hold_no_lightmaps() {
        assert!(
            LightmapFile::parse(&[])
                .expect("empty is valid")
                .maps
                .is_empty()
        );
    }

    #[test]
    fn rejects_truncated_and_oversized_records() {
        let mut bytes = record(1, 4, 4, &[0; 16]);
        bytes.pop();
        assert!(LightmapFile::parse(&bytes).is_err());
        assert!(LightmapFile::parse(&[1, 0, 0, 0, 4, 0]).is_err());
        assert!(LightmapFile::parse(&record(1, 1 << 20, 1, &[])).is_err());
        assert!(LightmapFile::parse(&record(1, 0, 4, &[])).is_err());
    }
}
