use std::collections::{BTreeMap, BTreeSet};

use bevy::{math::DVec3, prelude::*};
use serde::Deserialize;

use super::patches::validate_bond_patches;

#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub schema: String,
    pub schema_version: u32,
    pub name: String,
    pub asset_id: String,
    pub coordinates: Coordinates,
    pub materials: BTreeMap<String, PhysicalMaterial>,
    pub chunks: Vec<ChunkRecord>,
    pub bonds: Vec<BondRecord>,
    pub anchors: Vec<AnchorRecord>,
    pub mesh_file: String,
    pub mesh_sha256: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Coordinates {
    pub units: String,
    pub up: String,
    pub handedness: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PhysicalMaterial {
    pub density: f32,
    pub color: [f32; 4],
    pub tension_strength: f32,
    pub compression_strength: f32,
    pub shear_strength: f32,
    pub young_modulus: f32,
    pub poisson_ratio: f32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChunkRecord {
    pub id: String,
    pub part_id: String,
    pub kind: String,
    pub material: String,
    pub node: String,
    pub position: [f32; 3],
    pub rotation: [f32; 4],
    pub volume: f32,
    pub mass: f32,
    pub center_of_mass: [f32; 3],
    pub inertia: [[f32; 3]; 3],
    pub colliders: Vec<ColliderRecord>,
}

impl ChunkRecord {
    pub fn transform(&self) -> Transform {
        Transform::from_translation(Vec3::from_array(self.position))
            .with_rotation(Quat::from_array(self.rotation))
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ColliderRecord {
    #[serde(rename = "type")]
    pub kind: String,
    pub vertices: Vec<[f32; 3]>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BondRecord {
    pub id: String,
    pub a: String,
    pub b: String,
    pub area: f32,
    pub center: [f32; 3],
    pub normal: [f32; 3],
    pub area_covariance: [[f32; 3]; 3],
    pub patches: Vec<BondPatchRecord>,
    pub state: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BondPatchRecord {
    pub area: f32,
    pub center: [f32; 3],
    pub normal: [f32; 3],
    pub area_covariance: [[f32; 3]; 3],
    pub vertices: Vec<[f32; 3]>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AnchorRecord {
    pub chunk: String,
    pub kind: String,
    pub position: [f32; 3],
}

impl Manifest {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != "bevy_destruction_asset" || self.schema_version != 2 {
            return Err("只支持 bevy_destruction_asset 格式 v2".into());
        }
        if self.coordinates.units != "meters"
            || self.coordinates.up != "+Y"
            || self.coordinates.handedness != "right"
        {
            return Err("资产必须采用米制、右手系、Y-up".into());
        }
        if self.name.is_empty() || !is_sha256(&self.asset_id) || !is_sha256(&self.mesh_sha256) {
            return Err("资产名称或 SHA256 标识无效".into());
        }
        if !self.mesh_file.ends_with(".glb")
            || self.mesh_file.contains(['/', '\\', ':', '#'])
            || self.mesh_file == ".glb"
        {
            return Err("mesh_file 必须为同目录的 GLB 文件名".into());
        }
        if self.chunks.is_empty() || self.chunks.len() > 2048 {
            return Err("碎块数量必须在 1..=2048 内".into());
        }
        for (id, material) in &self.materials {
            if id.is_empty()
                || ![
                    material.density,
                    material.tension_strength,
                    material.compression_strength,
                    material.shear_strength,
                    material.young_modulus,
                ]
                .into_iter()
                .all(positive)
                || !material
                    .color
                    .into_iter()
                    .all(|c| c.is_finite() && (0.0..=1.0).contains(&c))
                || !material.poisson_ratio.is_finite()
                || !(-1.0..0.5).contains(&material.poisson_ratio)
            {
                return Err(format!("材料 {id} 的物理参数无效"));
            }
        }
        let mut chunk_ids = BTreeSet::new();
        let mut node_ids = BTreeSet::new();
        for chunk in &self.chunks {
            if chunk.id.is_empty()
                || chunk.part_id.is_empty()
                || chunk.kind.is_empty()
                || chunk.node != chunk.id
                || !chunk_ids.insert(chunk.id.as_str())
                || !node_ids.insert(chunk.node.as_str())
                || !self.materials.contains_key(&chunk.material)
            {
                return Err(format!("碎块 {} 的 ID 或材料引用无效", chunk.id));
            }
            let rotation = Quat::from_array(chunk.rotation);
            if !finite3(chunk.position)
                || !finite3(chunk.center_of_mass)
                || !rotation.is_finite()
                || (rotation.length_squared() - 1.0).abs() > 1e-4
                || !positive(chunk.volume)
                || !positive(chunk.mass)
                || !valid_symmetric_tensor(chunk.inertia, true)
            {
                return Err(format!("碎块 {} 的位姿或质量数据无效", chunk.id));
            }
            let expected_mass = chunk.volume * self.materials[&chunk.material].density;
            if (chunk.mass - expected_mass).abs() > expected_mass * 1e-4 {
                return Err(format!("碎块 {} 的质量与密度不一致", chunk.id));
            }
            if chunk.colliders.is_empty()
                || chunk.colliders.iter().any(|collider| {
                    collider.kind != "convex_hull"
                        || collider.vertices.iter().any(|v| !finite3(*v))
                        || !has_volume(&collider.vertices)
                })
            {
                return Err(format!("碎块 {} 的凸碰撞代理无效", chunk.id));
            }
        }
        let mut bond_ids = BTreeSet::new();
        let mut bond_pairs = BTreeSet::new();
        let bounds = self
            .chunks
            .iter()
            .map(|chunk| {
                let transform = chunk.transform();
                let mut min = DVec3::splat(f64::INFINITY);
                let mut max = DVec3::splat(f64::NEG_INFINITY);
                for vertex in chunk.colliders.iter().flat_map(|proxy| &proxy.vertices) {
                    let point = transform.transform_point((*vertex).into()).as_dvec3();
                    min = min.min(point);
                    max = max.max(point);
                }
                (chunk.id.as_str(), (min, max))
            })
            .collect::<BTreeMap<_, _>>();
        for bond in &self.bonds {
            if bond.id.is_empty()
                || !bond_ids.insert(bond.id.as_str())
                || !bond_pairs.insert((bond.a.as_str(), bond.b.as_str()))
                || bond.a >= bond.b
                || !chunk_ids.contains(bond.a.as_str())
                || !chunk_ids.contains(bond.b.as_str())
                || !positive(bond.area)
                || !finite3(bond.center)
                || !finite3(bond.normal)
                || (Vec3::from_array(bond.normal).length_squared() - 1.0).abs() > 1e-3
                || !valid_symmetric_tensor(bond.area_covariance, false)
                || bond.state != "rigid"
            {
                return Err(format!("连接 {} 的端点或接触数据无效", bond.id));
            }
            if bond.patches.is_empty() {
                return Err(format!("v2 连接 {} 缺少真实接触面片", bond.id));
            }
            validate_bond_patches(bond, bounds[&*bond.a], bounds[&*bond.b])
                .map_err(|error| format!("连接 {}: {error}", bond.id))?;
        }
        let mut anchor_chunks = BTreeSet::new();
        for anchor in &self.anchors {
            if !chunk_ids.contains(anchor.chunk.as_str())
                || !anchor_chunks.insert(anchor.chunk.as_str())
                || anchor.kind != "fixed"
                || !finite3(anchor.position)
            {
                return Err(format!("锚点 {} 的引用或位置无效", anchor.chunk));
            }
        }
        Ok(())
    }
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|c| c.is_ascii_hexdigit())
}

pub(super) fn positive(value: f32) -> bool {
    value.is_finite() && value > 0.0
}

pub(super) fn finite3(value: [f32; 3]) -> bool {
    value.into_iter().all(f32::is_finite)
}

pub(super) fn valid_symmetric_tensor(rows: [[f32; 3]; 3], positive_definite: bool) -> bool {
    if !rows.into_iter().flatten().all(f32::is_finite) {
        return false;
    }
    let scale = rows.iter().flatten().map(|v| v.abs()).fold(0.0, f32::max);
    let tolerance = scale * 1e-5;
    for i in 0..3 {
        if rows[i][i] < 0.0 || (positive_definite && rows[i][i] <= 0.0) {
            return false;
        }
        for j in 0..i {
            if (rows[i][j] - rows[j][i]).abs() > tolerance {
                return false;
            }
            if rows[i][j].powi(2) > rows[i][i] * rows[j][j] + tolerance * scale {
                return false;
            }
        }
    }
    let determinant = Mat3::from_cols_array_2d(&rows).determinant();
    determinant >= -tolerance * scale * scale && (!positive_definite || determinant > 0.0)
}

fn has_volume(vertices: &[[f32; 3]]) -> bool {
    let Some(first) = vertices.first().copied().map(Vec3::from_array) else {
        return false;
    };
    let Some(edge) = vertices
        .iter()
        .map(|v| Vec3::from_array(*v) - first)
        .find(|v| v.length_squared() > 1e-12)
    else {
        return false;
    };
    let Some(normal) = vertices
        .iter()
        .map(|v| edge.cross(Vec3::from_array(*v) - first))
        .find(|v| v.length_squared() > 1e-16)
    else {
        return false;
    };
    let normal = normal.normalize();
    vertices
        .iter()
        .any(|v| normal.dot(Vec3::from_array(*v) - first).abs() > 1e-6)
}
