use std::fmt;
use std::fmt::Write as _;

const MATERIAL_BLOCK_OFFSET: usize = 0x1C;
const MATERIAL_RECORD_TAG: u32 = 0x00F0_0000;
const MESH_RECORD_TAG: u32 = 0xF400_0000;
const NODE_RECORD_TAG: u32 = 0xF500_0000;
const MESH_NAME_OFFSET: usize = 0xC4;
const MESH_COUNTS_OFFSET: usize = 0x144;
const POSITIONS_OFFSET: usize = 0x174;

/// Row-vector 4x4 matrix (Direct3D style): translation in the last row.
pub type Matrix = [[f32; 4]; 4];

/// A node of the model's hierarchy: a mesh or a bone.
///
/// Both kinds start with the same header. Word 0 is the node's **identifier** (counting down
/// through the file), word 48 the identifier of its **parent** (`-1` for the root), words
/// 15..30 the world matrix of the bind pose and words 31..46 its inverse; skin records name
/// bones by identifier.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelNode {
    pub id: i32,
    pub parent_id: i32,
    pub name: String,
    pub is_bone: bool,
    pub world: Matrix,
    pub inverse: Matrix,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModelFile {
    pub version: u32,
    pub node_count: u32,
    pub material_count: u32,
    pub bone_count: u32,
    pub materials: Vec<ModelMaterial>,
    pub meshes: Vec<ModelMesh>,
    pub bones: Vec<String>,
    /// Every mesh and bone in file order (children come before their parents).
    pub nodes: Vec<ModelNode>,
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
    /// Node identifier (word 0 of the record; it was once misread as a parent index).
    pub id: i32,
    /// Identifier of the parent node, `-1` for the root.
    pub parent_id: i32,
    /// World matrix of the node in the bind pose.
    pub world: Matrix,
    pub inverse: Matrix,
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
    /// One UV per vertex. For seam-split meshes this is the `T` regular UVs followed by the
    /// `S` seam UVs (`T + S` = vertex count).
    pub texture_coordinates: Vec<[f32; 2]>,
    pub face_groups: Vec<FaceGroup>,
    /// Seam-split meshes only: for each seam vertex (vertex `T + i`), the index `< T` of the
    /// vertex it duplicates. Skin weights are stored for the first `T` vertices, so this is
    /// how a seam vertex inherits them. Empty for meshes without seams.
    pub seam_sources: Vec<u32>,
    /// Skin influences of every vertex, when the mesh is skinned.
    pub skin: Option<MeshSkin>,
}

