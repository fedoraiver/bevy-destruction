"""Shared natural partitions of prismatic building sources, in Blender meters."""

from collections import defaultdict
from dataclasses import dataclass
import hashlib
import math
import random
import re
import struct

import bpy
from mathutils import Vector

from .fracture import mesh_metrics, validate_convex
from .records import Chunk
from .spec import wall_layout

QUANTUM = 1e-7
MAX_ATTEMPTS = 12
MAX_PROXIES_PER_CHUNK = 48
MAX_CELLS_PER_GROUP = 12000
SAMPLING_FACTOR = 0.28
NOISE_OCTAVES = ((2.5, 0.32), (1.0, 0.12), (0.45, 0.04))


def _dot(a, b):
    return sum(x * y for x, y in zip(a, b))


def _cross(a, b):
    return (
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    )


def _area(poly):
    return (
        math.fsum(a[0] * b[1] - a[1] * b[0] for a, b in zip(poly, poly[1:] + poly[:1]))
        * 0.5
    )


def _ccw(poly):
    return list(poly) if _area(poly) > 0 else list(reversed(poly))


def _canonical(point):
    return tuple(round(value / QUANTUM) * QUANTUM for value in point)


def _hash(x, z, seed):
    value = (x * 0x9E3779B185EBCA87 ^ z * 0xC2B2AE3D27D4EB4F ^ seed) & ((1 << 64) - 1)
    value = ((value ^ (value >> 30)) * 0xBF58476D1CE4E5B9) & ((1 << 64) - 1)
    value = ((value ^ (value >> 27)) * 0x94D049BB133111EB) & ((1 << 64) - 1)
    return value ^ (value >> 31)


def _noise(x, z, seed):
    ix, iz = math.floor(x), math.floor(z)
    fx, fz = x - ix, z - iz
    d = math.sqrt(0.5)
    gradients = ((1, 0), (-1, 0), (0, 1), (0, -1), (d, d), (-d, d), (d, -d), (-d, -d))

    def corner(dx, dz):
        gx, gz = gradients[_hash(ix + dx, iz + dz, seed) % 8]
        return gx * (fx - dx) + gz * (fz - dz)

    u = fx**3 * (fx * (fx * 6 - 15) + 10)
    v = fz**3 * (fz * (fz * 6 - 15) + 10)
    return (corner(0, 0) * (1 - u) + corner(1, 0) * u) * (1 - v) + (
        corner(0, 1) * (1 - u) + corner(1, 1) * u
    ) * v


def _clip(poly, distance):
    if not poly:
        return []
    result = []
    previous = poly[-1]
    dp = distance(previous)
    for current in poly:
        dc = distance(current)
        if (dp >= 0) != (dc >= 0):
            ratio = dp / (dp - dc)
            result.append(
                tuple(
                    (
                        a + ratio * (b - a)
                        if isinstance(a, (int, float))
                        else tuple(x + ratio * (y - x) for x, y in zip(a, b))
                    )
                    for a, b in zip(previous, current)
                )
            )
        if dc >= 0:
            result.append(current)
        previous, dp = current, dc
    clean = []
    for point in result:
        if not clean or math.dist(point[:2], clean[-1][:2]) > 1e-10:
            clean.append(point)
    if len(clean) > 1 and math.dist(clean[0][:2], clean[-1][:2]) < 1e-10:
        clean.pop()
    return clean


def _clip_domain(poly, boundary):
    for a, b in zip(boundary, boundary[1:] + boundary[:1]):
        poly = _clip(
            poly,
            lambda p, a=a, b=b: (b[0] - a[0]) * (p[1] - a[1])
            - (b[1] - a[1]) * (p[0] - a[0]),
        )
        if len(poly) < 3:
            return []
    return poly


@dataclass
class Region:
    polygon: list
    low: tuple
    high: tuple
    source_part: str

    def depth(self, p, upper):
        a, b = self.high if upper else self.low
        return a * p[0] + b


