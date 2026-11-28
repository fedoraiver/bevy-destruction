use std::collections::BTreeMap;

use bevy::{
    math::{DVec2, DVec3},
    prelude::Vec3,
};

use crate::asset::Manifest;

use super::{
    CONTACT_TOLERANCE, ContactPatch, MIN_VISUAL_CONTACT_AREA, SurfaceGeometry,
    polygons::{clip_polygon, connected_polygons, polygon_area},
    rims::{minimum_width, patch_rims},
};

#[cfg(test)]
use bevy::{asset::RenderAssetUsages, mesh::PrimitiveTopology, prelude::Mesh};

#[cfg(test)]
pub(crate) fn contact_surface_mesh(patches: &[ContactPatch], chunk: &str) -> Option<Mesh> {
    let vertex_count = patches
        .iter()
        .filter(|patch| patch.a == chunk || patch.b == chunk)
        .map(|patch| patch.triangles.len() * 3)
        .sum::<usize>();
    if vertex_count == 0 {
        return None;
    }
    let mut positions = Vec::with_capacity(vertex_count);
    let mut normals = Vec::with_capacity(vertex_count);
    for patch in patches {
        let side = if patch.a == chunk {
            1.0
        } else if patch.b == chunk {
            -1.0
        } else {
            continue;
        };
        let normal = patch.normal * side;
        for &source in &patch.triangles {
            let mut triangle = source;
            let points = source.map(|point| point.as_dvec3());
            if (points[1] - points[0])
                .cross(points[2] - points[0])
                .dot(normal.as_dvec3())
                < 0.0
            {
                triangle.swap(1, 2);
            }
            positions.extend(triangle.map(|point| point.to_array()));
            normals.extend([normal.to_array(); 3]);
        }
    }
    Some(
        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals),
    )
}

pub(super) struct Plane {
    pub(super) normal: DVec3,
    origin: DVec3,
    tangent: DVec3,
    bitangent: DVec3,
    polygons: Vec<Vec<DVec2>>,
}

impl Plane {
    pub(super) fn new(normal: DVec3, point: DVec3) -> Self {
        let axis = if normal.x.abs() < 0.8 {
            DVec3::X
        } else {
            DVec3::Y
        };
        let tangent = normal.cross(axis).normalize();
        Self {
            normal,
            origin: normal * normal.dot(point),
            tangent,
            bitangent: normal.cross(tangent),
            polygons: Vec::new(),
        }
    }

    pub(super) fn project(&self, point: DVec3) -> DVec2 {
        let relative = point - self.origin;
        DVec2::new(relative.dot(self.tangent), relative.dot(self.bitangent))
    }

    fn unproject(&self, point: DVec2) -> Vec3 {
        (self.origin + self.tangent * point.x + self.bitangent * point.y).as_vec3()
    }
}

pub fn build_patches(
    manifest: &Manifest,
    geometry: &BTreeMap<String, SurfaceGeometry>,
) -> Result<Vec<ContactPatch>, String> {
    let widths: BTreeMap<_, _> = geometry
        .iter()
        .map(|(id, surface)| (id.as_str(), minimum_width(surface)))
        .collect();
    let mut result = Vec::new();
    for bond in &manifest.bonds {
        if bond.area < MIN_VISUAL_CONTACT_AREA {
            continue;
        }
        let a = geometry
            .get(&bond.a)
            .ok_or_else(|| format!("Contact {} is missing chunk {}", bond.id, bond.a))?;
        let b = geometry
            .get(&bond.b)
            .ok_or_else(|| format!("Contact {} is missing chunk {}", bond.id, bond.b))?;
        if bond.patches.is_empty() {
            return Err(format!("Contact {} is missing interface polygons", bond.id));
        }
        result.extend(baked_contact_patches(
            bond,
            a,
            b,
            widths[bond.a.as_str()],
            widths[bond.b.as_str()],
        )?);
    }
    result.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(result)
}

pub(super) fn mesh_patch_coverage(
    plane: &Plane,
    boundary: &[DVec2],
    surface: &SurfaceGeometry,
    orientation: f64,
) -> Result<f64, String> {
    let mut covered = Vec::<Vec<DVec2>>::new();
    for triangle in &surface.triangles {
        let triangle = triangle.map(|point| point.as_dvec3());
        let cross = (triangle[1] - triangle[0]).cross(triangle[2] - triangle[0]);
        if cross.length_squared() <= 1e-24
            || plane.normal.dot(cross.normalize()) * orientation < 0.98
        {
            continue;
        }
        let projected = triangle.iter().map(|point| plane.project(*point)).collect();
        let overlap = clip_polygon(projected, boundary);
        let area = polygon_area(&overlap).abs();
        if area <= 1e-14 {
            continue;
        }
        let surface_normal = cross.normalize();
        if overlap.iter().any(|point| {
            let position = plane.origin + plane.tangent * point.x + plane.bitangent * point.y;
            surface_normal.dot(position - triangle[0]).abs() > CONTACT_TOLERANCE
        }) {
            continue;
        }
        for previous in &covered {
            let duplicated = polygon_area(&clip_polygon(overlap.clone(), previous)).abs();
            if duplicated > (area.min(polygon_area(previous).abs()) * 0.005).max(1e-12) {
                return Err("Display mesh overlaps itself on a baked interface".into());
            }
        }
        covered.push(overlap);
    }
    Ok(covered
        .iter()
        .map(|polygon| polygon_area(polygon).abs())
        .sum())
}

