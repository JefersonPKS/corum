//! Skeleton evaluation: turns a model's nodes and one animation into world matrices.
//!
//! Matrices are Direct3D style: row vectors (`point * matrix`), translation in the last row,
//! and `a * b` applies `a` first. A node's world matrix is `local * parent_world`.

use crate::model::{Matrix, ModelFile};
use crate::motion::{MotionFile, MotionRecord};

pub const IDENTITY: Matrix = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

/// `a * b`: transform by `a`, then by `b`.
#[must_use]
pub fn multiply(a: &Matrix, b: &Matrix) -> Matrix {
    let mut result = [[0.0_f32; 4]; 4];
    for (row, output) in result.iter_mut().enumerate() {
        for (column, cell) in output.iter_mut().enumerate() {
            *cell = (0..4).map(|k| a[row][k] * b[k][column]).sum();
        }
    }
    result
}

/// `point * matrix`, including the translation row.
#[must_use]
pub fn transform_point(matrix: &Matrix, point: [f32; 3]) -> [f32; 3] {
    let mut result = matrix[3];
    for (axis, row) in matrix.iter().take(3).enumerate() {
        for column in 0..3 {
            result[column] += point[axis] * row[column];
        }
    }
    [result[0], result[1], result[2]]
}

/// `vector * matrix` without the translation, for normals and directions.
#[must_use]
pub fn transform_vector(matrix: &Matrix, vector: [f32; 3]) -> [f32; 3] {
    let mut result = [0.0_f32; 3];
    for (axis, row) in matrix.iter().take(3).enumerate() {
        for column in 0..3 {
            result[column] += vector[axis] * row[column];
        }
    }
    result
}

