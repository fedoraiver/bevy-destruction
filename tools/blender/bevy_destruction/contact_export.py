"""Audit baked contact footprints against the actual serialized GLB triangles.

Only sub-ULP boundary differences may be corrected. Render geometry and collision
geometry are never modified by this pass.
"""

import itertools
import json
import math
import struct


def f32(value):
    return struct.unpack("<f", struct.pack("<f", value))[0]


def _sub(a, b):
    return tuple(x - y for x, y in zip(a, b))


def _dot(a, b):
    return math.fsum(x * y for x, y in zip(a, b))


def _cross(a, b):
    return (
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    )


def _unit(value):
    length = math.sqrt(_dot(value, value))
    return tuple(x / length for x in value)


def _area(polygon):
    if len(polygon) < 3:
        return 0.0
    origin = polygon[0]
    return (
        math.fsum(
            (a[0] - origin[0]) * (b[1] - origin[1])
            - (a[1] - origin[1]) * (b[0] - origin[0])
            for a, b in zip(polygon, polygon[1:] + polygon[:1])
        )
        * 0.5
    )


def _side(a, b, point):
    return (b[0] - a[0]) * (point[1] - a[1]) - (b[1] - a[1]) * (point[0] - a[0])


def clip_polygon(subject, boundary):
    if _area(boundary) < 0:
        boundary = list(reversed(boundary))
    result = list(subject)
    for a, b in zip(boundary, boundary[1:] + boundary[:1]):
        output = []
        for previous, current in zip(result, result[1:] + result[:1]):
            dp, dc = _side(a, b, previous), _side(a, b, current)
            if (dp >= 0) != (dc >= 0):
                t = dp / (dp - dc)
                output.append(
                    tuple(
                        previous[i] + t * (current[i] - previous[i]) for i in range(2)
                    )
                )
            if dc >= 0:
                output.append(current)
        result = output
        if len(result) < 3:
            return []
    return result


def _merge_convex_supports(polygons):
    """Erase triangulation diagonals only when the exact union is convex."""
    polygons = list(polygons)
    changed = True
    while changed:
        changed = False
        for i, a in enumerate(polygons):
            for j in range(i + 1, len(polygons)):
                b = polygons[j]
                points = sorted(set(a + b))
                hull = []
                for values in (points, list(reversed(points))):
                    chain = []
                    for point in values:
                        while (
                            len(chain) >= 2 and _side(chain[-2], chain[-1], point) <= 0
                        ):
                            chain.pop()
                        chain.append(point)
                    hull.extend(chain[:-1])
                union_area = (
                    abs(_area(a)) + abs(_area(b)) - abs(_area(clip_polygon(a, b)))
                )
                if abs(abs(_area(hull)) - union_area) > max(union_area * 1e-12, 1e-16):
                    continue
                polygons[i] = hull
                polygons.pop(j)
                changed = True
                break
            if changed:
                break
    return polygons


def read_triangles(path):
    """Read the translation-only chunk meshes exported by this authoring tool."""
    data = path.read_bytes()
    json_length = struct.unpack_from("<I", data, 12)[0]
    document = json.loads(data[20 : 20 + json_length])
    binary = data[28 + json_length :]

    def accessor(index):
        record = document["accessors"][index]
        view = document["bufferViews"][record["bufferView"]]
        component = {5126: "f", 5123: "H", 5125: "I", 5121: "B"}[
            record["componentType"]
        ]
        count = {"VEC3": 3, "SCALAR": 1}[record["type"]]
        layout = "<" + component * count
        stride = view.get("byteStride", struct.calcsize(layout))
        offset = view.get("byteOffset", 0) + record.get("byteOffset", 0)
        return [
            struct.unpack_from(layout, binary, offset + i * stride)
            for i in range(record["count"])
        ]

    result = {}
    for node in document["nodes"]:
        identifier = node.get("extras", {}).get("bd_chunk_id")
        if identifier is None:
            continue
        if (
            "matrix" in node
            or node.get("rotation", [0, 0, 0, 1]) != [0, 0, 0, 1]
            or node.get("scale", [1, 1, 1]) != [1, 1, 1]
        ):
            raise ValueError(
                "Exported chunk has an unexpected non-translation transform"
            )
        translation = tuple(f32(value) for value in node.get("translation", [0, 0, 0]))
        triangles = []
        for primitive in document["meshes"][node["mesh"]]["primitives"]:
            if primitive.get("mode", 4) != 4:
                raise ValueError("Exported chunk is not triangulated")
            points = [
                tuple(f32(p[i] + translation[i]) for i in range(3))
                for p in accessor(primitive["attributes"]["POSITION"])
            ]
            indices = [item[0] for item in accessor(primitive["indices"])]
            triangles.extend(
                tuple(points[indices[i + j]] for j in range(3))
                for i in range(0, len(indices), 3)
            )
        result[identifier] = triangles
    return result


