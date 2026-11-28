use avian3d::prelude::{Collider, ColliderAabb, TrimeshFlags};
use bevy::{math::Affine3A, prelude::*};

pub(super) struct LocalRay {
    origin: Vec3,
    direction: Vec3,
    scale: f32,
}

pub(super) fn local_ray(ray: Ray3d, world_from_local: Affine3A) -> Option<LocalRay> {
    let determinant = world_from_local.matrix3.determinant();
    if !world_from_local.is_finite() || !determinant.is_finite() || determinant == 0.0 {
        return None;
    }
    let local_from_world = world_from_local.inverse();
    let origin = local_from_world.transform_point3(ray.origin);
    let vector = local_from_world.transform_vector3(*ray.direction);
    let scale = vector.length();
    if !origin.is_finite() || !scale.is_finite() || scale <= 0.0 {
        return None;
    }
    Some(LocalRay {
        origin,
        direction: vector / scale,
        scale,
    })
}

pub(super) fn source_bounds(triangles: &[[Vec3; 3]]) -> Option<ColliderAabb> {
    let mut bounds = ColliderAabb::INVALID;
    let mut has_surface = false;
    for triangle in triangles {
        if triangle.iter().any(|point| !point.is_finite()) {
            return None;
        }
        let area = (triangle[1] - triangle[0])
            .cross(triangle[2] - triangle[0])
            .length_squared();
        if !area.is_finite() {
            return None;
        }
        if area == 0.0 {
            continue;
        }
        has_surface = true;
        for &point in triangle {
            bounds.min = bounds.min.min(point);
            bounds.max = bounds.max.max(point);
        }
    }
    has_surface.then_some(bounds)
}

pub(super) fn source_collider(triangles: &[[Vec3; 3]]) -> Option<Collider> {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for triangle in triangles {
        if (triangle[1] - triangle[0])
            .cross(triangle[2] - triangle[0])
            .length_squared()
            == 0.0
        {
            continue;
        }
        let first = u32::try_from(vertices.len()).ok()?;
        let last = first.checked_add(2)?;
        indices.push([first, first + 1, last]);
        // Avian's vertex weld hashes coordinate bits, so give signed zero one representation.
        vertices
            .extend(triangle.map(|point| Vec3::select(point.cmpeq(Vec3::ZERO), Vec3::ZERO, point)));
    }
    Collider::try_trimesh_with_config(vertices, indices, TrimeshFlags::MERGE_DUPLICATE_VERTICES)
        .ok()
}

pub(super) fn cast(collider: &Collider, ray: &LocalRay, world_limit: f32) -> Option<f32> {
    let local_limit = world_limit * ray.scale;
    if !local_limit.is_finite() {
        return None;
    }
    collider
        .cast_ray(
            Vec3::ZERO,
            Quat::IDENTITY,
            ray.origin,
            ray.direction,
            local_limit,
            false,
        )
        .and_then(|(distance, _)| {
            let distance = distance / ray.scale;
            (distance.is_finite() && distance >= 0.0 && distance <= world_limit).then_some(distance)
        })
}

pub(super) fn ray_bounds(bounds: ColliderAabb, ray: &LocalRay, world_limit: f32) -> bool {
    let mut near: f32 = 0.0;
    let mut far = world_limit * ray.scale;
    for axis in 0..3 {
        if ray.direction[axis].abs() <= 1e-20 {
            if ray.origin[axis] < bounds.min[axis] || ray.origin[axis] > bounds.max[axis] {
                return false;
            }
        } else {
            let a = (bounds.min[axis] - ray.origin[axis]) / ray.direction[axis];
            let b = (bounds.max[axis] - ray.origin[axis]) / ray.direction[axis];
            near = near.max(a.min(b));
            far = far.min(a.max(b));
            if near > far {
                return false;
            }
        }
    }
    true
}
