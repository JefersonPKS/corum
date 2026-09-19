use std::fmt;

const FILE_HEADER_SIZE: usize = 16;
const MATERIAL_SIZE: usize = 168;
const MATERIAL_TEXTURE_OFFSET: usize = 28;
const OBJECT_NAME_OFFSET: usize = 0xc0;
const OBJECT_COUNTS_OFFSET: usize = 0x140;
const OBJECT_DATA_OFFSET: usize = 0x170;
const GROUP_HEADER_SIZE: usize = 28;
const MAX_COUNT: usize = 10_000_000;

#[derive(Debug, Clone, PartialEq)]
pub struct StaticModelFile {
    pub version: u32,
    pub materials: Vec<StaticMaterial>,
    pub objects: Vec<StaticObject>,
    pub skipped_objects: Vec<SkippedStaticObject>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticMaterial {
    pub texture_name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StaticObject {
    pub name: String,
    pub object_type: u32,
    pub positions: Vec<[f32; 3]>,
    pub texture_coordinates: Vec<[f32; 2]>,
    pub groups: Vec<StaticFaceGroup>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticFaceGroup {
    pub material_index: u32,
    pub faces: Vec<[u16; 3]>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedStaticObject {
    pub name: String,
    pub object_type: u32,
}

impl StaticModelFile {
    pub fn parse(bytes: &[u8]) -> Result<Self, StmError> {
        require(bytes, 0, FILE_HEADER_SIZE)?;
        let version = u32_at(bytes, 0)?;
        if version != 1 {
            return Err(StmError::new(
                0,
                format!("unsupported STM version {version}"),
            ));
        }
        let material_count = count(bytes, 4, "material count")?;
        let materials_end = FILE_HEADER_SIZE
            .checked_add(
                material_count
                    .checked_mul(MATERIAL_SIZE)
                    .ok_or_else(|| StmError::new(4, "material table size overflow"))?,
            )
            .ok_or_else(|| StmError::new(4, "material table end overflow"))?;
        require(bytes, 0, materials_end)?;

        let mut materials = Vec::with_capacity(material_count);
        for index in 0..material_count {
            let start = FILE_HEADER_SIZE + index * MATERIAL_SIZE;
            materials.push(StaticMaterial {
                texture_name: c_string(bytes, start + MATERIAL_TEXTURE_OFFSET, 128)?,
            });
        }

        let mut objects = Vec::new();
        let mut skipped_objects = Vec::new();
        let mut cursor = materials_end;
        while is_object_header(bytes, cursor) {
            let name = c_string(bytes, cursor + OBJECT_NAME_OFFSET, 128)?;
            let counts = cursor + OBJECT_COUNTS_OFFSET;
            let vertex_count = count(bytes, counts, "vertex count")?;
            let secondary_count = count(bytes, counts + 12, "secondary index count")?;
            let group_count = count(bytes, counts + 24, "face group count")?;
            let object_type = u32_at(bytes, counts + 28)?;

            if !matches!(object_type, 1 | 3) {
                skipped_objects.push(SkippedStaticObject { name, object_type });
                cursor =
                    find_next_object(bytes, cursor + OBJECT_DATA_OFFSET).unwrap_or(bytes.len());
                continue;
            }

            let (object, end) = parse_visual_object(
                bytes,
                cursor,
                name,
                vertex_count,
                secondary_count,
                group_count,
                object_type,
            )?;
            objects.push(object);
            cursor = end;
        }

        Ok(Self {
            version,
            materials,
            objects,
            skipped_objects,
        })
    }
}

fn parse_visual_object(
    bytes: &[u8],
    object_start: usize,
    name: String,
    vertex_count: usize,
    secondary_count: usize,
    group_count: usize,
    object_type: u32,
) -> Result<(StaticObject, usize), StmError> {
    let mut cursor = object_start + OBJECT_DATA_OFFSET;
    let mut positions = Vec::with_capacity(vertex_count);
    for _ in 0..vertex_count {
        require(bytes, cursor, 12)?;
        positions.push([
            f32_at(bytes, cursor)?,
            f32_at(bytes, cursor + 4)?,
            f32_at(bytes, cursor + 8)?,
        ]);
        cursor += 12;
    }
    let mut texture_coordinates = Vec::with_capacity(vertex_count);
    for _ in 0..vertex_count {
        require(bytes, cursor, 8)?;
        texture_coordinates.push([f32_at(bytes, cursor)?, f32_at(bytes, cursor + 4)?]);
        cursor += 8;
    }

    let secondary_bytes = secondary_count
        .checked_mul(4)
        .ok_or_else(|| StmError::new(cursor, "secondary index array overflow"))?;
    require(bytes, cursor, secondary_bytes)?;
    cursor += secondary_bytes;

    let mut groups = Vec::with_capacity(group_count);
    for _ in 0..group_count {
        require(bytes, cursor, GROUP_HEADER_SIZE)?;
        let material_index = u32_at(bytes, cursor)?;
        let face_count = count(bytes, cursor + 8, "face count")?;
        let repeated_face_count = count(bytes, cursor + 12, "repeated face count")?;
        let lightmap_coordinate_count = count(bytes, cursor + 20, "lightmap coordinate count")?;
        if face_count != repeated_face_count {
            return Err(StmError::new(cursor + 12, "face count fields disagree"));
        }
        cursor += GROUP_HEADER_SIZE;
        let mut faces = Vec::with_capacity(face_count);
        for _ in 0..face_count {
            require(bytes, cursor, 6)?;
            let face = [
                u16_at(bytes, cursor)?,
                u16_at(bytes, cursor + 2)?,
                u16_at(bytes, cursor + 4)?,
            ];
            if face.iter().any(|index| usize::from(*index) >= vertex_count) {
                return Err(StmError::new(
                    cursor,
                    "face references a vertex outside the object",
                ));
            }
            faces.push(face);
            cursor += 6;
        }
        groups.push(StaticFaceGroup {
            material_index,
            faces,
        });
        if object_type == 3 {
            let lightmap_bytes = lightmap_coordinate_count
                .checked_mul(8)
                .ok_or_else(|| StmError::new(cursor, "lightmap coordinate array overflow"))?;
            require(bytes, cursor, lightmap_bytes)?;
            cursor += lightmap_bytes;
        }
    }

    if object_type == 3 {
        cursor = find_next_object(bytes, cursor).unwrap_or(bytes.len());
    } else {
        require(bytes, cursor, 16)?;
        cursor += 16;
        let vector_bytes = vertex_count
            .checked_mul(12)
            .ok_or_else(|| StmError::new(cursor, "final vector array overflow"))?;
        require(bytes, cursor, vector_bytes)?;
        cursor += vector_bytes;
    }

    Ok((
        StaticObject {
            name,
            object_type,
            positions,
            texture_coordinates,
            groups,
        },
        cursor,
    ))
}

fn is_object_header(bytes: &[u8], start: usize) -> bool {
    let Some(name_start) = start.checked_add(OBJECT_NAME_OFFSET) else {
        return false;
    };
    let Some(counts) = start.checked_add(OBJECT_COUNTS_OFFSET) else {
        return false;
    };
    if require(bytes, start, OBJECT_DATA_OFFSET).is_err()
        || u32_at(bytes, name_start.saturating_sub(4)).ok() != Some(u32::MAX)
    {
        return false;
    }
    let Ok(name) = c_string(bytes, name_start, 128) else {
        return false;
    };
    if name.len() < 3 || name.chars().any(char::is_control) {
        return false;
    }
    [0, 4, 8, 12, 16, 24]
        .into_iter()
        .all(|offset| count(bytes, counts + offset, "object count").is_ok())
}

fn find_next_object(bytes: &[u8], from: usize) -> Option<usize> {
    (from..bytes.len().saturating_sub(OBJECT_DATA_OFFSET))
        .find(|candidate| is_object_header(bytes, *candidate))
}

fn count(bytes: &[u8], offset: usize, description: &str) -> Result<usize, StmError> {
    let value = usize::try_from(u32_at(bytes, offset)?)
        .map_err(|_| StmError::new(offset, format!("{description} does not fit in memory")))?;
    if value > MAX_COUNT {
        return Err(StmError::new(
            offset,
            format!("{description} {value} exceeds the safety limit"),
        ));
    }
    Ok(value)
}

fn c_string(bytes: &[u8], offset: usize, maximum: usize) -> Result<String, StmError> {
    require(bytes, offset, maximum)?;
    let value = &bytes[offset..offset + maximum];
    let end = value.iter().position(|byte| *byte == 0).unwrap_or(maximum);
    Ok(String::from_utf8_lossy(&value[..end]).into_owned())
}

fn u16_at(bytes: &[u8], offset: usize) -> Result<u16, StmError> {
    let value = bytes
        .get(offset..offset + 2)
        .ok_or_else(|| StmError::new(offset, "missing 16-bit value"))?;
    Ok(u16::from_le_bytes([value[0], value[1]]))
}

fn u32_at(bytes: &[u8], offset: usize) -> Result<u32, StmError> {
    let value = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| StmError::new(offset, "missing 32-bit value"))?;
    Ok(u32::from_le_bytes([value[0], value[1], value[2], value[3]]))
}

fn f32_at(bytes: &[u8], offset: usize) -> Result<f32, StmError> {
    Ok(f32::from_bits(u32_at(bytes, offset)?))
}

fn require(bytes: &[u8], offset: usize, size: usize) -> Result<(), StmError> {
    let end = offset
        .checked_add(size)
        .ok_or_else(|| StmError::new(offset, "byte range overflow"))?;
    if end > bytes.len() {
        return Err(StmError::new(
            offset,
            format!(
                "need {size} bytes, but the file ends at 0x{:X}",
                bytes.len()
            ),
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StmError {
    pub offset: usize,
    pub message: String,
}

impl StmError {
    fn new(offset: usize, message: impl Into<String>) -> Self {
        Self {
            offset,
            message: message.into(),
        }
    }
}

impl fmt::Display for StmError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid STM at offset 0x{:X}: {}",
            self.offset, self.message
        )
    }
}

impl std::error::Error for StmError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_visual_triangle_object() {
        let object_start = FILE_HEADER_SIZE + MATERIAL_SIZE;
        let data_start = object_start + OBJECT_DATA_OFFSET;
        let file_size = data_start + 36 + 24 + GROUP_HEADER_SIZE + 6 + 16 + 36;
        let mut bytes = vec![0_u8; file_size];
        bytes[0..4].copy_from_slice(&1_u32.to_le_bytes());
        bytes[4..8].copy_from_slice(&1_u32.to_le_bytes());
        bytes[FILE_HEADER_SIZE + MATERIAL_TEXTURE_OFFSET..][..8].copy_from_slice(b"tile.tga");
        let name = object_start + OBJECT_NAME_OFFSET;
        bytes[name - 4..name].copy_from_slice(&u32::MAX.to_le_bytes());
        bytes[name..name + 8].copy_from_slice(b"triangle");
        let counts = object_start + OBJECT_COUNTS_OFFSET;
        for offset in [0, 4, 16] {
            bytes[counts + offset..counts + offset + 4].copy_from_slice(&3_u32.to_le_bytes());
        }
        bytes[counts + 24..counts + 28].copy_from_slice(&1_u32.to_le_bytes());
        bytes[counts + 28..counts + 32].copy_from_slice(&1_u32.to_le_bytes());
        let positions = [[0.0_f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
        let mut cursor = data_start;
        for position in positions {
            for component in position {
                bytes[cursor..cursor + 4].copy_from_slice(&component.to_le_bytes());
                cursor += 4;
            }
        }
        cursor += 24;
        bytes[cursor..cursor + 4].copy_from_slice(&0_u32.to_le_bytes());
        bytes[cursor + 8..cursor + 12].copy_from_slice(&1_u32.to_le_bytes());
        bytes[cursor + 12..cursor + 16].copy_from_slice(&1_u32.to_le_bytes());
        cursor += GROUP_HEADER_SIZE;
        for index in [0_u16, 1, 2] {
            bytes[cursor..cursor + 2].copy_from_slice(&index.to_le_bytes());
            cursor += 2;
        }

        let model = StaticModelFile::parse(&bytes).expect("synthetic STM should parse");
        assert_eq!(model.materials[0].texture_name, "tile.tga");
        assert_eq!(model.objects.len(), 1);
        assert_eq!(model.objects[0].groups[0].faces, vec![[0, 1, 2]]);
    }

    #[test]
    fn parses_a_lightmapped_triangle_object() {
        let object_start = FILE_HEADER_SIZE + MATERIAL_SIZE;
        let data_start = object_start + OBJECT_DATA_OFFSET;
        let file_size = data_start + 36 + 24 + GROUP_HEADER_SIZE + 6 + 24 + 128;
        let mut bytes = vec![0_u8; file_size];
        bytes[0..4].copy_from_slice(&1_u32.to_le_bytes());
        bytes[4..8].copy_from_slice(&1_u32.to_le_bytes());
        bytes[FILE_HEADER_SIZE + MATERIAL_TEXTURE_OFFSET..][..8].copy_from_slice(b"tile.tga");
        let name = object_start + OBJECT_NAME_OFFSET;
        bytes[name - 4..name].copy_from_slice(&u32::MAX.to_le_bytes());
        bytes[name..name + 5].copy_from_slice(b"floor");
        let counts = object_start + OBJECT_COUNTS_OFFSET;
        for offset in [0, 4, 16] {
            bytes[counts + offset..counts + offset + 4].copy_from_slice(&3_u32.to_le_bytes());
        }
        bytes[counts + 24..counts + 28].copy_from_slice(&1_u32.to_le_bytes());
        bytes[counts + 28..counts + 32].copy_from_slice(&3_u32.to_le_bytes());

        let mut cursor = data_start + 36 + 24;
        bytes[cursor..cursor + 4].copy_from_slice(&0_u32.to_le_bytes());
        bytes[cursor + 8..cursor + 12].copy_from_slice(&1_u32.to_le_bytes());
        bytes[cursor + 12..cursor + 16].copy_from_slice(&1_u32.to_le_bytes());
        bytes[cursor + 20..cursor + 24].copy_from_slice(&3_u32.to_le_bytes());
        cursor += GROUP_HEADER_SIZE;
        for index in [0_u16, 1, 2] {
            bytes[cursor..cursor + 2].copy_from_slice(&index.to_le_bytes());
            cursor += 2;
        }

        let model = StaticModelFile::parse(&bytes).expect("lightmapped STM should parse");
        assert_eq!(model.objects.len(), 1);
        assert_eq!(model.objects[0].name, "floor");
        assert_eq!(model.objects[0].groups[0].faces, vec![[0, 1, 2]]);
    }
}
