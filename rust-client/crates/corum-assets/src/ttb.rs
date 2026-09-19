use std::fmt;

const HEADER_SIZE: usize = 16;
const MAX_TILE_COUNT: usize = 16_777_216;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileAttribute {
    pub raw: u16,
    pub attribute: u8,
    pub occupied: u8,
    pub section: u8,
}

impl TileAttribute {
    #[must_use]
    pub fn from_raw(raw: u16) -> Self {
        Self {
            raw,
            attribute: (raw & 0x0f) as u8,
            occupied: ((raw >> 4) & 0x0f) as u8,
            section: (raw >> 8) as u8,
        }
    }

    #[must_use]
    pub fn is_walkable(self) -> bool {
        self.attribute != 1 && self.occupied == 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TileMap {
    pub width: u32,
    pub height: u32,
    pub tile_size: u32,
    pub declared_object_count: u32,
    pub tiles: Vec<TileAttribute>,
    pub declared_section_count: Option<u16>,
    pub trailing_bytes: usize,
}

impl TileMap {
    pub fn parse(bytes: &[u8]) -> Result<Self, TtbError> {
        if bytes.len() < HEADER_SIZE {
            return Err(TtbError::new(
                0,
                "file is smaller than the 16-byte TTB header",
            ));
        }

        let width = u32_at(bytes, 0)?;
        let height = u32_at(bytes, 4)?;
        let tile_size = u32_at(bytes, 8)?;
        let declared_object_count = u32_at(bytes, 12)?;
        if width == 0 || height == 0 {
            return Err(TtbError::new(0, "map dimensions must not be zero"));
        }
        if tile_size == 0 {
            return Err(TtbError::new(8, "tile size must not be zero"));
        }

        let tile_count = usize::try_from(width)
            .ok()
            .and_then(|width| {
                usize::try_from(height)
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .ok_or_else(|| TtbError::new(0, "map dimensions overflow the host address space"))?;
        if tile_count > MAX_TILE_COUNT {
            return Err(TtbError::new(
                0,
                format!("map declares {tile_count} tiles, above the safety limit"),
            ));
        }

        let tile_bytes = tile_count
            .checked_mul(2)
            .ok_or_else(|| TtbError::new(HEADER_SIZE, "tile byte count overflow"))?;
        let tiles_end = HEADER_SIZE
            .checked_add(tile_bytes)
            .ok_or_else(|| TtbError::new(HEADER_SIZE, "tile range overflow"))?;
        if bytes.len() < tiles_end {
            return Err(TtbError::new(
                bytes.len(),
                format!("tile array ends early: expected at least {tiles_end} bytes"),
            ));
        }

        let (tile_chunks, remainder) = bytes[HEADER_SIZE..tiles_end].as_chunks::<2>();
        debug_assert!(remainder.is_empty());
        let tiles = tile_chunks
            .iter()
            .map(|chunk| TileAttribute::from_raw(u16::from_le_bytes(*chunk)))
            .collect();
        let declared_section_count = bytes
            .get(tiles_end..tiles_end + 2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]));

        Ok(Self {
            width,
            height,
            tile_size,
            declared_object_count,
            tiles,
            declared_section_count,
            trailing_bytes: bytes.len() - tiles_end,
        })
    }

    #[must_use]
    pub fn tile(&self, x: u32, z: u32) -> Option<TileAttribute> {
        if x >= self.width || z >= self.height {
            return None;
        }
        let index = usize::try_from(z.checked_mul(self.width)?.checked_add(x)?).ok()?;
        self.tiles.get(index).copied()
    }

    #[must_use]
    pub fn is_walkable(&self, x: u32, z: u32) -> bool {
        self.tile(x, z).is_some_and(TileAttribute::is_walkable)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TtbError {
    pub offset: usize,
    pub message: String,
}

impl TtbError {
    fn new(offset: usize, message: impl Into<String>) -> Self {
        Self {
            offset,
            message: message.into(),
        }
    }
}

impl fmt::Display for TtbError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid TTB at offset 0x{:X}: {}",
            self.offset, self.message
        )
    }
}

impl std::error::Error for TtbError {}

fn u32_at(bytes: &[u8], offset: usize) -> Result<u32, TtbError> {
    let value = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| TtbError::new(offset, "missing 32-bit value"))?;
    Ok(u32::from_le_bytes([value[0], value[1], value[2], value[3]]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_header_tiles_and_sections() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&2_u32.to_le_bytes());
        bytes.extend_from_slice(&2_u32.to_le_bytes());
        bytes.extend_from_slice(&125_u32.to_le_bytes());
        bytes.extend_from_slice(&3_u32.to_le_bytes());
        bytes.extend_from_slice(&0x0000_u16.to_le_bytes());
        bytes.extend_from_slice(&0x0001_u16.to_le_bytes());
        bytes.extend_from_slice(&0x0319_u16.to_le_bytes());
        bytes.extend_from_slice(&0x0200_u16.to_le_bytes());
        bytes.extend_from_slice(&7_u16.to_le_bytes());

        let map = TileMap::parse(&bytes).expect("synthetic TTB should parse");
        assert_eq!((map.width, map.height, map.tile_size), (2, 2, 125));
        assert!(map.is_walkable(0, 0));
        assert!(!map.is_walkable(1, 0));
        assert_eq!(map.tile(0, 1).expect("tile").attribute, 9);
        assert_eq!(map.tile(0, 1).expect("tile").occupied, 1);
        assert_eq!(map.tile(0, 1).expect("tile").section, 3);
        assert_eq!(map.declared_section_count, Some(7));
    }

    #[test]
    fn rejects_truncated_tile_array() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&100_u32.to_le_bytes());
        bytes.extend_from_slice(&100_u32.to_le_bytes());
        bytes.extend_from_slice(&125_u32.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        assert!(TileMap::parse(&bytes).is_err());
    }
}
