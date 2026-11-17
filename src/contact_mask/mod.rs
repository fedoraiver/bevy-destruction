use std::collections::BTreeMap;

use bevy::{
    math::{DVec2, DVec3},
    mesh::{PrimitiveTopology, VertexAttributeValues},
    prelude::*,
};

use crate::fracture_detail::ContactPatch;

const PLANE_TOLERANCE: f64 = 4e-5;
const MIN_OVERLAP_AREA: f64 = 1e-14;
const MAX_DATA_ROWS: usize = 1 << 20;

/// One asset's immutable contact field. Row zero is the empty candidate list.
pub struct ContactMaskBuilder {
    data: Vec<Vec4>,
    polygons: BTreeMap<String, Vec<MaskPolygon>>,
    headers: BTreeMap<Vec<u32>, u32>,
}

impl Default for ContactMaskBuilder {
    fn default() -> Self {
        Self {
            data: vec![Vec4::ZERO],
            polygons: BTreeMap::new(),
            headers: BTreeMap::new(),
        }
    }
}

impl ContactMaskBuilder {
    pub fn data(&self) -> &[Vec4] {
        &self.data
    }

    /// Only UV1.y changes. Meshes with candidates duplicate attributes in index
    /// order; meshes without candidates retain their original indexed layout.
    pub fn mask_mesh(
        &mut self,
        mut mesh: Mesh,
        patches: &[ContactPatch],
        chunk: &str,
    ) -> Result<Mesh, String> {
        if mesh.primitive_topology() != PrimitiveTopology::TriangleList {
            return Err("Contact mask requires triangle-list source geometry".into());
        }
        let Some(VertexAttributeValues::Float32x2(xy)) = mesh.attribute(Mesh::ATTRIBUTE_UV_0)
        else {
            return Err("Contact mask requires float32 rest XY coordinates".into());
        };
        let Some(VertexAttributeValues::Float32x2(z)) = mesh.attribute(Mesh::ATTRIBUTE_UV_1) else {
            return Err("Contact mask requires float32 rest Z coordinates".into());
        };
        let indices: Vec<_> = mesh
            .indices()
            .map_or_else(|| (0..xy.len()).collect(), |i| i.iter().collect());
        if xy.len() != z.len() || !indices.len().is_multiple_of(3) {
            return Err("Contact mask rest coordinates do not form complete triangles".into());
        }
        let rest: Vec<_> = indices
            .iter()
            .map(|&index| {
                let xy = xy
                    .get(index)
                    .ok_or("Contact mask source index is out of bounds")?;
                let z = z
                    .get(index)
                    .ok_or("Contact mask source index is out of bounds")?;
                Ok(DVec3::new(
                    f64::from(xy[0]),
                    f64::from(xy[1]),
                    f64::from(z[0]),
                ))
            })
            .collect::<Result<_, String>>()?;
        let selected: Vec<_> = patches
            .iter()
            .filter_map(|patch| {
                if patch.a == chunk {
                    Some((patch, 1.0))
                } else if patch.b == chunk {
                    Some((patch, -1.0))
                } else {
                    None
                }
            })
            .collect();
        for &(patch, _) in &selected {
            if let std::collections::btree_map::Entry::Vacant(entry) =
                self.polygons.entry(patch.id.clone())
            {
                entry.insert(prepare_patch(patch)?);
            }
        }
        let mut triangle_headers = Vec::with_capacity(rest.len() / 3);
        for triangle in rest.as_chunks::<3>().0 {
            let mut candidates = Vec::new();
            for &(patch, orientation) in &selected {
                for polygon in self.polygons.get_mut(&patch.id).unwrap() {
                    if !polygon.overlaps(triangle, orientation) {
                        continue;
                    }
                    let offset = if let Some(offset) = polygon.offset {
                        offset
                    } else {
                        let rows = polygon.gpu_rows();
                        let offset = append_rows(&mut self.data, &rows)?;
                        polygon.offset = Some(offset);
                        offset
                    };
                    candidates.push(offset);
                }
            }
            candidates.sort_unstable();
            candidates.dedup();
            let header = if candidates.is_empty() {
                0
            } else if let Some(&header) = self.headers.get(&candidates) {
                header
            } else {
                let mut rows = Vec::with_capacity(candidates.len() + 1);
                rows.push(Vec4::new(candidates.len() as f32, 0.0, 0.0, 0.0));
                rows.extend(
                    candidates
                        .iter()
                        .map(|&offset| Vec4::new(offset as f32, 0.0, 0.0, 0.0)),
                );
                let header = append_rows(&mut self.data, &rows)?;
                self.headers.insert(candidates, header);
                header
            };
            triangle_headers.push(header);
        }
        if triangle_headers.iter().any(|&header| header != 0) {
            mesh.duplicate_vertices();
        }
        let Some(VertexAttributeValues::Float32x2(mut uv1)) =
            mesh.remove_attribute(Mesh::ATTRIBUTE_UV_1)
        else {
            unreachable!()
        };
        if triangle_headers.iter().all(|&header| header == 0) {
            for uv in &mut uv1 {
                uv[1] = 0.0;
            }
        } else {
            for (&header, triangle) in triangle_headers.iter().zip(uv1.as_chunks_mut::<3>().0) {
                for uv in triangle {
                    uv[1] = header as f32;
                }
            }
        }
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, uv1);
        Ok(mesh)
    }
}

fn append_rows(data: &mut Vec<Vec4>, rows: &[Vec4]) -> Result<u32, String> {
    if data.len().saturating_add(rows.len()) >= MAX_DATA_ROWS {
        return Err("Contact mask exceeds the exact interpolated float-index budget".into());
    }
    let offset = data.len() as u32;
    data.extend_from_slice(rows);
    Ok(offset)
}

struct MaskPolygon {
    normal: DVec3,
    origin: DVec3,
    tangent: DVec3,
    bitangent: DVec3,
    vertices: Vec<DVec3>,
    boundary: Vec<DVec2>,
    offset: Option<u32>,
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

    fn overlaps(&self, triangle: &[DVec3; 3], orientation: f64) -> bool {
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

    fn gpu_rows(&self) -> Vec<Vec4> {
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

fn prepare_patch(patch: &ContactPatch) -> Result<Vec<MaskPolygon>, String> {
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

#[cfg(test)]
mod tests;