/// Rotation matrix of an `.ANM` quaternion `(x, y, z, w)`.
///
/// It is the **transpose** of `D3DXMatrixRotationQuaternion`'s matrix: the tracks store the
/// inverse rotation (the 3ds Max convention). Measured on the bones of an ogre, the frame-0
/// quaternion converted this way matches the bind-pose local rotation to 0.001..0.008, while the
/// D3DX matrix is off by 0.3..1.7.
#[must_use]
pub fn from_quaternion(quaternion: [f32; 4]) -> Matrix {
    let length = quaternion.iter().map(|v| v * v).sum::<f32>().sqrt();
    let [x, y, z, w] = if length > 1e-6 {
        quaternion.map(|v| v / length)
    } else {
        [0.0, 0.0, 0.0, 1.0]
    };
    [
        [
            1.0 - 2.0 * (y * y + z * z),
            2.0 * (x * y - z * w),
            2.0 * (x * z + y * w),
            0.0,
        ],
        [
            2.0 * (x * y + z * w),
            1.0 - 2.0 * (x * x + z * z),
            2.0 * (y * z - x * w),
            0.0,
        ],
        [
            2.0 * (x * z - y * w),
            2.0 * (y * z + x * w),
            1.0 - 2.0 * (x * x + y * y),
            0.0,
        ],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

/// The hierarchy of a model with its bind pose, ready to be posed by a motion.
#[derive(Debug, Clone)]
pub struct Skeleton {
    names: Vec<String>,
    ids: Vec<i32>,
    parents: Vec<Option<usize>>,
    bind_local: Vec<Matrix>,
    bind_world: Vec<Matrix>,
    bind_inverse: Vec<Matrix>,
}

impl Skeleton {
    /// Builds the hierarchy from the model's nodes (meshes and bones). A node whose parent id
    /// names no node becomes a root.
    #[must_use]
    pub fn new(model: &ModelFile) -> Self {
        let nodes = &model.nodes;
        let index_of = |id: i32| nodes.iter().position(|node| node.id == id);
        let parents: Vec<Option<usize>> = nodes
            .iter()
            .enumerate()
            .map(|(index, node)| index_of(node.parent_id).filter(|parent| *parent != index))
            .collect();
        let bind_local = nodes
            .iter()
            .zip(&parents)
            .map(|(node, parent)| match parent {
                Some(parent) => multiply(&node.world, &nodes[*parent].inverse),
                None => node.world,
            })
            .collect();
        Self {
            names: nodes.iter().map(|node| node.name.clone()).collect(),
            ids: nodes.iter().map(|node| node.id).collect(),
            parents,
            bind_local,
            bind_world: nodes.iter().map(|node| node.world).collect(),
            bind_inverse: nodes.iter().map(|node| node.inverse).collect(),
        }
    }

    #[must_use]
    pub fn node_count(&self) -> usize {
        self.names.len()
    }

    /// Index of the node with this identifier (the value skin records use to name a bone).
    #[must_use]
    pub fn index_of_id(&self, id: i32) -> Option<usize> {
        self.ids.iter().position(|candidate| *candidate == id)
    }

    #[must_use]
    pub fn bind_world(&self) -> &[Matrix] {
        &self.bind_world
    }

    #[must_use]
    pub fn bind_inverse(&self) -> &[Matrix] {
        &self.bind_inverse
    }

    /// The motion record that animates each node, matched by node name.
    #[must_use]
    pub fn tracks_for<'a>(&self, motion: &'a MotionFile) -> Vec<Option<&'a MotionRecord>> {
        self.names
            .iter()
            .map(|name| {
                motion
                    .records
                    .iter()
                    .find(|record| record.candidate_name == *name && record.has_tracks())
            })
            .collect()
    }

    /// World matrix of every node at `frame`. Rotation and position come from the node's
    /// tracks when it has them; otherwise the bind value is kept, so nodes without tracks
    /// simply follow their parents.
    #[must_use]
    pub fn pose(&self, tracks: &[Option<&MotionRecord>], frame: f32) -> Vec<Matrix> {
        let mut world: Vec<Option<Matrix>> = vec![None; self.node_count()];
        for index in 0..self.node_count() {
            self.resolve(index, tracks, frame, &mut world, 0);
        }
        world
            .into_iter()
            .map(|matrix| matrix.unwrap_or(IDENTITY))
            .collect()
    }

    fn resolve(
        &self,
        index: usize,
        tracks: &[Option<&MotionRecord>],
        frame: f32,
        world: &mut [Option<Matrix>],
        depth: usize,
    ) -> Matrix {
        if let Some(done) = world[index] {
            return done;
        }
        let mut local = self.bind_local[index];
        if let Some(Some(record)) = tracks.get(index) {
            if let Some(rotation) = record.rotation_at(frame) {
                let matrix = from_quaternion(rotation);
                for (row, values) in matrix.iter().take(3).enumerate() {
                    local[row][..3].copy_from_slice(&values[..3]);
                }
            }
            if let Some(position) = record.position_at(frame) {
                local[3][..3].copy_from_slice(&position);
            }
        }
        // A malformed hierarchy (a cycle) is cut instead of recursing forever.
        let result = match self.parents[index] {
            Some(parent) if depth < self.node_count() => {
                let parent_world = self.resolve(parent, tracks, frame, world, depth + 1);
                multiply(&local, &parent_world)
            }
            _ => local,
        };
        world[index] = Some(result);
        result
    }

    /// Matrix that moves a rigid mesh from its bind pose to its posed position:
    /// `inverse_bind * world`.
    #[must_use]
    pub fn rigid_matrix(&self, index: usize, posed_world: &[Matrix]) -> Matrix {
        multiply(&self.bind_inverse[index], &posed_world[index])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ModelNode;

    fn translation(x: f32, y: f32, z: f32) -> Matrix {
        let mut matrix = IDENTITY;
        matrix[3] = [x, y, z, 1.0];
        matrix
    }

    fn approx(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-4)
    }

    fn node(id: i32, parent_id: i32, name: &str, world: Matrix, inverse: Matrix) -> ModelNode {
        ModelNode {
            id,
            parent_id,
            name: name.to_owned(),
            is_bone: true,
            world,
            inverse,
        }
    }

    fn model(nodes: Vec<ModelNode>) -> ModelFile {
        ModelFile {
            version: 1,
            node_count: nodes.len() as u32,
            material_count: 0,
            bone_count: nodes.len() as u32,
            materials: Vec::new(),
            meshes: Vec::new(),
            bones: Vec::new(),
            nodes,
            unsupported_records: Vec::new(),
        }
    }

    #[test]
    fn quaternion_matrix_is_the_inverse_of_the_d3dx_one() {
        // D3DX would send +X to -Z for this quaternion; the ANM convention is the inverse.
        let half = std::f32::consts::FRAC_1_SQRT_2;
        let matrix = from_quaternion([0.0, half, 0.0, half]);
        assert!(approx(
            transform_vector(&matrix, [1.0, 0.0, 0.0]),
            [0.0, 0.0, 1.0]
        ));
        assert!(approx(
            transform_vector(&matrix, [0.0, 1.0, 0.0]),
            [0.0, 1.0, 0.0]
        ));
    }

    #[test]
    fn multiply_applies_the_left_matrix_first() {
        let moved = multiply(&translation(1.0, 0.0, 0.0), &translation(0.0, 2.0, 0.0));
        assert!(approx(
            transform_point(&moved, [0.0, 0.0, 0.0]),
            [1.0, 2.0, 0.0]
        ));
    }

    #[test]
    fn bind_pose_reproduces_the_stored_world_matrices() {
        let root_world = translation(0.0, 10.0, 0.0);
        let child_world = translation(0.0, 10.0, 5.0);
        let file = model(vec![
            node(1, 0, "child", child_world, translation(0.0, -10.0, -5.0)),
            node(0, -1, "root", root_world, translation(0.0, -10.0, 0.0)),
        ]);
        let skeleton = Skeleton::new(&file);
        let pose = skeleton.pose(&[None, None], 0.0);
        assert!(approx(
            transform_point(&pose[0], [0.0; 3]),
            [0.0, 10.0, 5.0]
        ));
        assert!(approx(
            transform_point(&pose[1], [0.0; 3]),
            [0.0, 10.0, 0.0]
        ));
        // Bind local translation of the child is its offset from the parent.
        assert!(approx(
            skeleton.bind_local[0][3][..3].try_into().unwrap(),
            [0.0, 0.0, 5.0]
        ));
    }

    #[test]
    fn a_rotation_track_swings_the_child_around_its_parent() {
        use crate::motion::{Keyframe24, MotionRecord};
        let file = model(vec![
            node(
                1,
                0,
                "child",
                translation(0.0, 0.0, 5.0),
                translation(0.0, 0.0, -5.0),
            ),
            node(0, -1, "root", IDENTITY, IDENTITY),
        ]);
        let skeleton = Skeleton::new(&file);
        let half = std::f32::consts::FRAC_1_SQRT_2;
        let record = MotionRecord {
            offset: 0,
            payload_size: 0,
            counters: [0; 5],
            candidate_name: "root".to_owned(),
            track_24: vec![Keyframe24 {
                tick: 0,
                frame_index: 0,
                value: [0.0, half, 0.0, half],
            }],
            track_20: Vec::new(),
            track_36: Vec::new(),
            morph_key_count: 0,
            morph_bytes: 0,
        };
        let pose = skeleton.pose(&[None, Some(&record)], 0.0);
        // The root turns a quarter about Y, so the child's local (0, 0, 5) offset maps to (-5, 0, 0).
        assert!(approx(
            transform_point(&pose[0], [0.0; 3]),
            [-5.0, 0.0, 0.0]
        ));
    }
}
