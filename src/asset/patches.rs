use bevy::math::{DVec2, DVec3};

use super::manifest::{BondPatchRecord, BondRecord, finite3, positive, valid_symmetric_tensor};

const PATCH_POSITION_TOLERANCE: f64 = 4e-5;
const PATCH_METRIC_TOLERANCE: f64 = 0.002;

pub(super) struct PatchMetrics {
    area: f64,
    center: DVec3,
    normal: DVec3,
    covariance: [[f64; 3]; 3],
}

pub(super) fn patch_metrics(patch: &BondPatchRecord) -> Result<PatchMetrics, String> {
    if !(3..=1024).contains(&patch.vertices.len())
        || !positive(patch.area)
        || !finite3(patch.center)
        || !finite3(patch.normal)
        || !valid_symmetric_tensor(patch.area_covariance, false)
        || patch.vertices.iter().any(|point| !finite3(*point))
    {
        return Err("面片的顶点或物理量无效".into());
    }
    let normal = DVec3::from_array(patch.normal.map(f64::from));
    if (normal.length_squared() - 1.0).abs() > 1e-3 {
        return Err("面片法线不是单位向量".into());
    }
    let normal = normal.normalize();
    let points = patch
        .vertices
        .iter()
        .map(|point| DVec3::from_array(point.map(f64::from)))
        .collect::<Vec<_>>();
    let origin = points[0];
    let geometric_area = (1..points.len() - 1)
        .map(|index| (points[index] - origin).cross(points[index + 1] - origin))
        .sum::<DVec3>()
        * 0.5;
    if geometric_area.length() <= 1e-12 || geometric_area.normalize().dot(normal) < 1.0 - 1e-5 {
        return Err("面片法线与顶点几何方向不符".into());
    }
    let normal = geometric_area.normalize();
    if points
        .iter()
        .any(|point| normal.dot(*point - origin).abs() > PATCH_POSITION_TOLERANCE)
    {
        return Err("面片顶点不共面".into());
    }
    for (index, &a) in points.iter().enumerate() {
        let edge = points[(index + 1) % points.len()] - a;
        if edge.length_squared() <= 1e-20
            || points
                .iter()
                .any(|point| normal.dot(edge.cross(*point - a)) < -1e-10 * edge.length())
        {
            return Err("面片必须为按 a→b 法线绕序的凸多边形".into());
        }
    }
    let mut area = 0.0;
    let mut first = DVec3::ZERO;
    let mut triangles = Vec::new();
    for index in 1..points.len() - 1 {
        let a = points[index] - origin;
        let b = points[index + 1] - origin;
        let triangle_area = normal.dot(a.cross(b)) * 0.5;
        if triangle_area < -1e-12 {
            return Err("面片绕序无效".into());
        }
        area += triangle_area;
        first += (a + b) * (triangle_area / 3.0);
        triangles.push((a, b, triangle_area));
    }
    if area <= 1e-12 {
        return Err("面片没有可解析的二维面积".into());
    }
    let relative_center = first / area;
    let center = origin + relative_center;
    let mut covariance = [[0.0; 3]; 3];
    for (a, b, triangle_area) in triangles {
        let points = [-relative_center, a - relative_center, b - relative_center];
        let sum = points.iter().copied().sum::<DVec3>();
        for row in 0..3 {
            for column in 0..3 {
                covariance[row][column] += triangle_area / 12.0
                    * (sum[row] * sum[column]
                        + points
                            .iter()
                            .map(|point| point[row] * point[column])
                            .sum::<f64>());
            }
        }
    }
    if (area - f64::from(patch.area)).abs() > (area * PATCH_METRIC_TOLERANCE).max(1e-12)
        || center.distance(DVec3::from_array(patch.center.map(f64::from)))
            > PATCH_POSITION_TOLERANCE
        || !covariance_matches(&covariance, &patch.area_covariance)
    {
        return Err("面片面积、中心或二阶矩与真实顶点不符".into());
    }
    let tangent = normal.any_orthonormal_vector();
    let bitangent = normal.cross(tangent);
    let quadratic = |a: DVec3, b: DVec3| {
        (0..3)
            .map(|row| {
                (0..3)
                    .map(|column| {
                        a[row] * f64::from(patch.area_covariance[row][column]) * b[column]
                    })
                    .sum::<f64>()
            })
            .sum::<f64>()
    };
    let qt = quadratic(tangent, tangent);
    let qb = quadratic(bitangent, bitangent);
    let qtb = quadratic(tangent, bitangent);
    if qt <= 0.0 || qb <= 0.0 || qt * qb - qtb * qtb <= 0.0 {
        return Err("面片二阶矩没有可解析的二维截面".into());
    }
    Ok(PatchMetrics {
        area,
        center,
        normal,
        covariance,
    })
}

