use std::fmt;

use crate::dds::DecodedImage;

const MAX_DIMENSION: u32 = 8_192;
const MAX_VALUES: u32 = 1 << 20;

const TAG_WIDTH: u16 = 256;
const TAG_HEIGHT: u16 = 257;
const TAG_BITS_PER_SAMPLE: u16 = 258;
const TAG_COMPRESSION: u16 = 259;
const TAG_PHOTOMETRIC: u16 = 262;
const TAG_STRIP_OFFSETS: u16 = 273;
const TAG_SAMPLES_PER_PIXEL: u16 = 277;
const TAG_STRIP_BYTE_COUNTS: u16 = 279;
const TAG_PLANAR: u16 = 284;

const TYPE_SHORT: u16 = 3;
const TYPE_LONG: u16 = 4;

impl DecodedImage {
    /// Decodes the uncompressed 8-bit RGB/RGBA TIFFs the map packages use (`Map_tif.pak`).
    ///
    /// Only little-endian files with no compression, RGB photometric and chunky samples are
    /// accepted. About a third of the shipped files have `0xCCCC` where the TIFF magic
    /// `0x002A` belongs; the magic is not checked because the rest of the file is standard.
    /// A fourth sample is read as alpha; a missing one becomes fully opaque.
    pub fn from_tiff(bytes: &[u8]) -> Result<Self, TiffError> {
        if bytes.len() < 8 || &bytes[..2] != b"II" {
            return Err(TiffError::new("not a little-endian TIFF"));
        }
        let ifd = u32_at(bytes, 4)? as usize;
        let entry_count = usize::from(u16_at(bytes, ifd)?);

        let mut tags = Vec::with_capacity(entry_count);
        for index in 0..entry_count {
            let entry = ifd + 2 + index * 12;
            tags.push(Entry {
                tag: u16_at(bytes, entry)?,
                kind: u16_at(bytes, entry + 2)?,
                count: u32_at(bytes, entry + 4)?,
                value_offset: entry + 8,
            });
        }
        let find = |tag: u16| tags.iter().find(|entry| entry.tag == tag);
        let single = |tag: u16, default: Option<u32>| -> Result<u32, TiffError> {
            match find(tag) {
                Some(entry) => entry
                    .values(bytes)?
                    .first()
                    .copied()
                    .ok_or_else(|| TiffError::new(format!("tag {tag} has no value"))),
                None => default.ok_or_else(|| TiffError::new(format!("missing tag {tag}"))),
            }
        };

        let width = single(TAG_WIDTH, None)?;
        let height = single(TAG_HEIGHT, None)?;
        if width == 0 || height == 0 || width > MAX_DIMENSION || height > MAX_DIMENSION {
            return Err(TiffError::new(format!("unsupported size {width}x{height}")));
        }
        if single(TAG_COMPRESSION, Some(1))? != 1 {
            return Err(TiffError::new("compressed TIFFs are not supported"));
        }
        if single(TAG_PHOTOMETRIC, None)? != 2 {
            return Err(TiffError::new("only RGB photometric is supported"));
        }
        if single(TAG_PLANAR, Some(1))? != 1 {
            return Err(TiffError::new("planar sample layout is not supported"));
        }
        let samples = single(TAG_SAMPLES_PER_PIXEL, Some(1))? as usize;
        if !(3..=8).contains(&samples) {
            return Err(TiffError::new(format!("{samples} samples per pixel")));
        }
        let bits = find(TAG_BITS_PER_SAMPLE)
            .ok_or_else(|| TiffError::new("missing bits per sample"))?
            .values(bytes)?;
        if bits.iter().any(|bits| *bits != 8) {
            return Err(TiffError::new("only 8 bits per sample are supported"));
        }

        let offsets = find(TAG_STRIP_OFFSETS)
            .ok_or_else(|| TiffError::new("missing strip offsets"))?
            .values(bytes)?;
        let lengths = find(TAG_STRIP_BYTE_COUNTS)
            .ok_or_else(|| TiffError::new("missing strip byte counts"))?
            .values(bytes)?;
        if offsets.len() != lengths.len() {
            return Err(TiffError::new("strip offsets and byte counts disagree"));
        }
        let pixel_count = width as usize * height as usize;
        let needed = pixel_count
            .checked_mul(samples)
            .ok_or_else(|| TiffError::new("image size overflow"))?;
        let mut data = Vec::with_capacity(needed);
        for (offset, length) in offsets.iter().zip(&lengths) {
            let start = *offset as usize;
            let end = start
                .checked_add(*length as usize)
                .filter(|end| *end <= bytes.len())
                .ok_or_else(|| TiffError::new("strip runs past the end of the file"))?;
            data.extend_from_slice(&bytes[start..end]);
        }
        if data.len() < needed {
            return Err(TiffError::new("pixel data is truncated"));
        }

        let mut rgba = Vec::with_capacity(pixel_count * 4);
        for pixel in data[..needed].chunks_exact(samples) {
            let alpha = if samples >= 4 { pixel[3] } else { 255 };
            rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], alpha]);
        }
        Ok(Self {
            width,
            height,
            rgba,
        })
    }
}

struct Entry {
    tag: u16,
    kind: u16,
    count: u32,
    /// Offset of the 4-byte value/offset field inside the entry.
    value_offset: usize,
}

