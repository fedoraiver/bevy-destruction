"""Isolated curved-wall partition proof. Run with background factory-startup Blender."""

import collections
import argparse
import heapq
import hashlib
import json
import math
from pathlib import Path
import random
import sys
import time

import bpy
from mathutils import Vector
from mathutils.bvhtree import BVHTree

ROOT = next(
    parent
    for parent in Path(__file__).resolve().parents
    if (parent / "Cargo.toml").is_file()
)
sys.path.insert(0, str(ROOT / "tools/blender"))
from bevy_destruction.fracture import (
    mesh_metrics,
    validate_convex,
    _mesh_bmesh,
    _face_normal,
)

OUT = ROOT / "output/irregular-fracture/natural"
WIDTH, HEIGHT, DEPTH = 6.0, 4.0, 0.2
WINDOW = (2.4, 3.6, 1.2, 2.8)
SEED = 20261117
TARGET_SIZE = 1.4
AREA_FACTORS = (0.25, 2.75)
MAX_DIAGONAL_FACTOR = 3.0
MAX_ASPECT = 6.0
MIN_COMPACTNESS = 0.12
MAX_CONFIG_ATTEMPTS = 12
NOISE_OCTAVES = ((2.5, 0.32), (1.0, 0.12), (0.45, 0.04))
CURRENT_CONFIG = {}


def seeded_values(seed, target_size=None):
    target_size = TARGET_SIZE if target_size is None else target_size
    rng = random.Random(seed)
    solid_area = WIDTH * HEIGHT - (WINDOW[1] - WINDOW[0]) * (WINDOW[3] - WINDOW[2])
    count = max(4, round(solid_area / target_size**2))
    minimum_distance = target_size * 0.65
    padding = target_size * 0.12
    sites = []
    for _ in range(count):
        candidates = []
        for _ in range(128):
            point = (
                rng.uniform(padding, WIDTH - padding),
                rng.uniform(padding, HEIGHT - padding),
            )
            if (
                WINDOW[0] - padding < point[0] < WINDOW[1] + padding
                and WINDOW[2] - padding < point[1] < WINDOW[3] + padding
            ):
                continue
            distance = min(
                (math.dist(point, other) for other in sites), default=math.inf
            )
            candidates.append((distance, point))
        if not candidates:
            raise ValueError("No source-domain site candidates")
        distance, point = (
            max(candidates, key=lambda item: item[0]) if sites else candidates[0]
        )
        if distance < minimum_distance:
            raise ValueError(
                "Blue-noise placement could not meet minimum site separation"
            )
        sites.append(point)
    return tuple(sites), seed


SITES, FIELD_SEED = seeded_values(SEED)
PALETTE = (
    (0.18, 0.43, 0.52, 1),
    (0.49, 0.64, 0.56, 1),
    (0.83, 0.63, 0.39, 1),
    (0.68, 0.38, 0.35, 1),
    (0.40, 0.43, 0.64, 1),
)


def area(poly):
    return (
        math.fsum(a[0] * b[1] - a[1] * b[0] for a, b in zip(poly, poly[1:] + poly[:1]))
        * 0.5
    )


def edges(poly):
    return list(zip(poly, poly[1:] + poly[:1]))


def canonical(point):
    return (round(point[0], 9), round(point[1], 9))


def edge_key(a, b):
    return tuple(sorted((a, b)))


def window_overlap_area(poly):
    clipped = list(poly)
    for axis, bound, sign in (
        (0, WINDOW[0], 1),
        (0, WINDOW[1], -1),
        (1, WINDOW[2], 1),
        (1, WINDOW[3], -1),
    ):
        if not clipped:
            return 0.0
        result = []
        previous = clipped[-1]
        dp = (previous[axis] - bound) * sign
        for current in clipped:
            dc = (current[axis] - bound) * sign
            if (dp >= 0) != (dc >= 0):
                ratio = dp / (dp - dc)
                result.append(
                    tuple(a + ratio * (b - a) for a, b in zip(previous, current))
                )
            if dc >= 0:
                result.append(current)
            previous, dp = current, dc
        clipped = result
    return abs(area(clipped)) if len(clipped) >= 3 else 0.0


def hash_grid(x, z, seed):
    value = (x * 0x9E3779B185EBCA87 ^ z * 0xC2B2AE3D27D4EB4F ^ seed) & ((1 << 64) - 1)
    value = ((value ^ (value >> 30)) * 0xBF58476D1CE4E5B9) & ((1 << 64) - 1)
    value = ((value ^ (value >> 27)) * 0x94D049BB133111EB) & ((1 << 64) - 1)
    return value ^ (value >> 31)


def gradient_noise(x, z, seed):
    ix, iz = math.floor(x), math.floor(z)
    fx, fz = x - ix, z - iz
    diagonal = math.sqrt(0.5)
    gradients = (
        (1, 0),
        (-1, 0),
        (0, 1),
        (0, -1),
        (diagonal, diagonal),
        (-diagonal, diagonal),
        (diagonal, -diagonal),
        (-diagonal, -diagonal),
    )

    def corner(dx, dz):
        gx, gz = gradients[hash_grid(ix + dx, iz + dz, seed) % 8]
        return gx * (fx - dx) + gz * (fz - dz)

    u = fx**3 * (fx * (fx * 6 - 15) + 10)
    v = fz**3 * (fz * (fz * 6 - 15) + 10)
    low = corner(0, 0) * (1 - u) + corner(1, 0) * u
    high = corner(0, 1) * (1 - u) + corner(1, 1) * u
    return low * (1 - v) + high * v


