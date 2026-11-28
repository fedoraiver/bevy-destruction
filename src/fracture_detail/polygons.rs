use bevy::math::DVec2;

use super::CONTACT_TOLERANCE;

const MIN_AREA: f64 = 1e-10;

pub(super) fn polygon_area(points: &[DVec2]) -> f64 {
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

pub(super) fn clip_polygon(mut subject: Vec<DVec2>, boundary: &[DVec2]) -> Vec<DVec2> {
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

pub(super) fn convex_hull(mut points: Vec<DVec2>) -> Vec<DVec2> {
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

pub(super) fn connected_polygons(polygons: &[Vec<DVec2>]) -> Result<Vec<Vec<DVec2>>, String> {
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
