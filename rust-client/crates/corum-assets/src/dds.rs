use std::fmt;

const MAGIC: &[u8; 4] = b"DDS ";
const HEADER_SIZE: usize = 128;
const PIXEL_FORMAT_OFFSET: usize = 76;
const MAX_DIMENSION: u32 = 8_192;

const DDPF_ALPHAPIXELS: u32 = 0x1;
const DDPF_FOURCC: u32 = 0x4;
const DDPF_RGB: u32 = 0x40;

/// Top mip level of a DDS texture, decoded to 8-bit RGBA.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl DecodedImage {
    /// Decodes the largest mip level of a DDS file (DXT1/3/5 or uncompressed RGB/RGBA).
    pub fn from_dds(bytes: &[u8]) -> Result<Self, DdsError> {
        if bytes.len() < HEADER_SIZE || &bytes[..4] != MAGIC {
            return Err(DdsError::new("missing DDS header"));
        }
        let height = u32_at(bytes, 12);
        let width = u32_at(bytes, 16);
        if width == 0 || height == 0 || width > MAX_DIMENSION || height > MAX_DIMENSION {
            return Err(DdsError::new(format!("unsupported size {width}x{height}")));
        }
        let flags = u32_at(bytes, PIXEL_FORMAT_OFFSET + 4);
        let four_cc = &bytes[PIXEL_FORMAT_OFFSET + 8..PIXEL_FORMAT_OFFSET + 12];
        let data = &bytes[HEADER_SIZE..];
        let pixel_count = width as usize * height as usize;

        let rgba = if flags & DDPF_FOURCC != 0 {
            let block_bytes = match four_cc {
                b"DXT1" => 8,
                b"DXT3" | b"DXT5" => 16,
                other => {
                    return Err(DdsError::new(format!(
                        "unsupported FourCC {:?}",
                        String::from_utf8_lossy(other)
                    )));
                }
            };
            decode_blocks(data, width, height, block_bytes, four_cc)?
        } else if flags & DDPF_RGB != 0 {
            decode_uncompressed(data, bytes, pixel_count, flags & DDPF_ALPHAPIXELS != 0)?
        } else {
            return Err(DdsError::new("unsupported pixel format"));
        };
        Ok(Self {
            width,
            height,
            rgba,
        })
    }
}

fn decode_uncompressed(
    data: &[u8],
    file: &[u8],
    pixel_count: usize,
    has_alpha: bool,
) -> Result<Vec<u8>, DdsError> {
    let bits = u32_at(file, PIXEL_FORMAT_OFFSET + 12);
    let masks = [
        u32_at(file, PIXEL_FORMAT_OFFSET + 16),
        u32_at(file, PIXEL_FORMAT_OFFSET + 20),
        u32_at(file, PIXEL_FORMAT_OFFSET + 24),
        u32_at(file, PIXEL_FORMAT_OFFSET + 28),
    ];
    let stride = match bits {
        24 => 3,
        32 => 4,
        other => return Err(DdsError::new(format!("unsupported {other}-bit RGB layout"))),
    };
    let needed = pixel_count
        .checked_mul(stride)
        .ok_or_else(|| DdsError::new("image size overflow"))?;
    if data.len() < needed {
        return Err(DdsError::new("pixel data is truncated"));
    }
    let mut rgba = Vec::with_capacity(pixel_count * 4);
    for pixel in data[..needed].chunks_exact(stride) {
        let mut value = 0_u32;
        for (shift, byte) in pixel.iter().enumerate() {
            value |= u32::from(*byte) << (shift * 8);
        }
        rgba.push(extract_channel(value, masks[0]));
        rgba.push(extract_channel(value, masks[1]));
        rgba.push(extract_channel(value, masks[2]));
        rgba.push(if has_alpha {
            extract_channel(value, masks[3])
        } else {
            255
        });
    }
    Ok(rgba)
}

fn extract_channel(value: u32, mask: u32) -> u8 {
    if mask == 0 {
        return 0;
    }
    let shift = mask.trailing_zeros();
    let max = mask >> shift;
    let channel = (value & mask) >> shift;
    ((channel * 255 + max / 2) / max) as u8
}