def score_displacement(point):
    x, z = point
    result = []
    for channel in range(2):
        value = 0.0
        for octave, (wavelength, amplitude) in enumerate(NOISE_OCTAVES):
            salt = hash_grid(channel, octave, FIELD_SEED)
            angle = (salt % 1000003) / 1000003 * math.tau
            cosine, sine = math.cos(angle), math.sin(angle)
            u = (cosine * x - sine * z) / (wavelength * TARGET_SIZE) + 17.173
            v = (sine * x + cosine * z) / (wavelength * TARGET_SIZE) - 8.419
            value += TARGET_SIZE * amplitude * gradient_noise(u, v, salt)
        result.append(value)
    return tuple(result)


def scores(point, curved):
    x, z = point
    if curved:
        dx, dz = score_displacement(point)
        x, z = x + dx, z + dz
    return tuple((x - a) ** 2 + (z - b) ** 2 for a, b in SITES)


def clip(poly, own, other):
    result = []
    previous = poly[-1]
    dp = previous[2][own] - previous[2][other]
    for current in poly:
        dc = current[2][own] - current[2][other]
        if (dp <= 0) != (dc <= 0):
            ratio = dp / (dp - dc)
            result.append(
                (
                    previous[0] + ratio * (current[0] - previous[0]),
                    previous[1] + ratio * (current[1] - previous[1]),
                    tuple(a + ratio * (b - a) for a, b in zip(previous[2], current[2])),
                )
            )
        if dc <= 0:
            result.append(current)
        previous, dp = current, dc
    return result


def partition(step, curved):
    nx, nz = round(WIDTH / step), round(HEIGHT / step)
    polygons = collections.defaultdict(list)
    source_triangles = 0
    minimum_area = math.inf
    score_cache = {}
    for ix in range(nx):
        for iz in range(nz):
            x, z = (ix + 0.5) * step, (iz + 0.5) * step
            if WINDOW[0] < x < WINDOW[1] and WINDOW[2] < z < WINDOW[3]:
                continue
            corners = [
                (ix * step, iz * step),
                ((ix + 1) * step, iz * step),
                ((ix + 1) * step, (iz + 1) * step),
                (ix * step, (iz + 1) * step),
            ]
            if (ix + iz) % 2:
                triangles = ((0, 1, 3), (1, 2, 3))
            else:
                triangles = ((0, 1, 2), (0, 2, 3))
            for indices in triangles:
                source_triangles += 1
                triangle = []
                for i in indices:
                    point = corners[i]
                    if point not in score_cache:
                        score_cache[point] = scores(point, curved)
                    triangle.append((point[0], point[1], score_cache[point]))
                for own in range(len(SITES)):
                    poly = triangle
                    for other in range(len(SITES)):
                        if own != other and poly:
                            poly = clip(poly, own, other)
                    points = []
                    for point in poly:
                        point = canonical(point)
                        if not points or point != points[-1]:
                            points.append(point)
                    if len(points) > 1 and points[0] == points[-1]:
                        points.pop()
                    if len(points) < 3:
                        continue
                    value = area(points)
                    if value <= 0:
                        if abs(value) > 1e-13:
                            raise ValueError("Wrong polygon orientation")
                        continue
                    minimum_area = min(minimum_area, value)
                    polygons[own].append(points)
    connected = {}
    for label, pieces in sorted(polygons.items()):
        owners = collections.defaultdict(list)
        for index, piece in enumerate(pieces):
            for a, b in edges(piece):
                owners[edge_key(a, b)].append(index)
        adjacent = collections.defaultdict(set)
        for entries in owners.values():
            for first in entries:
                adjacent[first].update(other for other in entries if other != first)
        remaining = set(range(len(pieces)))
        component = 0
        while remaining:
            first = min(remaining)
            remaining.remove(first)
            pending, indices = [first], {first}
            while pending:
                neighbors = adjacent[pending.pop()].intersection(remaining)
                remaining.difference_update(neighbors)
                indices.update(neighbors)
                pending.extend(neighbors)
            connected[label + len(SITES) * component] = [
                pieces[index] for index in sorted(indices)
            ]
            component += 1
    return connected, source_triangles, minimum_area


def section_quality(polygons):
    points = [p for polygon in polygons for p in polygon]
    section_area = math.fsum(area(poly) for poly in polygons)
    dx = max(p[0] for p in points) - min(p[0] for p in points)
    dz = max(p[1] for p in points) - min(p[1] for p in points)
    perimeter = math.fsum(math.dist(a, b) for a, b in boundary_edges(polygons))
    return {
        "area_m2": section_area,
        "volume_m3": section_area * DEPTH,
        "aabb_diagonal_m": math.hypot(dx, dz),
        "aabb_max_dimension_m": max(dx, dz),
        "aabb_aspect_ratio": max(dx, dz) / min(dx, dz),
        "compactness_4pi_area_over_perimeter2": 4
        * math.pi
        * section_area
        / perimeter**2,
    }


