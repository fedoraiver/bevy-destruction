mod geometry;

use std::collections::BTreeMap;

use avian3d::prelude::{Collider, ColliderAabb, SimpleCollider};
use bevy::{math::Affine3A, prelude::*};

use crate::asset::DestructibleAsset;

use geometry::{cast, local_ray, ray_bounds, source_bounds, source_collider};

pub type ChunkKey = (Entity, String);

pub struct VisibilityChunk<'a> {
    pub key: ChunkKey,
    pub asset: AssetId<DestructibleAsset>,
    /// Immutable source triangles in building-rest coordinates. Invalidate the asset on edits.
    pub triangles: &'a [[Vec3; 3]],
    pub world_from_rest: Affine3A,
    pub visible: bool,
}

pub struct VisibilityOccluder<'a> {
    pub collider: &'a Collider,
    /// The collider already contains its physics scale; do not apply that scale twice.
    pub world_from_local: Affine3A,
}

#[derive(Clone, Debug)]
pub struct SurfaceHit {
    pub key: ChunkKey,
    pub point: Vec3,
}

#[derive(Default)]
pub struct ForceVisibilityCache {
    geometry: BTreeMap<(AssetId<DestructibleAsset>, String), Option<SourceGeometry>>,
}

struct SourceGeometry {
    bounds: ColliderAabb,
    collider: Option<Collider>,
}

impl ForceVisibilityCache {
    pub fn invalidate(&mut self, asset: AssetId<DestructibleAsset>) {
        self.geometry.retain(|(id, _), _| *id != asset);
    }

    /// Picks the nearest rendered source surface, not a collision proxy or a chunk center.
    /// Coincident surfaces and malformed visible geometry conservatively yield no selection.
    pub fn pick(
        &mut self,
        ray: Ray3d,
        max_distance: f32,
        chunks: &[VisibilityChunk<'_>],
        extras: &[VisibilityOccluder<'_>],
    ) -> Option<SurfaceHit> {
        if !ray.origin.is_finite()
            || !ray.direction.is_finite()
            || !max_distance.is_finite()
            || max_distance <= 0.0
        {
            return None;
        }
        let mut hits = Vec::new();
        let mut nearest_distance = max_distance;
        for chunk in chunks.iter().filter(|chunk| chunk.visible) {
            let local_ray = local_ray(ray, chunk.world_from_rest)?;
            let key = (chunk.asset, chunk.key.1.clone());
            let cached = self.geometry.entry(key).or_insert_with(|| {
                source_bounds(chunk.triangles).map(|bounds| SourceGeometry {
                    bounds,
                    collider: None,
                })
            });
            let geometry = cached.as_mut()?;
            let limit = (nearest_distance + hit_epsilon(nearest_distance)).min(max_distance);
            if !ray_bounds(geometry.bounds, &local_ray, limit) {
                continue;
            }
            if geometry.collider.is_none() {
                geometry.collider = source_collider(chunk.triangles);
                if geometry.collider.is_none() {
                    *cached = None;
                    return None;
                }
            }
            let Some(distance) = cast(geometry.collider.as_ref().unwrap(), &local_ray, limit)
            else {
                continue;
            };
            nearest_distance = nearest_distance.min(distance);
            hits.push((&chunk.key, distance));
        }
        hits.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(b.0)));
        let &(key, distance) = hits.first()?;
        if hits.iter().skip(1).any(|(other, other_distance)| {
            *other != key
                && *other_distance - distance <= hit_epsilon(distance.max(*other_distance))
        }) {
            return None;
        }
        let limit = (distance + hit_epsilon(distance)).min(max_distance);
        for extra in extras {
            let local_ray = local_ray(ray, extra.world_from_local)?;
            let bounds = extra.collider.aabb(Vec3::ZERO, Quat::IDENTITY, 0.0);
            if !bounds.min.is_finite() || !bounds.max.is_finite() {
                return None;
            }
            if ray_bounds(bounds, &local_ray, limit)
                && cast(extra.collider, &local_ray, limit).is_some()
            {
                return None;
            }
        }
        Some(SurfaceHit {
            key: key.clone(),
            point: ray.get_point(distance),
        })
    }
}

fn hit_epsilon(distance: f32) -> f32 {
    8.0 * f32::EPSILON * distance.abs().max(1.0)
}

#[cfg(test)]
mod tests;