fn decode_blocks(
    data: &[u8],
    width: u32,
    height: u32,
    block_bytes: usize,
    four_cc: &[u8],
) -> Result<Vec<u8>, DdsError> {
    let blocks_x = width.div_ceil(4) as usize;
    let blocks_y = height.div_ceil(4) as usize;
    let needed = blocks_x
        .checked_mul(blocks_y)
        .and_then(|blocks| blocks.checked_mul(block_bytes))
        .ok_or_else(|| DdsError::new("image size overflow"))?;
    if data.len() < needed {
        return Err(DdsError::new("block data is truncated"));
    }

    let mut rgba = vec![0_u8; width as usize * height as usize * 4];
    for block_y in 0..blocks_y {
        for block_x in 0..blocks_x {
            let block = &data[(block_y * blocks_x + block_x) * block_bytes..][..block_bytes];
            let (alpha, color) = match four_cc {
                b"DXT3" => (Some(decode_explicit_alpha(&block[..8])), &block[8..]),
                b"DXT5" => (Some(decode_interpolated_alpha(&block[..8])), &block[8..]),
                _ => (None, block),
            };
            let colors = decode_color_block(color, alpha.is_none());
            for row in 0..4 {
                for column in 0..4 {
                    let x = block_x * 4 + column;
                    let y = block_y * 4 + row;
                    if x >= width as usize || y >= height as usize {
                        continue;
                    }
                    let mut pixel = colors[row * 4 + column];
                    if let Some(alpha) = &alpha {
                        pixel[3] = alpha[row * 4 + column];
                    }
                    let offset = (y * width as usize + x) * 4;
                    rgba[offset..offset + 4].copy_from_slice(&pixel);
                }
            }
        }
    }
    Ok(rgba)
}

fn decode_color_block(block: &[u8], punch_through: bool) -> [[u8; 4]; 16] {
    let color0 = u16::from_le_bytes([block[0], block[1]]);
    let color1 = u16::from_le_bytes([block[2], block[3]]);
    let c0 = rgb565(color0);
    let c1 = rgb565(color1);
    let mut palette = [[0_u8; 4]; 4];
    palette[0] = [c0[0], c0[1], c0[2], 255];
    palette[1] = [c1[0], c1[1], c1[2], 255];
    if color0 > color1 || !punch_through {
        for channel in 0..3 {
            palette[2][channel] = ((2 * u16::from(c0[channel]) + u16::from(c1[channel])) / 3) as u8;
            palette[3][channel] = ((u16::from(c0[channel]) + 2 * u16::from(c1[channel])) / 3) as u8;
        }
        palette[2][3] = 255;
        palette[3][3] = 255;
    } else {
        for channel in 0..3 {
            palette[2][channel] = ((u16::from(c0[channel]) + u16::from(c1[channel])) / 2) as u8;
        }
        palette[2][3] = 255;
        palette[3] = [0, 0, 0, 0];
    }

    let indices = u32::from_le_bytes([block[4], block[5], block[6], block[7]]);
    let mut colors = [[0_u8; 4]; 16];
    for (pixel, color) in colors.iter_mut().enumerate() {
        *color = palette[((indices >> (pixel * 2)) & 0b11) as usize];
    }
    colors
}

fn rgb565(value: u16) -> [u8; 3] {
    let r = u32::from((value >> 11) & 0x1f);
    let g = u32::from((value >> 5) & 0x3f);
    let b = u32::from(value & 0x1f);
    [
        ((r * 255 + 15) / 31) as u8,
        ((g * 255 + 31) / 63) as u8,
        ((b * 255 + 15) / 31) as u8,
    ]
}

fn decode_explicit_alpha(block: &[u8]) -> [u8; 16] {
    let mut alpha = [0_u8; 16];
    for (pixel, value) in alpha.iter_mut().enumerate() {
        let nibble = (block[pixel / 2] >> ((pixel % 2) * 4)) & 0xf;
        *value = nibble * 17;
    }
    alpha
}

fn decode_interpolated_alpha(block: &[u8]) -> [u8; 16] {
    let a0 = u16::from(block[0]);
    let a1 = u16::from(block[1]);
    let mut palette = [0_u8; 8];
    palette[0] = a0 as u8;
    palette[1] = a1 as u8;
    if a0 > a1 {
        for step in 1..7 {
            palette[step + 1] = (((7 - step as u16) * a0 + step as u16 * a1) / 7) as u8;
        }
    } else {
        for step in 1..5 {
            palette[step + 1] = (((5 - step as u16) * a0 + step as u16 * a1) / 5) as u8;
        }
        palette[6] = 0;
        palette[7] = 255;
    }

    let mut bits = 0_u64;
    for (index, byte) in block[2..8].iter().enumerate() {
        bits |= u64::from(*byte) << (index * 8);
    }
    let mut alpha = [0_u8; 16];
    for (pixel, value) in alpha.iter_mut().enumerate() {
        *value = palette[((bits >> (pixel * 3)) & 0b111) as usize];
    }
    alpha
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
pub struct DdsError {
    message: String,
}

impl DdsError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for DdsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid DDS: {}", self.message)
    }
}

