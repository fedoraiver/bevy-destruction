use std::collections::{BTreeMap, BTreeSet};

use avian3d::prelude::{Collider, SimpleCollider};
use bevy::{math::Affine3A, prelude::*};

use crate::asset::DestructibleAsset;

pub type ChunkKey = (Entity, String);

pub struct VisibilityCamera {
    pub clip_from_world: Mat4,
    pub view_from_world: Affine3A,
    pub near: f32,
    pub far: f32,
}

pub struct VisibilityChunk<'a> {
    pub key: ChunkKey,
    pub asset: AssetId<DestructibleAsset>,
    /// Immutable source triangles in building-rest coordinates. Invalidate the asset on edits.
    pub triangles: &'a [[Vec3; 3]],
    pub world_from_rest: Affine3A,
    pub visible: bool,
}

pub struct VisibilityOccluder<'a> {
    pub entity: Entity,
    pub collider: &'a Collider,
    /// The collider already contains its physics scale; do not apply that scale twice.
    pub world_from_local: Affine3A,
    pub revision: u64,
}

#[derive(Clone, Debug, Default)]
pub struct VisibilityStats {
    /// Actual BVH/shape ray queries, including queries against the target itself.
    pub rays: usize,
    pub cache_hits: usize,
    pub geometry_builds: usize,
    pub tested_chunks: usize,
    pub pending_chunks: usize,
    pub frustum_rejected: usize,
    pub invalid_chunks: usize,
}

#[derive(Clone, Debug, Default)]
pub struct VisibilityResult {
    pub visible: BTreeSet<ChunkKey>,
    pub stats: VisibilityStats,
}

#[derive(Resource)]
pub struct ForceVisibilityCache {
    pub max_samples_per_chunk: usize,
    pub max_rays: usize,
    geometry: BTreeMap<(AssetId<DestructibleAsset>, String), Option<SourceGeometry>>,
    generation: u64,
    frame: Option<FrameMemo>,
}

impl Default for ForceVisibilityCache {
    fn default() -> Self {
        Self {
            max_samples_per_chunk: 64,
            max_rays: 8192,
            geometry: BTreeMap::new(),
            generation: 0,
            frame: None,
        }
    }
}

struct SourceGeometry {
    collider: Collider,
    bounds: Bounds,
}

#[derive(Clone, Copy)]
struct Bounds {
    min: Vec3,
    max: Vec3,
}

struct Body<'a> {
    collider: &'a Collider,
    bounds: Bounds,
    local_from_world: Affine3A,
}

#[derive(PartialEq, Eq)]
struct SceneStamp {
    camera: Vec<u32>,
    generation: u64,
    samples: usize,
    budget: usize,
    chunks: Vec<ChunkStamp>,
    extras: Vec<(Entity, [u32; 12], u64)>,
}

#[derive(PartialEq, Eq)]
struct ChunkStamp {
    key: ChunkKey,
    asset: AssetId<DestructibleAsset>,
    pose: [u32; 12],
    visible: bool,
    triangles: usize,
}

struct FrameMemo {
    stamp: SceneStamp,
    classified: BTreeSet<ChunkKey>,
    visible: BTreeSet<ChunkKey>,
    next_sample: BTreeMap<ChunkKey, usize>,
}

struct Target {
    index: usize,
    area: f32,
    samples: Vec<Vec3>,
}

enum RayHit {
    Miss,
    Hit(f32),
    Budget,
}

impl ForceVisibilityCache {
    pub fn invalidate(&mut self, asset: AssetId<DestructibleAsset>) {
        self.geometry.retain(|(id, _), _| *id != asset);
        self.generation = self.generation.wrapping_add(1);
        self.frame = None;
    }