/// Per-vertex skin data. Position at any pose:
/// `sum(weight * (offset, 1) * posed_world[bone])`, which at the bind pose reproduces the
/// mesh's position array (exactly, in 247 of the 495 skinned meshes checked; the rest differ by
/// under two units because the stored pose is not exactly the bind pose).
#[derive(Debug, Clone, PartialEq)]
pub struct MeshSkin {
    /// `V` lists (seam vertices point at the same records as their source).
    pub influences: Vec<Vec<Influence>>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Influence {
    /// Identifier of the bone node.
    pub bone_id: i32,
    pub weight: f32,
    /// Vertex position in the bone's own space.
    pub offset: [f32; 3],
    /// Vertex normal in the bone's own space.
    pub normal: [f32; 3],
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
        let mut nodes = Vec::new();
        let mut unsupported_records = Vec::new();
        let mut first_record = true;
        while cursor < bytes.len() {
            require(bytes, cursor, 8)?;
            let tag = u32_at(bytes, cursor)?;
            let payload_size = usize_at(bytes, cursor + 4)?;
            let payload = checked_add(cursor, 8, cursor)?;
            require(bytes, payload, payload_size)?;
            match tag {
                MESH_RECORD_TAG => {
                    let mesh = parse_mesh(
                        &bytes[payload..payload + payload_size],
                        payload,
                        if first_record { first_flags } else { 0 },
                    )?;
                    nodes.push(ModelNode {
                        id: mesh.id,
                        parent_id: mesh.parent_id,
                        name: mesh.name.clone(),
                        is_bone: false,
                        world: mesh.world,
                        inverse: mesh.inverse,
                    });
                    meshes.push(mesh);
                }
                NODE_RECORD_TAG => {
                    let name = if payload_size >= MESH_NAME_OFFSET + 128 {
                        c_string(bytes, payload + MESH_NAME_OFFSET, 128)?
                    } else {
                        String::new()
                    };
                    if let Ok(header) =
                        read_node_header(&bytes[payload..payload + payload_size], payload)
                    {
                        nodes.push(ModelNode {
                            id: header.id,
                            parent_id: header.parent_id,
                            name: name.clone(),
                            is_bone: true,
                            world: header.world,
                            inverse: header.inverse,
                        });
                    }
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
            nodes,
            unsupported_records,
        })
    }

    /// Index into [`materials`](Self::materials) that a face group's `material` value names.
    ///
    /// The value is **not** an index: it is the material's selector. The first material has no
    /// selector and answers to `1`; the others carry selectors counting down (a model with
    /// materials `[None, 7, 6, 5, 4, 3, 2]` has groups numbered `7..=1`). Returns `None` for a
    /// value no material answers to.
    #[must_use]
    pub fn material_index_for_group(&self, group_material: u32) -> Option<usize> {
        self.materials
            .iter()
            .position(|material| material.selector == Some(group_material))
            .or_else(|| (group_material == 1 && !self.materials.is_empty()).then_some(0))
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
    let header = read_node_header(bytes, absolute)?;
    let vertex_count = u32_at_local(bytes, absolute, MESH_COUNTS_OFFSET)?;
    let texture_vertex_count = u32_at_local(bytes, absolute, MESH_COUNTS_OFFSET + 8)?;
    let seam_vertex_count = u32_at_local(bytes, absolute, MESH_COUNTS_OFFSET + 12)?;
    let pivot = [
        f32_at_local(bytes, absolute, MESH_COUNTS_OFFSET + 36)?,
        f32_at_local(bytes, absolute, MESH_COUNTS_OFFSET + 40)?,
        f32_at_local(bytes, absolute, MESH_COUNTS_OFFSET + 44)?,
    ];

    let (geometry, geometry_issue) =
        if vertex_count == texture_vertex_count.saturating_add(seam_vertex_count) {
            match parse_geometry(
                bytes,
                absolute,
                vertex_count,
                texture_vertex_count,
                seam_vertex_count,
            ) {
                Ok(geometry) => (Some(geometry), None),
                Err(failure) => (None, Some(failure.to_string())),
            }
        } else {
            (
                None,
                Some("vertex, texture and seam counts do not add up".to_owned()),
            )
        };

    Ok(ModelMesh {
        name,
        flags,
        id: header.id,
        parent_id: header.parent_id,
        world: header.world,
        inverse: header.inverse,
        vertex_count,
        texture_vertex_count,
        seam_vertex_count,
        pivot,
        geometry,
        geometry_issue,
    })
}

/// The header every node record starts with (see [`ModelNode`]).
struct NodeHeader {
    id: i32,
    parent_id: i32,
    world: Matrix,
    inverse: Matrix,
}

fn read_node_header(bytes: &[u8], absolute: usize) -> Result<NodeHeader, ModelError> {
    require_local(bytes, absolute, 0, POSITIONS_OFFSET)?;
    let matrix = |first_word: usize| -> Result<Matrix, ModelError> {
        let mut matrix = [[0.0_f32; 4]; 4];
        for (row, cells) in matrix.iter_mut().enumerate() {
            for (column, cell) in cells.iter_mut().enumerate() {
                *cell = f32_at_local(bytes, absolute, (first_word + row * 4 + column) * 4)?;
            }
        }
        Ok(matrix)
    };
    Ok(NodeHeader {
        id: i32_at_local(bytes, absolute, 0)?,
        parent_id: i32_at_local(bytes, absolute, 48 * 4)?,
        world: matrix(15)?,
        inverse: matrix(31)?,
    })
}

/// Mesh geometry. `T` UVs, then `S` more UVs for vertices duplicated along texture seams
/// (`T + S = V`; `S = 0` for meshes without seams), then `S` `u32` seam sources, then face
/// groups with no leading count.
///
/// Payload layout after the `0x174` header: positions (`V * 12`), UVs (`T * 8`), seam UVs
/// (`S * 8`), seam sources (`S * 4`), groups. A group is 28 bytes, `material, x, faces, faces,
/// 0, ?, 0` (the same header the STM uses), followed by `faces` triangles of three `u16`
/// indices, packed with no padding. Groups are read until the pattern stops holding; the rest
/// of the payload (per-face and per-vertex float blocks, and skin data) is not decoded yet.
fn parse_geometry(
    bytes: &[u8],
    absolute: usize,
    vertex_count: u32,
    texture_vertex_count: u32,
    seam_vertex_count: u32,
) -> Result<MeshGeometry, ModelError> {
    const GROUP_HEADER_SIZE: usize = 28;
    let vertices = count(vertex_count, absolute + MESH_COUNTS_OFFSET)?;
    let regular_uvs = count(texture_vertex_count, absolute + MESH_COUNTS_OFFSET + 8)?;
    let seams = count(seam_vertex_count, absolute + MESH_COUNTS_OFFSET + 12)?;

    let positions_size = checked_mul(vertices, 12, absolute + POSITIONS_OFFSET)?;
    require_local(bytes, absolute, POSITIONS_OFFSET, positions_size)?;
    let mut positions = Vec::with_capacity(vertices);
    for index in 0..vertices {
        let offset = POSITIONS_OFFSET + index * 12;
        positions.push([
            f32_at_local(bytes, absolute, offset)?,
            f32_at_local(bytes, absolute, offset + 4)?,
            f32_at_local(bytes, absolute, offset + 8)?,
        ]);
    }

    // `T` regular UVs followed by `S` seam UVs are contiguous: one UV per vertex.
    let uv_start = POSITIONS_OFFSET + positions_size;
    let uv_size = checked_mul(vertices, 8, absolute + uv_start)?;
    require_local(bytes, absolute, uv_start, uv_size)?;
    let mut texture_coordinates = Vec::with_capacity(vertices);
    for index in 0..vertices {
        let offset = uv_start + index * 8;
        texture_coordinates.push([
            f32_at_local(bytes, absolute, offset)?,
            f32_at_local(bytes, absolute, offset + 4)?,
        ]);
    }
    debug_assert_eq!(regular_uvs + seams, vertices);

    let sources_start = uv_start + uv_size;
    let sources_size = checked_mul(seams, 4, absolute + sources_start)?;
    require_local(bytes, absolute, sources_start, sources_size)?;
    let mut seam_sources = Vec::with_capacity(seams);
    for index in 0..seams {
        let source = u32_at_local(bytes, absolute, sources_start + index * 4)?;
        if usize::try_from(source).is_ok_and(|source| source >= regular_uvs) {
            return Err(error(
                absolute + sources_start + index * 4,
                "seam source points past the regular vertices",
            ));
        }
        seam_sources.push(source);
    }

    let mut cursor = sources_start + sources_size;
    let mut face_groups = Vec::new();
    while cursor + GROUP_HEADER_SIZE <= bytes.len() {
        let material_index = u32_at_local(bytes, absolute, cursor)?;
        let face_count = u32_at_local(bytes, absolute, cursor + 8)?;
        let repeated = u32_at_local(bytes, absolute, cursor + 12)?;
        let (before, after) = (
            u32_at_local(bytes, absolute, cursor + 16)?,
            u32_at_local(bytes, absolute, cursor + 24)?,
        );
        if face_count == 0 || face_count != repeated || before != 0 || after != 0 {
            break;
        }
        let face_count = count(face_count, absolute + cursor + 8)?;
        let index_bytes = checked_mul(face_count, 6, absolute + cursor)?;
        let indices_start = cursor + GROUP_HEADER_SIZE;
        if indices_start
            .checked_add(index_bytes)
            .is_none_or(|end| end > bytes.len())
        {
            break;
        }
        let mut faces = Vec::with_capacity(face_count);
        for face in 0..face_count {
            let offset = indices_start + face * 6;
            let triangle = [
                u16_at_local(bytes, absolute, offset)?,
                u16_at_local(bytes, absolute, offset + 2)?,
                u16_at_local(bytes, absolute, offset + 4)?,
            ];
            if triangle.iter().any(|index| usize::from(*index) >= vertices) {
                break;
            }
            faces.push(triangle);
        }
        if faces.len() != face_count {
            break;
        }
        face_groups.push(FaceGroup {
            material_index,
            faces,
        });
        cursor = indices_start + index_bytes;
    }
    if face_groups.is_empty() {
        return Err(error(
            absolute + cursor,
            "no face group found after the seam table",
        ));
    }

    let skin = parse_skin(bytes, cursor, vertices, regular_uvs);
    Ok(MeshGeometry {
        positions,
        texture_coordinates,
        face_groups,
        seam_sources,
        skin,
    })
}

/// The block after the face groups of a skinned mesh:
///
/// ```text
/// V, T, T                       (u32 x 3)
/// V entries of 5 bytes          influence count (u8), index of the first record (u32)
/// N records of 32 bytes         bone id (u32), weight (f32), offset (3 x f32), normal (3 x f32)
/// V normals of 12 bytes         (not read)
/// ```
///
/// `N` is whatever the size leaves over. Returns `None` for meshes without skin (whose block is
/// just three zero words and the normals) or when the block does not add up.
fn parse_skin(bytes: &[u8], cursor: usize, vertices: usize, regular: usize) -> Option<MeshSkin> {
    const ENTRY: usize = 5;
    const RECORD: usize = 32;
    let read_u32 = |offset: usize| -> Option<u32> {
        bytes
            .get(offset..offset.checked_add(4)?)
            .map(|value| u32::from_le_bytes([value[0], value[1], value[2], value[3]]))
    };
    let read_f32 = |offset: usize| read_u32(offset).map(f32::from_bits);
    let matches = |offset: usize, expected: usize| {
        read_u32(offset).and_then(|value| usize::try_from(value).ok()) == Some(expected)
    };
    if !(matches(cursor, vertices) && matches(cursor + 4, regular) && matches(cursor + 8, regular))
    {
        return None;
    }
    let table = cursor + 12;
    let records_start = table.checked_add(vertices.checked_mul(ENTRY)?)?;
    let records_size = bytes
        .len()
        .checked_sub(records_start)?
        .checked_sub(vertices.checked_mul(12)?)?;
    if records_size % RECORD != 0 {
        return None;
    }
    let record_count = records_size / RECORD;
    let mut influences = Vec::with_capacity(vertices);
    for vertex in 0..vertices {
        let entry = table + vertex * ENTRY;
        let count = usize::from(*bytes.get(entry)?);
        let first = usize::try_from(read_u32(entry + 1)?).ok()?;
        if count == 0 || first.checked_add(count)? > record_count {
            return None;
        }
        let mut list = Vec::with_capacity(count);
        for record in first..first + count {
            let at = records_start + record * RECORD;
            list.push(Influence {
                bone_id: i32::try_from(read_u32(at)?).ok()?,
                weight: read_f32(at + 4)?,
                offset: [read_f32(at + 8)?, read_f32(at + 12)?, read_f32(at + 16)?],
                normal: [read_f32(at + 20)?, read_f32(at + 24)?, read_f32(at + 28)?],
            });
        }
        influences.push(list);
    }
    Some(MeshSkin { influences })
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
mod geometry_tests {
    use super::*;

    fn put(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    /// A quad split along a texture seam: 4 vertices, 2 regular UVs and 2 seam UVs.
    #[test]
    fn decodes_a_seam_split_quad() {
        let (vertices, regular, seams) = (4_usize, 2_usize, 2_usize);
        let mut bytes = vec![0_u8; POSITIONS_OFFSET];
        put(&mut bytes, MESH_COUNTS_OFFSET, vertices as u32);
        put(&mut bytes, MESH_COUNTS_OFFSET + 8, regular as u32);
        put(&mut bytes, MESH_COUNTS_OFFSET + 12, seams as u32);
        for index in 0..vertices {
            for component in [index as f32, 0.0, 1.0] {
                bytes.extend_from_slice(&component.to_le_bytes());
            }
        }
        for index in 0..vertices {
            for component in [index as f32 * 0.25, 0.5] {
                bytes.extend_from_slice(&component.to_le_bytes());
            }
        }
        for source in [1_u32, 0] {
            bytes.extend_from_slice(&source.to_le_bytes());
        }
        // group: material 3, x 5, two faces, repeated, zeros
        for value in [3_u32, 5, 2, 2, 0, 0, 0] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        for index in [0_u16, 1, 2, 2, 3, 0] {
            bytes.extend_from_slice(&index.to_le_bytes());
        }
        // trailing data that is not a group must be ignored
        bytes.extend_from_slice(&[0xAB; 20]);

        let mesh = parse_geometry(&bytes, 0, vertices as u32, regular as u32, seams as u32)
            .expect("seam-split quad should decode");
        assert_eq!(mesh.positions.len(), 4);
        assert_eq!(mesh.texture_coordinates.len(), 4);
        assert_eq!(mesh.texture_coordinates[3], [0.75, 0.5]);
        assert_eq!(mesh.seam_sources, vec![1, 0]);
        assert_eq!(mesh.face_groups.len(), 1);
        assert_eq!(mesh.face_groups[0].material_index, 3);
        assert_eq!(mesh.face_groups[0].faces, vec![[0, 1, 2], [2, 3, 0]]);
    }

    /// The same layout with no seams (`S = 0`), which the simple meshes use.
    #[test]
    fn decodes_a_mesh_without_seams() {
        let mut bytes = vec![0_u8; POSITIONS_OFFSET];
        put(&mut bytes, MESH_COUNTS_OFFSET, 3);
        put(&mut bytes, MESH_COUNTS_OFFSET + 8, 3);
        for index in 0..3_u32 {
            for component in [index as f32, 1.0, 2.0] {
                bytes.extend_from_slice(&component.to_le_bytes());
            }
        }
        for _ in 0..3 {
            for component in [0.5_f32, 0.5] {
                bytes.extend_from_slice(&component.to_le_bytes());
            }
        }
        for value in [2_u32, 0, 1, 1, 0, 0, 0] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        for index in [0_u16, 1, 2] {
            bytes.extend_from_slice(&index.to_le_bytes());
        }
        let mesh = parse_geometry(&bytes, 0, 3, 3, 0).expect("triangle should decode");
        assert!(mesh.seam_sources.is_empty());
        assert_eq!(mesh.face_groups[0].material_index, 2);
        assert_eq!(mesh.face_groups[0].faces, vec![[0, 1, 2]]);
    }

    #[test]
    fn rejects_seam_sources_that_point_at_seam_vertices() {
        let mut bytes = vec![0_u8; POSITIONS_OFFSET + 2 * 12 + 2 * 8 + 4];
        put(&mut bytes, MESH_COUNTS_OFFSET, 2);
        put(&mut bytes, MESH_COUNTS_OFFSET + 8, 1);
        put(&mut bytes, MESH_COUNTS_OFFSET + 12, 1);
        let sources = POSITIONS_OFFSET + 2 * 12 + 2 * 8;
        put(&mut bytes, sources, 1); // must be < regular UV count (1)
        assert!(parse_geometry(&bytes, 0, 2, 1, 1).is_err());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model_with_selectors(selectors: &[Option<u32>]) -> ModelFile {
        ModelFile {
            version: 1,
            node_count: 0,
            material_count: selectors.len() as u32,
            bone_count: 0,
            materials: selectors
                .iter()
                .map(|selector| ModelMaterial {
                    selector: *selector,
                    flags: 0,
                    texture_name: String::new(),
                    name: String::new(),
                })
                .collect(),
            meshes: Vec::new(),
            bones: Vec::new(),
            nodes: Vec::new(),
            unsupported_records: Vec::new(),
        }
    }

    #[test]
    fn group_material_values_are_selectors_not_indices() {
        let model = model_with_selectors(&[None, Some(7), Some(6), Some(2)]);
        assert_eq!(model.material_index_for_group(1), Some(0));
        assert_eq!(model.material_index_for_group(7), Some(1));
        assert_eq!(model.material_index_for_group(6), Some(2));
        assert_eq!(model.material_index_for_group(2), Some(3));
        assert_eq!(model.material_index_for_group(3), None);
        assert_eq!(model_with_selectors(&[]).material_index_for_group(1), None);
    }

    #[test]
    fn obj_name_replaces_spaces() {
        assert_eq!(obj_name("a mesh"), "a_mesh");
        assert_eq!(obj_name(""), "unnamed");
    }
}