@dataclass
class SourceGroup:
    id: str
    kind: str
    material: str
    parts: list
    u: tuple
    v: tuple
    w: tuple
    regions: list
    volume: float

    def world(self, p, depth):
        return _canonical(
            tuple(
                self.u[i] * p[0] + self.v[i] * p[1] + self.w[i] * depth
                for i in range(3)
            )
        )

    def inside(self, p):
        return any(
            all(
                (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]) >= -1e-10
                for a, b in zip(region.polygon, region.polygon[1:] + region.polygon[:1])
            )
            for region in self.regions
        )


def _union_depth_profiles(regions):
    """Remove authoring panel seams inside a shared vertical extrusion domain."""
    u_stops = {p[0] for region in regions for p in region.polygon}
    v_stops = sorted({p[1] for region in regions for p in region.polygon})
    lines = set(line for region in regions for line in (region.low, region.high))
    minimum, maximum = min(u_stops), max(u_stops)
    for first in lines:
        for second in lines:
            if abs(first[0] - second[0]) > 1e-12:
                crossing = (second[1] - first[1]) / (first[0] - second[0])
                if minimum + 1e-9 < crossing < maximum - 1e-9:
                    u_stops.add(crossing)
    u_stops = sorted(u_stops)
    bounds = [
        (
            min(p[0] for p in region.polygon),
            max(p[0] for p in region.polygon),
            min(p[1] for p in region.polygon),
            max(p[1] for p in region.polygon),
            region,
        )
        for region in regions
    ]
    output = []
    for left, right in zip(u_stops, u_stops[1:]):
        if right - left < 1e-9:
            continue
        middle = (left + right) * 0.5
        value = lambda line: line[0] * middle + line[1]
        for bottom, top in zip(v_stops, v_stops[1:]):
            center = (bottom + top) * 0.5
            active = [
                region
                for a, b, c, d, region in bounds
                if a < middle < b and c < center < d
            ]
            intervals = []
            for region in sorted(active, key=lambda region: value(region.low)):
                if intervals and value(region.low) <= value(intervals[-1][1]) + 1e-10:
                    if value(region.high) > value(intervals[-1][1]):
                        intervals[-1][1] = region.high
                else:
                    intervals.append([region.low, region.high])
            for low, high in intervals:
                output.append(
                    Region(
                        [(left, bottom), (right, bottom), (right, top), (left, top)],
                        low,
                        high,
                        "shared-domain",
                    )
                )
    joined = defaultdict(list)
    for region in output:
        key = (region.polygon[0][1], region.polygon[2][1], region.low, region.high)
        joined[key].append((region.polygon[0][0], region.polygon[1][0]))
    result = []
    for (bottom, top, low, high), spans in sorted(joined.items()):
        intervals = []
        for left, right in sorted(spans):
            if intervals and abs(intervals[-1][1] - left) <= 1e-9:
                intervals[-1][1] = right
            else:
                intervals.append([left, right])
        result.extend(
            Region(
                [(left, bottom), (right, bottom), (right, top), (left, top)],
                low,
                high,
                "shared-domain",
            )
            for left, right in intervals
        )
    return result


def _group_id(part):
    if part.kind == "wall":
        match = re.match(r"^(.*-l\d+)-p\d+-z\d+$", part.id)
    elif part.kind == "frame":
        match = re.match(r"^(.*-o\d+)-\d+$", part.id)
    elif part.kind == "floor":
        match = re.match(r"^(floor-\d+)-\d+$", part.id)
    elif part.kind == "roof":
        match = re.match(r"^(roof)-\d+$", part.id)
    else:
        match = None
    return match[1] if match else part.id