def normalize_chunk_sizes(groups):
    groups = {label: list(polygons) for label, polygons in groups.items()}
    minimum, maximum = (value * TARGET_SIZE**2 for value in AREA_FACTORS)
    initial_area = math.fsum(
        area(poly) for polygons in groups.values() for poly in polygons
    )
    transfers = []
    while True:
        areas = {
            label: math.fsum(area(poly) for poly in polygons)
            for label, polygons in groups.items()
        }
        small = sorted(
            (value, label) for label, value in areas.items() if value < minimum
        )
        if not small:
            break
        value, label = small[0]
        owner = collections.defaultdict(list)
        for other, polygons in groups.items():
            for a, b in boundary_edges(polygons):
                owner[edge_key(a, b)].append(other)
        neighbors = collections.defaultdict(float)
        for key, labels in owner.items():
            if label in labels:
                for other in labels:
                    if other != label:
                        neighbors[other] += math.dist(*key)
        candidates = [
            (length, other)
            for other, length in neighbors.items()
            if areas[other] + value <= maximum
        ]
        if not candidates:
            raise ValueError(
                f"Tiny component {label} area={value:.6g} has no face-adjacent merge within the maximum area"
            )
        shared, other = max(candidates, key=lambda item: (item[0], -item[1]))
        groups[other].extend(groups.pop(label))
        transfers.append(
            {
                "from_component": label,
                "to_component": other,
                "transferred_area_m2": value,
                "shared_boundary_length_m": shared,
                "reason": "below minimum connected chunk area",
            }
        )
    qualities = {label: section_quality(polygons) for label, polygons in groups.items()}
    for label, quality in qualities.items():
        if not minimum <= quality["area_m2"] <= maximum:
            raise ValueError(
                f"Component {label} area outside [{minimum},{maximum}]: {quality}"
            )
        if (
            quality["aabb_diagonal_m"] > MAX_DIAGONAL_FACTOR * TARGET_SIZE
            or quality["aabb_aspect_ratio"] > MAX_ASPECT
            or quality["compactness_4pi_area_over_perimeter2"] < MIN_COMPACTNESS
        ):
            raise ValueError(
                f"Component {label} failed dimension/aspect/compactness bounds: {quality}"
            )
    final_area = math.fsum(q["area_m2"] for q in qualities.values())
    assert abs(initial_area - final_area) < 1e-8
    return groups, {
        "merges": transfers,
        "qualities": qualities,
        "area_before_m2": initial_area,
        "area_after_m2": final_area,
        "volume_before_m3": initial_area * DEPTH,
        "volume_after_m3": final_area * DEPTH,
        "mass_before_kg_at_2400": initial_area * DEPTH * 2400,
        "mass_after_kg_at_2400": final_area * DEPTH * 2400,
    }


def select_configuration(seed, target_size, step):
    global SITES, FIELD_SEED, TARGET_SIZE, CURRENT_CONFIG
    TARGET_SIZE = target_size
    failures = []
    for attempt in range(MAX_CONFIG_ATTEMPTS):
        actual_seed = seed + attempt * 104729
        try:
            SITES, FIELD_SEED = seeded_values(actual_seed, target_size)
            for spacing, curved in ((step * 2, False), (step * 2, True), (step, True)):
                normalize_chunk_sizes(partition(spacing, curved)[0])
        except ValueError as error:
            failures.append(
                {
                    "attempt": attempt,
                    "configuration_seed": actual_seed,
                    "reason": str(error),
                }
            )
            CURRENT_CONFIG = {
                "requested_seed": seed,
                "rejected_attempts": list(failures),
                "attempt_limit": MAX_CONFIG_ATTEMPTS,
            }
            continue
        CURRENT_CONFIG = {
            "requested_seed": seed,
            "accepted_configuration_seed": actual_seed,
            "attempt": attempt,
            "rejected_attempts": failures,
            "attempt_limit": MAX_CONFIG_ATTEMPTS,
            "site_minimum_separation_m": min(
                math.dist(a, b) for i, a in enumerate(SITES) for b in SITES[:i]
            ),
        }
        return CURRENT_CONFIG
    raise ValueError(
        f"No accepted configuration after {MAX_CONFIG_ATTEMPTS} attempts: {failures}"
    )


def union_convex(a, b):
    boundary = set(edges(a))
    shared = False
    for edge in edges(b):
        reverse = tuple(reversed(edge))
        if reverse in boundary:
            boundary.remove(reverse)
            shared = True
        else:
            boundary.add(edge)
    if not shared:
        return None
    links = {}
    for first, second in boundary:
        if first in links:
            return None
        links[first] = second
    start = min(links)
    result = [start]
    while links[result[-1]] != start:
        result.append(links[result[-1]])
        if len(result) > len(links):
            return None
    if len(result) != len(links):
        return None
    for previous, current, following in zip(
        result[-1:] + result[:-1], result, result[1:] + result[:1]
    ):
        cross = (current[0] - previous[0]) * (following[1] - current[1]) - (
            current[1] - previous[1]
        ) * (following[0] - current[0])
        if cross < -1e-9:
            return None
    for first, second in edges(result):
        dx, dz = second[0] - first[0], second[1] - first[1]
        if min(
            dx * (point[1] - first[1]) - dz * (point[0] - first[0]) for point in result
        ) < -1e-8 * math.hypot(dx, dz):
            return None
    if abs(area(result) - area(a) - area(b)) > 1e-8:
        raise ValueError("Convex merge changed area")
    return result


def merge_convex(polygons):
    active = dict(enumerate(polygons))
    owners = collections.defaultdict(set)
    queue = []
    serial = len(active)

    def insert(index, poly):
        for a, b in edges(poly):
            key = edge_key(a, b)
            for other in owners[key]:
                heapq.heappush(
                    queue, (-((a[0] - b[0]) ** 2 + (a[1] - b[1]) ** 2), index, other)
                )
            owners[key].add(index)

    for index, poly in active.items():
        insert(index, poly)
    while queue:
        _, first, second = heapq.heappop(queue)
        if first not in active or second not in active:
            continue
        combined = union_convex(active[first], active[second])
        if combined is None:
            continue
        for old in (first, second):
            for a, b in edges(active.pop(old)):
                owners[edge_key(a, b)].remove(old)
        active[serial] = combined
        insert(serial, combined)
        serial += 1
    return list(active.values())


def boundary_edges(polygons):
    boundary = set()
    for poly in polygons:
        for edge in edges(poly):
            reverse = tuple(reversed(edge))
            if reverse in boundary:
                boundary.remove(reverse)
            else:
                boundary.add(edge)
    return sorted(boundary)


def simplify_collinear(poly):
    result = list(poly)
    changed = True
    while changed and len(result) > 3:
        changed = False
        for index, current in enumerate(result):
            first, second = result[index - 1], result[(index + 1) % len(result)]
            length = math.dist(first, second)
            cross = abs(
                (second[0] - first[0]) * (current[1] - first[1])
                - (second[1] - first[1]) * (current[0] - first[0])
            )
            if length > 0 and cross / length < 1e-8:
                result.pop(index)
                changed = True
                break
    return result


