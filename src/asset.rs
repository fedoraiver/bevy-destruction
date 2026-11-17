use std::collections::{BTreeMap, BTreeSet};

use bevy::{
    asset::{AssetLoader, LoadContext, io::Reader},
    gltf::{Gltf, GltfMaterial, GltfMesh, GltfNode},
    math::{DVec2, DVec3},
    mesh::{PrimitiveTopology, VertexAttributeValues},
    prelude::*,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use thiserror::Error;

pub struct DestructibleAssetPlugin;

impl Plugin for DestructibleAssetPlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<DestructibleAsset>()
            .init_asset_loader::<DestructibleAssetLoader>();
    }
}

#[derive(Asset, TypePath, Debug)]
pub struct DestructibleAsset {
    pub manifest: Manifest,
    #[dependency]
    pub gltf: Handle<Gltf>,
}

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
    #[serde(default)]
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

#[derive(Default, TypePath)]
pub struct DestructibleAssetLoader;

#[derive(Debug, Error)]
pub enum DestructibleAssetError {
    #[error("读取建筑资产失败：{0}")]
    Io(#[from] std::io::Error),
    #[error("建筑清单 JSON 无效：{0}")]
    Json(#[from] serde_json::Error),
    #[error("建筑资产无效：{0}")]
    Invalid(String),
}

impl AssetLoader for DestructibleAssetLoader {
    type Asset = DestructibleAsset;
    type Settings = ();
    type Error = DestructibleAssetError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &(),
        load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        let manifest: Manifest = serde_json::from_slice(&bytes)?;
        manifest
            .validate()
            .map_err(DestructibleAssetError::Invalid)?;
        let mesh_path = load_context
            .path()
            .resolve_embed_str(&manifest.mesh_file)
            .map_err(|error| DestructibleAssetError::Invalid(error.to_string()))?;
        let mesh_bytes = load_context
            .read_asset_bytes(mesh_path.clone())
            .await
            .map_err(|error| DestructibleAssetError::Invalid(error.to_string()))?;
        validate_mesh_hash(&mesh_bytes, &manifest.mesh_sha256)
            .map_err(DestructibleAssetError::Invalid)?;
        let gltf = load_context.load(mesh_path);
        Ok(DestructibleAsset { manifest, gltf })
    }

    fn extensions(&self) -> &[&str] {
        &["destruction.json"]
    }
}

fn validate_mesh_hash(bytes: &[u8], expected: &str) -> Result<(), String> {
    let actual = format!("{:x}", Sha256::digest(bytes));
    if actual != expected.to_ascii_lowercase() {
        return Err("GLB 的 SHA256 与清单不匹配".into());
    }
    Ok(())
}

impl Manifest {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != "bevy_destruction_asset" || !matches!(self.schema_version, 1 | 2) {
            return Err("只支持 bevy_destruction_asset 格式 v1/v2".into());
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
            if self.schema_version == 2 && bond.patches.is_empty() {
                return Err(format!("v2 连接 {} 缺少真实接触面片", bond.id));
            }
            if !bond.patches.is_empty() {
                validate_bond_patches(bond, bounds[&*bond.a], bounds[&*bond.b])
                    .map_err(|error| format!("连接 {}: {error}", bond.id))?;
            }
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

const PATCH_POSITION_TOLERANCE: f64 = 4e-5;
const PATCH_METRIC_TOLERANCE: f64 = 0.002;

struct PatchMetrics {
    area: f64,
    center: DVec3,
    normal: DVec3,
    covariance: [[f64; 3]; 3],
}

fn patch_metrics(patch: &BondPatchRecord) -> Result<PatchMetrics, String> {
    if !(3..=1024).contains(&patch.vertices.len())
        || !positive(patch.area)
        || !finite3(patch.center)
        || !finite3(patch.normal)
        || !valid_symmetric_tensor(patch.area_covariance, false)
        || patch.vertices.iter().any(|point| !finite3(*point))
    {
        return Err("面片的顶点或物理量无效".into());
    }
    let normal = DVec3::from_array(patch.normal.map(f64::from));
    if (normal.length_squared() - 1.0).abs() > 1e-3 {
        return Err("面片法线不是单位向量".into());
    }
    let normal = normal.normalize();
    let points = patch
        .vertices
        .iter()
        .map(|point| DVec3::from_array(point.map(f64::from)))
        .collect::<Vec<_>>();
    let origin = points[0];
    let geometric_area = (1..points.len() - 1)
        .map(|index| (points[index] - origin).cross(points[index + 1] - origin))
        .sum::<DVec3>()
        * 0.5;
    if geometric_area.length() <= 1e-12 || geometric_area.normalize().dot(normal) < 1.0 - 1e-5 {
        return Err("面片法线与顶点几何方向不符".into());
    }
    let normal = geometric_area.normalize();
    if points
        .iter()
        .any(|point| normal.dot(*point - origin).abs() > PATCH_POSITION_TOLERANCE)
    {
        return Err("面片顶点不共面".into());
    }
    for (index, &a) in points.iter().enumerate() {
        let edge = points[(index + 1) % points.len()] - a;
        if edge.length_squared() <= 1e-20
            || points
                .iter()
                .any(|point| normal.dot(edge.cross(*point - a)) < -1e-10 * edge.length())
        {
            return Err("面片必须为按 a→b 法线绕序的凸多边形".into());
        }
    }
    let mut area = 0.0;
    let mut first = DVec3::ZERO;
    let mut triangles = Vec::new();
    for index in 1..points.len() - 1 {
        let a = points[index] - origin;
        let b = points[index + 1] - origin;
        let triangle_area = normal.dot(a.cross(b)) * 0.5;
        if triangle_area < -1e-12 {
            return Err("面片绕序无效".into());
        }
        area += triangle_area;
        first += (a + b) * (triangle_area / 3.0);
        triangles.push((a, b, triangle_area));
    }
    if area <= 1e-12 {
        return Err("面片没有可解析的二维面积".into());
    }
    let relative_center = first / area;
    let center = origin + relative_center;
    let mut covariance = [[0.0; 3]; 3];
    for (a, b, triangle_area) in triangles {
        let points = [-relative_center, a - relative_center, b - relative_center];
        let sum = points.iter().copied().sum::<DVec3>();
        for row in 0..3 {
            for column in 0..3 {
                covariance[row][column] += triangle_area / 12.0
                    * (sum[row] * sum[column]
                        + points
                            .iter()
                            .map(|point| point[row] * point[column])
                            .sum::<f64>());
            }
        }
    }
    if (area - f64::from(patch.area)).abs() > (area * PATCH_METRIC_TOLERANCE).max(1e-12)
        || center.distance(DVec3::from_array(patch.center.map(f64::from)))
            > PATCH_POSITION_TOLERANCE
        || !covariance_matches(&covariance, &patch.area_covariance)
    {
        return Err("面片面积、中心或二阶矩与真实顶点不符".into());
    }
    let tangent = normal.any_orthonormal_vector();
    let bitangent = normal.cross(tangent);
    let quadratic = |a: DVec3, b: DVec3| {
        (0..3)
            .map(|row| {
                (0..3)
                    .map(|column| {
                        a[row] * f64::from(patch.area_covariance[row][column]) * b[column]
                    })
                    .sum::<f64>()
            })
            .sum::<f64>()
    };
    let qt = quadratic(tangent, tangent);
    let qb = quadratic(bitangent, bitangent);
    let qtb = quadratic(tangent, bitangent);
    if qt <= 0.0 || qb <= 0.0 || qt * qb - qtb * qtb <= 0.0 {
        return Err("面片二阶矩没有可解析的二维截面".into());
    }
    Ok(PatchMetrics {
        area,
        center,
        normal,
        covariance,
    })
}

fn covariance_matches(expected: &[[f64; 3]; 3], actual: &[[f32; 3]; 3]) -> bool {
    let scale = expected
        .iter()
        .flatten()
        .copied()
        .map(f64::abs)
        .fold(0.0, f64::max);
    expected
        .iter()
        .flatten()
        .zip(actual.iter().flatten())
        .all(|(expected, actual)| {
            (expected - f64::from(*actual)).abs() <= (scale * PATCH_METRIC_TOLERANCE).max(1e-24)
        })
}

fn validate_bond_patches(
    bond: &BondRecord,
    bounds_a: (DVec3, DVec3),
    bounds_b: (DVec3, DVec3),
) -> Result<(), String> {
    if bond.patches.len() > 8192 {
        return Err("接触面片数量超过 8192".into());
    }
    let mut metrics = Vec::with_capacity(bond.patches.len());
    for patch in &bond.patches {
        let metric = patch_metrics(patch)?;
        for vertex in &patch.vertices {
            let point = DVec3::from_array(vertex.map(f64::from));
            if [bounds_a, bounds_b].into_iter().any(|(min, max)| {
                point
                    .cmplt(min - DVec3::splat(PATCH_POSITION_TOLERANCE))
                    .any()
                    || point
                        .cmpgt(max + DVec3::splat(PATCH_POSITION_TOLERANCE))
                        .any()
            }) {
                return Err("接触面片超出端点碰撞几何范围".into());
            }
        }
        metrics.push(metric);
    }
    for (index, first) in bond.patches.iter().enumerate() {
        for second in &bond.patches[..index] {
            if coplanar_overlap(first, second)
                > f64::from(first.area.min(second.area)) * PATCH_METRIC_TOLERANCE
            {
                return Err("同一连接的接触面片重复或重叠".into());
            }
        }
    }
    let area = metrics.iter().map(|metric| metric.area).sum::<f64>();
    let center = metrics
        .iter()
        .map(|metric| metric.center * metric.area)
        .sum::<DVec3>()
        / area;
    let normal = metrics
        .iter()
        .map(|metric| metric.normal * metric.area)
        .sum::<DVec3>();
    if normal.length() <= area * 1e-6 {
        return Err("接触面片法线近似相消".into());
    }
    let mut covariance = [[0.0; 3]; 3];
    for metric in &metrics {
        let offset = metric.center - center;
        for row in 0..3 {
            for column in 0..3 {
                covariance[row][column] +=
                    metric.covariance[row][column] + metric.area * offset[row] * offset[column];
            }
        }
    }
    if (area - f64::from(bond.area)).abs() > area * PATCH_METRIC_TOLERANCE
        || center.distance(DVec3::from_array(bond.center.map(f64::from))) > PATCH_POSITION_TOLERANCE
        || normal
            .normalize()
            .distance(DVec3::from_array(bond.normal.map(f64::from)))
            > 1e-3
        || !covariance_matches(&covariance, &bond.area_covariance)
    {
        return Err("连接汇总数据与真实面片不符".into());
    }
    Ok(())
}

fn coplanar_overlap(first: &BondPatchRecord, second: &BondPatchRecord) -> f64 {
    let normal = DVec3::from_array(first.normal.map(f64::from)).normalize();
    let origin = DVec3::from_array(first.vertices[0].map(f64::from));
    if normal
        .dot(DVec3::from_array(second.normal.map(f64::from)).normalize())
        .abs()
        < 1.0 - 1e-8
        || second.vertices.iter().any(|point| {
            normal
                .dot(DVec3::from_array(point.map(f64::from)) - origin)
                .abs()
                > 1e-6
        })
    {
        return 0.0;
    }
    let tangent = normal.any_orthonormal_vector();
    let bitangent = normal.cross(tangent);
    let project = |point: &[f32; 3]| {
        let point = DVec3::from_array(point.map(f64::from)) - origin;
        DVec2::new(point.dot(tangent), point.dot(bitangent))
    };
    let boundary = first.vertices.iter().map(project).collect::<Vec<_>>();
    let mut polygon = second.vertices.iter().map(project).collect::<Vec<_>>();
    for index in 0..boundary.len() {
        if polygon.is_empty() {
            return 0.0;
        }
        let a = boundary[index];
        let edge = boundary[(index + 1) % boundary.len()] - a;
        let mut output = Vec::new();
        let mut previous = *polygon.last().unwrap();
        let mut previous_side = edge.perp_dot(previous - a);
        for &point in &polygon {
            let side = edge.perp_dot(point - a);
            if (side >= 0.0) != (previous_side >= 0.0) {
                output.push(previous.lerp(point, previous_side / (previous_side - side)));
            }
            if side >= 0.0 {
                output.push(point);
            }
            previous = point;
            previous_side = side;
        }
        polygon = output;
    }
    if polygon.len() < 3 {
        return 0.0;
    }
    let origin = polygon[0];
    polygon
        .iter()
        .zip(polygon.iter().cycle().skip(1))
        .take(polygon.len())
        .map(|(a, b)| (*a - origin).perp_dot(*b - origin))
        .sum::<f64>()
        .abs()
        * 0.5
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|c| c.is_ascii_hexdigit())
}

fn positive(value: f32) -> bool {
    value.is_finite() && value > 0.0
}

fn finite3(value: [f32; 3]) -> bool {
    value.into_iter().all(f32::is_finite)
}

fn valid_symmetric_tensor(rows: [[f32; 3]; 3], positive_definite: bool) -> bool {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceRole {
    Exterior,
    Fracture,
}

pub struct ResolvedPrimitive {
    pub mesh: Handle<Mesh>,
    pub material: Option<Handle<GltfMaterial>>,
    pub surface_role: SurfaceRole,
}

pub struct ResolvedChunk {
    pub record_index: usize,
    pub primitives: Vec<ResolvedPrimitive>,
    pub triangles: Vec<[Vec3; 3]>,
}

pub struct ResolvedBuilding {
    pub chunks: Vec<ResolvedChunk>,
    pub bounds_min: Vec3,
    pub bounds_max: Vec3,
}

pub fn resolve_building(
    asset: &DestructibleAsset,
    gltfs: &Assets<Gltf>,
    nodes: &Assets<GltfNode>,
    gltf_meshes: &Assets<GltfMesh>,
    meshes: &Assets<Mesh>,
    materials: &Assets<GltfMaterial>,
) -> Result<ResolvedBuilding, String> {
    let gltf = gltfs.get(&asset.gltf).ok_or("GLB 依赖尚未加载")?;
    let legacy_interior = gltf.named_materials.get("BD_DestructionInterior");
    let mut result = ResolvedBuilding {
        chunks: Vec::with_capacity(asset.manifest.chunks.len()),
        bounds_min: Vec3::splat(f32::INFINITY),
        bounds_max: Vec3::splat(f32::NEG_INFINITY),
    };
    for (record_index, chunk) in asset.manifest.chunks.iter().enumerate() {
        let node = gltf
            .named_nodes
            .get(chunk.node.as_str())
            .and_then(|handle| nodes.get(handle))
            .ok_or_else(|| format!("缺少碎块 GLB 节点 {}", chunk.node))?;
        let extras: serde_json::Value = serde_json::from_str(
            &node
                .extras
                .as_ref()
                .ok_or_else(|| format!("节点 {} 缺少稳定 ID", chunk.id))?
                .value,
        )
        .map_err(|error| error.to_string())?;
        if extras["bd_chunk_id"].as_str() != Some(chunk.id.as_str())
            || extras["bd_part_id"].as_str() != Some(chunk.part_id.as_str())
            || !node.children.is_empty()
            || node
                .transform
                .translation
                .distance(Vec3::from_array(chunk.position))
                > 2e-4
            || node.transform.scale.distance(Vec3::ONE) > 1e-5
            || node
                .transform
                .rotation
                .angle_between(Quat::from_array(chunk.rotation))
                .abs()
                > 1e-4
        {
            return Err(format!("节点 {} 的标识或位姿与清单不匹配", chunk.id));
        }
        let gltf_mesh = node
            .mesh
            .as_ref()
            .and_then(|handle| gltf_meshes.get(handle))
            .ok_or_else(|| format!("节点 {} 缺少显示网格", chunk.id))?;
        if gltf_mesh.primitives.is_empty() {
            return Err(format!("碎块 {} 没有 primitive", chunk.id));
        }
        let mut resolved = ResolvedChunk {
            record_index,
            primitives: Vec::new(),
            triangles: Vec::new(),
        };
        for primitive in &gltf_mesh.primitives {
            let mesh = meshes.get(&primitive.mesh).ok_or("显示网格依赖缺失")?;
            if primitive
                .material
                .as_ref()
                .is_some_and(|handle| materials.get(handle).is_none())
            {
                return Err("GLB 材料依赖缺失".into());
            }
            let surface_role = surface_role(
                primitive
                    .material_extras
                    .as_ref()
                    .map(|extras| extras.value.as_str()),
                primitive
                    .material
                    .as_ref()
                    .is_some_and(|handle| Some(handle) == legacy_interior),
            )?;
            let triangles = mesh_triangles(mesh)?;
            for position in triangles.iter().flatten() {
                let position = chunk.transform().transform_point(*position);
                result.bounds_min = result.bounds_min.min(position);
                result.bounds_max = result.bounds_max.max(position);
            }
            resolved.triangles.extend(triangles);
            resolved.primitives.push(ResolvedPrimitive {
                mesh: primitive.mesh.clone(),
                material: primitive.material.clone(),
                surface_role,
            });
        }
        result.chunks.push(resolved);
    }
    Ok(result)
}

fn surface_role(extras: Option<&str>, legacy_interior: bool) -> Result<SurfaceRole, String> {
    if let Some(extras) = extras {
        let extras: serde_json::Value =
            serde_json::from_str(extras).map_err(|error| error.to_string())?;
        if let Some(role) = extras.get("bd_surface_role") {
            return match role.as_str() {
                Some("exterior") => Ok(SurfaceRole::Exterior),
                Some("fracture") => Ok(SurfaceRole::Fracture),
                _ => Err("未知的 bd_surface_role".into()),
            };
        }
    }
    Ok(if legacy_interior {
        SurfaceRole::Fracture
    } else {
        SurfaceRole::Exterior
    })
}

pub fn mesh_triangles(mesh: &Mesh) -> Result<Vec<[Vec3; 3]>, String> {
    if mesh.primitive_topology() != PrimitiveTopology::TriangleList {
        return Err("显示网格必须使用三角面列表".into());
    }
    let Some(VertexAttributeValues::Float32x3(positions)) =
        mesh.attribute(Mesh::ATTRIBUTE_POSITION)
    else {
        return Err("显示网格缺少 Float32x3 POSITION".into());
    };
    if positions.iter().any(|position| !finite3(*position)) {
        return Err("显示网格含非有限顶点".into());
    }
    let indices: Vec<usize> = match mesh.indices() {
        Some(indices) => indices.iter().collect(),
        None => (0..positions.len()).collect(),
    };
    if indices.is_empty()
        || !indices.len().is_multiple_of(3)
        || indices.iter().any(|i| *i >= positions.len())
    {
        return Err("显示网格的三角面索引无效".into());
    }
    Ok(indices
        .as_chunks::<3>()
        .0
        .iter()
        .map(|triangle| {
            [
                Vec3::from_array(positions[triangle[0]]),
                Vec3::from_array(positions[triangle[1]]),
                Vec3::from_array(positions[triangle[2]]),
            ]
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Manifest {
        serde_json::from_value(serde_json::json!({
            "schema": "bevy_destruction_asset", "schema_version": 1,
            "name": "test", "asset_id": "a".repeat(64),
            "coordinates": { "units": "meters", "up": "+Y", "handedness": "right" },
            "materials": { "concrete": { "density": 2400.0, "color": [0.5,0.5,0.5,1.0],
                "tension_strength": 3e6, "compression_strength": 30e6,
                "shear_strength": 4e6, "young_modulus": 30e9, "poisson_ratio": 0.2 } },
            "chunks": [{ "id": "a", "part_id": "wall", "kind": "wall", "material": "concrete", "node": "a",
                "position": [1.0,2.0,3.0], "rotation": [0.0,0.0,0.0,1.0], "volume": 1.0, "mass": 2400.0,
                "center_of_mass": [0.0,0.0,0.0], "inertia": [[1.0,0.0,0.0],[0.0,1.0,0.0],[0.0,0.0,1.0]],
                "colliders": [{ "type": "convex_hull", "vertices": [[0.0,0.0,0.0],[1.0,0.0,0.0],[0.0,1.0,0.0],[0.0,0.0,1.0]] }] }],
            "bonds": [], "anchors": [{ "chunk": "a", "kind": "fixed", "position": [1.0,0.0,3.0] }],
            "mesh_file": "test.glb", "mesh_sha256": "b".repeat(64)
        })).unwrap()
    }

    #[test]
    fn validates_units_ids_references_and_geometry() {
        let mut asset = fixture();
        assert!(asset.validate().is_ok());
        asset.coordinates.up = "+Z".into();
        assert!(asset.validate().is_err());
        asset = fixture();
        asset.chunks.push(asset.chunks[0].clone());
        assert!(asset.validate().is_err());
        asset = fixture();
        asset.anchors[0].chunk = "missing".into();
        assert!(asset.validate().is_err());
        asset = fixture();
        asset.chunks[0].colliders[0].vertices[3] = [0.5, 0.5, 0.0];
        assert!(asset.validate().is_err());
    }

    #[test]
    fn rejects_bad_mass_quaternions_paths_and_tensors() {
        for mutation in [
            |asset: &mut Manifest| asset.chunks[0].mass = 2.0,
            |asset: &mut Manifest| asset.chunks[0].rotation = [0.0; 4],
            |asset: &mut Manifest| asset.chunks[0].inertia[0][1] = 2.0,
            |asset: &mut Manifest| asset.mesh_file = "../escape.glb".into(),
            |asset: &mut Manifest| asset.chunks[0].position[0] = f32::NAN,
        ] {
            let mut asset = fixture();
            mutation(&mut asset);
            assert!(asset.validate().is_err());
        }
    }

    fn multipatch_fixture() -> Manifest {
        let mut asset = fixture();
        asset.schema_version = 2;
        let mut a = asset.chunks[0].clone();
        a.position = [0.0; 3];
        a.colliders[0].vertices = [-1.0, 0.0]
            .into_iter()
            .flat_map(|x| {
                [0.0, 1.0]
                    .into_iter()
                    .flat_map(move |y| [0.0, 1.0].into_iter().map(move |z| [x, y, z]))
            })
            .collect();
        let mut b = a.clone();
        b.id = "b".into();
        b.node = "b".into();
        for point in &mut b.colliders[0].vertices {
            point[0] += 1.0;
        }
        asset.chunks = vec![a, b];
        asset.bonds = vec![BondRecord {
            id: "ab".into(),
            a: "a".into(),
            b: "b".into(),
            area: 1.0,
            center: [0.0, 0.5, 0.5],
            normal: [1.0, 0.0, 0.0],
            area_covariance: [
                [0.0, 0.0, 0.0],
                [0.0, 1.0 / 12.0, 0.0],
                [0.0, 0.0, 1.0 / 12.0],
            ],
            patches: [0.0, 0.5]
                .into_iter()
                .map(|y| BondPatchRecord {
                    area: 0.5,
                    center: [0.0, y + 0.25, 0.5],
                    normal: [1.0, 0.0, 0.0],
                    area_covariance: [
                        [0.0, 0.0, 0.0],
                        [0.0, 1.0 / 96.0, 0.0],
                        [0.0, 0.0, 1.0 / 24.0],
                    ],
                    vertices: vec![
                        [0.0, y, 0.0],
                        [0.0, y + 0.5, 0.0],
                        [0.0, y + 0.5, 1.0],
                        [0.0, y, 1.0],
                    ],
                })
                .collect(),
            state: "rigid".into(),
        }];
        asset
    }

    #[test]
    fn v2_patch_metrics_and_parent_parallel_axis_aggregation_are_validated() {
        let asset = multipatch_fixture();
        asset.validate().unwrap();
        for mutate in [
            |asset: &mut Manifest| asset.bonds[0].patches.clear(),
            |asset: &mut Manifest| asset.bonds[0].patches[0].area *= 2.0,
            |asset: &mut Manifest| asset.bonds[0].patches[0].center[1] += 0.1,
            |asset: &mut Manifest| asset.bonds[0].patches[0].area_covariance[1][1] *= 2.0,
            |asset: &mut Manifest| asset.bonds[0].area *= 1.5,
            |asset: &mut Manifest| asset.bonds[0].center[1] += 0.1,
            |asset: &mut Manifest| asset.bonds[0].area_covariance[1][1] *= 2.0,
            |asset: &mut Manifest| asset.bonds[0].normal = [0.0, 1.0, 0.0],
        ] {
            let mut asset = asset.clone();
            mutate(&mut asset);
            assert!(asset.validate().is_err());
        }
        let legacy = fixture();
        assert_eq!(legacy.schema_version, 1);
        legacy.validate().unwrap();
    }

    #[test]
    fn rejects_nonplanar_reversed_concave_duplicated_and_out_of_bounds_patches() {
        for mutate in [
            |asset: &mut Manifest| asset.bonds[0].patches[0].vertices[0][0] = 0.1,
            |asset: &mut Manifest| asset.bonds[0].patches[0].vertices.reverse(),
            |asset: &mut Manifest| asset.bonds[0].patches[0].vertices[1] = [0.0, 0.1, 0.5],
            |asset: &mut Manifest| {
                asset.bonds[0].patches[1] = asset.bonds[0].patches[0].clone();
            },
            |asset: &mut Manifest| {
                for patch in &mut asset.bonds[0].patches {
                    patch.center[0] += 2.0;
                    for vertex in &mut patch.vertices {
                        vertex[0] += 2.0;
                    }
                }
                asset.bonds[0].center[0] += 2.0;
            },
        ] {
            let mut asset = multipatch_fixture();
            mutate(&mut asset);
            assert!(asset.validate().is_err());
        }
    }

    #[test]
    fn small_patch_cannot_hide_an_incorrect_normal_inside_position_tolerance() {
        let patch = BondPatchRecord {
            area: 1.6e-8,
            center: [0.0, 1e-5, 0.0005],
            normal: [0.8, 0.6, 0.0],
            area_covariance: [
                [0.0, 0.0, 0.0],
                [0.0, 1.6e-8 * 4e-10 / 12.0, 0.0],
                [0.0, 0.0, 1.6e-8 * 1e-6 / 12.0],
            ],
            vertices: vec![
                [0.0, 0.0, 0.0],
                [0.0, 2e-5, 0.0],
                [0.0, 2e-5, 0.001],
                [0.0, 0.0, 0.001],
            ],
        };
        assert!(patch_metrics(&patch).is_err());
    }

    #[test]
    fn rejects_mesh_hash_mismatch() {
        let bytes = b"test GLB contents";
        let digest = format!("{:x}", Sha256::digest(bytes));
        assert!(validate_mesh_hash(bytes, &digest).is_ok());
        assert!(validate_mesh_hash(b"modified", &digest).is_err());
    }

    #[test]
    fn surface_roles_use_explicit_extras_and_legacy_name() {
        assert_eq!(
            surface_role(Some(r#"{"bd_surface_role":"fracture"}"#), false).unwrap(),
            SurfaceRole::Fracture
        );
        assert_eq!(
            surface_role(Some(r#"{"bd_surface_role":"exterior"}"#), true).unwrap(),
            SurfaceRole::Exterior
        );
        assert_eq!(surface_role(None, true).unwrap(), SurfaceRole::Fracture);
        assert!(surface_role(Some(r#"{"bd_surface_role":"unknown"}"#), false).is_err());
    }

    #[test]
    fn triangles_support_indexed_and_nonindexed_primitives() {
        use bevy::{asset::RenderAssetUsages, mesh::Indices};
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        );
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_POSITION,
            vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        );
        assert_eq!(mesh_triangles(&mesh).unwrap().len(), 1);
        mesh.insert_indices(Indices::U16(vec![2, 1, 0]));
        assert_eq!(mesh_triangles(&mesh).unwrap()[0][0], Vec3::Y);
        mesh.insert_indices(Indices::U16(vec![0, 1, 9]));
        assert!(mesh_triangles(&mesh).is_err());
    }

    #[test]
    fn bundled_buildings_match_manifests_and_all_material_primitives() {
        let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/buildings");
        for name in ["rectangle", "l_shape", "courtyard"] {
            let manifest: Manifest = serde_json::from_slice(
                &std::fs::read(directory.join(format!("{name}.destruction.json"))).unwrap(),
            )
            .unwrap();
            manifest
                .validate()
                .unwrap_or_else(|error| panic!("{name}: {error}"));
            let bytes = std::fs::read(directory.join(&manifest.mesh_file)).unwrap();
            validate_mesh_hash(&bytes, &manifest.mesh_sha256).unwrap();
            let word =
                |offset: usize| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
            assert_eq!(word(0), 0x46546c67);
            assert_eq!(word(4), 2);
            assert_eq!(word(8) as usize, bytes.len());
            assert_eq!(word(16), 0x4e4f534a);
            let document: serde_json::Value =
                serde_json::from_slice(&bytes[20..20 + word(12) as usize]).unwrap();
            let nodes = document["nodes"].as_array().unwrap();
            assert_eq!(nodes.len(), manifest.chunks.len());
            let nodes: BTreeMap<_, _> = nodes
                .iter()
                .map(|node| (node["name"].as_str().unwrap(), node))
                .collect();
            assert_eq!(nodes.len(), manifest.chunks.len());
            let mut primitive_count = 0;
            for chunk in &manifest.chunks {
                let node = nodes[chunk.node.as_str()];
                assert_eq!(node["extras"]["bd_chunk_id"], chunk.id);
                assert_eq!(node["extras"]["bd_part_id"], chunk.part_id);
                let position: [f32; 3] =
                    serde_json::from_value(node["translation"].clone()).unwrap();
                assert!(
                    Vec3::from_array(position).distance(Vec3::from_array(chunk.position)) < 2e-4
                );
                let mesh = &document["meshes"][node["mesh"].as_u64().unwrap() as usize];
                let primitives = mesh["primitives"].as_array().unwrap();
                assert!(!primitives.is_empty());
                primitive_count += primitives.len();
                for primitive in primitives {
                    assert!(primitive["attributes"]["POSITION"].is_u64());
                    assert!(primitive["attributes"]["NORMAL"].is_u64());
                    let material =
                        &document["materials"][primitive["material"].as_u64().unwrap() as usize];
                    let role = material["extras"]["bd_surface_role"].as_str().unwrap();
                    assert!(matches!(role, "exterior" | "fracture"));
                }
            }
            assert!(primitive_count > manifest.chunks.len());
            assert!(!manifest.anchors.is_empty());
            assert!(!manifest.bonds.is_empty());
        }
    }
}
