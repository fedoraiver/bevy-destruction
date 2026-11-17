"""Find finite contact patches between convex chunks, including across parts."""

import hashlib
import math

from mathutils import Matrix, Vector


def _cross(a, b):
    return a[0] * b[1] - a[1] * b[0]


def _sub(a, b):
    return (a[0] - b[0], a[1] - b[1])


def _polygon_area(points):
    return sum(_cross(a, b) for a, b in zip(points, points[1:] + points[:1])) * 0.5


def _minimum_width(points):
    widths = []
    for a, b in zip(points, points[1:] + points[:1]):
        edge = _sub(b, a)
        length = math.hypot(*edge)
        if length > 0:
            distances = [_cross(edge, _sub(p, a)) / length for p in points]
            widths.append(max(distances) - min(distances))
    return min(widths, default=0.0)


def _clip(subject, boundary, tolerance):
    if _polygon_area(boundary) < 0:
        boundary = list(reversed(boundary))
    for a, b in zip(boundary, boundary[1:] + boundary[:1]):
        if not subject:
            break
        edge = _sub(b, a)
        output = []
        previous = subject[-1]
        previous_distance = _cross(edge, _sub(previous, a))
        for current in subject:
            distance = _cross(edge, _sub(current, a))
            if (distance >= -tolerance) != (previous_distance >= -tolerance):
                factor = max(
                    0.0, min(1.0, previous_distance / (previous_distance - distance))
                )
                output.append(
                    tuple(
                        previous[i] + factor * (current[i] - previous[i])
                        for i in range(2)
                    )
                )
            if distance >= -tolerance:
                output.append(current)
            previous, previous_distance = current, distance
        subject = output
    return subject


def _area_centroid(points):
    twice_area = sum(_cross(a, b) for a, b in zip(points, points[1:] + points[:1]))
    if abs(twice_area) < 1e-12:
        return 0.0, (0.0, 0.0)
    centroid = [
        sum(
            (a[i] + b[i]) * _cross(a, b)
            for a, b in zip(points, points[1:] + points[:1])
        )
        / (3 * twice_area)
        for i in range(2)
    ]
    return abs(twice_area) * 0.5, centroid


def _area_covariance(points, center):
    shifted = [(p[0] - center[0], p[1] - center[1]) for p in points]
    xx = yy = xy = 0.0
    for a, b in zip(shifted, shifted[1:] + shifted[:1]):
        cross = _cross(a, b)
        xx += (a[0] ** 2 + a[0] * b[0] + b[0] ** 2) * cross / 12
        yy += (a[1] ** 2 + a[1] * b[1] + b[1] ** 2) * cross / 12
        xy += (
            (2 * a[0] * a[1] + a[0] * b[1] + b[0] * a[1] + 2 * b[0] * b[1]) * cross / 24
        )
    sign = 1 if _polygon_area(shifted) >= 0 else -1
    return [[xx * sign, xy * sign], [xy * sign, yy * sign]]


def _faces(obj):
    return [
        (face.normal.copy(), [obj.data.vertices[i].co.copy() for i in face.vertices])
        for face in obj.data.polygons
    ]


def _bounds(obj):
    points = [v.co for v in obj.data.vertices]
    return (
        [min(v[i] for v in points) for i in range(3)],
        [max(v[i] for v in points) for i in range(3)],
    )