def _source_groups(parts, spec):
    grouped = defaultdict(list)
    axes = {}
    if spec is not None and not spec.parts:
        for wall in wall_layout(spec):
            key = (
                f"outer-{wall['key']}"
                if isinstance(wall["key"], int)
                else wall["key"].replace(":", "-")
            )
            axes[key] = (*wall["u"], 0.0)
    for part in parts:
        validate_convex(part.obj)
        grouped[(_group_id(part), part.kind, part.material)].append(part)
    result = []
    identifier_counts = {
        identifier: sum(key[0] == identifier for key in grouped)
        for identifier, _, _ in grouped
    }
    for (identifier, kind, material), members in sorted(grouped.items()):
        if identifier_counts[identifier] > 1:
            identifier = f"{identifier}/{kind}/{material}"
        points = [tuple(v.co) for part in members for v in part.obj.data.vertices]
        extents = [
            max(p[i] for p in points) - min(p[i] for p in points) for i in range(3)
        ]
        horizontal = kind in {"floor", "roof"} or extents[2] < min(extents[:2]) * 0.5
        key = re.sub(r"^(?:wall|frame)-", "", identifier).split("-l")[0]
        if horizontal:
            u, v, w = (1.0, 0.0, 0.0), (0.0, 1.0, 0.0), (0.0, 0.0, 1.0)
        else:
            u = axes.get(key)
            if u is None:
                candidates = []
                for part in members:
                    for edge in part.obj.data.edges:
                        a, b = (part.obj.data.vertices[i].co for i in edge.vertices)
                        delta = (float(b.x - a.x), float(b.y - a.y), 0.0)
                        candidates.append(delta)
                u = max(candidates, key=lambda p: _dot(p, p))
                length = math.sqrt(_dot(u, u))
                u = tuple(c / length for c in u)
                if u[0] < -1e-7 or abs(u[0]) < 1e-7 and u[1] < 0:
                    u = tuple(-c for c in u)
            v = (0.0, 0.0, 1.0)
            w = _cross(u, v)
        regions = []
        for part in sorted(members, key=lambda p: p.id):
            points = [tuple(vertex.co) for vertex in part.obj.data.vertices]
            low_z, high_z = min(p[2] for p in points), max(p[2] for p in points)
            bottom_xy = {(p[0], p[1]) for p in points if abs(p[2] - low_z) < 2e-6}
            top_xy = {(p[0], p[1]) for p in points if abs(p[2] - high_z) < 2e-6}
            if bottom_xy != top_xy or any(
                min(abs(p[2] - low_z), abs(p[2] - high_z)) > 2e-6 for p in points
            ):
                raise ValueError(
                    f"{part.id}: natural authoring requires the original Z-extruded prism source"
                )
            footprint = list(
                dict.fromkeys((p[0], p[1]) for p in points if abs(p[2] - low_z) < 2e-6)
            )
            center = tuple(
                math.fsum(p[i] for p in footprint) / len(footprint) for i in range(2)
            )
            footprint = _ccw(
                sorted(
                    footprint,
                    key=lambda p: math.atan2(p[1] - center[1], p[0] - center[0]),
                )
            )
            # Mesh vertex order around a prism is retained by generate_building.
            if horizontal:
                regions.append(Region(footprint, (0.0, low_z), (0.0, high_z), part.id))
                continue
            profile = _ccw(
                [(_dot((*p, 0.0), u), _dot((*p, 0.0), w)) for p in footprint]
            )
            stops = sorted(set(round(p[0], 9) for p in profile))
            for left, right in zip(stops, stops[1:]):
                if right - left < 1e-9:
                    continue
                middle = (left + right) * 0.5
                lines = []
                for a, b in zip(profile, profile[1:] + profile[:1]):
                    if min(a[0], b[0]) < middle < max(a[0], b[0]):
                        slope = (b[1] - a[1]) / (b[0] - a[0])
                        lines.append((slope, slope * -a[0] + a[1]))
                if len(lines) != 2:
                    raise ValueError(
                        f"{part.id}: source prism profile has {len(lines)} intersections"
                    )
                lines.sort(key=lambda line: line[0] * middle + line[1])
                regions.append(
                    Region(
                        [
                            (left, low_z),
                            (right, low_z),
                            (right, high_z),
                            (left, high_z),
                        ],
                        lines[0],
                        lines[1],
                        part.id,
                    )
                )
        if horizontal:
            from .bonds import merge_coplanar_polygons

            by_profile = defaultdict(list)
            for region in regions:
                by_profile[(region.low, region.high)].append(
                    [(x, y, 0.0) for x, y in region.polygon]
                )
            regions = [
                Region([(x, y) for x, y, _ in polygon], low, high, "shared-domain")
                for (low, high), polygons in sorted(by_profile.items())
                for polygon in merge_coplanar_polygons(polygons, tolerance=1e-8)
            ]
        else:
            regions = _union_depth_profiles(regions)
        result.append(
            SourceGroup(
                identifier,
                kind,
                material,
                members,
                u,
                v,
                w,
                regions,
                math.fsum(mesh_metrics(part.obj)["volume"] for part in members),
            )
        )
    return result