fn baked_contact_patches(
    bond: &crate::asset::BondRecord,
    a: &SurfaceGeometry,
    b: &SurfaceGeometry,
    width_a: f32,
    width_b: f32,
) -> Result<Vec<ContactPatch>, String> {
    let mut planes = Vec::<Plane>::new();
    for patch in &bond.patches {
        if patch.vertices.len() < 3 {
            return Err(format!(
                "Contact {} has an empty interface polygon",
                bond.id
            ));
        }
        let normal = Vec3::from_array(patch.normal).as_dvec3().normalize();
        let plane = Plane::new(normal, Vec3::from_array(patch.center).as_dvec3());
        let vertices = patch
            .vertices
            .iter()
            .map(|point| Vec3::from_array(*point).as_dvec3())
            .collect::<Vec<_>>();
        let polygon = vertices
            .iter()
            .map(|point| plane.project(*point))
            .collect::<Vec<_>>();
        let expected = f64::from(patch.area);
        for (surface, orientation) in [(a, 1.0), (b, -1.0)] {
            let covered = mesh_patch_coverage(&plane, &polygon, surface, orientation)
                .map_err(|error| format!("Contact {}: {error}", bond.id))?;
            if covered <= 0.0 || (covered - expected).abs() > (expected * 0.01).max(1e-9) {
                return Err(format!(
                    "Contact {} baked area {expected:.9} has mesh coverage {covered:.9}",
                    bond.id
                ));
            }
        }
        if let Some(existing) = planes.iter_mut().find(|existing| {
            existing.normal.dot(normal) > 1.0 - 1e-10
                && vertices
                    .iter()
                    .all(|point| existing.normal.dot(*point - existing.origin).abs() <= 1e-6)
        }) {
            existing.polygons.push(
                vertices
                    .iter()
                    .map(|point| existing.project(*point))
                    .collect(),
            );
        } else {
            let mut plane = plane;
            plane.polygons.push(polygon);
            planes.push(plane);
        }
    }
    let mut result = Vec::new();
    for plane in planes {
        // Merge convex coplanar unions; retain valid per-facet polygons for concave unions.
        let original_area = plane
            .polygons
            .iter()
            .map(|polygon| polygon_area(polygon).abs())
            .sum::<f64>();
        let polygons = connected_polygons(&plane.polygons)
            .ok()
            .filter(|polygons| {
                let merged_area = polygons
                    .iter()
                    .map(|polygon| polygon_area(polygon).abs())
                    .sum::<f64>();
                (merged_area - original_area).abs() <= (original_area * 1e-6).max(1e-12)
            })
            .unwrap_or_else(|| plane.polygons.clone());
        for polygon in polygons {
            let points = polygon
                .iter()
                .map(|point| plane.unproject(*point))
                .collect::<Vec<_>>();
            let triangles = (1..points.len() - 1)
                .map(|index| [points[0], points[index], points[index + 1]])
                .collect();
            result.push(ContactPatch {
                id: String::new(),
                bond_id: bond.id.clone(),
                a: bond.a.clone(),
                b: bond.b.clone(),
                normal: plane.normal.as_vec3(),
                triangles,
                area: polygon_area(&polygon).abs() as f32,
                rims: patch_rims(&points, a, b, width_a, width_b),
            });
        }
    }
    result.sort_by(|a, b| triangle_order(&a.triangles[0], &b.triangles[0]));
    for (index, patch) in result.iter_mut().enumerate() {
        patch.id = format!("{}:patch:{index:03}", bond.id);
    }
    Ok(result)
}

fn triangle_order(a: &[Vec3; 3], b: &[Vec3; 3]) -> std::cmp::Ordering {
    a.iter()
        .flat_map(|p| p.to_array())
        .zip(b.iter().flat_map(|p| p.to_array()))
        .map(|(a, b)| a.total_cmp(&b))
        .find(|order| !order.is_eq())
        .unwrap_or(std::cmp::Ordering::Equal)
}