    /// Positive visibility witnesses from a bounded set of real surface points.
    /// Very small exposed slivers may be missed; this is not a per-pixel ID pass.
    pub fn update(
        &mut self,
        enabled: bool,
        camera: &VisibilityCamera,
        chunks: &[VisibilityChunk<'_>],
        extras: &[VisibilityOccluder<'_>],
    ) -> VisibilityResult {
        if !enabled {
            return VisibilityResult::default();
        }
        let mut stats = VisibilityStats::default();
        let stamp = scene_stamp(self, camera, chunks, extras);
        if self.frame.as_ref().is_none_or(|memo| memo.stamp != stamp) {
            self.frame = Some(FrameMemo {
                stamp,
                classified: BTreeSet::new(),
                visible: BTreeSet::new(),
                next_sample: BTreeMap::new(),
            });
        }
        let memo = self.frame.as_mut().unwrap();
        stats.cache_hits = memo.classified.len();
        if memo.classified.len() == chunks.len() {
            return VisibilityResult {
                visible: memo.visible.clone(),
                stats,
            };
        }
        if !valid_camera(camera) {
            stats.pending_chunks = chunks.iter().filter(|chunk| chunk.visible).count();
            return VisibilityResult {
                visible: BTreeSet::new(),
                stats,
            };
        }
        for chunk in chunks.iter().filter(|chunk| chunk.visible) {
            let key = (chunk.asset, chunk.key.1.clone());
            self.geometry.entry(key).or_insert_with(|| {
                stats.geometry_builds += 1;
                source_geometry(chunk.triangles)
            });
        }
        let bodies = chunks
            .iter()
            .map(|chunk| {
                let geometry = self
                    .geometry
                    .get(&(chunk.asset, chunk.key.1.clone()))?
                    .as_ref()?;
                (chunk.visible && valid_affine(chunk.world_from_rest)).then(|| Body {
                    collider: &geometry.collider,
                    bounds: geometry.bounds,
                    local_from_world: chunk.world_from_rest.inverse(),
                })
            })
            .collect::<Vec<_>>();
        let extra_bodies = extras
            .iter()
            .filter(|extra| valid_affine(extra.world_from_local))
            .map(|extra| {
                let bounds = extra.collider.aabb(Vec3::ZERO, Quat::IDENTITY, 0.0);
                Body {
                    collider: extra.collider,
                    bounds: Bounds {
                        min: bounds.min,
                        max: bounds.max,
                    },
                    local_from_world: extra.world_from_local.inverse(),
                }
            })
            .collect::<Vec<_>>();
        let mut targets = Vec::new();
        for (index, chunk) in chunks.iter().enumerate() {
            if memo.classified.contains(&chunk.key) {
                continue;
            }
            if !chunk.visible {
                memo.classified.insert(chunk.key.clone());
                continue;
            }
            if bodies[index].is_none() {
                stats.invalid_chunks += 1;
                memo.classified.insert(chunk.key.clone());
                continue;
            }
            let (area, samples) = surface_samples(camera, chunk, self.max_samples_per_chunk);
            if samples.is_empty() {
                stats.frustum_rejected += 1;
                memo.classified.insert(chunk.key.clone());
            } else {
                targets.push(Target {
                    index,
                    area,
                    samples,
                });
            }
        }
        targets.sort_by(|a, b| {
            b.area
                .total_cmp(&a.area)
                .then_with(|| chunks[a.index].key.cmp(&chunks[b.index].key))
        });
        let eye = camera
            .view_from_world
            .inverse()
            .transform_point3(Vec3::ZERO);
        for target in targets {
            let key = &chunks[target.index].key;
            let mut pending = false;
            let start = memo.next_sample.get(key).copied().unwrap_or(0);
            for (sample_index, point) in target.samples.into_iter().enumerate().skip(start) {
                let vector = point - eye;
                let distance = vector.length();
                if distance <= 0.0 || !distance.is_finite() {
                    continue;
                }
                let direction = vector / distance;
                let view_depth = -camera.view_from_world.transform_vector3(direction).z;
                if view_depth <= 0.0 {
                    continue;
                }
                let near_distance = camera.near / view_depth;
                let origin = eye + direction * near_distance;
                let limit = (distance - near_distance).max(0.0) + hit_epsilon(distance);
                let own = bodies[target.index].as_ref().unwrap();
                let hit_distance =
                    match cast(own, origin, direction, limit, &mut stats, self.max_rays) {
                        RayHit::Hit(distance) => distance,
                        RayHit::Miss => continue,
                        RayHit::Budget => {
                            memo.next_sample.insert(key.clone(), sample_index);
                            pending = true;
                            break;
                        }
                    };
                let mut blocked = false;
                let hit_limit = hit_distance + hit_epsilon(hit_distance);
                for other in bodies
                    .iter()
                    .enumerate()
                    .filter_map(|(index, body)| {
                        (index != target.index).then_some(body.as_ref()).flatten()
                    })
                    .chain(extra_bodies.iter())
                {
                    match cast(
                        other,
                        origin,
                        direction,
                        hit_limit,
                        &mut stats,
                        self.max_rays,
                    ) {
                        RayHit::Hit(_) => {
                            blocked = true;
                            break;
                        }
                        RayHit::Miss => {}
                        RayHit::Budget => {
                            memo.next_sample.insert(key.clone(), sample_index);
                            pending = true;
                            break;
                        }
                    }
                }
                if pending {
                    break;
                }
                if !blocked {
                    memo.visible.insert(key.clone());
                    break;
                }
            }
            if pending {
                stats.pending_chunks += 1;
            } else {
                stats.tested_chunks += 1;
                memo.classified.insert(key.clone());
                memo.next_sample.remove(key);
            }
        }
        VisibilityResult {
            visible: memo.visible.clone(),
            stats,
        }
    }
}

fn scene_stamp(
    cache: &ForceVisibilityCache,
    camera: &VisibilityCamera,
    chunks: &[VisibilityChunk<'_>],
    extras: &[VisibilityOccluder<'_>],
) -> SceneStamp {
    SceneStamp {
        camera: camera
            .clip_from_world
            .to_cols_array()
            .into_iter()
            .chain(camera.view_from_world.to_cols_array())
            .chain([camera.near, camera.far])
            .map(f32::to_bits)
            .collect(),
        generation: cache.generation,
        samples: cache.max_samples_per_chunk,
        budget: cache.max_rays,
        chunks: chunks
            .iter()
            .map(|chunk| ChunkStamp {
                key: chunk.key.clone(),
                asset: chunk.asset,
                pose: chunk.world_from_rest.to_cols_array().map(f32::to_bits),
                visible: chunk.visible,
                triangles: chunk.triangles.len(),
            })
            .collect(),
        extras: extras
            .iter()
            .map(|extra| {
                (
                    extra.entity,
                    extra.world_from_local.to_cols_array().map(f32::to_bits),
                    extra.revision,
                )
            })
            .collect(),
    }
}

fn valid_camera(camera: &VisibilityCamera) -> bool {
    camera.clip_from_world.is_finite()
        && valid_affine(camera.view_from_world)
        && camera.near.is_finite()
        && camera.near > 0.0
        && camera.far.is_finite()
        && camera.far > camera.near
}

fn valid_affine(pose: Affine3A) -> bool {
    let determinant = pose.matrix3.determinant();
    pose.is_finite() && determinant.is_finite() && determinant != 0.0
}

fn source_geometry(triangles: &[[Vec3; 3]]) -> Option<SourceGeometry> {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    let mut unique = BTreeMap::new();
    let mut bounds = Bounds {
        min: Vec3::INFINITY,
        max: Vec3::NEG_INFINITY,
    };
    for triangle in triangles {
        if triangle.iter().any(|point| !point.is_finite()) {
            return None;
        }
        if (triangle[1] - triangle[0])
            .cross(triangle[2] - triangle[0])
            .length_squared()
            == 0.0
        {
            continue;
        }
        indices.push(triangle.map(|point| {
            bounds.min = bounds.min.min(point);
            bounds.max = bounds.max.max(point);
            let key = point.to_array().map(|coordinate| {
                if coordinate == 0.0 {
                    0
                } else {
                    coordinate.to_bits()
                }
            });
            *unique.entry(key).or_insert_with(|| {
                let index = vertices.len() as u32;
                vertices.push(point);
                index
            })
        }));
    }
    if indices.is_empty() {
        return None;
    }
    let collider = Collider::try_trimesh(vertices, indices).ok()?;
    Some(SourceGeometry { collider, bounds })
}

fn hit_epsilon(distance: f32) -> f32 {
    8.0 * f32::EPSILON * distance.abs().max(1.0)
}

fn cast(
    body: &Body<'_>,
    origin: Vec3,
    direction: Vec3,
    max_distance: f32,
    stats: &mut VisibilityStats,
    budget: usize,
) -> RayHit {
    let origin = body.local_from_world.transform_point3(origin);
    let vector = body.local_from_world.transform_vector3(direction);
    let scale = vector.length();
    if scale <= 0.0 || !scale.is_finite() {
        return RayHit::Miss;
    }
    let direction = vector / scale;
    let limit = max_distance * scale;
    if !ray_bounds(body.bounds, origin, direction, limit) {
        return RayHit::Miss;
    }
    if stats.rays >= budget {
        return RayHit::Budget;
    }
    stats.rays += 1;
    match body
        .collider
        .cast_ray(Vec3::ZERO, Quat::IDENTITY, origin, direction, limit, false)
    {
        Some((distance, _)) => RayHit::Hit(distance / scale),
        None => RayHit::Miss,
    }
}

fn ray_bounds(bounds: Bounds, origin: Vec3, direction: Vec3, limit: f32) -> bool {
    let mut near: f32 = 0.0;
    let mut far = limit;
    for axis in 0..3 {
        if direction[axis].abs() <= 1e-20 {
            if origin[axis] < bounds.min[axis] || origin[axis] > bounds.max[axis] {
                return false;
            }
        } else {
            let a = (bounds.min[axis] - origin[axis]) / direction[axis];
            let b = (bounds.max[axis] - origin[axis]) / direction[axis];
            near = near.max(a.min(b));
            far = far.min(a.max(b));
            if near > far {
                return false;
            }
        }
    }
    true
}

#[derive(Clone, Copy, Default)]
struct ClipVertex {
    world: Vec3,
    clip: Vec4,
    depth: f32,
}

impl ClipVertex {
    fn lerp(self, other: Self, t: f32) -> Self {
        Self {
            world: self.world.lerp(other.world, t),
            clip: self.clip.lerp(other.clip, t),
            depth: self.depth + (other.depth - self.depth) * t,
        }
    }

    fn distance(self, plane: usize, camera: &VisibilityCamera) -> f32 {
        match plane {
            0 => self.clip.w + self.clip.x,
            1 => self.clip.w - self.clip.x,
            2 => self.clip.w + self.clip.y,
            3 => self.clip.w - self.clip.y,
            4 => self.depth - camera.near,
            _ => camera.far - self.depth,
        }
    }
}

struct SurfacePolygon {
    vertices: [ClipVertex; 12],
    len: usize,
    center: Vec3,
    area: f32,
}

fn clipped_polygon(camera: &VisibilityCamera, triangle: [Vec3; 3]) -> Option<SurfacePolygon> {
    let mut polygon = [ClipVertex::default(); 12];
    let mut scratch = polygon;
    for (index, world) in triangle.into_iter().enumerate() {
        polygon[index] = ClipVertex {
            world,
            clip: camera.clip_from_world * world.extend(1.0),
            depth: -camera.view_from_world.transform_point3(world).z,
        };
    }
    let mut len = 3;
    for plane in 0..6 {
        let mut count = 0;
        let mut previous = polygon[len - 1];
        let mut previous_distance = previous.distance(plane, camera);
        for current in polygon.iter().take(len).copied() {
            let current_distance = current.distance(plane, camera);
            if (previous_distance >= 0.0) != (current_distance >= 0.0) {
                scratch[count] = previous.lerp(
                    current,
                    previous_distance / (previous_distance - current_distance),
                );
                count += 1;
            }
            if current_distance >= 0.0 {
                scratch[count] = current;
                count += 1;
            }
            previous = current;
            previous_distance = current_distance;
        }
        if count < 3 {
            return None;
        }
        std::mem::swap(&mut polygon, &mut scratch);
        len = count;
    }
    let mut area = 0.0;
    let mut center = Vec3::ZERO;
    for index in 0..len {
        let a = polygon[index].clip;
        let b = polygon[(index + 1) % len].clip;
        if a.w <= 0.0 || !a.is_finite() {
            return None;
        }
        area += (a.x * b.y - a.y * b.x) / (a.w * b.w);
        center += polygon[index].world;
    }
    let area = area.abs() * 0.5;
    (area > 0.0 && area.is_finite()).then_some(SurfacePolygon {
        vertices: polygon,
        len,
        center: center / len as f32,
        area,
    })
}

fn surface_samples(
    camera: &VisibilityCamera,
    chunk: &VisibilityChunk<'_>,
    limit: usize,
) -> (f32, Vec<Vec3>) {
    if limit == 0 {
        return (0.0, Vec::new());
    }
    let mut polygons = chunk
        .triangles
        .iter()
        .filter_map(|triangle| {
            clipped_polygon(
                camera,
                triangle.map(|point| chunk.world_from_rest.transform_point3(point)),
            )
        })
        .collect::<Vec<_>>();
    polygons.sort_by(|a, b| b.area.total_cmp(&a.area));
    let area = polygons.iter().map(|polygon| polygon.area).sum();
    polygons.truncate((limit / 2).max(1));
    let mut samples = polygons
        .iter()
        .map(|polygon| polygon.center)
        .collect::<Vec<_>>();
    for corner in 0..12 {
        for polygon in &polygons {
            if samples.len() == limit {
                return (area, samples);
            }
            if corner < polygon.len {
                samples.push(polygon.vertices[corner].world.lerp(polygon.center, 0.02));
            }
        }
    }
    for corner in 0..12 {
        for polygon in &polygons {
            if samples.len() == limit {
                return (area, samples);
            }
            if corner < polygon.len {
                let edge = (polygon.vertices[corner].world
                    + polygon.vertices[(corner + 1) % polygon.len].world)
                    * 0.5;
                samples.push(edge.lerp(polygon.center, 0.02));
            }
        }
    }
    (area, samples)
}

#[cfg(test)]
mod tests;