def _field(group, seed, target, count):
    rng = random.Random(seed)
    points = [p for region in group.regions for p in region.polygon]
    low = tuple(min(p[i] for p in points) for i in range(2))
    high = tuple(max(p[i] for p in points) for i in range(2))
    sites = []
    separation = min(
        target * 0.6,
        math.hypot(high[0] - low[0], high[1] - low[1])
        / max(1, math.sqrt(count))
        * 0.75,
    )
    for _ in range(count):
        candidates = []
        for _ in range(512):
            p = tuple(rng.uniform(low[i], high[i]) for i in range(2))
            if group.inside(p):
                candidates.append(
                    (min((math.dist(p, s) for s in sites), default=math.inf), p)
                )
        if not candidates:
            raise ValueError(f"{group.id}: no source-domain sites")
        distance, p = (
            max(candidates, key=lambda item: item[0]) if sites else candidates[0]
        )
        if distance < separation and len(sites) > 1:
            raise ValueError(f"{group.id}: could not meet site separation")
        sites.append(p)
    octaves = []
    for channel in range(2):
        octaves.append([])
        for octave, (wave, amplitude) in enumerate(NOISE_OCTAVES):
            salt = _hash(channel, octave, seed)
            angle = salt % 1000003 / 1000003 * math.tau
            octaves[-1].append(
                (
                    math.cos(angle),
                    math.sin(angle),
                    wave * target,
                    amplitude * target,
                    salt,
                )
            )

    def scores(p):
        displacement = []
        for channel in octaves:
            displacement.append(
                math.fsum(
                    amplitude
                    * _noise(
                        (cosine * p[0] - sine * p[1]) / wave + 17.173,
                        (sine * p[0] + cosine * p[1]) / wave - 8.419,
                        salt,
                    )
                    for cosine, sine, wave, amplitude, salt in channel
                )
            )
        x, z = p[0] + displacement[0], p[1] + displacement[1]
        return tuple((x - a) ** 2 + (z - b) ** 2 for a, b in sites)

    return scores, low, high


def _faces(group, poly, region):
    low = [group.world(p, region.depth(p, False)) for p in poly]
    high = [group.world(p, region.depth(p, True)) for p in poly]
    candidates = [list(reversed(low)), high]
    candidates.extend(
        [low[i], low[(i + 1) % len(low)], high[(i + 1) % len(high)], high[i]]
        for i in range(len(poly))
    )
    faces = []
    for face in candidates:
        points = []
        for p in face:
            if not points or p != points[-1]:
                points.append(p)
        if len(points) > 1 and points[0] == points[-1]:
            points.pop()
        if len(points) < 3:
            continue
        origin = points[0]
        normal = tuple(
            math.fsum(
                _cross(
                    tuple(a[i] - origin[i] for i in range(3)),
                    tuple(b[i] - origin[i] for i in range(3)),
                )[axis]
                for a, b in zip(points, points[1:] + points[:1])
            )
            for axis in range(3)
        )
        if _dot(normal, normal) > 1e-22:
            faces.append(points)
    return faces


