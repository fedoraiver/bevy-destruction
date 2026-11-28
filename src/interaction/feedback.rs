use bevy::prelude::*;

use crate::{
    asset::Manifest,
    fracture_detail::ContactPatch,
    scene::{BuildingInstance, ChunkInstance},
    vfx::{BreakBurst, EmissionSurface},
};

pub(crate) fn build_bond_feedback<'a>(
    instance: &BuildingInstance,
    manifest: &Manifest,
    broken: &[String],
    chunk_at: impl Fn(Entity) -> (&'a ChunkInstance, Option<Entity>),
    patches: impl Iterator<Item = &'a ContactPatch>,
    world_pose: impl Fn(Option<Entity>) -> Transform,
) -> Option<BreakBurst> {
    let bond = manifest
        .bonds
        .iter()
        .find(|bond| broken.contains(&bond.id))?;
    let (chunk, body) = chunk_at(instance.chunks[&bond.a]);
    let pose = world_pose(body);
    let mut surfaces = Vec::new();
    let mut area = 0.0;
    for patch in patches.filter(|patch| broken.contains(&patch.bond_id)) {
        let (_, body) = chunk_at(instance.chunks[&patch.a]);
        let pose = world_pose(body);
        area += patch.area;
        surfaces.extend(patch.triangles.iter().map(|triangle| EmissionSurface {
            triangle: triangle.map(|point| pose.transform_point(point)),
            normal: pose.rotation * patch.normal,
        }));
    }
    surfaces.truncate(256);
    Some(BreakBurst {
        point: pose.transform_point(bond.center.into()),
        normal: pose.rotation * Vec3::from(bond.normal),
        seed: broken
            .iter()
            .flat_map(|id| id.bytes())
            .fold(0xcbf29ce484222325u64, |hash, byte| {
                (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
            }),
        material: chunk.material.clone(),
        removed_volume: 0.0,
        exposed_area: area,
        surfaces,
    })
}