class Footprint:
    def __init__(self, patch):
        self.normal = _unit(tuple(f32(v) for v in patch["normal"]))
        axis = (1, 0, 0) if abs(self.normal[0]) < 0.8 else (0, 1, 0)
        self.tangent = _unit(_cross(self.normal, axis))
        self.bitangent = _cross(self.normal, self.tangent)
        distance = _dot(self.normal, tuple(f32(v) for v in patch["center"]))
        self.origin = tuple(v * distance for v in self.normal)
        self.boundary = [self.project(point) for point in patch["vertices"]]

    def project(self, point):
        relative = _sub(point, self.origin)
        return (_dot(relative, self.tangent), _dot(relative, self.bitangent))

    def unproject(self, point):
        return tuple(
            self.origin[i] + self.tangent[i] * point[0] + self.bitangent[i] * point[1]
            for i in range(3)
        )

    def supports(self, triangles, orientation):
        result = []
        for triangle in triangles:
            cross = _cross(
                _sub(triangle[1], triangle[0]), _sub(triangle[2], triangle[0])
            )
            if _dot(cross, cross) <= 1e-24:
                continue
            normal = _unit(cross)
            if _dot(self.normal, normal) * orientation < 0.98:
                continue
            boundary = [self.project(point) for point in triangle]
            if _area(boundary) < 0:
                boundary.reverse()
            overlap = clip_polygon(self.boundary, boundary)
            if abs(_area(overlap)) <= 1e-14:
                continue
            if any(
                abs(_dot(normal, _sub(self.unproject(point), triangle[0]))) > 4e-5
                for point in overlap
            ):
                continue
            result.append(boundary)
        return result


def _neighbors(value):
    value = f32(value)
    bits = struct.unpack("<I", struct.pack("<f", value))[0]
    if value == 0:
        return [0.0, -(2**-149), 2**-149]
    return [
        struct.unpack("<f", struct.pack("<I", bits + offset))[0]
        for offset in range(-2, 3)
    ]


def _quantize_inside(point, plane, boundaries, budget, originals):
    candidates = itertools.product(*(_neighbors(value) for value in point))
    preferred = min(originals, key=lambda candidate: math.dist(candidate, point))
    if math.dist(preferred, point) > budget:
        preferred = point
    candidates = sorted(
        candidates,
        key=lambda candidate: (
            math.dist(candidate, preferred),
            math.dist(candidate, point),
        ),
    )
    for candidate in candidates:
        if math.dist(candidate, point) > budget:
            continue
        projected = plane.project(candidate)
        if all(
            _side(a, b, projected) >= -1e-16
            for boundary in boundaries
            for a, b in zip(boundary, boundary[1:] + boundary[:1])
        ):
            return candidate
    raise ValueError("Contact clipping has no bounded inward float32 representation")