def _partition(group, seed, target, count):
    scores, low, high = _field(group, seed, target, count)
    spacing = target * SAMPLING_FACTOR
    stops = []
    for axis in range(2):
        features = {low[axis], high[axis]}
        features.update(p[axis] for region in group.regions for p in region.polygon)
        values = set(features)
        divisions = max(1, math.ceil((high[axis] - low[axis]) / spacing))
        for i in range(divisions + 1):
            sample = low[axis] + (high[axis] - low[axis]) * i / divisions
            if all(abs(sample - feature) > QUANTUM * 4 for feature in features):
                values.add(sample)
        stops.append(sorted(values))
    cells, labels, areas = [], [], []
    cache = {}
    for ix, (left, right) in enumerate(zip(stops[0], stops[0][1:])):
        if right - left < 1e-8:
            continue
        for iz, (bottom, top) in enumerate(zip(stops[1], stops[1][1:])):
            if top - bottom < 1e-8:
                continue
            corners = ((left, bottom), (right, bottom), (right, top), (left, top))
            for indices in (
                ((0, 1, 3), (1, 2, 3)) if (ix + iz) % 2 else ((0, 1, 2), (0, 2, 3))
            ):
                triangle = []
                for i in indices:
                    p = corners[i]
                    if p not in cache:
                        cache[p] = scores(p)
                    triangle.append((p[0], p[1], cache[p]))
                for region in group.regions:
                    if (
                        max(p[0] for p in region.polygon) < left
                        or min(p[0] for p in region.polygon) > right
                        or max(p[1] for p in region.polygon) < bottom
                        or min(p[1] for p in region.polygon) > top
                    ):
                        continue
                    source = _clip_domain(triangle, region.polygon)
                    if len(source) < 3 or _area(source) <= 1e-15:
                        continue
                    for own in range(count):
                        poly = source
                        for other in range(count):
                            if other != own:
                                poly = _clip(
                                    poly,
                                    lambda p, own=own, other=other: p[2][other]
                                    - p[2][own],
                                )
                                if len(poly) < 3:
                                    break
                        if len(poly) < 3:
                            continue
                        value = _area(poly)
                        if value <= 1e-15:
                            continue
                        faces = _faces(group, poly, region)
                        if len(faces) < 4:
                            raise ValueError(
                                f"{group.id}/{region.source_part}: positive region became a degenerate prism; area={value:g}, points={[p[:2] for p in poly]}, depths={[(region.depth(p,False),region.depth(p,True)) for p in poly]}"
                            )
                        cells.append(faces)
                        labels.append(own)
                        areas.append(value)
                        if len(cells) > MAX_CELLS_PER_GROUP:
                            raise ValueError(f"{group.id}: microcell budget exceeded")
    return cells, labels, areas


def _object(name, faces, collection, materials=()):
    from .natural_union import mesh_data

    vertices, indices = mesh_data(faces)
    mesh = bpy.data.meshes.new(name)
    mesh.from_pydata(vertices, [], indices)
    mesh.update(calc_edges=True)
    for material in materials:
        mesh.materials.append(material)
    obj = bpy.data.objects.new(name, mesh)
    collection.objects.link(obj)
    return obj


def _combined_mass(metrics):
    volume = math.fsum(item["volume"] for item in metrics)
    center = tuple(
        math.fsum(item["volume"] * item["center"][i] for item in metrics) / volume
        for i in range(3)
    )
    inertia = [
        [
            math.fsum(
                item["inertia"][i][j]
                + item["volume"]
                * (
                    (
                        math.fsum(
                            (item["center"][a] - center[a]) ** 2 for a in range(3)
                        )
                        if i == j
                        else 0
                    )
                    - (item["center"][i] - center[i]) * (item["center"][j] - center[j])
                )
                for item in metrics
            )
            for j in range(3)
        ]
        for i in range(3)
    ]
    return volume, center, inertia


def _mass_error(expected, actual):
    scale = max(abs(v) for row in expected[2] for v in row)
    return (
        math.dist(expected[1], actual[1]),
        max(
            abs(expected[2][i][j] - actual[2][i][j]) for i in range(3) for j in range(3)
        )
        / scale,
    )