def _face_contacts(faces_a, faces_b, tolerance, minimum_width=None):
    """Yield real convex overlap polygons in a common oriented face basis."""
    for normal, points_a in faces_a:
        origin = points_a[0]
        axis = Vector((1, 0, 0)) if abs(normal.x) < 0.8 else Vector((0, 1, 0))
        tangent = normal.cross(axis).normalized()
        bitangent = normal.cross(tangent)
        a2 = [
            ((p - origin).dot(tangent), (p - origin).dot(bitangent)) for p in points_a
        ]
        for other_normal, points_b in faces_b:
            if normal.dot(other_normal) > -1 + 1e-5:
                continue
            b2 = [
                ((p - origin).dot(tangent), (p - origin).dot(bitangent))
                for p in points_b
            ]
            overlap = _clip(a2, b2, tolerance * tolerance)
            if len(overlap) < 3:
                continue
            area, _ = _area_centroid(overlap)
            width_limit = tolerance if minimum_width is None else minimum_width
            if area <= 1e-16 or _minimum_width(overlap) <= width_limit:
                continue
            # A small serialized face can have a noisy normal. Only the actual
            # shared footprint must lie on B; distant vertices of B are irrelevant.
            if any(
                abs(
                    other_normal.dot(
                        origin + tangent * point[0] + bitangent * point[1] - points_b[0]
                    )
                )
                > tolerance
                for point in overlap
            ):
                continue
            yield normal, origin, tangent, bitangent, overlap


def _contact(faces_a, faces_b, tolerance, detailed=False):
    total_area = 0.0
    center_sum = Vector()
    normal_sum = Vector()
    patches = []
    polygons = []
    for normal, origin, tangent, bitangent, overlap in _face_contacts(
        faces_a, faces_b, tolerance, minimum_width=0.0 if detailed else None
    ):
        area, center = _area_centroid(overlap)
        total_area += area
        world_center = origin + tangent * center[0] + bitangent * center[1]
        center_sum += world_center * area
        normal_sum += normal * area
        covariance = _area_covariance(overlap, center)
        basis = (tangent, bitangent)
        tensor = Matrix(
            [
                [
                    sum(
                        basis[a][i] * covariance[a][b] * basis[b][j]
                        for a in range(2)
                        for b in range(2)
                    )
                    for j in range(3)
                ]
                for i in range(3)
            ]
        )
        patches.append((area, world_center, tensor))
        polygons.append(
            [tuple(origin + tangent * p[0] + bitangent * p[1]) for p in overlap]
        )
    if total_area <= tolerance * tolerance:
        return None
    center = center_sum / total_area
    tensor = Matrix(((0, 0, 0), (0, 0, 0), (0, 0, 0)))
    for area, patch_center, patch_tensor in patches:
        offset = patch_center - center
        tensor += patch_tensor + Matrix(
            [[area * offset[i] * offset[j] for j in range(3)] for i in range(3)]
        )
    if detailed:
        polygons = merge_coplanar_polygons(polygons, tolerance)
    return total_area, center, normal_sum.normalized(), tensor, polygons