def retriangulate_boundaries(groups):
    incident = collections.defaultdict(set)
    boundaries = {label: boundary_edges(polygons) for label, polygons in groups.items()}
    for boundary in boundaries.values():
        for a, b in boundary:
            incident[a].add(b)
            incident[b].add(a)
    removable = set()
    for current, neighbors in incident.items():
        if len(neighbors) != 2:
            continue
        first, second = sorted(neighbors)
        length = math.dist(first, second)
        cross = abs(
            (second[0] - first[0]) * (current[1] - first[1])
            - (second[1] - first[1]) * (current[0] - first[0])
        )
        if length > 0 and cross / length < 1e-8:
            removable.add(current)
    result = {}
    for label, boundary in boundaries.items():
        links = dict(boundary)
        assert len(links) == len(boundary)
        loops = []
        remaining = set(links)
        while remaining:
            start = min(remaining)
            loop = [start]
            remaining.remove(start)
            while links[loop[-1]] != start:
                following = links[loop[-1]]
                remaining.remove(following)
                loop.append(following)
            loops.append([point for point in loop if point not in removable])
        if len(loops) != 1:
            result[label] = groups[label]
            continue
        polygon = loops[0]
        triangles = ear_clip(polygon)
        assert (
            abs(
                sum(area(poly) for poly in triangles)
                - sum(area(poly) for poly in groups[label])
            )
            < 1e-7
        )
        result[label] = triangles
    return result


def ear_clip(polygon):
    def orient(a, b, c):
        return (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])

    remaining = list(range(len(polygon)))
    triangles = []
    while len(remaining) > 3:
        for position, index in enumerate(remaining):
            first, last = (
                remaining[position - 1],
                remaining[(position + 1) % len(remaining)],
            )
            a, b, c = polygon[first], polygon[index], polygon[last]
            if orient(a, b, c) <= 1e-13:
                continue
            if any(
                min(
                    orient(a, b, polygon[other]),
                    orient(b, c, polygon[other]),
                    orient(c, a, polygon[other]),
                )
                >= -1e-13
                for other in remaining
                if other not in (first, index, last)
            ):
                continue
            triangles.append([a, b, c])
            remaining.pop(position)
            break
        else:
            raise ValueError(
                (
                    "Cannot triangulate a simple boundary",
                    [polygon[index] for index in remaining],
                )
            )
    final = [polygon[index] for index in remaining]
    assert area(final) > 0
    triangles.append(final)
    return triangles


def object_mesh(name, polygons, boundary=None):
    vertices, faces, mapping = [], [], {}

    def vertex(point, y):
        key = (point[0], y, point[1])
        if key not in mapping:
            mapping[key] = len(vertices)
            vertices.append(key)
        return mapping[key]

    for poly in polygons:
        faces.append(tuple(vertex(p, -DEPTH / 2) for p in poly))
        faces.append(tuple(vertex(p, DEPTH / 2) for p in reversed(poly)))
    for a, b in boundary if boundary is not None else boundary_edges(polygons):
        faces.append(
            (
                vertex(a, -DEPTH / 2),
                vertex(a, DEPTH / 2),
                vertex(b, DEPTH / 2),
                vertex(b, -DEPTH / 2),
            )
        )
    mesh = bpy.data.meshes.new(name)
    mesh.from_pydata(vertices, [], faces)
    mesh.update(calc_edges=True)
    obj = bpy.data.objects.new(name, mesh)
    bpy.context.scene.collection.objects.link(obj)
    return obj


def interface_records(groups):
    owners = collections.defaultdict(list)
    for label, polygons in groups.items():
        for a, b in boundary_edges(polygons):
            owners[edge_key(a, b)].append((label, a, b))
    interfaces = collections.defaultdict(list)
    external = []
    for key, values in owners.items():
        if len(values) == 1:
            x, z = ((key[0][i] + key[1][i]) / 2 for i in range(2))
            on_source = min(abs(x), abs(x - WIDTH), abs(z), abs(z - HEIGHT)) < 1e-7
            on_window = (
                WINDOW[0] - 1e-7 <= x <= WINDOW[1] + 1e-7
                and min(abs(z - WINDOW[2]), abs(z - WINDOW[3])) < 1e-7
            ) or (
                WINDOW[2] - 1e-7 <= z <= WINDOW[3] + 1e-7
                and min(abs(x - WINDOW[0]), abs(x - WINDOW[1])) < 1e-7
            )
            assert on_source or on_window, ("Unmatched interior boundary", key)
            external.append(key)
            continue
        assert len(values) == 2, values
        (own, a, b), (other, c, d) = sorted(values)
        assert a == d and b == c
        length = math.dist(a, b)
        tangent = ((b[0] - a[0]) / length, 0.0, (b[1] - a[1]) / length)
        normal = (tangent[2], 0.0, -tangent[0])
        value = length * DEPTH
        center = ((a[0] + b[0]) / 2, 0.0, (a[1] + b[1]) / 2)
        q = [
            [
                value
                * (length**2 * tangent[i] * tangent[j] + DEPTH**2 * (i == 1) * (j == 1))
                / 12
                for j in range(3)
            ]
            for i in range(3)
        ]
        interfaces[(own, other)].append(
            {"area": value, "center": center, "normal": normal, "q": q, "edge": (a, b)}
        )
    records = []
    for pair, facets in sorted(interfaces.items()):
        total_area = math.fsum(f["area"] for f in facets)
        center = [
            math.fsum(f["area"] * f["center"][i] for f in facets) / total_area
            for i in range(3)
        ]
        normal = Vector(
            tuple(
                math.fsum(f["area"] * f["normal"][i] for f in facets) for i in range(3)
            )
        ).normalized()
        q = [
            [
                math.fsum(
                    f["q"][i][j]
                    + f["area"]
                    * (f["center"][i] - center[i])
                    * (f["center"][j] - center[j])
                    for f in facets
                )
                for j in range(3)
            ]
            for i in range(3)
        ]
        nonplanar = max(
            abs(sum(normal[i] * (p[i] - center[i]) for i in range(3)))
            for f in facets
            for p in (
                (f["edge"][0][0], 0, f["edge"][0][1]),
                (f["edge"][1][0], 0, f["edge"][1][1]),
            )
        )
        spread = max(
            math.acos(max(-1, min(1, Vector(f["normal"]).dot(normal)))) for f in facets
        )
        incident = collections.defaultdict(list)
        for facet in facets:
            for endpoint in facet["edge"]:
                incident[endpoint].append(Vector(facet["normal"]))
        adjacent_angle = max(
            (
                math.acos(max(-1, min(1, first.dot(second))))
                for ns in incident.values()
                for first in ns
                for second in ns
            ),
            default=0,
        )
        remaining = set(range(len(facets)))
        components = 0
        while remaining:
            components += 1
            pending = [remaining.pop()]
            while pending:
                current = pending.pop()
                ends = set(facets[current]["edge"])
                adjacent = {
                    other
                    for other in remaining
                    if ends.intersection(facets[other]["edge"])
                }
                pending.extend(adjacent)
                remaining.difference_update(adjacent)
        records.append(
            {
                "chunks": pair,
                "area": total_area,
                "center": center,
                "normal": list(normal),
                "q": q,
                "facet_count": len(facets),
                "connected_surface_components": components,
                "max_distance_from_average_plane_m": nonplanar,
                "max_normal_deviation_rad": spread,
                "facets": facets,
            }
        )
        records[-1]["max_adjacent_facet_angle_rad"] = adjacent_angle
    return records, external


