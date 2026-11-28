use bevy::{
    math::{DVec2, DVec3},
    prelude::Vec4,
};

use crate::fracture_detail::ContactPatch;

pub(super) const PLANE_TOLERANCE: f64 = 4e-5;
const MIN_OVERLAP_AREA: f64 = 1e-14;

pub(super) struct MaskPolygon {
    normal: DVec3,
    origin: DVec3,
    tangent: DVec3,
    bitangent: DVec3,
    vertices: Vec<DVec3>,
    boundary: Vec<DVec2>,
    pub(super) offset: Option<u32>,
}

impl MaskPolygon {
    fn new(vertices: Vec<DVec3>, normal: DVec3) -> Self {
        let origin = vertices[0];
        let axis = if normal.x.abs() < 0.8 {
            DVec3::X
        } else {
            DVec3::Y
        };
        let tangent = normal.cross(axis).normalize();
        let bitangent = normal.cross(tangent);
        let mut polygon = Self {
            normal,
            origin,
            tangent,
            bitangent,
            vertices,
            boundary: Vec::new(),
            offset: None,
        };
        polygon.boundary = polygon
            .vertices
            .iter()
            .map(|&point| polygon.project(point))
            .collect();
        if area(&polygon.boundary) < 0.0 {
            polygon.vertices.reverse();
            polygon.boundary.reverse();
        }
        polygon
    }

    fn project(&self, point: DVec3) -> DVec2 {
        let relative = point - self.origin;
        DVec2::new(relative.dot(self.tangent), relative.dot(self.bitangent))
    }

    pub(super) fn overlaps(&self, triangle: &[DVec3; 3], orientation: f64) -> bool {
        let cross = (triangle[1] - triangle[0]).cross(triangle[2] - triangle[0]);
        if cross.length_squared() <= 1e-24 {
            return false;
        }
        let source_normal = cross.normalize();
        if source_normal.dot(self.normal) * orientation < 0.98 {
            return false;
        }
        let projected = triangle.iter().map(|&point| self.project(point)).collect();
        let overlap = clip(projected, &self.boundary);
        area(&overlap).abs() > MIN_OVERLAP_AREA
            && overlap.iter().all(|point| {
                let lifted = self.origin + self.tangent * point.x + self.bitangent * point.y;
                source_normal.dot(lifted - triangle[0]).abs() <= PLANE_TOLERANCE
            })
    }

    pub(super) fn gpu_rows(&self) -> Vec<Vec4> {
        let origin = self.origin.as_vec3();
        let mut rows = vec![origin.extend(self.vertices.len() as f32)];
        for (&a, &b) in self
            .vertices
            .iter()
            .zip(self.vertices.iter().cycle().skip(1))
        {
            let inward = self.normal.cross(b - a).normalize().as_vec3();
            let constant = -inward.as_dvec3().dot(a - origin.as_dvec3()) as f32;
            rows.push(inward.extend(constant));
        }
        rows
    }
}

pub(super) fn prepare_patch(patch: &ContactPatch) -> Result<Vec<MaskPolygon>, String> {
    let Some(normal) = patch.normal.as_dvec3().try_normalize() else {
        return Err(format!("Contact {} has an invalid mask normal", patch.id));
    };
    let triangles: Vec<_> = patch
        .triangles
        .iter()
        .map(|t| t.map(|p| p.as_dvec3()))
        .collect();
    let Some(first) = triangles.first() else {
        return Ok(Vec::new());
    };
    let mut outline = first.to_vec();
    let mut fan = true;
    for triangle in &triangles[1..] {
        if triangle[0] != first[0] || triangle[1] != *outline.last().unwrap() {
            fan = false;
            break;
        }
        outline.push(triangle[2]);
    }
    if fan {
        let polygon = MaskPolygon::new(outline, normal);
        let convex = polygon.boundary.iter().enumerate().all(|(i, &a)| {
            let b = polygon.boundary[(i + 1) % polygon.boundary.len()];
            polygon
                .boundary
                .iter()
                .all(|&point| (b - a).perp_dot(point - a) >= -1e-16)
        });
        let expected = triangles
            .iter()
            .map(|triangle| {
                let projected = triangle.map(|point| polygon.project(point));
                area(&projected).abs()
            })
            .sum::<f64>();
        let actual = area(&polygon.boundary);
        let has_zero_edge = polygon
            .vertices
            .iter()
            .zip(polygon.vertices.iter().cycle().skip(1))
            .any(|(&a, &b)| a == b);
        if convex
            && !has_zero_edge
            && actual > MIN_OVERLAP_AREA
            && (actual - expected).abs() <= (expected * 1e-10).max(1e-16)
        {
            return Ok(vec![polygon]);
        }
    }
    // Non-convex or disconnected interfaces retain the actual triangle union.
    Ok(triangles
        .into_iter()
        .filter(|triangle| {
            let cross = (triangle[1] - triangle[0]).cross(triangle[2] - triangle[0]);
            cross.length_squared() > 1e-24
        })
        .map(|triangle| MaskPolygon::new(triangle.to_vec(), normal))
        .collect())
}

fn area(polygon: &[DVec2]) -> f64 {
    let Some(&origin) = polygon.first() else {
        return 0.0;
    };
    polygon
        .iter()
        .zip(polygon.iter().cycle().skip(1))
        .map(|(&a, &b)| (a - origin).perp_dot(b - origin))
        .sum::<f64>()
        * 0.5
}

fn clip(mut subject: Vec<DVec2>, boundary: &[DVec2]) -> Vec<DVec2> {
    for (&a, &b) in boundary.iter().zip(boundary.iter().cycle().skip(1)) {
        let mut output = Vec::new();
        for (&previous, &current) in subject.iter().zip(subject.iter().cycle().skip(1)) {
            let dp = (b - a).perp_dot(previous - a);
            let dc = (b - a).perp_dot(current - a);
            if (dp >= 0.0) != (dc >= 0.0) {
                output.push(previous.lerp(current, dp / (dp - dc)));
            }
            if dc >= 0.0 {
                output.push(current);
            }
        }
        subject = output;
        if subject.len() < 3 {
            return Vec::new();
        }
    }
    subject
}
