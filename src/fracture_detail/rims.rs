use bevy::{asset::RenderAssetUsages, mesh::PrimitiveTopology, prelude::*};

use crate::rim_profile::rim_profile;

use super::{CONTACT_TOLERANCE, ContactPatch, RimEdge, SurfaceGeometry};

const MIN_RIM_LENGTH: f32 = 0.002;
const MAX_RIM_DEPTH: f32 = 0.02;

pub(super) fn minimum_width(surface: &SurfaceGeometry) -> f32 {
    let points: Vec<_> = surface.triangles.iter().flatten().copied().collect();
    surface
        .triangles
        .iter()
        .filter_map(|triangle| {
            let normal = (triangle[1] - triangle[0])
                .cross(triangle[2] - triangle[0])
                .try_normalize()?;
            let min = points
                .iter()
                .map(|p| p.dot(normal))
                .fold(f32::INFINITY, f32::min);
            let max = points
                .iter()
                .map(|p| p.dot(normal))
                .fold(f32::NEG_INFINITY, f32::max);
            Some(max - min)
        })
        .filter(|width| *width > 1e-6)
        .fold(f32::INFINITY, f32::min)
}

fn exterior_intervals(points: [Vec3; 2], surface: &SurfaceGeometry) -> Vec<(f32, f32, Vec3)> {
    let mut result = Vec::new();
    for triangle in &surface.exterior {
        let Some(normal) = (triangle[1] - triangle[0])
            .cross(triangle[2] - triangle[0])
            .try_normalize()
        else {
            continue;
        };
        if points
            .iter()
            .any(|point| normal.dot(*point - triangle[0]).abs() > CONTACT_TOLERANCE as f32 * 2.0)
        {
            continue;
        }
        let mut low = 0.0_f32;
        let mut high = 1.0_f32;
        for i in 0..3 {
            let a = triangle[i];
            let edge = triangle[(i + 1) % 3] - a;
            let first = edge.cross(points[0] - a).dot(normal);
            let delta = edge.cross(points[1] - points[0]).dot(normal);
            let tolerance = CONTACT_TOLERANCE as f32 * edge.length();
            if delta.abs() < 1e-10 {
                if first < -tolerance {
                    high = -1.0;
                    break;
                }
            } else {
                let t = (-tolerance - first) / delta;
                if delta > 0.0 {
                    low = low.max(t);
                } else {
                    high = high.min(t);
                }
            }
        }
        if high > low && (high - low) * points[0].distance(points[1]) > MIN_RIM_LENGTH {
            result.push((low.max(0.0), high.min(1.0), normal));
        }
    }
    result
}

fn normals_match(a: Option<Vec3>, b: Option<Vec3>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.dot(b) > 0.9999,
        (None, None) => true,
        _ => false,
    }
}

pub(super) fn patch_rims(
    polygon: &[Vec3],
    a: &SurfaceGeometry,
    b: &SurfaceGeometry,
    width_a: f32,
    width_b: f32,
) -> Vec<RimEdge> {
    let mut result: Vec<RimEdge> = Vec::new();
    for (&first, &second) in polygon
        .iter()
        .zip(polygon.iter().cycle().skip(1))
        .take(polygon.len())
    {
        if first.distance(second) < MIN_RIM_LENGTH {
            continue;
        }
        let intervals_a = exterior_intervals([first, second], a);
        let intervals_b = exterior_intervals([first, second], b);
        let mut stops = vec![0.0_f32, 1.0];
        stops.extend(
            intervals_a
                .iter()
                .chain(&intervals_b)
                .flat_map(|(a, b, _)| [*a, *b]),
        );
        stops.sort_by(f32::total_cmp);
        stops.dedup_by(|a, b| (*a - *b).abs() * first.distance(second) < MIN_RIM_LENGTH * 0.1);
        for pair in stops.windows(2) {
            if (pair[1] - pair[0]) * first.distance(second) < MIN_RIM_LENGTH {
                continue;
            }
            let midpoint = (pair[0] + pair[1]) * 0.5;
            let exterior_a = intervals_a
                .iter()
                .find(|(low, high, _)| midpoint >= *low && midpoint <= *high)
                .map(|(_, _, normal)| *normal);
            let exterior_b = intervals_b
                .iter()
                .find(|(low, high, _)| midpoint >= *low && midpoint <= *high)
                .map(|(_, _, normal)| *normal);
            if exterior_a.is_none() && exterior_b.is_none() {
                continue;
            }
            let rim = RimEdge {
                points: [first.lerp(second, pair[0]), first.lerp(second, pair[1])],
                exterior_a,
                exterior_b,
                thickness_a: width_a,
                thickness_b: width_b,
            };
            if let Some(previous) = result.last_mut()
                && previous.points[1].distance(rim.points[0]) < MIN_RIM_LENGTH
                && normals_match(previous.exterior_a, rim.exterior_a)
                && normals_match(previous.exterior_b, rim.exterior_b)
                && (previous.points[1] - previous.points[0])
                    .normalize()
                    .dot((rim.points[1] - rim.points[0]).normalize())
                    > 0.9999
            {
                previous.points[1] = rim.points[1];
            } else {
                result.push(rim);
            }
        }
    }
    result
}

