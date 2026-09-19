use std::fmt;
use std::fmt::Write as _;

const MATERIAL_BLOCK_OFFSET: usize = 0x1C;
const MATERIAL_RECORD_TAG: u32 = 0x00F0_0000;
const MESH_RECORD_TAG: u32 = 0xF400_0000;
const NODE_RECORD_TAG: u32 = 0xF500_0000;
const MESH_NAME_OFFSET: usize = 0xC4;
const MESH_COUNTS_OFFSET: usize = 0x144;
const POSITIONS_OFFSET: usize = 0x174;

#[derive(Debug, Clone, PartialEq)]
pub struct ModelFile {
    pub version: u32,
    pub node_count: u32,
    pub material_count: u32,
    pub bone_count: u32,
    pub materials: Vec<ModelMaterial>,
    pub meshes: Vec<ModelMesh>,
    pub bones: Vec<String>,
    pub unsupported_records: Vec<ModelRecordSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelMaterial {
    pub selector: Option<u32>,
    pub flags: u32,
    pub texture_name: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModelMesh {
    pub name: String,
    pub flags: u32,
    pub parent_index: i32,
    pub vertex_count: u32,
    pub texture_vertex_count: u32,
    pub seam_vertex_count: u32,
    pub pivot: [f32; 3],
    pub geometry: Option<MeshGeometry>,
    pub geometry_issue: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelRecordSummary {
    pub tag: u32,
    pub payload_size: usize,
    pub candidate_name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MeshGeometry {
    pub positions: Vec<[f32; 3]>,
    pub texture_coordinates: Vec<[f32; 2]>,
    pub face_groups: Vec<FaceGroup>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FaceGroup {
    pub material_index: u32,
    pub faces: Vec<[u16; 3]>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelError {
    pub offset: usize,
    pub message: String,
}

impl fmt::Display for ModelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid MOD at offset 0x{:X}: {}",
            self.offset, self.message
        )
    }
}

impl std::error::Error for ModelError {}

impl ModelFile {
    pub fn parse(bytes: &[u8]) -> Result<Self, ModelError> {
        require(bytes, 0, 0x24)?;
        let version = u32_at(bytes, 0)?;
        if version != 1 {
            return Err(error(0, format!("unsupported version {version}")));
        }

        let node_count = u32_at(bytes, 4)?;
        let material_count = u32_at(bytes, 8)?;
        let bone_count = u32_at(bytes, 24)?;
        let material_capacity = usize::try_from(material_count)
            .map_err(|_| error(8, "material count does not fit in memory"))?;
        let mut materials = Vec::with_capacity(material_capacity);
        let mut cursor = MATERIAL_BLOCK_OFFSET;
        for index in 0..material_count {
            let (selector, flags) = if index == 0 {
                (None, 0)
            } else {
                require(bytes, cursor, 8)?;
                let values = (Some(u32_at(bytes, cursor)?), u32_at(bytes, cursor + 4)?);
                cursor += 8;
                values
            };
            require(bytes, cursor, 8)?;
            let tag = u32_at(bytes, cursor)?;
            if tag != MATERIAL_RECORD_TAG {
                return Err(error(
                    cursor,
                    format!("expected material tag 0x{MATERIAL_RECORD_TAG:08X}, found 0x{tag:08X}"),
                ));
            }
            let block_size = usize_at(bytes, cursor + 4)?;
            if block_size < 0x5A4 {
                return Err(error(cursor + 4, "material record is unexpectedly small"));
            }
            require(bytes, cursor, block_size)?;
            materials.push(ModelMaterial {
                selector,
                flags,
                texture_name: c_string(bytes, cursor + 0xA4, 128)?,
                name: c_string(bytes, cursor + 0x524, 128)?,
            });
            cursor = checked_add(cursor, block_size, cursor + 4)?;
        }

        let declared_mesh_count = usize_at(bytes, 12)?;
        let root_count = u32_at(bytes, cursor)?;
        cursor = checked_add(cursor, 4, cursor)?;
        if root_count == 0 {
            return Err(error(cursor - 4, "model has no root record"));
        }
        let first_flags = u32_at(bytes, cursor)?;
        cursor = checked_add(cursor, 4, cursor)?;
        let mut meshes = Vec::with_capacity(declared_mesh_count);
        let mut bones = Vec::new();
        let mut unsupported_records = Vec::new();
        let mut first_record = true;
        while cursor < bytes.len() {
            require(bytes, cursor, 8)?;
            let tag = u32_at(bytes, cursor)?;
            let payload_size = usize_at(bytes, cursor + 4)?;
            let payload = checked_add(cursor, 8, cursor)?;
            require(bytes, payload, payload_size)?;
            match tag {
                MESH_RECORD_TAG => meshes.push(parse_mesh(
                    &bytes[payload..payload + payload_size],
                    payload,
                    if first_record { first_flags } else { 0 },
                )?),
                NODE_RECORD_TAG => {
                    let name = if payload_size >= MESH_NAME_OFFSET + 128 {
                        c_string(bytes, payload + MESH_NAME_OFFSET, 128)?
                    } else {
                        String::new()
                    };
                    bones.push(name);
                }
                _ if tag & 0xF000_0000 == 0xF000_0000 => {
                    let candidate_name = if payload_size >= MESH_NAME_OFFSET + 128 {
                        c_string(bytes, payload + MESH_NAME_OFFSET, 128)?
                    } else {
                        String::new()
                    };
                    unsupported_records.push(ModelRecordSummary {
                        tag,
                        payload_size,
                        candidate_name,
                    });
                }
                _ => {
                    return Err(error(
                        cursor,
                        format!("invalid model record tag 0x{tag:08X}"),
                    ));
                }
            }
            cursor = checked_add(payload, payload_size, cursor + 4)?;
            first_record = false;
        }

        if meshes.len() != declared_mesh_count {
            return Err(error(
                12,
                format!(
                    "declares {declared_mesh_count} meshes but contains {}",
                    meshes.len()
                ),
            ));
        }
        if bones.len() != bone_count as usize {
            return Err(error(
                24,
                format!("declares {bone_count} bones but contains {}", bones.len()),
            ));
        }
        if u64::from(node_count)
            != meshes.len() as u64 + bones.len() as u64 + unsupported_records.len() as u64
        {
            return Err(error(
                4,
                format!(
                    "declares {node_count} nodes but contains {} meshes and {bone_count} bones",
                    meshes.len()
                ),
            ));
        }

        Ok(Self {
            version,
            node_count,
            material_count,
            bone_count,
            materials,
            meshes,
            bones,
            unsupported_records,
        })
    }

    pub fn to_obj(&self) -> Result<String, ModelError> {
        let mut output = String::from("# Corum Online MOD export\n");
        let mut base_vertex = 1_u32;
        let mut exported = 0_u32;

        for mesh in &self.meshes {
            let Some(geometry) = &mesh.geometry else {
                continue;
            };
            exported += 1;
            writeln!(output, "o {}", obj_name(&mesh.name)).unwrap();
            for position in &geometry.positions {
                writeln!(output, "v {} {} {}", position[0], position[1], position[2]).unwrap();
            }
            for uv in &geometry.texture_coordinates {
                writeln!(output, "vt {} {}", uv[0], 1.0 - uv[1]).unwrap();
            }
            for group in &geometry.face_groups {
                writeln!(output, "g material_{}", group.material_index).unwrap();
                for face in &group.faces {
                    let indices = face.map(|index| base_vertex + u32::from(index));
                    if indices
                        .iter()
                        .any(|index| *index >= base_vertex + mesh.vertex_count)
                    {
                        return Err(error(
                            0,
                            format!("mesh '{}' has an invalid face index", mesh.name),
                        ));
                    }
                    writeln!(
                        output,
                        "f {0}/{0} {1}/{1} {2}/{2}",
                        indices[0], indices[1], indices[2]
                    )
                    .unwrap();
                }
            }
            base_vertex = base_vertex
                .checked_add(mesh.vertex_count)
                .ok_or_else(|| error(0, "OBJ vertex index overflow"))?;
        }

        if exported == 0 {
            return Err(error(
                0,
                "no directly exportable mesh (skinned/seam-split layout is not decoded yet)",
            ));
        }
        Ok(output)
    }
}

fn parse_mesh(bytes: &[u8], absolute: usize, flags: u32) -> Result<ModelMesh, ModelError> {
    require_local(bytes, absolute, 0, POSITIONS_OFFSET)?;
    let name = c_string_local(bytes, absolute, MESH_NAME_OFFSET, 128)?;
    let parent_index = i32_at_local(bytes, absolute, 0)?;
    let vertex_count = u32_at_local(bytes, absolute, MESH_COUNTS_OFFSET)?;
    let texture_vertex_count = u32_at_local(bytes, absolute, MESH_COUNTS_OFFSET + 8)?;
    let seam_vertex_count = u32_at_local(bytes, absolute, MESH_COUNTS_OFFSET + 12)?;
    let pivot = [
        f32_at_local(bytes, absolute, MESH_COUNTS_OFFSET + 36)?,
        f32_at_local(bytes, absolute, MESH_COUNTS_OFFSET + 40)?,
        f32_at_local(bytes, absolute, MESH_COUNTS_OFFSET + 44)?,
    ];

    let (geometry, geometry_issue) =
        if vertex_count == texture_vertex_count && seam_vertex_count == 0 {
            match parse_simple_geometry(bytes, absolute, vertex_count, texture_vertex_count) {
                Ok(geometry) => (Some(geometry), None),
                Err(failure) => (None, Some(failure.to_string())),
            }
        } else {
            (
                None,
                Some("skinned/seam-split geometry layout is not decoded yet".to_owned()),
            )
        };

    Ok(ModelMesh {
        name,
        flags,
        parent_index,
        vertex_count,
        texture_vertex_count,
        seam_vertex_count,
        pivot,
        geometry,
        geometry_issue,
    })
}

fn parse_simple_geometry(
    bytes: &[u8],
    absolute: usize,
    vertex_count: u32,
    texture_vertex_count: u32,
) -> Result<MeshGeometry, ModelError> {
    let vertex_count = count(vertex_count, absolute + MESH_COUNTS_OFFSET)?;
    let texture_vertex_count = count(texture_vertex_count, absolute + MESH_COUNTS_OFFSET + 8)?;
    let positions_size = checked_mul(vertex_count, 12, absolute + POSITIONS_OFFSET)?;
    let uv_offset = checked_add(
        POSITIONS_OFFSET,
        positions_size,
        absolute + POSITIONS_OFFSET,
    )?;
    let uv_size = checked_mul(texture_vertex_count, 8, absolute + uv_offset)?;
    let mut cursor = checked_add(uv_offset, uv_size, absolute + uv_offset)?;
    require_local(bytes, absolute, POSITIONS_OFFSET, positions_size)?;
    require_local(bytes, absolute, uv_offset, uv_size)?;

    let mut positions = Vec::with_capacity(vertex_count);
    for index in 0..vertex_count {
        let offset = POSITIONS_OFFSET + index * 12;
        positions.push([
            f32_at_local(bytes, absolute, offset)?,
            f32_at_local(bytes, absolute, offset + 4)?,
            f32_at_local(bytes, absolute, offset + 8)?,
        ]);
    }

    let mut texture_coordinates = Vec::with_capacity(texture_vertex_count);
    for index in 0..texture_vertex_count {
        let offset = uv_offset + index * 8;
        texture_coordinates.push([
            f32_at_local(bytes, absolute, offset)?,
            f32_at_local(bytes, absolute, offset + 4)?,
        ]);
    }

    let face_group_count = count(u32_at_local(bytes, absolute, cursor)?, absolute + cursor)?;
    cursor = checked_add(cursor, 4, absolute + cursor)?;
    let mut face_groups = Vec::with_capacity(face_group_count);
    for _ in 0..face_group_count {
        require_local(bytes, absolute, cursor, 24)?;
        let material_index = u32_at_local(bytes, absolute, cursor)?;
        let face_count = count(
            u32_at_local(bytes, absolute, cursor + 4)?,
            absolute + cursor + 4,
        )?;
        let texture_face_count = count(
            u32_at_local(bytes, absolute, cursor + 8)?,
            absolute + cursor + 8,
        )?;
        if face_count != texture_face_count {
            return Err(error(
                absolute + cursor + 8,
                "separate position/texture face lists are not supported yet",
            ));
        }
        cursor = checked_add(cursor, 24, absolute + cursor)?;
        let index_bytes = checked_mul(face_count, 6, absolute + cursor)?;
        require_local(bytes, absolute, cursor, index_bytes)?;
        let mut faces = Vec::with_capacity(face_count);
        for _ in 0..face_count {
            faces.push([
                u16_at_local(bytes, absolute, cursor)?,
                u16_at_local(bytes, absolute, cursor + 2)?,
                u16_at_local(bytes, absolute, cursor + 4)?,
            ]);
            cursor += 6;
        }
        face_groups.push(FaceGroup {
            material_index,
            faces,
        });
    }

    Ok(MeshGeometry {
        positions,
        texture_coordinates,
        face_groups,
    })
}

fn obj_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|character| {
            if character.is_whitespace() {
                '_'
            } else {
                character
            }
        })
        .collect();
    if cleaned.is_empty() {
        "unnamed".to_owned()
    } else {
        cleaned
    }
}

fn c_string(bytes: &[u8], offset: usize, size: usize) -> Result<String, ModelError> {
    require(bytes, offset, size)?;
    let field = &bytes[offset..offset + size];
    let end = field.iter().position(|byte| *byte == 0).unwrap_or(size);
    Ok(String::from_utf8_lossy(&field[..end]).into_owned())
}

fn c_string_local(
    bytes: &[u8],
    absolute: usize,
    offset: usize,
    size: usize,
) -> Result<String, ModelError> {
    require_local(bytes, absolute, offset, size)?;
    let field = &bytes[offset..offset + size];
    let end = field.iter().position(|byte| *byte == 0).unwrap_or(size);
    Ok(String::from_utf8_lossy(&field[..end]).into_owned())
}

fn count(value: u32, offset: usize) -> Result<usize, ModelError> {
    let value =
        usize::try_from(value).map_err(|_| error(offset, "count does not fit in memory"))?;
    if value > 10_000_000 {
        return Err(error(offset, format!("unreasonable count {value}")));
    }
    Ok(value)
}

fn usize_at(bytes: &[u8], offset: usize) -> Result<usize, ModelError> {
    usize::try_from(u32_at(bytes, offset)?)
        .map_err(|_| error(offset, "value does not fit in memory"))
}

fn u32_at(bytes: &[u8], offset: usize) -> Result<u32, ModelError> {
    require(bytes, offset, 4)?;
    Ok(u32::from_le_bytes(
        bytes[offset..offset + 4].try_into().unwrap(),
    ))
}

fn u32_at_local(bytes: &[u8], absolute: usize, offset: usize) -> Result<u32, ModelError> {
    require_local(bytes, absolute, offset, 4)?;
    Ok(u32::from_le_bytes(
        bytes[offset..offset + 4].try_into().unwrap(),
    ))
}

fn i32_at_local(bytes: &[u8], absolute: usize, offset: usize) -> Result<i32, ModelError> {
    require_local(bytes, absolute, offset, 4)?;
    Ok(i32::from_le_bytes(
        bytes[offset..offset + 4].try_into().unwrap(),
    ))
}

fn u16_at_local(bytes: &[u8], absolute: usize, offset: usize) -> Result<u16, ModelError> {
    require_local(bytes, absolute, offset, 2)?;
    Ok(u16::from_le_bytes(
        bytes[offset..offset + 2].try_into().unwrap(),
    ))
}

fn f32_at_local(bytes: &[u8], absolute: usize, offset: usize) -> Result<f32, ModelError> {
    Ok(f32::from_bits(u32_at_local(bytes, absolute, offset)?))
}

fn checked_add(left: usize, right: usize, offset: usize) -> Result<usize, ModelError> {
    left.checked_add(right)
        .ok_or_else(|| error(offset, "offset overflow"))
}

fn checked_mul(left: usize, right: usize, offset: usize) -> Result<usize, ModelError> {
    left.checked_mul(right)
        .ok_or_else(|| error(offset, "size overflow"))
}

fn require(bytes: &[u8], offset: usize, size: usize) -> Result<(), ModelError> {
    if offset.checked_add(size).is_none_or(|end| end > bytes.len()) {
        return Err(error(offset, format!("truncated {size}-byte field")));
    }
    Ok(())
}

fn require_local(
    bytes: &[u8],
    absolute: usize,
    offset: usize,
    size: usize,
) -> Result<(), ModelError> {
    require(bytes, offset, size).map_err(|mut failure| {
        failure.offset += absolute;
        failure
    })
}

fn error(offset: usize, message: impl Into<String>) -> ModelError {
    ModelError {
        offset,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn obj_name_replaces_spaces() {
        assert_eq!(obj_name("a mesh"), "a_mesh");
        assert_eq!(obj_name(""), "unnamed");
    }
}