def material(name, color):
    mat = bpy.data.materials.new(name)
    mat.diffuse_color = color
    mat.use_nodes = True
    bsdf = mat.node_tree.nodes.get("Principled BSDF")
    bsdf.inputs["Base Color"].default_value = color
    bsdf.inputs["Roughness"].default_value = 0.8
    return mat


def build_variant(step, curved):
    started = time.perf_counter()
    groups, triangles, minimum_area = partition(step, curved)
    raw_signature = hashlib.sha256(
        json.dumps(groups, sort_keys=True, separators=(",", ":")).encode()
    ).hexdigest()
    raw_piece_count = sum(len(polys) for polys in groups.values())
    groups, size_log = normalize_chunk_sizes(groups)
    groups = retriangulate_boundaries(groups)
    partition_ms = (time.perf_counter() - started) * 1000
    source_volume = (
        WIDTH * HEIGHT - (WINDOW[1] - WINDOW[0]) * (WINDOW[3] - WINDOW[2])
    ) * DEPTH
    prefix = ("irregular" if curved else "planar") + f"-seed{SEED}-step{step}"
    chunks, proxies = [], []
    pieces_before, pieces_after, proxy_vertices, proxy_triangles = 0, 0, 0, 0
    merge_started = time.perf_counter()
    merged = {label: merge_convex(polys) for label, polys in groups.items()}
    window_overlap = max(
        window_overlap_area(poly) for polygons in merged.values() for poly in polygons
    )
    assert window_overlap < 1e-9
    merge_ms = (time.perf_counter() - merge_started) * 1000
    for label, polygons in groups.items():
        obj = object_mesh(prefix + f"-chunk-{label}", polygons)
        metrics = mesh_metrics(obj)
        obj.data.materials.append(
            material(prefix + f"-material-{label}", PALETTE[label % len(PALETTE)])
        )
        obj["prototype_chunk"] = label
        obj["variant"] = prefix
        local_proxies = []
        for index, polygon in enumerate(merged[label]):
            proxy = object_mesh(
                prefix + f"-proxy-{label}-{index}", [simplify_collinear(polygon)]
            )
            try:
                validate_convex(proxy)
            except Exception:
                bm = _mesh_bmesh(proxy)
                diagnostics = []
                for face in bm.faces:
                    normal = _face_normal(face)
                    point = face.verts[0].co
                    distance = max(normal.dot(v.co - point) for v in bm.verts)
                    if distance > 1e-6:
                        diagnostics.append(
                            {
                                "normal": list(normal),
                                "distance": distance,
                                "face": [list(v.co) for v in face.verts],
                            }
                        )
                bm.free()
                (OUT / "invalid-proxy.json").write_text(
                    json.dumps(
                        {
                            "name": proxy.name,
                            "polygon": polygon,
                            "diagnostics": diagnostics,
                        },
                        indent=2,
                    )
                )
                raise
            pm = mesh_metrics(proxy)
            proxy.hide_render = True
            proxy.hide_viewport = True
            proxy["prototype_collision"] = True
            proxy_vertices += len(proxy.data.vertices)
            proxy.data.calc_loop_triangles()
            proxy_triangles += len(proxy.data.loop_triangles)
            local_proxies.append(pm["volume"])
            proxies.append(proxy)
        pieces_before += len(polygons)
        pieces_after += len(merged[label])
        assert abs(sum(local_proxies) - metrics["volume"]) < source_volume * 1e-5
        obj.data.calc_loop_triangles()
        chunks.append(
            {
                "label": label,
                "coarse_cell": label % len(SITES),
                "size_quality": size_log["qualities"][label],
                "volume": metrics["volume"],
                "center": metrics["center"],
                "inertia_density_one": metrics["inertia"],
                "mesh_vertices": len(obj.data.vertices),
                "mesh_triangles": len(obj.data.loop_triangles),
                "proxy_count": len(merged[label]),
                "proxy_vertices": [
                    [list(v.co) for v in proxy.data.vertices]
                    for proxy in proxies[-len(merged[label]) :]
                ],
            }
        )
    total_volume = math.fsum(c["volume"] for c in chunks)
    assert abs(total_volume - source_volume) < source_volume * 1e-5
    total_center = [
        math.fsum(c["volume"] * c["center"][i] for c in chunks) / total_volume
        for i in range(3)
    ]
    expected_center = (WIDTH / 2, 0, HEIGHT / 2)
    full = WIDTH * HEIGHT * DEPTH
    hole = (WINDOW[1] - WINDOW[0]) * (WINDOW[3] - WINDOW[2]) * DEPTH
    spans = (
        (DEPTH, HEIGHT, DEPTH, WINDOW[3] - WINDOW[2]),
        (WIDTH, HEIGHT, WINDOW[1] - WINDOW[0], WINDOW[3] - WINDOW[2]),
        (WIDTH, DEPTH, WINDOW[1] - WINDOW[0], DEPTH),
    )
    expected_inertia = [
        full * (a * a + b * b) / 12 - hole * (c * c + d * d) / 12
        for a, b, c, d in spans
    ]
    inertia = [
        [
            math.fsum(
                c["inertia_density_one"][i][j]
                + c["volume"]
                * (
                    (
                        sum(
                            (c["center"][a] - expected_center[a]) ** 2 for a in range(3)
                        )
                        if i == j
                        else 0
                    )
                    - (c["center"][i] - expected_center[i])
                    * (c["center"][j] - expected_center[j])
                )
                for c in chunks
            )
            for j in range(3)
        ]
        for i in range(3)
    ]
    inertia_error = max(
        abs(inertia[i][j] - (expected_inertia[i] if i == j else 0))
        for i in range(3)
        for j in range(3)
    ) / max(expected_inertia)
    assert math.dist(total_center, expected_center) < 1e-5
    assert inertia_error < 1e-5
    proxy_trees = [
        BVHTree.FromPolygons(
            [v.co for v in obj.data.vertices],
            [list(face.vertices) for face in obj.data.polygons],
        )
        for obj in proxies
    ]
    rays = []
    for x in (2.55, 3.0, 3.45):
        for z in (1.35, 2.0, 2.65):
            hit = any(
                tree.ray_cast(Vector((x, -1, z)), Vector((0, 1, 0)), 2)[0] is not None
                for tree in proxy_trees
            )
            assert not hit, (prefix, x, z)
            rays.append({"x": x, "z": z, "hit_proxy": hit})
    assert any(
        tree.ray_cast(Vector((0.5, -1, 2)), Vector((0, 1, 0)), 2)[0] is not None
        for tree in proxy_trees
    )
    interfaces, exterior = interface_records(groups)
    return {
        "variant": prefix,
        "curved": curved,
        "step_m": step,
        "chunks": chunks,
        "coarse_cell_count": len(SITES),
        "connected_chunk_count": len(chunks),
        "target_size_m": TARGET_SIZE,
        "size_normalization": size_log,
        "size_distribution": distribution(
            [c["size_quality"]["area_m2"] for c in chunks]
        ),
        "size_bounds": {
            "area_factors": AREA_FACTORS,
            "max_diagonal_factor": MAX_DIAGONAL_FACTOR,
            "max_aabb_aspect": MAX_ASPECT,
            "min_compactness": MIN_COMPACTNESS,
        },
        "configuration": dict(CURRENT_CONFIG),
        "original_micro_pieces": raw_piece_count,
        "raw_partition_signature": raw_signature,
        "source_triangles": triangles,
        "minimum_piece_area_m2": minimum_area,
        "source_volume_m3": source_volume,
        "total_volume_m3": total_volume,
        "volume_relative_error": abs(total_volume - source_volume) / source_volume,
        "total_mass_kg_at_2400": total_volume * 2400,
        "initial_convex_pieces": pieces_before,
        "total_center_m": total_center,
        "inertia_density_one_about_source_center": inertia,
        "source_inertia_relative_error": inertia_error,
        "merged_convex_proxies": pieces_after,
        "proxy_vertices": proxy_vertices,
        "proxy_triangles": proxy_triangles,
        "render_vertices": sum(c["mesh_vertices"] for c in chunks),
        "render_triangles": sum(c["mesh_triangles"] for c in chunks),
        "partition_ms": partition_ms,
        "convex_merge_ms": merge_ms,
        "build_validate_ms": (time.perf_counter() - started) * 1000,
        "window_clearance_rays": rays,
        "interfaces": interfaces,
        "max_proxy_overlap_window_area_m2": window_overlap,
        "errors": [],
    }, groups