def _serialized_proxy(cell, audit=None):
    from .natural_union import convex_hull, volume

    rounded = [
        [
            tuple(
                struct.unpack("<f", struct.pack("<f", coordinate))[0]
                for coordinate in point
            )
            for point in face
        ]
        for face in cell
    ]
    hull = convex_hull([rounded])
    original, actual = volume(cell), volume(hull)
    displacement = max(
        math.dist(before, after)
        for face, quantized in zip(cell, rounded)
        for before, after in zip(face, quantized)
    )

    def surface_area(faces):
        return math.fsum(
            math.sqrt(_dot(cross, cross)) * 0.5
            for face in faces
            for index in range(1, len(face) - 1)
            for cross in [
                _cross(
                    tuple(face[index][axis] - face[0][axis] for axis in range(3)),
                    tuple(face[index + 1][axis] - face[0][axis] for axis in range(3)),
                )
            ]
        )

    points = [point for face in cell for point in face]
    diameter_bound = math.sqrt(
        math.fsum(
            (max(p[axis] for p in points) - min(p[axis] for p in points)) ** 2
            for axis in range(3)
        )
    )
    # Rounded vertices move the convex hull by at most this Hausdorff distance.
    # Bound the corresponding thin surface shell rather than the relative
    # volume of a microscopic cell; complete chunk/source audits remain below.
    rounding_bound = 2 * max(
        surface_area(cell), surface_area(hull)
    ) * displacement + 4 * math.pi * displacement**2 * (diameter_bound + displacement)
    error = abs(actual - original)
    if original <= 0 or actual <= 0 or error > max(rounding_bound, 1e-12):
        raise ValueError(
            f"Float32 convex proxy exceeded its rounding shell ({original:g}->{actual:g}, bound={rounding_bound:g})"
        )
    if audit is not None:
        audit["absolute_proxy_volume_change_m3"] += error
        audit["maximum_proxy_vertex_rounding_m"] = max(
            audit["maximum_proxy_vertex_rounding_m"], displacement
        )
    return hull


def _realize_group(group, prepared, seed, collection):
    created = []
    group_chunks = []
    try:
        materials = list(group.parts[0].obj.data.materials)
        volumes = []
        output_metrics = []
        for component_index, outer, proxies in prepared:
            identifier = f"{group.id}:natural:{seed}:{component_index:04d}"
            obj = _object(identifier, outer, collection, materials)
            created.append(obj)
            metrics = mesh_metrics(obj)
            output_metrics.append(metrics)
            collider_objects = []
            proxy_volume = 0.0
            proxy_metrics = []
            for index, proxy in enumerate(proxies):
                collider = _object(f"collider_{identifier}_{index}", proxy, collection)
                created.append(collider)
                validate_convex(collider)
                proxy_metric = mesh_metrics(collider)
                proxy_metrics.append(proxy_metric)
                proxy_volume += proxy_metric["volume"]
                collider.hide_render = True
                collider_objects.append(collider)
            if abs(proxy_volume - metrics["volume"]) > max(group.volume * 1e-5, 1e-9):
                raise ValueError(f"{identifier}: compound proxy changed source volume")
            proxy_center_error, proxy_inertia_error = _mass_error(
                _combined_mass([metrics]), _combined_mass(proxy_metrics)
            )
            if proxy_center_error > 2e-5 or proxy_inertia_error > 1e-4:
                raise ValueError(
                    f"{identifier}: compound proxy changed center/inertia ({proxy_center_error:g},{proxy_inertia_error:g})"
                )
            group_chunks.append(
                Chunk(
                    identifier,
                    group.id,
                    group.kind,
                    group.material,
                    obj,
                    collider_objects[0],
                    metrics["volume"],
                    metrics["center"],
                    metrics["inertia"],
                    collider_objects,
                )
            )
            volumes.append(metrics["volume"])
        if abs(math.fsum(volumes) - group.volume) > max(group.volume * 1e-4, 1e-9):
            raise ValueError(
                f"{group.id}: natural partition did not conserve source volume"
            )
        center_error, inertia_error = _mass_error(
            _combined_mass([mesh_metrics(part.obj) for part in group.parts]),
            _combined_mass(output_metrics),
        )
        if center_error > 2e-5 or inertia_error > 1e-4:
            raise ValueError(
                f"{group.id}: source center/inertia not conserved ({center_error:g},{inertia_error:g})"
            )
        return group_chunks, volumes, center_error, inertia_error
    except Exception:
        for obj in reversed(created):
            mesh = obj.data
            bpy.data.objects.remove(obj, do_unlink=True)
            if mesh.users == 0:
                bpy.data.meshes.remove(mesh)
        raise