def merge_coplanar_polygons(polygons, tolerance=2e-5):
    """Merge only convex, face-adjacent coplanar coverage, keeping concavities."""

    def dot(a, b):
        return math.fsum(x * y for x, y in zip(a, b))

    def frame(poly):
        origin = poly[0]
        edges = [tuple(p[i] - origin[i] for i in range(3)) for p in poly]
        normal = tuple(
            math.fsum(
                (a[(i + 1) % 3] * b[(i + 2) % 3] - a[(i + 2) % 3] * b[(i + 1) % 3])
                for a, b in zip(edges, edges[1:] + edges[:1])
            )
            for i in range(3)
        )
        length = math.sqrt(dot(normal, normal))
        if length <= 1e-20:
            return None
        normal = tuple(v / length for v in normal)
        tangent = max(edges, key=lambda p: dot(p, p))
        tangent_length = math.sqrt(dot(tangent, tangent))
        tangent = tuple(v / tangent_length for v in tangent)
        bitangent = tuple(
            normal[(i + 1) % 3] * tangent[(i + 2) % 3]
            - normal[(i + 2) % 3] * tangent[(i + 1) % 3]
            for i in range(3)
        )
        return origin, normal, tangent, bitangent

    def hull(points):
        points = sorted(set(points))
        if len(points) < 3:
            return points
        sides = []
        for values in (points, list(reversed(points))):
            chain = []
            for point in values:
                while (
                    len(chain) > 1
                    and _cross(_sub(chain[-1], chain[-2]), _sub(point, chain[-1])) <= 0
                ):
                    chain.pop()
                chain.append(point)
            sides.extend(chain[:-1])
        return sides

    remaining = [list(dict.fromkeys(tuple(p) for p in poly)) for poly in polygons]
    remaining = [poly for poly in remaining if len(poly) >= 3]
    changed = True
    while changed:
        changed = False
        for first in range(len(remaining)):
            basis = frame(remaining[first])
            if basis is None:
                continue
            origin, normal, tangent, bitangent = basis
            project = lambda p: (
                dot(tuple(p[i] - origin[i] for i in range(3)), tangent),
                dot(tuple(p[i] - origin[i] for i in range(3)), bitangent),
            )
            a = [project(p) for p in remaining[first]]
            for second in range(first + 1, len(remaining)):
                other = frame(remaining[second])
                if other is None or dot(normal, other[1]) < 1 - 1e-8:
                    continue
                if any(
                    abs(dot(tuple(p[i] - origin[i] for i in range(3)), normal))
                    > tolerance * 0.1
                    for p in remaining[second]
                ):
                    continue
                b = [project(p) for p in remaining[second]]
                overlap = _clip(a, b, 0.0)
                overlap_area = abs(_polygon_area(overlap)) if len(overlap) >= 3 else 0.0
                all_points = {
                    project(p): p for p in remaining[first] + remaining[second]
                }
                outline = hull(list(all_points))
                hull_area = abs(_polygon_area(outline))
                union_area = (
                    abs(_polygon_area(a)) + abs(_polygon_area(b)) - overlap_area
                )
                # Hull equality forbids filling a finite notch, opening or separated gap.
                if abs(hull_area - union_area) > max(union_area * 2e-9, 1e-13):
                    continue
                remaining[first] = [all_points[p] for p in outline]
                remaining.pop(second)
                changed = True
                break
            if changed:
                break
    return remaining


def generate_bonds(chunks, tolerance=2e-5, detailed=False):
    """Return contacts in building-local Blender coordinates; normal points a -> b."""
    entries = [(chunk, _bounds(chunk.obj), _faces(chunk.obj)) for chunk in chunks]
    entries.sort(key=lambda item: (item[1][0][0], item[0].id))
    bonds = []
    active = []
    for chunk_b, bounds_b, faces_b in entries:
        active = [
            entry for entry in active if entry[1][1][0] >= bounds_b[0][0] - tolerance
        ]
        for chunk_a, bounds_a, faces_a in active:
            if any(
                bounds_a[1][i] < bounds_b[0][i] - tolerance
                or bounds_b[1][i] < bounds_a[0][i] - tolerance
                for i in (1, 2)
            ):
                continue
            patch = _contact(faces_a, faces_b, tolerance, detailed=detailed)
            if patch is None:
                continue
            area, center, normal, tensor, polygons = patch
            a, b = chunk_a.id, chunk_b.id
            if a > b:
                a, b = b, a
                normal = -normal
                polygons = [list(reversed(poly)) for poly in polygons]
            identifier = hashlib.sha256((a + "|" + b).encode()).hexdigest()[:24]
            bonds.append(
                {
                    "id": "bond_" + identifier,
                    "a": a,
                    "b": b,
                    "area": area,
                    "center": list(center),
                    "normal": list(normal),
                    "area_covariance": [list(row) for row in tensor],
                }
            )
            if detailed:
                bonds[-1]["patch_vertices"] = polygons
        active.append((chunk_b, bounds_b, faces_b))
    return sorted(bonds, key=lambda bond: bond["id"])


def disconnected_chunks(chunks, bonds, anchors):
    adjacent = {chunk.id: [] for chunk in chunks}
    for bond in bonds:
        adjacent[bond["a"]].append(bond["b"])
        adjacent[bond["b"]].append(bond["a"])
    supported = set(anchors)
    pending = list(supported)
    while pending:
        for other in adjacent[pending.pop()]:
            if other not in supported:
                supported.add(other)
                pending.append(other)
    return sorted(set(adjacent) - supported)