def distribution(values):
    values = sorted(values)

    def percentile(fraction):
        offset = fraction * (len(values) - 1)
        first = math.floor(offset)
        last = math.ceil(offset)
        return values[first] + (values[last] - values[first]) * (offset - first)

    return {
        "minimum_area_m2": values[0],
        "p10_area_m2": percentile(0.1),
        "median_area_m2": percentile(0.5),
        "p90_area_m2": percentile(0.9),
        "maximum_area_m2": values[-1],
    }


def svg_panel(groups, interfaces, title, offset):
    palette = ["#347b92", "#7da38e", "#d3a164", "#ad6159", "#666ea3"]
    content = [f'<g transform="translate({offset},40) scale(70,-70)">']
    for label, polys in groups.items():
        for poly in polys:
            content.append(
                '<polygon points="'
                + " ".join(f"{x},{z-4}" for x, z in poly)
                + f'" fill="{palette[label%len(palette)]}"/>'
            )
    for interface in interfaces:
        for facet in interface["facets"]:
            a, b = facet["edge"]
            content.append(
                f'<path d="M{a[0]} {a[1]-4} L{b[0]} {b[1]-4}" stroke="#162331" stroke-width="0.017"/>'
            )
    content.append(
        f'</g><text x="{offset}" y="22" fill="#172b3a" font-family="sans-serif" font-size="18">{title}</text>'
    )
    return "".join(content)