fn stable_value(id: &str, position: Vec3) -> f32 {
    let mut hash = 2166136261_u32;
    for byte in id.bytes().chain(
        position
            .to_array()
            .into_iter()
            .flat_map(|v| ((v * 10000.0).round() as i32).to_le_bytes()),
    ) {
        hash = (hash ^ u32::from(byte)).wrapping_mul(16777619);
    }
    (hash >> 8) as f32 / 16777216.0
}

pub fn detail_mesh(patch: &ContactPatch, survivor: &str) -> Option<Mesh> {
    let is_a = survivor == patch.a;
    if !is_a && survivor != patch.b {
        return None;
    }
    let outward = if is_a { patch.normal } else { -patch.normal };
    let mut positions = Vec::<[f32; 3]>::new();
    let mut normals = Vec::<[f32; 3]>::new();
    let mut colors = Vec::<[f32; 4]>::new();
    for rim in &patch.rims {
        let (Some(exterior), thickness) = (
            if is_a { rim.exterior_a } else { rim.exterior_b },
            if is_a {
                rim.thickness_a
            } else {
                rim.thickness_b
            },
        ) else {
            continue;
        };
        let Some(across) = (outward - exterior * outward.dot(exterior)).try_normalize() else {
            continue;
        };
        if outward.dot(exterior).abs() > 0.97 {
            continue;
        }
        let depth = MAX_RIM_DEPTH.min(thickness * 0.08);
        if depth < 0.001 {
            continue;
        }
        let [start, end] = rim.points;
        let length = start.distance(end);
        if length < depth * 2.0 {
            continue;
        }
        let tangent = (end - start) / length;
        let inward = -(exterior + outward).normalize();
        let mut rings = Vec::new();
        for sample in rim_profile(&patch.id, start, end) {
            let t = sample.t;
            let point = start.lerp(end, t);
            let seed = stable_value(&patch.id, point);
            let endpoint_fade = (length * t / (depth * 2.0))
                .min(length * (1.0 - t) / (depth * 2.0))
                .min(1.0);
            let amplitude = depth * sample.height * endpoint_fade;
            let embed = depth * sample.embed;
            let width = depth * sample.width;
            let surface_offset = 0.00015_f32.min(embed * across.dot(outward) * 0.25);
            let ring = [
                (
                    point - across * embed + exterior * surface_offset,
                    [1.0, seed, 1.0, 1.0],
                ),
                (
                    point + across * amplitude + exterior * surface_offset,
                    [0.0, seed, 1.0, 1.0],
                ),
                (
                    point - across * embed + inward * width,
                    [1.0, seed, 1.0, 1.0],
                ),
            ];
            rings.push(ring);
        }
        let order = if (rings[0][1].0 - rings[0][0].0)
            .cross(rings[0][2].0 - rings[0][0].0)
            .dot(tangent)
            > 0.0
        {
            [0, 1, 2]
        } else {
            [0, 2, 1]
        };
        let mut triangles = Vec::new();
        triangles.push([rings[0][order[0]], rings[0][order[2]], rings[0][order[1]]]);
        let last = rings.len() - 1;
        triangles.push([
            rings[last][order[0]],
            rings[last][order[1]],
            rings[last][order[2]],
        ]);
        for pair in rings.windows(2) {
            for edge in 0..3 {
                let a = order[edge];
                let b = order[(edge + 1) % 3];
                triangles.push([pair[0][a], pair[0][b], pair[1][b]]);
                triangles.push([pair[0][a], pair[1][b], pair[1][a]]);
            }
        }
        for triangle in triangles {
            let Some(normal) = (triangle[1].0 - triangle[0].0)
                .cross(triangle[2].0 - triangle[0].0)
                .try_normalize()
            else {
                continue;
            };
            for (point, color) in triangle {
                positions.push(point.to_array());
                normals.push(normal.to_array());
                colors.push(color);
            }
        }
    }
    (!positions.is_empty()).then(|| {
        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors)
    })
}
