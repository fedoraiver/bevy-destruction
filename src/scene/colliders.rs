use avian3d::{
    parry::{shape::SharedShape, transformation::try_convex_hull},
    prelude::*,
};
use bevy::prelude::*;

use crate::asset::ChunkRecord;

pub(crate) fn chunk_collider(record: &ChunkRecord) -> Result<Collider, String> {
    let shapes = record
        .colliders
        .iter()
        .enumerate()
        .map(|(index, proxy)| {
            let points = proxy
                .vertices
                .iter()
                .copied()
                .map(Vec3::from)
                .collect::<Vec<_>>();
            Collider::convex_hull(&points)
                .or_else(|| conditioned_convex_collider(&points))
                .map(|shape| (Vec3::ZERO, Quat::IDENTITY, shape))
                .ok_or_else(|| {
                    format!(
                        "Invalid convex collider: {} (proxy {})",
                        record.id,
                        index + 1
                    )
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Collider::compound(shapes))
}

pub(crate) fn chunk_mass_properties(
    id: &str,
    collider: &Collider,
    density: f32,
) -> Result<MassProperties3d, String> {
    let properties = collider.mass_properties(density);
    let inertia = properties.principal_angular_inertia;
    if !properties.mass.is_finite()
        || properties.mass <= 0.0
        || !properties.center_of_mass.is_finite()
        || !properties.local_inertial_frame.is_finite()
        || !inertia.is_finite()
        || inertia.min_element() <= 0.0
    {
        return Err(format!("Invalid collider mass properties: {id}"));
    }
    Ok(properties)
}

fn conditioned_convex_collider(points: &[Vec3]) -> Option<Collider> {
    let first = *points.first()?;
    let (min, max) = points.iter().fold((first, first), |(min, max), &point| {
        (min.min(point), max.max(point))
    });
    let extent = max - min;
    if !extent.is_finite() || extent.min_element() <= 0.0 {
        return None;
    }

    // Parry can classify a thin solid as a line after uniform normalization.
    // Find the faces in better-scaled coordinates, then build from the original vertices.
    let normalized: Vec<_> = points.iter().map(|&point| (point - min) / extent).collect();
    let (vertices, faces) = try_convex_hull(&normalized).ok()?;
    if vertices.len() < 4 || faces.len() < 4 {
        return None;
    }
    let origin = vertices[0].as_dvec3();
    let signed_volume: f64 = faces
        .iter()
        .map(|face| {
            let [a, b, c] = face.map(|index| vertices[index as usize].as_dvec3() - origin);
            a.dot(b.cross(c))
        })
        .sum();
    if signed_volume <= f64::EPSILON {
        return None;
    }
    let vertices = vertices
        .iter()
        .map(|vertex| {
            normalized
                .iter()
                .position(|point| point == vertex)
                .map(|index| points[index])
        })
        .collect::<Option<Vec<_>>>()?;
    SharedShape::convex_mesh(vertices, &faces).map(Collider::from)
}
