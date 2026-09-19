//! Decodificador de TGA (imagens da interface: `menu_1.tga`…), até RGBA de 8 bits, de cima para baixo.
//!
//! **[confirmado]** Os 192 `.tga` do pacote `UI` são TGA de cor verdadeira sem compressão (tipo 2), 24 bits
//! (sem canal alfa), 256 × 256, com a origem embaixo à esquerda (bit 5 do descritor desligado): as linhas
//! vêm de baixo para cima e são invertidas aqui. Também aceita o tipo 10 (RLE), 32 bits e origem no topo.

use crate::dds::DecodedImage;
use std::fmt;

const HEADER_SIZE: usize = 18;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TgaError(String);

impl fmt::Display for TgaError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid TGA: {}", self.0)
    }
}

impl std::error::Error for TgaError {}

impl DecodedImage {
    /// Decodifica um TGA de cor verdadeira (tipo 2 ou 10, 24 ou 32 bits). Sem canal alfa (24 bits) a
    /// imagem sai opaca.
    pub fn from_tga(bytes: &[u8]) -> Result<Self, TgaError> {
        if bytes.len() < HEADER_SIZE {
            return Err(TgaError("smaller than the 18-byte header".to_owned()));
        }
        let id_length = usize::from(bytes[0]);
        let color_map_type = bytes[1];
        let image_type = bytes[2];
        let width = u32::from(u16::from_le_bytes([bytes[12], bytes[13]]));
        let height = u32::from(u16::from_le_bytes([bytes[14], bytes[15]]));
        let bits = bytes[16];
        let top_origin = bytes[17] & 0x20 != 0;
        if color_map_type != 0 || !matches!(image_type, 2 | 10) {
            return Err(TgaError(format!(
                "only true-colour images are supported (type {image_type}, colour map {color_map_type})"
            )));
        }
        if !matches!(bits, 24 | 32) || width == 0 || height == 0 {
            return Err(TgaError(format!(
                "unsupported {bits}-bit {width}x{height} image"
            )));
        }
        let bytes_per_pixel = usize::from(bits / 8);
        let pixel_count = width as usize * height as usize;
        let data = bytes
            .get(HEADER_SIZE + id_length..)
            .ok_or_else(|| TgaError("the image id runs past the end".to_owned()))?;

        // Píxeis brutos (BGR ou BGRA) na ordem do arquivo.
        let raw: Vec<u8> = if image_type == 2 {
            data.get(..pixel_count * bytes_per_pixel)
                .ok_or_else(|| TgaError("pixel data is truncated".to_owned()))?
                .to_vec()
        } else {
            let mut raw = Vec::with_capacity(pixel_count * bytes_per_pixel);
            let mut at = 0;
            while raw.len() < pixel_count * bytes_per_pixel {
                let header = *data
                    .get(at)
                    .ok_or_else(|| TgaError("RLE data is truncated".to_owned()))?;
                at += 1;
                let count = usize::from(header & 0x7F) + 1;
                if header & 0x80 != 0 {
                    let pixel = data
                        .get(at..at + bytes_per_pixel)
                        .ok_or_else(|| TgaError("RLE run is truncated".to_owned()))?;
                    at += bytes_per_pixel;
                    for _ in 0..count {
                        raw.extend_from_slice(pixel);
                    }
                } else {
                    let span = count * bytes_per_pixel;
                    raw.extend_from_slice(
                        data.get(at..at + span)
                            .ok_or_else(|| TgaError("RLE literal is truncated".to_owned()))?,
                    );
                    at += span;
                }
            }
            raw.truncate(pixel_count * bytes_per_pixel);
            raw
        };

        let row_bytes = width as usize * bytes_per_pixel;
        let mut rgba = Vec::with_capacity(pixel_count * 4);
        for row in 0..height as usize {
            let source_row = if top_origin {
                row
            } else {
                height as usize - 1 - row
            };
            let line = &raw[source_row * row_bytes..(source_row + 1) * row_bytes];
            for pixel in line.chunks_exact(bytes_per_pixel) {
                rgba.extend_from_slice(&[
                    pixel[2],
                    pixel[1],
                    pixel[0],
                    if bytes_per_pixel == 4 { pixel[3] } else { 255 },
                ]);
            }
        }
        Ok(Self {
            width,
            height,
            rgba,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(image_type: u8, width: u16, height: u16, bits: u8, descriptor: u8) -> Vec<u8> {
        let mut bytes = vec![0, 0, image_type, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        bytes.extend(width.to_le_bytes());
        bytes.extend(height.to_le_bytes());
        bytes.extend([bits, descriptor]);
        bytes
    }

    #[test]
    fn bottom_origin_rows_are_flipped_and_bgr_becomes_rgba() {
        // 1 × 2: a linha de baixo (primeira no arquivo) é azul, a de cima é vermelha.
        let mut file = header(2, 1, 2, 24, 0);
        file.extend([255, 0, 0]); // B G R do azul
        file.extend([0, 0, 255]); // vermelho
        let image = DecodedImage::from_tga(&file).unwrap();
        assert_eq!(image.rgba, [255, 0, 0, 255, 0, 0, 255, 255]);
    }

    #[test]
    fn top_origin_rows_keep_their_order_and_alpha_is_read() {
        let mut file = header(2, 1, 2, 32, 0x28);
        file.extend([1, 2, 3, 4]);
        file.extend([5, 6, 7, 8]);
        let image = DecodedImage::from_tga(&file).unwrap();
        assert_eq!(image.rgba, [3, 2, 1, 4, 7, 6, 5, 8]);
    }

    #[test]
    fn rle_runs_and_literals_expand() {
        let mut file = header(10, 4, 1, 24, 0x20);
        file.extend([0x81, 9, 8, 7]); // 2 × (B=9, G=8, R=7)
        file.extend([0x01, 1, 2, 3, 4, 5, 6]); // 2 literais
        let image = DecodedImage::from_tga(&file).unwrap();
        assert_eq!(
            image.rgba,
            [7, 8, 9, 255, 7, 8, 9, 255, 3, 2, 1, 255, 6, 5, 4, 255]
        );
    }

    #[test]
    fn unsupported_and_truncated_files_are_rejected() {
        assert!(DecodedImage::from_tga(&[0; 5]).is_err());
        assert!(
            DecodedImage::from_tga(&header(1, 1, 1, 8, 0)).is_err(),
            "colour-mapped"
        );
        let mut short = header(2, 2, 2, 24, 0);
        short.extend([0; 5]);
        assert!(DecodedImage::from_tga(&short).is_err());
    }
}
