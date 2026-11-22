use std::collections::BTreeMap;

use bevy::{
    asset::RenderAssetUsages,
    math::{DVec2, DVec3},
    mesh::PrimitiveTopology,
    prelude::*,
};

use crate::{asset::Manifest, rim_profile::rim_profile};

const CONTACT_TOLERANCE: f64 = 0.00004;
const MIN_AREA: f64 = 1e-10;
pub const MIN_VISUAL_CONTACT_AREA: f32 = 1e-7;
const MIN_RIM_LENGTH: f32 = 0.002;
const MAX_RIM_DEPTH: f32 = 0.02;

#[derive(Clone, Default)]
pub struct SurfaceGeometry {
    pub triangles: Vec<[Vec3; 3]>,
    pub exterior: Vec<[Vec3; 3]>,
}

#[derive(Clone, Debug)]
pub struct RimEdge {
    pub points: [Vec3; 2],
    pub exterior_a: Option<Vec3>,
    pub exterior_b: Option<Vec3>,
    pub thickness_a: f32,
    pub thickness_b: f32,
}

#[derive(Clone, Debug)]
pub struct ContactPatch {
    pub id: String,
    pub bond_id: String,
    pub a: String,
    pub b: String,
    pub normal: Vec3,
    pub triangles: Vec<[Vec3; 3]>,
    pub area: f32,
    pub rims: Vec<RimEdge>,
}

impl ContactPatch {
    pub fn exposed_survivor(&self, alive: impl Fn(&str) -> bool) -> Option<&str> {
        match (alive(&self.a), alive(&self.b)) {
            (true, false) => Some(&self.a),
            (false, true) => Some(&self.b),
            _ => None,
        }
    }
}

#[cfg(any(test, feature = "acceptance-test"))]
pub fn contact_surface_mesh(patches: &[ContactPatch], chunk: &str) -> Option<Mesh> {
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

struct Plane {
    normal: DVec3,
    origin: DVec3,
    tangent: DVec3,
    bitangent: DVec3,
    polygons: Vec<Vec<DVec2>>,
}

impl Plane {
    fn new(normal: DVec3, point: DVec3) -> Self {
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

    fn project(&self, point: DVec3) -> DVec2 {
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

fn mesh_patch_coverage(
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

fn polygon_area(points: &[DVec2]) -> f64 {
    if points.len() < 3 {
        return 0.0;
    }
    let origin = points[0];
    points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
        .map(|(a, b)| (*a - origin).perp_dot(*b - origin))
        .sum::<f64>()
        * 0.5
}

fn clip_polygon(mut subject: Vec<DVec2>, boundary: &[DVec2]) -> Vec<DVec2> {
    let sign = polygon_area(boundary).signum();
    for (a, b) in boundary
        .iter()
        .zip(boundary.iter().cycle().skip(1))
        .take(boundary.len())
    {
        if subject.is_empty() {
            break;
        }
        let edge = (*b - *a) * sign;
        let mut output = Vec::new();
        let mut previous = *subject.last().unwrap();
        let mut previous_distance = edge.perp_dot(previous - *a);
        for current in subject {
            let distance = edge.perp_dot(current - *a);
            if (distance >= 0.0) != (previous_distance >= 0.0) {
                output.push(
                    previous.lerp(current, previous_distance / (previous_distance - distance)),
                );
            }
            if distance >= 0.0 {
                output.push(current);
            }
            previous = current;
            previous_distance = distance;
        }
        subject = output;
    }
    subject
}

fn polygons_touch(a: &[DVec2], b: &[DVec2]) -> bool {
    for (p, q) in a.iter().zip(a.iter().cycle().skip(1)).take(a.len()) {
        let edge = *q - *p;
        let length = edge.length();
        if length <= CONTACT_TOLERANCE {
            continue;
        }
        let tangent = edge / length;
        for (r, s) in b.iter().zip(b.iter().cycle().skip(1)).take(b.len()) {
            if tangent.perp_dot(*r - *p).abs() > CONTACT_TOLERANCE
                || tangent.perp_dot(*s - *p).abs() > CONTACT_TOLERANCE
            {
                continue;
            }
            let first = tangent.dot(*r - *p);
            let second = tangent.dot(*s - *p);
            if first.max(second).min(length) - first.min(second).max(0.0) > CONTACT_TOLERANCE {
                return true;
            }
        }
    }
    false
}

fn convex_hull(mut points: Vec<DVec2>) -> Vec<DVec2> {
    points.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    let mut unique: Vec<DVec2> = Vec::new();
    for point in points {
        if !unique.iter().any(|other| {
            other.distance_squared(point) < CONTACT_TOLERANCE * CONTACT_TOLERANCE * 0.01
        }) {
            unique.push(point);
        }
    }
    let points = unique;
    let mut hull: Vec<DVec2> = Vec::new();
    for &point in &points {
        while hull.len() >= 2
            && (hull[hull.len() - 1] - hull[hull.len() - 2]).perp_dot(point - hull[hull.len() - 1])
                <= 0.0
        {
            hull.pop();
        }
        hull.push(point);
    }
    let lower = hull.len();
    for &point in points.iter().rev().skip(1) {
        while hull.len() > lower
            && (hull[hull.len() - 1] - hull[hull.len() - 2]).perp_dot(point - hull[hull.len() - 1])
                <= 0.0
        {
            hull.pop();
        }
        hull.push(point);
    }
    hull.pop();
    let mut index = 0;
    while hull.len() > 3 && index < hull.len() {
        let previous = hull[(index + hull.len() - 1) % hull.len()];
        let current = hull[index];
        let next = hull[(index + 1) % hull.len()];
        let chord = next - previous;
        let along = (current - previous).dot(chord) / chord.length_squared();
        let distance = (current - previous).perp_dot(chord).abs() / chord.length();
        if (0.0..=1.0).contains(&along) && distance < CONTACT_TOLERANCE * 0.1 {
            hull.remove(index);
            index = 0;
        } else {
            index += 1;
        }
    }
    hull
}

fn connected_polygons(polygons: &[Vec<DVec2>]) -> Result<Vec<Vec<DVec2>>, String> {
    let mut remaining: Vec<_> = (0..polygons.len()).collect();
    let mut result = Vec::new();
    while let Some(first) = remaining.pop() {
        let mut component = vec![first];
        let mut cursor = 0;
        while cursor < component.len() {
            let current = component[cursor];
            let mut index = 0;
            while index < remaining.len() {
                if polygons_touch(&polygons[current], &polygons[remaining[index]]) {
                    component.push(remaining.swap_remove(index));
                } else {
                    index += 1;
                }
            }
            cursor += 1;
        }
        let area: f64 = component
            .iter()
            .map(|index| polygon_area(&polygons[*index]).abs())
            .sum();
        let hull = convex_hull(
            component
                .iter()
                .flat_map(|index| polygons[*index].iter().copied())
                .collect(),
        );
        let hull_area = polygon_area(&hull).abs();
        if (area - hull_area).abs() > (area * 0.005).max(0.00001) {
            return Err(format!(
                "Contact mesh is non-convex or overlapping: polygon area {area:.7}, hull area {hull_area:.7}"
            ));
        }
        if hull_area > MIN_AREA {
            result.push(hull);
        }
    }
    Ok(result)
}

fn minimum_width(surface: &SurfaceGeometry) -> f32 {
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

fn patch_rims(
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

#[cfg(test)]
mod tests;