fn covariance_matches(expected: &[[f64; 3]; 3], actual: &[[f32; 3]; 3]) -> bool {
    let scale = expected
        .iter()
        .flatten()
        .copied()
        .map(f64::abs)
        .fold(0.0, f64::max);
    expected
        .iter()
        .flatten()
        .zip(actual.iter().flatten())
        .all(|(expected, actual)| {
            (expected - f64::from(*actual)).abs() <= (scale * PATCH_METRIC_TOLERANCE).max(1e-24)
        })
}

pub(super) fn validate_bond_patches(
    bond: &BondRecord,
    bounds_a: (DVec3, DVec3),
    bounds_b: (DVec3, DVec3),
) -> Result<(), String> {
    if bond.patches.len() > 8192 {
        return Err("接触面片数量超过 8192".into());
    }
    let mut metrics = Vec::with_capacity(bond.patches.len());
    for patch in &bond.patches {
        let metric = patch_metrics(patch)?;
        for vertex in &patch.vertices {
            let point = DVec3::from_array(vertex.map(f64::from));
            if [bounds_a, bounds_b].into_iter().any(|(min, max)| {
                point
                    .cmplt(min - DVec3::splat(PATCH_POSITION_TOLERANCE))
                    .any()
                    || point
                        .cmpgt(max + DVec3::splat(PATCH_POSITION_TOLERANCE))
                        .any()
            }) {
                return Err("接触面片超出端点碰撞几何范围".into());
            }
        }
        metrics.push(metric);
    }
    for (index, first) in bond.patches.iter().enumerate() {
        for second in &bond.patches[..index] {
            if coplanar_overlap(first, second)
                > f64::from(first.area.min(second.area)) * PATCH_METRIC_TOLERANCE
            {
                return Err("同一连接的接触面片重复或重叠".into());
            }
        }
    }
    let area = metrics.iter().map(|metric| metric.area).sum::<f64>();
    let center = metrics
        .iter()
        .map(|metric| metric.center * metric.area)
        .sum::<DVec3>()
        / area;
    let normal = metrics
        .iter()
        .map(|metric| metric.normal * metric.area)
        .sum::<DVec3>();
    if normal.length() <= area * 1e-6 {
        return Err("接触面片法线近似相消".into());
    }
    let mut covariance = [[0.0; 3]; 3];
    for metric in &metrics {
        let offset = metric.center - center;
        for row in 0..3 {
            for column in 0..3 {
                covariance[row][column] +=
                    metric.covariance[row][column] + metric.area * offset[row] * offset[column];
            }
        }
    }
    if (area - f64::from(bond.area)).abs() > area * PATCH_METRIC_TOLERANCE
        || center.distance(DVec3::from_array(bond.center.map(f64::from))) > PATCH_POSITION_TOLERANCE
        || normal
            .normalize()
            .distance(DVec3::from_array(bond.normal.map(f64::from)))
            > 1e-3
        || !covariance_matches(&covariance, &bond.area_covariance)
    {
        return Err("连接汇总数据与真实面片不符".into());
    }
    Ok(())
}

fn coplanar_overlap(first: &BondPatchRecord, second: &BondPatchRecord) -> f64 {
    let normal = DVec3::from_array(first.normal.map(f64::from)).normalize();
    let origin = DVec3::from_array(first.vertices[0].map(f64::from));
    if normal
        .dot(DVec3::from_array(second.normal.map(f64::from)).normalize())
        .abs()
        < 1.0 - 1e-8
        || second.vertices.iter().any(|point| {
            normal
                .dot(DVec3::from_array(point.map(f64::from)) - origin)
                .abs()
                > 1e-6
        })
    {
        return 0.0;
    }
    let tangent = normal.any_orthonormal_vector();
    let bitangent = normal.cross(tangent);
    let project = |point: &[f32; 3]| {
        let point = DVec3::from_array(point.map(f64::from)) - origin;
        DVec2::new(point.dot(tangent), point.dot(bitangent))
    };
    let boundary = first.vertices.iter().map(project).collect::<Vec<_>>();
    let mut polygon = second.vertices.iter().map(project).collect::<Vec<_>>();
    for index in 0..boundary.len() {
        if polygon.is_empty() {
            return 0.0;
        }
        let a = boundary[index];
        let edge = boundary[(index + 1) % boundary.len()] - a;
        let mut output = Vec::new();
        let mut previous = *polygon.last().unwrap();
        let mut previous_side = edge.perp_dot(previous - a);
        for &point in &polygon {
            let side = edge.perp_dot(point - a);
            if (side >= 0.0) != (previous_side >= 0.0) {
                output.push(previous.lerp(point, previous_side / (previous_side - side)));
            }
            if side >= 0.0 {
                output.push(point);
            }
            previous = point;
            previous_side = side;
        }
        polygon = output;
    }
    if polygon.len() < 3 {
        return 0.0;
    }
    let origin = polygon[0];
    polygon
        .iter()
        .zip(polygon.iter().cycle().skip(1))
        .take(polygon.len())
        .map(|(a, b)| (*a - origin).perp_dot(*b - origin))
        .sum::<f64>()
        .abs()
        * 0.5
}