def original_chunk_objects(variant):
    return [
        obj
        for obj in bpy.data.objects
        if obj.get("variant") == variant
        and "prototype_chunk" in obj
        and obj.name == f"{variant}-chunk-{int(obj['prototype_chunk'])}"
    ]


def render_comparison(variants, filename="comparison.png"):
    scene = bpy.context.scene
    scene.render.engine = "CYCLES"
    scene.cycles.device = "CPU"
    scene.cycles.samples = 24
    scene.cycles.use_denoising = True
    scene.render.resolution_x = 600 * len(variants)
    scene.render.resolution_y = 650
    scene.render.resolution_percentage = 100
    scene.world.use_nodes = True
    scene.world.node_tree.nodes["Background"].inputs["Color"].default_value = (
        0.65,
        0.7,
        0.75,
        1,
    )
    scene.world.node_tree.nodes["Background"].inputs["Strength"].default_value = 0.65
    scene.view_settings.view_transform = "AgX"
    for obj in bpy.data.objects:
        obj.hide_render = True
    source_objects = {
        variant["variant"]: original_chunk_objects(variant["variant"])
        for variant, _, _ in variants
    }
    dark = material("comparison fracture guides", (0.025, 0.04, 0.05, 1))
    for panel, (variant, title, explode) in enumerate(variants):
        offset = Vector((panel * 7.4, 0, 0))
        chunk_objects = source_objects[variant["variant"]]
        for obj in chunk_objects:
            displayed = obj.copy()
            displayed.data = obj.data.copy()
            scene.collection.objects.link(displayed)
            label = int(obj["prototype_chunk"])
            center = Vector(
                next(c["center"] for c in variant["chunks"] if c["label"] == label)
            )
            displacement = (
                Vector((center.x - 3, -0.5 * (label % 2), center.z - 2)) * 0.07
                if explode
                else Vector()
            )
            displayed.location = offset + displacement
            displayed.hide_render = False
        if not explode:
            for interface in variant["interfaces"]:
                for facet in interface["facets"]:
                    a, b = facet["edge"]
                    curve = bpy.data.curves.new("shared crack guide", "CURVE")
                    curve.dimensions = "3D"
                    curve.bevel_depth = 0.009
                    curve.bevel_resolution = 0
                    spline = curve.splines.new("POLY")
                    spline.points.add(1)
                    for point, p in zip(spline.points, (a, b)):
                        point.co = (p[0] + offset.x, -DEPTH / 2 - 0.008, p[1], 1)
                    guide = bpy.data.objects.new(
                        "crack location guide (display only)", curve
                    )
                    scene.collection.objects.link(guide)
                    guide.data.materials.append(dark)
        text = bpy.data.curves.new(title, "FONT")
        text.body = title
        text.align_x = "LEFT"
        text.size = 0.28
        caption = bpy.data.objects.new(title, text)
        scene.collection.objects.link(caption)
        caption.location = offset + Vector((0, -0.2, 4.45))
        caption.rotation_euler = (math.pi / 2, 0, 0)
        text.materials.append(dark)
    bpy.ops.mesh.primitive_plane_add(size=200, location=(10, 0, -0.4))
    ground = bpy.context.object
    ground.data.materials.append(material("studio ground", (0.77, 0.8, 0.82, 1)))
    light_data = bpy.data.lights.new("softbox", "AREA")
    light = bpy.data.objects.new("softbox", light_data)
    scene.collection.objects.link(light)
    light.location = (7, -7, 12)
    light_data.energy = 1600
    light_data.shape = "RECTANGLE"
    light_data.size = 18
    camera_data = bpy.data.cameras.new("comparison camera")
    camera = bpy.data.objects.new("comparison camera", camera_data)
    scene.collection.objects.link(camera)
    camera_x = ((len(variants) - 1) * 7.4 + 6) / 2
    camera.location = (camera_x, -24, 9)
    camera.rotation_euler = (
        (Vector((camera_x, 0, 2.1)) - camera.location)
        .to_track_quat("-Z", "Y")
        .to_euler()
    )
    camera_data.type = "ORTHO"
    camera_data.ortho_scale = len(variants) * 7.4 + 1.0
    scene.camera = camera
    scene.render.filepath = str(OUT / filename)
    bpy.ops.wm.save_as_mainfile(filepath=str(OUT / (Path(filename).stem + ".blend")))
    bpy.ops.render.render(write_still=True)