impl Entry {
    /// The entry's values, whether stored inline (up to 4 bytes) or at an offset.
    fn values(&self, bytes: &[u8]) -> Result<Vec<u32>, TiffError> {
        let size = match self.kind {
            TYPE_SHORT => 2,
            TYPE_LONG => 4,
            other => {
                return Err(TiffError::new(format!(
                    "tag {} has unsupported type {other}",
                    self.tag
                )));
            }
        };
        if self.count == 0 || self.count > MAX_VALUES {
            return Err(TiffError::new(format!(
                "tag {} has {} values",
                self.tag, self.count
            )));
        }
        let count = self.count as usize;
        let start = if count * size <= 4 {
            self.value_offset
        } else {
            u32_at(bytes, self.value_offset)? as usize
        };
        (0..count)
            .map(|index| {
                let offset = start + index * size;
                if size == 2 {
                    u16_at(bytes, offset).map(u32::from)
                } else {
                    u32_at(bytes, offset)
                }
            })
            .collect()
    }
}

fn u16_at(bytes: &[u8], offset: usize) -> Result<u16, TiffError> {
    bytes
        .get(offset..offset + 2)
        .map(|value| u16::from_le_bytes([value[0], value[1]]))
        .ok_or_else(|| TiffError::new("read past the end of the file"))
}

fn u32_at(bytes: &[u8], offset: usize) -> Result<u32, TiffError> {
    bytes
        .get(offset..offset + 4)
        .map(|value| u32::from_le_bytes([value[0], value[1], value[2], value[3]]))
        .ok_or_else(|| TiffError::new("read past the end of the file"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TiffError {
    message: String,
}

impl TiffError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for TiffError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid TIFF: {}", self.message)
    }
}

impl std::error::Error for TiffError {}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a one-strip TIFF: header, pixel data, then the directory.
    fn build(
        magic: u16,
        width: u32,
        height: u32,
        samples: u32,
        compression: u32,
        pixels: &[u8],
    ) -> Vec<u8> {
        let mut bytes = vec![b'I', b'I'];
        bytes.extend_from_slice(&magic.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes()); // directory offset, patched below
        let data_offset = bytes.len() as u32;
        bytes.extend_from_slice(pixels);
        let bits_offset = bytes.len() as u32;
        for _ in 0..samples {
            bytes.extend_from_slice(&8_u16.to_le_bytes());
        }
        let directory = bytes.len() as u32;
        bytes[4..8].copy_from_slice(&directory.to_le_bytes());

        let entries: [(u16, u16, u32, u32); 9] = [
            (TAG_WIDTH, TYPE_LONG, 1, width),
            (TAG_HEIGHT, TYPE_LONG, 1, height),
            (TAG_BITS_PER_SAMPLE, TYPE_SHORT, samples, bits_offset),
            (TAG_COMPRESSION, TYPE_SHORT, 1, compression),
            (TAG_PHOTOMETRIC, TYPE_SHORT, 1, 2),
            (TAG_STRIP_OFFSETS, TYPE_LONG, 1, data_offset),
            (TAG_SAMPLES_PER_PIXEL, TYPE_SHORT, 1, samples),
            (TAG_STRIP_BYTE_COUNTS, TYPE_LONG, 1, pixels.len() as u32),
            (TAG_PLANAR, TYPE_SHORT, 1, 1),
        ];
        bytes.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        for (tag, kind, count, value) in entries {
            bytes.extend_from_slice(&tag.to_le_bytes());
            bytes.extend_from_slice(&kind.to_le_bytes());
            bytes.extend_from_slice(&count.to_le_bytes());
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn decodes_rgba_and_keeps_the_alpha_channel() {
        let bytes = build(0x002a, 2, 1, 4, 1, &[10, 20, 30, 40, 50, 60, 70, 80]);
        let image = DecodedImage::from_tiff(&bytes).expect("RGBA TIFF should decode");
        assert_eq!((image.width, image.height), (2, 1));
        assert_eq!(image.rgba, vec![10, 20, 30, 40, 50, 60, 70, 80]);
    }

    #[test]
    fn rgb_becomes_opaque_and_the_broken_magic_is_accepted() {
        let bytes = build(0xcccc, 1, 2, 3, 1, &[1, 2, 3, 4, 5, 6]);
        let image = DecodedImage::from_tiff(&bytes).expect("RGB TIFF should decode");
        assert_eq!(image.rgba, vec![1, 2, 3, 255, 4, 5, 6, 255]);
    }

    #[test]
    fn extra_samples_beyond_alpha_are_ignored() {
        let bytes = build(0x002a, 1, 1, 5, 1, &[9, 8, 7, 6, 5]);
        let image = DecodedImage::from_tiff(&bytes).expect("5-sample TIFF should decode");
        assert_eq!(image.rgba, vec![9, 8, 7, 6]);
    }

    #[test]
    fn rejects_compression_truncation_and_bad_headers() {
        assert!(DecodedImage::from_tiff(&build(0x002a, 1, 1, 4, 5, &[0; 4])).is_err());
        assert!(DecodedImage::from_tiff(&build(0x002a, 4, 4, 4, 1, &[0; 8])).is_err());
        assert!(DecodedImage::from_tiff(b"MM\0*\0\0\0\x08").is_err());
        assert!(DecodedImage::from_tiff(b"II").is_err());
    }
}