impl std::error::Error for DdsError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(width: u32, height: u32, flags: u32, four_cc: &[u8; 4]) -> Vec<u8> {
        let mut bytes = vec![0_u8; HEADER_SIZE];
        bytes[..4].copy_from_slice(MAGIC);
        bytes[12..16].copy_from_slice(&height.to_le_bytes());
        bytes[16..20].copy_from_slice(&width.to_le_bytes());
        bytes[PIXEL_FORMAT_OFFSET + 4..PIXEL_FORMAT_OFFSET + 8]
            .copy_from_slice(&flags.to_le_bytes());
        bytes[PIXEL_FORMAT_OFFSET + 8..PIXEL_FORMAT_OFFSET + 12].copy_from_slice(four_cc);
        bytes
    }

    #[test]
    fn decodes_a_solid_dxt1_block() {
        let mut bytes = header(4, 4, DDPF_FOURCC, b"DXT1");
        // color0 = pure red (0xF800), color1 = black, every index selects color0.
        bytes.extend_from_slice(&[0x00, 0xf8, 0x00, 0x00, 0, 0, 0, 0]);
        let image = DecodedImage::from_dds(&bytes).expect("DXT1 block should decode");
        assert_eq!((image.width, image.height), (4, 4));
        assert!(
            image
                .rgba
                .as_chunks::<4>()
                .0
                .iter()
                .all(|p| *p == [255, 0, 0, 255])
        );
    }

    #[test]
    fn dxt1_punch_through_uses_transparent_index_three() {
        let mut bytes = header(4, 4, DDPF_FOURCC, b"DXT1");
        // color0 <= color1 enables the 1-bit alpha mode; every index selects entry 3.
        bytes.extend_from_slice(&[0x00, 0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]);
        let image = DecodedImage::from_dds(&bytes).expect("DXT1 block should decode");
        assert!(image.rgba.as_chunks::<4>().0.iter().all(|p| p[3] == 0));
    }

    #[test]
    fn decodes_dxt5_alpha_and_crops_partial_blocks() {
        let mut bytes = header(2, 2, DDPF_FOURCC, b"DXT5");
        bytes.extend_from_slice(&[200, 100, 0, 0, 0, 0, 0, 0]);
        bytes.extend_from_slice(&[0x00, 0xf8, 0x00, 0xf8, 0, 0, 0, 0]);
        let image = DecodedImage::from_dds(&bytes).expect("DXT5 block should decode");
        assert_eq!(image.rgba.len(), 2 * 2 * 4);
        assert!(
            image
                .rgba
                .as_chunks::<4>()
                .0
                .iter()
                .all(|p| *p == [255, 0, 0, 200])
        );
    }

    #[test]
    fn decodes_uncompressed_bgra() {
        let mut bytes = header(1, 1, DDPF_RGB | DDPF_ALPHAPIXELS, b"\0\0\0\0");
        bytes[PIXEL_FORMAT_OFFSET + 12..PIXEL_FORMAT_OFFSET + 16]
            .copy_from_slice(&32_u32.to_le_bytes());
        for (slot, mask) in [0xff0000_u32, 0xff00, 0xff, 0xff00_0000]
            .into_iter()
            .enumerate()
        {
            let offset = PIXEL_FORMAT_OFFSET + 16 + slot * 4;
            bytes[offset..offset + 4].copy_from_slice(&mask.to_le_bytes());
        }
        bytes.extend_from_slice(&[10, 20, 30, 40]); // B, G, R, A
        let image = DecodedImage::from_dds(&bytes).expect("BGRA should decode");
        assert_eq!(image.rgba, vec![30, 20, 10, 40]);
    }

    #[test]
    fn rejects_truncated_and_oversized_files() {
        assert!(DecodedImage::from_dds(b"DDS ").is_err());
        let bytes = header(64, 64, DDPF_FOURCC, b"DXT1");
        assert!(DecodedImage::from_dds(&bytes).is_err());
        let bytes = header(1 << 20, 4, DDPF_FOURCC, b"DXT1");
        assert!(DecodedImage::from_dds(&bytes).is_err());
    }
}