def main():
    global SITES, FIELD_SEED, SEED, OUT
    if not bpy.app.background:
        raise RuntimeError("Prototype must run in isolated background Blender")
    arguments = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
    parser = argparse.ArgumentParser(
        description="Experimental natural irregular partition; not a production asset exporter"
    )
    parser.add_argument("--output", type=Path, default=OUT)
    parser.add_argument("--seed", type=int, default=SEED)
    parser.add_argument(
        "--target-size",
        type=float,
        default=TARGET_SIZE,
        help="Approximate logical chunk scale in meters",
    )
    parser.add_argument(
        "--step",
        type=float,
        default=0.1,
        help="Fine sampling spacing in meters; comparison also uses twice this value",
    )
    parser.add_argument(
        "--no-render",
        action="store_true",
        help="Run all geometry checks and SVG/report output without the studio render",
    )
    options = parser.parse_args(arguments)
    if not math.isfinite(options.target_size) or not 1.0 <= options.target_size <= 2.4:
        parser.error(
            "--target-size must be between 1.0 and 2.4 meters for this bounded wall proof"
        )
    if not math.isfinite(options.step) or not 0.05 <= options.step <= 0.2:
        parser.error("--step must be between 0.05 and 0.2 meters")
    if any(
        abs(value / (options.step * 2) - round(value / (options.step * 2))) > 1e-7
        for value in (WIDTH, HEIGHT, *WINDOW)
    ):
        parser.error("Twice --step must divide the wall and window coordinates exactly")
    OUT = options.output.resolve()
    SEED = options.seed
    OUT.mkdir(exist_ok=True, parents=True)
    (OUT / "report.json").write_text(
        json.dumps(
            {"status": "running", "errors": ["Prototype validation has not completed"]},
            indent=2,
        )
    )
    try:
        select_configuration(SEED, options.target_size, options.step)
    except ValueError as error:
        (OUT / "report.json").write_text(
            json.dumps(
                {"errors": [str(error)], "scenario": "natural irregular partition"},
                indent=2,
            )
        )
        raise
    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.object.delete(use_global=False)
    seed_a = SEED
    first_config = dict(CURRENT_CONFIG)
    first_sites = SITES
    results = []
    groups = []
    for step, curved in (
        (options.step * 2, False),
        (options.step * 2, True),
        (options.step, True),
    ):
        result, polygons = build_variant(step, curved)
        results.append(result)
        groups.append(polygons)
        print(
            json.dumps(
                {
                    key: result[key]
                    for key in (
                        "variant",
                        "connected_chunk_count",
                        "render_triangles",
                        "merged_convex_proxies",
                        "size_distribution",
                        "volume_relative_error",
                        "source_inertia_relative_error",
                    )
                }
            ),
            flush=True,
        )

    def signature(polygons):
        return hashlib.sha256(
            json.dumps(polygons, sort_keys=True, separators=(",", ":")).encode()
        ).hexdigest()

    baseline_signature = results[2]["raw_partition_signature"]
    assert signature(partition(options.step, True)[0]) == baseline_signature
    repeated_groups, repeated_sizes = normalize_chunk_sizes(
        partition(options.step, True)[0]
    )
    assert repeated_sizes["merges"] == results[2]["size_normalization"]["merges"]
    SEED = seed_a + 1
    select_configuration(SEED, options.target_size, options.step)
    result, polygons = build_variant(options.step, True)
    results.append(result)
    groups.append(polygons)
    print(
        json.dumps(
            {
                key: result[key]
                for key in (
                    "variant",
                    "connected_chunk_count",
                    "render_triangles",
                    "merged_convex_proxies",
                    "size_distribution",
                    "volume_relative_error",
                    "source_inertia_relative_error",
                )
            }
        ),
        flush=True,
    )
    changed_signature = results[3]["raw_partition_signature"]
    assert changed_signature != baseline_signature
    report = {
        "status": "passed",
        "scenario": "natural irregular nonperiodic boundaries with bounded logical chunk sizes",
        "seeds": [seed_a, seed_a + 1],
        "target_size_m": TARGET_SIZE,
        "seed_a_sites": first_sites,
        "seed_b_sites": SITES,
        "seed_a_configuration": first_config,
        "seed_b_configuration": dict(CURRENT_CONFIG),
        "field": "three independently rotated nonperiodic hashed gradient-noise octaves; shared min-score partition",
        "noise_wavelength_and_amplitude_factors": NOISE_OCTAVES,
        "geometry_unchanged": True,
        "runtime_piece_motion": "rigid",
        "source": "whole wall with an unchanged true window; no artificial construction-panel constraints",
        "compatibility": "multipatch interfaces and compound proxies are a research result, not integrated into the production asset format",
        "variants": results,
        "errors": [],
    }
    report["determinism"] = {
        "same_seed_repeat_signature": baseline_signature,
        "changed_seed_signature": changed_signature,
        "same_seed_merge_log_repeats": True,
    }
    report["collinear_proxy_simplification_tolerance_m"] = 1e-8
    report["timing_scope"] = (
        "Offline prototype wall-clock timings; concurrent CPU work may contend. Not runtime/frame-rate measurements."
    )
    (OUT / "report.json").write_text(json.dumps(report, indent=2))
    (OUT / "front-comparison.svg").write_text(
        '<svg xmlns="http://www.w3.org/2000/svg" width="930" height="350" viewBox="0 0 930 350"><rect width="930" height="350" fill="#f4f6f7"/>'
        + svg_panel(groups[0], results[0]["interfaces"], "Planar Voronoi", 20)
        + svg_panel(
            groups[2], results[2]["interfaces"], "Natural irregular partition", 490
        )
        + "</svg>"
    )
    (OUT / "seed-comparison.svg").write_text(
        '<svg xmlns="http://www.w3.org/2000/svg" width="930" height="350" viewBox="0 0 930 350"><rect width="930" height="350" fill="#f4f6f7"/>'
        + svg_panel(groups[2], results[2]["interfaces"], f"Irregular seed {seed_a}", 20)
        + svg_panel(
            groups[3], results[3]["interfaces"], f"Irregular seed {seed_a+1}", 490
        )
        + "</svg>"
    )
    if not options.no_render:
        render_comparison(
            (
                (results[0], "PLANAR CUTS", False),
                (results[2], "NATURAL IRREGULAR CUTS", False),
                (results[2], "RIGID PIECES / EXPLODED", True),
            )
        )
        render_comparison(
            (
                (results[2], f"IRREGULAR SEED {seed_a}", False),
                (results[3], f"IRREGULAR SEED {seed_a+1}", False),
            ),
            "seed-comparison.png",
        )


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        OUT.mkdir(parents=True, exist_ok=True)
        (OUT / "report.json").write_text(
            json.dumps(
                {
                    "status": "failed",
                    "errors": [str(error)],
                    "configuration": CURRENT_CONFIG,
                },
                indent=2,
            )
        )
        raise
