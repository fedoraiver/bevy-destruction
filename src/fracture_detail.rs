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
        if !bond.patches.is_empty() {
            result.extend(baked_contact_patches(
                bond,
                a,
                b,
                widths[bond.a.as_str()],
                widths[bond.b.as_str()],
            )?);
            continue;
        }
        if manifest.schema_version == 2 {
            return Err(format!(
                "Contact {} is missing v2 interface polygons",
                bond.id
            ));
        }
        let mut a_triangles: Vec<_> = a.triangles.iter().collect();
        let mut b_triangles: Vec<_> = b.triangles.iter().collect();
        a_triangles.sort_by(|a, b| triangle_order(a, b));
        b_triangles.sort_by(|a, b| triangle_order(a, b));
        // Skinny float32 triangles have unstable normals; keep their true outlines in one
        // contact frame instead of using each rendered triangle as a new clipping plane.
        let normal = Vec3::from_array(bond.normal).as_dvec3().normalize();
        let mut plane = Plane::new(normal, Vec3::from_array(bond.center).as_dvec3());
        for first in a_triangles {
            let first = first.map(|p| p.as_dvec3());
            let cross = (first[1] - first[0]).cross(first[2] - first[0]);
            if cross.length_squared() < MIN_AREA * MIN_AREA
                || normal.dot(cross.normalize()) < 0.98
                || first
                    .iter()
                    .any(|point| normal.dot(*point - plane.origin).abs() > CONTACT_TOLERANCE)
            {
                continue;
            }
            for second in &b_triangles {
                let second = second.map(|p| p.as_dvec3());
                let second_cross = (second[1] - second[0]).cross(second[2] - second[0]);
                if second_cross.length_squared() < MIN_AREA * MIN_AREA
                    || normal.dot(second_cross.normalize()) > -0.98
                    || second
                        .iter()
                        .any(|point| normal.dot(*point - plane.origin).abs() > CONTACT_TOLERANCE)
                {
                    continue;
                }
                if (0..3).any(|axis| {
                    let min_a = first.iter().map(|p| p[axis]).fold(f64::INFINITY, f64::min);
                    let max_a = first
                        .iter()
                        .map(|p| p[axis])
                        .fold(f64::NEG_INFINITY, f64::max);
                    let min_b = second.iter().map(|p| p[axis]).fold(f64::INFINITY, f64::min);
                    let max_b = second
                        .iter()
                        .map(|p| p[axis])
                        .fold(f64::NEG_INFINITY, f64::max);
                    max_a < min_b - CONTACT_TOLERANCE || max_b < min_a - CONTACT_TOLERANCE
                }) {
                    continue;
                }
                let subject: Vec<_> = first.iter().map(|p| plane.project(*p)).collect();
                let boundary: Vec<_> = second.iter().map(|p| plane.project(*p)).collect();
                let overlap = clip_polygon(subject, &boundary);
                if polygon_area(&overlap).abs() > MIN_AREA {
                    plane.polygons.push(overlap);
                }
            }
        }
        let first_patch = result.len();
        for polygon in connected_polygons(&plane.polygons)
            .map_err(|error| format!("{} plane {:?}: {error}", bond.id, plane.normal))?
        {
            let points: Vec<_> = polygon.iter().map(|p| plane.unproject(*p)).collect();
            let triangles = (1..points.len() - 1)
                .map(|i| [points[0], points[i], points[i + 1]])
                .collect();
            let rims = patch_rims(
                &points,
                a,
                b,
                widths[bond.a.as_str()],
                widths[bond.b.as_str()],
            );
            result.push(ContactPatch {
                id: String::new(),
                bond_id: bond.id.clone(),
                a: bond.a.clone(),
                b: bond.b.clone(),
                normal: plane.normal.as_vec3(),
                triangles,
                area: polygon_area(&polygon).abs() as f32,
                rims,
            });
        }
        let patches = &mut result[first_patch..];
        patches.sort_by(|a, b| triangle_order(&a.triangles[0], &b.triangles[0]));
        let area: f32 = patches.iter().map(|patch| patch.area).sum();
        if patches.is_empty() || (area - bond.area).abs() > (bond.area * 0.01).max(0.00002) {
            return Err(format!(
                "Contact {} mesh area {area:.7} does not match bond area {:.7}",
                bond.id, bond.area
            ));
        }
        for (index, patch) in patches.iter_mut().enumerate() {
            patch.id = format!("{}:patch:{index:03}", bond.id);
        }
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
            || triangle
                .iter()
                .any(|point| plane.normal.dot(*point - plane.origin).abs() > CONTACT_TOLERANCE)
        {
            continue;
        }
        let projected = triangle.iter().map(|point| plane.project(*point)).collect();
        let overlap = clip_polygon(projected, boundary);
        let area = polygon_area(&overlap).abs();
        if area <= 1e-14 {
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
mod tests {
    use super::*;
    use crate::asset::{BondRecord, Coordinates};
    use bevy::mesh::VertexAttributeValues;

    fn cube(min: Vec3, max: Vec3, hidden_axis: usize, hidden_positive: bool) -> SurfaceGeometry {
        let mut geometry = SurfaceGeometry::default();
        for axis in 0..3 {
            for positive in [false, true] {
                let u = (axis + 1) % 3;
                let v = (axis + 2) % 3;
                let mut points = [min; 4];
                for (i, point) in points.iter_mut().enumerate() {
                    point[axis] = if positive { max[axis] } else { min[axis] };
                    point[u] = if i == 1 || i == 2 { max[u] } else { min[u] };
                    point[v] = if i >= 2 { max[v] } else { min[v] };
                }
                if !positive {
                    points.reverse();
                }
                let triangles = [
                    [points[0], points[1], points[2]],
                    [points[0], points[2], points[3]],
                ];
                geometry.triangles.extend(triangles);
                if axis != hidden_axis || positive != hidden_positive {
                    geometry.exterior.extend(triangles);
                }
            }
        }
        geometry
    }

    fn manifest(area: f32) -> Manifest {
        Manifest {
            schema: "bevy_destruction_asset".into(),
            schema_version: 1,
            name: "test".into(),
            asset_id: String::new(),
            coordinates: Coordinates {
                units: "meters".into(),
                up: "+Y".into(),
                handedness: "right".into(),
            },
            materials: BTreeMap::new(),
            chunks: Vec::new(),
            bonds: vec![BondRecord {
                id: "bond".into(),
                a: "a".into(),
                b: "b".into(),
                area,
                center: [0.0, 0.5, 0.5],
                normal: [1.0, 0.0, 0.0],
                area_covariance: [[0.0; 3]; 3],
                patches: Vec::new(),
                state: "rigid".into(),
            }],
            anchors: Vec::new(),
            mesh_file: String::new(),
            mesh_sha256: String::new(),
        }
    }

    fn adjacent(thickness: f32) -> BTreeMap<String, SurfaceGeometry> {
        BTreeMap::from([
            (
                "a".into(),
                cube(
                    Vec3::new(-1.0, 0.0, 0.0),
                    Vec3::new(0.0, 1.0, thickness),
                    0,
                    true,
                ),
            ),
            (
                "b".into(),
                cube(Vec3::ZERO, Vec3::new(1.0, 1.0, thickness), 0, false),
            ),
        ])
    }

    #[test]
    fn exact_overlap_merges_triangle_diagonals_and_keeps_real_rim() {
        let patches = build_patches(&manifest(1.0), &adjacent(1.0)).unwrap();
        assert_eq!(patches.len(), 1);
        assert!((patches[0].area - 1.0).abs() < 1e-6);
        assert_eq!(patches[0].triangles.len(), 2);
        assert_eq!(patches[0].rims.len(), 4);
        assert!(patches[0].normal.dot(Vec3::X) > 0.999);
        assert!(patches[0].exposed_survivor(|_| true).is_none());
        assert_eq!(patches[0].exposed_survivor(|id| id == "a"), Some("a"));
        assert_eq!(patches[0].exposed_survivor(|id| id == "b"), Some("b"));
        assert!(patches[0].exposed_survivor(|_| false).is_none());
    }

    fn rectangular_patch(vertices: [[f32; 3]; 4]) -> crate::asset::BondPatchRecord {
        let points = vertices.map(Vec3::from_array);
        let first = points[1] - points[0];
        let second = points[3] - points[0];
        crate::asset::BondPatchRecord {
            area: first.cross(second).length(),
            center: ((points[0] + points[2]) * 0.5).to_array(),
            normal: first.cross(second).normalize().to_array(),
            area_covariance: [[0.0; 3]; 3],
            vertices: vertices.into(),
        }
    }

    #[test]
    fn baked_coplanar_facets_merge_without_visual_internal_seams() {
        let mut manifest = manifest(1.0);
        manifest.schema_version = 2;
        manifest.bonds[0].patches = [0.0, 0.5]
            .into_iter()
            .map(|y| {
                rectangular_patch([
                    [0.0, y, 0.0],
                    [0.0, y + 0.5, 0.0],
                    [0.0, y + 0.5, 1.0],
                    [0.0, y, 1.0],
                ])
            })
            .collect();
        let patches = build_patches(&manifest, &adjacent(1.0)).unwrap();
        assert_eq!(patches.len(), 1);
        assert_eq!(patches[0].rims.len(), 4);
        assert!((patches[0].area - 1.0).abs() < 1e-6);
        let original = patches[0].triangles.clone();
        manifest.bonds[0].patches.reverse();
        assert_eq!(
            build_patches(&manifest, &adjacent(1.0)).unwrap()[0].triangles,
            original
        );
    }

    #[test]
    fn baked_folded_interfaces_use_actual_faces_and_reject_fabricated_coverage() {
        let mut manifest = manifest(1.0);
        manifest.schema_version = 2;
        manifest.bonds[0].patches = vec![
            rectangular_patch([
                [0.0, 0.0, 0.0],
                [0.0, 0.5, 0.0],
                [0.0, 0.5, 1.0],
                [0.0, 0.0, 1.0],
            ]),
            rectangular_patch([
                [0.0, 0.5, 0.0],
                [0.1, 1.0, 0.0],
                [0.1, 1.0, 1.0],
                [0.0, 0.5, 1.0],
            ]),
        ];
        let mut triangles = Vec::new();
        for patch in &manifest.bonds[0].patches {
            let p = patch
                .vertices
                .iter()
                .copied()
                .map(Vec3::from_array)
                .collect::<Vec<_>>();
            triangles.extend([[p[0], p[1], p[2]], [p[0], p[2], p[3]]]);
        }
        let mut geometry = BTreeMap::from([
            (
                "a".into(),
                SurfaceGeometry {
                    triangles: triangles.clone(),
                    exterior: Vec::new(),
                },
            ),
            (
                "b".into(),
                SurfaceGeometry {
                    triangles: triangles.iter().map(|t| [t[0], t[2], t[1]]).collect(),
                    exterior: Vec::new(),
                },
            ),
        ]);
        let patches = build_patches(&manifest, &geometry).unwrap();
        assert_eq!(patches.len(), 2);
        assert!(patches[0].normal.dot(patches[1].normal) < 0.999);
        assert!(patches.iter().all(|patch| patch.rims.is_empty()));
        geometry.get_mut("b").unwrap().triangles.truncate(2);
        assert!(build_patches(&manifest, &geometry).is_err());
        geometry.get_mut("b").unwrap().triangles =
            vec![[triangles[0][0], triangles[0][2], triangles[0][1]]; 2];
        assert!(build_patches(&manifest, &geometry).is_err());
    }

    #[test]
    fn a_baked_facet_below_area_tolerance_still_needs_actual_mesh_coverage() {
        let mut manifest = manifest(1.0);
        manifest.schema_version = 2;
        manifest.bonds[0].patches = vec![rectangular_patch([
            [0.0, 0.0, 0.0],
            [0.0, 1e-5, 0.0],
            [0.0, 1e-5, 1e-5],
            [0.0, 0.0, 1e-5],
        ])];
        let geometry = BTreeMap::from([
            ("a".into(), SurfaceGeometry::default()),
            ("b".into(), SurfaceGeometry::default()),
        ]);
        assert!(build_patches(&manifest, &geometry).is_err());
    }

    #[test]
    fn baked_coplanar_merge_does_not_fill_a_resolvable_small_notch() {
        let mut manifest = manifest(1.0);
        manifest.schema_version = 2;
        manifest.bonds[0].patches = vec![
            rectangular_patch([
                [0.0, 0.0, 0.0],
                [0.0, 0.5, 0.0],
                [0.0, 0.5, 1.0],
                [0.0, 0.0, 1.0],
            ]),
            rectangular_patch([
                [0.0, 0.5, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 1.0, 0.99999],
                [0.0, 0.5, 0.99999],
            ]),
        ];
        let triangles = manifest.bonds[0]
            .patches
            .iter()
            .flat_map(|patch| {
                let p = patch
                    .vertices
                    .iter()
                    .copied()
                    .map(Vec3::from_array)
                    .collect::<Vec<_>>();
                [[p[0], p[1], p[2]], [p[0], p[2], p[3]]]
            })
            .collect::<Vec<_>>();
        let geometry = BTreeMap::from([
            (
                "a".into(),
                SurfaceGeometry {
                    triangles: triangles.clone(),
                    exterior: Vec::new(),
                },
            ),
            (
                "b".into(),
                SurfaceGeometry {
                    triangles: triangles.iter().map(|p| [p[0], p[2], p[1]]).collect(),
                    exterior: Vec::new(),
                },
            ),
        ]);
        let patches = build_patches(&manifest, &geometry).unwrap();
        assert_eq!(patches.len(), 2);
        let actual = patches.iter().map(|patch| patch.area).sum::<f32>();
        let expected = manifest.bonds[0]
            .patches
            .iter()
            .map(|patch| patch.area)
            .sum::<f32>();
        assert!((actual - expected).abs() < 1e-7);
    }

    #[test]
    fn partial_contact_does_not_mark_uncovered_larger_face_edges() {
        let mut geometry = adjacent(1.0);
        geometry.insert(
            "b".into(),
            cube(
                Vec3::new(0.0, 0.25, 0.25),
                Vec3::new(1.0, 0.75, 0.75),
                0,
                false,
            ),
        );
        let patches = build_patches(&manifest(0.25), &geometry).unwrap();
        assert_eq!(patches.len(), 1);
        assert!((patches[0].area - 0.25).abs() < 1e-6);
        assert!(
            patches[0]
                .rims
                .iter()
                .all(|rim| rim.exterior_a.is_none() && rim.exterior_b.is_some())
        );
        assert!(detail_mesh(&patches[0], "a").is_none());
    }

    #[test]
    fn disjoint_contacts_keep_the_gap_and_reject_wrong_bond_area() {
        let first = vec![DVec2::ZERO, DVec2::X, DVec2::Y];
        let second: Vec<_> = first.iter().map(|p| *p + DVec2::new(3.0, 0.0)).collect();
        let components = connected_polygons(&[first, second]).unwrap();
        assert_eq!(components.len(), 2);
        assert!(build_patches(&manifest(2.0), &adjacent(1.0)).is_err());
    }

    #[test]
    fn hull_welds_close_vertices_even_when_lexical_sort_interleaves_another_corner() {
        let points = vec![
            DVec2::ZERO,
            DVec2::new(1.0, 0.0),
            DVec2::new(1.0 + 1e-12, 1.0),
            DVec2::new(1.0 + 2e-12, 1e-12),
            DVec2::Y,
        ];
        let hull = convex_hull(points);
        assert_eq!(hull.len(), 4);
        assert!((polygon_area(&hull) - 1.0).abs() < 1e-8);
    }

    #[test]
    fn hull_keeps_extreme_corner_with_almost_equal_coordinates() {
        let hull = convex_hull(vec![
            DVec2::new(-0.22 - 1e-17, -4.06),
            DVec2::new(-0.22, -5.02),
            DVec2::new(-0.22, -4.64),
            DVec2::new(0.0, -5.02),
            DVec2::new(0.0, -4.06),
        ]);
        assert_eq!(hull.len(), 4);
        assert!((polygon_area(&hull) - 0.2112).abs() < 1e-8);
    }

    #[test]
    fn lip_geometry_is_closed_deterministic_and_thin_wall_bounded() {
        let patches = build_patches(&manifest(0.02), &adjacent(0.02)).unwrap();
        let mesh = detail_mesh(&patches[0], "a").unwrap();
        let VertexAttributeValues::Float32x3(points) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap()
        else {
            panic!()
        };
        assert!(
            points
                .iter()
                .all(|p| p.iter().all(|v| v.is_finite()) && p[0] <= 0.0017)
        );
        let mut edges = BTreeMap::new();
        let key = |point: [f32; 3]| point.map(|p| (p * 1000000.0).round() as i32);
        for triangle in points.as_chunks::<3>().0 {
            let p = triangle.map(Vec3::from_array);
            assert!((p[1] - p[0]).cross(p[2] - p[0]).length_squared() > 1e-18);
            for (a, b) in [
                (triangle[0], triangle[1]),
                (triangle[1], triangle[2]),
                (triangle[2], triangle[0]),
            ] {
                let (a, b) = (key(a), key(b));
                *edges.entry((a.min(b), a.max(b))).or_insert(0) += 1;
            }
        }
        assert!(edges.values().all(|count| *count == 2));
        let again = detail_mesh(&patches[0], "a").unwrap();
        assert_eq!(
            mesh.attribute(Mesh::ATTRIBUTE_POSITION),
            again.attribute(Mesh::ATTRIBUTE_POSITION)
        );
        assert!(detail_mesh(&patches[0], "unrelated").is_none());
    }

    #[test]
    fn oblique_lips_keep_embedded_roots_on_the_survivor_side() {
        for obliqueness in [-0.96_f32, 0.96] {
            let exterior = Vec3::new(obliqueness, 0.0, (1.0 - obliqueness.powi(2)).sqrt());
            let patch = ContactPatch {
                id: "oblique".into(),
                bond_id: "bond".into(),
                a: "a".into(),
                b: "b".into(),
                normal: Vec3::X,
                triangles: vec![],
                area: 1.0,
                rims: vec![RimEdge {
                    points: [Vec3::ZERO, Vec3::Y],
                    exterior_a: Some(exterior),
                    exterior_b: None,
                    thickness_a: 0.02,
                    thickness_b: 0.02,
                }],
            };
            let mesh = detail_mesh(&patch, "a").unwrap();
            let VertexAttributeValues::Float32x3(points) =
                mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap()
            else {
                panic!()
            };
            let VertexAttributeValues::Float32x4(colors) =
                mesh.attribute(Mesh::ATTRIBUTE_COLOR).unwrap()
            else {
                panic!()
            };
            for (point, color) in points.iter().zip(colors) {
                if color[0] == 1.0 {
                    assert!(point[0] < 0.0, "embedded root left the survivor");
                }
            }
        }
    }

    fn fixture_geometry(manifest: &Manifest, bytes: &[u8]) -> BTreeMap<String, SurfaceGeometry> {
        let json_length = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        let document: serde_json::Value =
            serde_json::from_slice(&bytes[20..20 + json_length]).unwrap();
        let binary = &bytes[28 + json_length..];
        let nodes: BTreeMap<_, _> = document["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|node| (node["name"].as_str().unwrap(), node))
            .collect();
        let accessor = |index: usize, element_size: usize| {
            let accessor = &document["accessors"][index];
            let view = &document["bufferViews"][accessor["bufferView"].as_u64().unwrap() as usize];
            let offset = accessor["byteOffset"].as_u64().unwrap_or(0) as usize
                + view["byteOffset"].as_u64().unwrap_or(0) as usize;
            (
                offset,
                view["byteStride"]
                    .as_u64()
                    .map_or(element_size, |value| value as usize),
                accessor["count"].as_u64().unwrap() as usize,
            )
        };
        let mut result = BTreeMap::new();
        for chunk in &manifest.chunks {
            let node = nodes[chunk.node.as_str()];
            let primitives =
                document["meshes"][node["mesh"].as_u64().unwrap() as usize]["primitives"]
                    .as_array()
                    .unwrap();
            let mut geometry = SurfaceGeometry::default();
            for primitive in primitives {
                let (offset, stride, count) = accessor(
                    primitive["attributes"]["POSITION"].as_u64().unwrap() as usize,
                    12,
                );
                let positions: Vec<_> = (0..count)
                    .map(|index| {
                        let offset = offset + index * stride;
                        Vec3::from_array(std::array::from_fn(|axis| {
                            f32::from_le_bytes(
                                binary[offset + axis * 4..offset + axis * 4 + 4]
                                    .try_into()
                                    .unwrap(),
                            )
                        }))
                    })
                    .collect();
                let indices: Vec<usize> = if let Some(index) = primitive["indices"].as_u64() {
                    let component = document["accessors"][index as usize]["componentType"]
                        .as_u64()
                        .unwrap();
                    let size = match component {
                        5121 => 1,
                        5123 => 2,
                        5125 => 4,
                        _ => panic!("Unsupported fixture index"),
                    };
                    let (offset, stride, count) = accessor(index as usize, size);
                    (0..count)
                        .map(|index| {
                            let offset = offset + index * stride;
                            match component {
                                5121 => usize::from(binary[offset]),
                                5123 => usize::from(u16::from_le_bytes(
                                    binary[offset..offset + 2].try_into().unwrap(),
                                )),
                                _ => u32::from_le_bytes(
                                    binary[offset..offset + 4].try_into().unwrap(),
                                ) as usize,
                            }
                        })
                        .collect()
                } else {
                    (0..positions.len()).collect()
                };
                let exterior = document["materials"]
                    [primitive["material"].as_u64().unwrap() as usize]["extras"]["bd_surface_role"]
                    == "exterior";
                for indices in indices.as_chunks::<3>().0 {
                    let triangle =
                        indices.map(|index| chunk.transform().transform_point(positions[index]));
                    geometry.triangles.push(triangle);
                    if exterior {
                        geometry.exterior.push(triangle);
                    }
                }
            }
            result.insert(chunk.id.clone(), geometry);
        }
        result
    }

    #[test]
    fn bundled_contact_caches_match_every_bond_and_cache_only_real_exterior_rims() {
        let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/buildings");
        for name in ["rectangle", "l_shape", "courtyard"] {
            let manifest: Manifest = serde_json::from_slice(
                &std::fs::read(directory.join(format!("{name}.destruction.json"))).unwrap(),
            )
            .unwrap();
            let bytes = std::fs::read(directory.join(&manifest.mesh_file)).unwrap();
            let geometry = fixture_geometry(&manifest, &bytes);
            let start = std::time::Instant::now();
            let patches = build_patches(&manifest, &geometry)
                .unwrap_or_else(|error| panic!("{name}: {error}"));
            assert!(
                patches.len()
                    >= manifest
                        .bonds
                        .iter()
                        .filter(|bond| bond.area >= MIN_VISUAL_CONTACT_AREA)
                        .count()
            );
            let mut maximum_error = 0.0_f32;
            for bond in &manifest.bonds {
                let area: f32 = patches
                    .iter()
                    .filter(|patch| patch.bond_id == bond.id)
                    .map(|patch| patch.area)
                    .sum();
                maximum_error = maximum_error.max((area - bond.area).abs());
                assert!((area - bond.area).abs() <= (bond.area * 0.01).max(0.00002));
            }
            let mut triangles = 0;
            let excluded: Vec<_> = manifest
                .bonds
                .iter()
                .filter(|bond| bond.area < MIN_VISUAL_CONTACT_AREA)
                .collect();
            for patch in &patches {
                for survivor in [&patch.a, &patch.b] {
                    if let Some(mesh) = detail_mesh(patch, survivor) {
                        let VertexAttributeValues::Float32x3(points) =
                            mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap()
                        else {
                            panic!()
                        };
                        assert!(points.iter().flatten().all(|value| value.is_finite()));
                        for triangle in points.as_chunks::<3>().0 {
                            let [a, b, c] = triangle.map(Vec3::from_array);
                            assert!((b - a).cross(c - a).length_squared() > 1e-18);
                        }
                        triangles += mesh.count_vertices() / 3;
                    }
                }
            }
            println!(
                "{name}: {} patches, {} rims, all-side upper-bound {triangles} detail triangles, maximum area error {maximum_error:.8}m2, excluded {} visual-only slivers totaling {:.12}m2, cache+geometry {:.2}ms",
                patches.len(),
                patches.iter().map(|patch| patch.rims.len()).sum::<usize>(),
                excluded.len(),
                excluded.iter().map(|bond| bond.area as f64).sum::<f64>(),
                start.elapsed().as_secs_f64() * 1000.0
            );
        }
    }
}