def fracture_parts(parts, *, seed, target_size, collection, spec=None):
    from .natural_union import cell_contacts, boundary_faces, merge_convex_cells

    if type(seed) is not int or not math.isfinite(target_size) or target_size <= 0:
        raise ValueError(
            "Natural fracture requires an integer seed and finite positive target size"
        )
    if not isinstance(collection, bpy.types.Collection):
        raise ValueError("Natural fracture requires a Blender output collection")
    parts = list(parts)
    if len({part.id for part in parts}) != len(parts):
        raise ValueError("Natural fracture source part IDs must be unique")
    groups = _source_groups(parts, spec)
    chunks = []
    audit = []
    for group in groups:
        if bpy.app.background:
            print(f"Natural partition: {group.id}", flush=True)
        points = [p for region in group.regions for p in region.polygon]
        extents = [
            max(p[i] for p in points) - min(p[i] for p in points) for i in range(2)
        ]
        count = max(
            1,
            math.ceil(
                math.prod(max(1.0, extent / target_size) for extent in extents) - 1e-7
            ),
        )
        source_area = math.fsum(_area(region.polygon) for region in group.regions)
        source_perimeter = math.fsum(
            math.dist(a, b)
            for region in group.regions
            for a, b in zip(region.polygon, region.polygon[1:] + region.polygon[:1])
        )
        inherited_width = 2 * source_area / source_perimeter
        aspect_limit = max(6.0, 2 * target_size / inherited_width)
        if count > 256:
            raise ValueError(f"{group.id}: logical chunk budget exceeded")
        failure_log = []
        for attempt in range(MAX_ATTEMPTS):
            digest = hashlib.sha256(
                f"natural-v2\0{seed}\0{group.id}\0{attempt}".encode()
            ).digest()
            field_seed = int.from_bytes(digest[:8], "little")
            try:
                cells, labels, areas = _partition(group, field_seed, target_size, count)
                if not cells:
                    raise ValueError("source produced no cells")
                contacts = cell_contacts(cells, tolerance=2e-6)
                neighbors = defaultdict(set)
                for first, second, _ in contacts:
                    if labels[first] == labels[second]:
                        neighbors[first].add(second)
                        neighbors[second].add(first)
                remaining = set(range(len(cells)))
                components = []
                while remaining:
                    first = min(remaining)
                    remaining.remove(first)
                    pending, component = [first], {first}
                    while pending:
                        adjacent = neighbors[pending.pop()].intersection(remaining)
                        remaining.difference_update(adjacent)
                        component.update(adjacent)
                        pending.extend(adjacent)
                    components.append(component)
                expected_area = math.fsum(areas) / count
                min_area, max_area = expected_area * 0.2, expected_area * 3.0
                transfers = []
                while True:
                    masses = [
                        math.fsum(areas[i] for i in component)
                        for component in components
                    ]
                    small = [
                        (value, index)
                        for index, value in enumerate(masses)
                        if value < min_area
                    ]
                    if not small:
                        break
                    value, index = min(small)
                    membership = {
                        cell: j
                        for j, component in enumerate(components)
                        for cell in component
                    }
                    shared = defaultdict(float)
                    for first, second, polygon in contacts:
                        a, b = membership[first], membership[second]
                        if a == b:
                            continue
                        other = b if a == index else a if b == index else None
                        if other is not None and masses[other] + value <= max_area:
                            origin = polygon[0]
                            normal = tuple(
                                math.fsum(
                                    _cross(
                                        tuple(p[i] - origin[i] for i in range(3)),
                                        tuple(q[i] - origin[i] for i in range(3)),
                                    )[axis]
                                    for p, q in zip(polygon, polygon[1:] + polygon[:1])
                                )
                                for axis in range(3)
                            )
                            shared[other] += math.sqrt(_dot(normal, normal)) * 0.5
                    if not shared:
                        raise ValueError(
                            f"tiny connected region {value:g} has no compatible face neighbor"
                        )
                    other = max(shared, key=lambda key: (shared[key], -key))
                    components[other].update(components[index])
                    components.pop(index)
                    transfers.append(value)
                if any(
                    math.fsum(areas[i] for i in component) > max_area
                    for component in components
                ):
                    raise ValueError("maximum logical area exceeded")
                prepared = []
                shape_report = []
                quantization_audit = {
                    "absolute_proxy_volume_change_m3": 0.0,
                    "maximum_proxy_vertex_rounding_m": 0.0,
                }
                for component_index, component in enumerate(components):
                    selected = [cells[i] for i in sorted(component)]
                    projected = [
                        (_dot(p, group.u), _dot(p, group.v))
                        for cell in selected
                        for face in cell
                        for p in face
                    ]
                    span = [
                        max(p[i] for p in projected) - min(p[i] for p in projected)
                        for i in range(2)
                    ]
                    if (
                        math.hypot(*span) > 3 * target_size
                        or max(span) / min(span) > aspect_limit
                    ):
                        raise ValueError(
                            f"logical chunk exceeds size/aspect bounds: span={span}, aspect_limit={aspect_limit:g}"
                        )
                    shape_report.append(
                        {
                            "projected_area_m2": math.fsum(areas[i] for i in component),
                            "projected_aabb_diagonal_m": math.hypot(*span),
                            "projected_aabb_aspect": max(span) / min(span),
                        }
                    )
                    proxies = merge_convex_cells(selected, tolerance=2e-6)
                    outer = boundary_faces(proxies, tolerance=2e-6)
                    if len(proxies) > MAX_PROXIES_PER_CHUNK:
                        raise ValueError(
                            f"convex proxy budget {len(proxies)}>{MAX_PROXIES_PER_CHUNK}"
                        )
                    prepared.append(
                        (
                            component_index,
                            outer,
                            [
                                _serialized_proxy(proxy, quantization_audit)
                                for proxy in proxies
                            ],
                        )
                    )
                if len(chunks) + len(prepared) > 2048:
                    raise ValueError("asset logical chunk budget exceeded")
                group_chunks, volumes, center_error, inertia_error = _realize_group(
                    group, prepared, seed, collection
                )
                break
            except ValueError as error:
                failure_log.append(str(error))
                if bpy.app.background:
                    print(f"  rejected configuration {attempt}: {error}", flush=True)
        else:
            raise ValueError(
                f"{group.id}: all {MAX_ATTEMPTS} natural partition attempts failed: {failure_log}"
            )
        chunks.extend(group_chunks)
        if bpy.app.background:
            print(
                f"  accepted configuration {attempt}: {len(prepared)} chunks, "
                f"{sum(len(p[2]) for p in prepared)} convex proxies",
                flush=True,
            )
        audit.append(
            {
                "group": group.id,
                "chunks": len(prepared),
                "microcells": len(cells),
                "proxies": sum(len(p[2]) for p in prepared),
                "attempt": attempt,
                "rejected": failure_log,
                "small_region_transfers_m2": transfers,
                "logical_shapes": shape_report,
                "area_bounds_m2": [min_area, max_area],
                "aspect_limit_from_source": aspect_limit,
                "source_volume": group.volume,
                "output_volume": math.fsum(volumes),
                "source_center_error_m": center_error,
                "source_inertia_relative_error": inertia_error,
                "proxy_quantization": quantization_audit,
            }
        )
    collection["bd_natural_audit"] = __import__("json").dumps(audit, sort_keys=True)
    return chunks