def correct_contacts(manifest, triangles, patch_metrics, aggregate_patches):
    """Correct only measured, float32-scale footprint overhangs; reject larger ones."""
    audit = {
        "corrected_patches": 0,
        "removed_contact_area_m2": 0.0,
        "maximum_vertex_adjustment_m": 0.0,
        "corrections": [],
    }
    for bond in manifest["bonds"]:
        final = []
        for index, patch in enumerate(bond["patches"]):
            plane = Footprint(patch)
            supports = [
                plane.supports(triangles[bond[key]], orientation)
                for key, orientation in (("a", 1), ("b", -1))
            ]
            expected = abs(_area(plane.boundary))
            coverage = [
                math.fsum(
                    abs(_area(clip_polygon(plane.boundary, boundary)))
                    for boundary in side
                )
                for side in supports
            ]
            if any(
                value > expected + max(expected * 0.005, 5e-10) for value in coverage
            ):
                raise ValueError(
                    f"Contact {bond['id']} patch {index} has overlapping actual GLB support"
                )
            # Keep a factor-of-two margin to the runtime's 1% / 1e-9m² gate.
            if all(
                value >= expected - max(expected * 0.005, 5e-10) for value in coverage
            ):
                final.append(patch)
                continue
            supports = [_merge_convex_supports(side) for side in supports]
            budget = (
                max(
                    math.ulp(1.0),
                    max(
                        abs(_neighbors(value)[-1] - f32(value))
                        for point in patch["vertices"]
                        for value in point
                    ),
                )
                * 2
            )
            budget = min(budget, 4e-5)
            pieces = []
            for a in supports[0]:
                first = clip_polygon(plane.boundary, a)
                if abs(_area(first)) <= 1e-14:
                    continue
                for b in supports[1]:
                    polygon = clip_polygon(first, b)
                    if abs(_area(polygon)) <= 1e-14:
                        continue
                    try:
                        rounded = [
                            _quantize_inside(
                                plane.unproject(point),
                                plane,
                                [plane.boundary, a, b],
                                budget,
                                patch["vertices"],
                            )
                            for point in polygon
                        ]
                    except ValueError as error:
                        raise ValueError(
                            f"Contact {bond['id']} patch {index}: {error}; coverage={coverage}, area={expected}"
                        ) from error
                    try:
                        metrics = patch_metrics(rounded, allow_edge_contact=True)
                    except ValueError as error:
                        raise ValueError(
                            f"Contact {bond['id']} patch {index}: {error}; rounded={rounded}, coverage={coverage}, area={expected}"
                        ) from error
                    if metrics is not None:
                        pieces.append(metrics)
            if not pieces:
                raise ValueError(
                    f"Contact {bond['id']} patch {index} has no actual GLB footprint"
                )
            if len(pieces) > 64:
                raise ValueError(
                    f"Contact {bond['id']} patch {index} needs too many convex correction pieces"
                )
            removed = patch["area"] - math.fsum(piece["area"] for piece in pieces)
            perimeter = math.fsum(
                math.dist(a, b)
                for a, b in zip(
                    patch["vertices"], patch["vertices"][1:] + patch["vertices"][:1]
                )
            )
            if (
                removed < -1e-12
                or removed > perimeter * budget * 2 + math.pi * budget**2
            ):
                raise ValueError(
                    f"Contact {bond['id']} patch {index} exceeds float32 footprint correction budget"
                )
            displacement = max(
                min(math.dist(point, other) for other in patch["vertices"])
                for piece in pieces
                for point in piece["vertices"]
            )
            if displacement > budget:
                raise ValueError(
                    f"Contact {bond['id']} patch {index} would introduce a nonlocal boundary split"
                )
            for piece in pieces:
                checked = Footprint(piece)
                for key, orientation in (("a", 1), ("b", -1)):
                    coverage = math.fsum(
                        abs(_area(clip_polygon(checked.boundary, boundary)))
                        for boundary in checked.supports(
                            triangles[bond[key]], orientation
                        )
                    )
                    if abs(coverage - piece["area"]) > max(piece["area"] * 1e-5, 1e-12):
                        raise ValueError(
                            f"Contact {bond['id']} patch {index} has unresolved corrected GLB coverage"
                        )
            final.extend(pieces)
            audit["corrected_patches"] += 1
            audit["removed_contact_area_m2"] += removed
            audit["maximum_vertex_adjustment_m"] = max(
                audit["maximum_vertex_adjustment_m"], displacement
            )
            audit["corrections"].append(
                {
                    "bond": bond["id"],
                    "patch": index,
                    "removed_area_m2": removed,
                    "maximum_vertex_adjustment_m": displacement,
                    "float32_budget_m": budget,
                }
            )
        bond.update(aggregate_patches(final))
        bond["patches"] = final
    manifest["glb_contact_audit"] = audit
    return audit
