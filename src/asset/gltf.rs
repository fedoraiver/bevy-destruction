use bevy::{
    gltf::{Gltf, GltfMaterial, GltfMesh, GltfNode},
    mesh::{PrimitiveTopology, VertexAttributeValues},
    prelude::*,
};

use super::{DestructibleAsset, manifest::finite3};

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

pub(super) fn surface_role(extras: Option<&str>) -> Result<SurfaceRole, String> {
    let extras = extras.ok_or("显示材质缺少 bd_surface_role")?;
    let extras: serde_json::Value =
        serde_json::from_str(extras).map_err(|error| error.to_string())?;
    match extras
        .get("bd_surface_role")
        .and_then(serde_json::Value::as_str)
    {
        Some("exterior") => Ok(SurfaceRole::Exterior),
        Some("fracture") => Ok(SurfaceRole::Fracture),
        _ => Err("显示材质缺少有效的 bd_surface_role".into()),
    }
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
