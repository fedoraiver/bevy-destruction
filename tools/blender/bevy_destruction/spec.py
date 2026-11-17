"""Validated, Blender-independent input for the building authoring pipeline."""

from __future__ import annotations

import copy
import json
import math
from dataclasses import dataclass, field
from pathlib import Path

EPS = 1e-8


def _cross(a, b, c):
    return (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])


def _area(poly):
    return sum(a[0] * b[1] - b[0] * a[1] for a, b in zip(poly, poly[1:] + poly[:1])) / 2


def _clean(poly):
    result = []
    for p in poly:
        if not result or math.dist(p, result[-1]) > EPS:
            result.append(tuple(p))
    if len(result) > 1 and math.dist(result[0], result[-1]) <= EPS:
        result.pop()
    changed = True
    while changed and len(result) > 3:
        changed = False
        for i in range(len(result)):
            if abs(_cross(result[i - 1], result[i], result[(i + 1) % len(result)])) <= EPS:
                del result[i]
                changed = True
                break
    return result


def _ccw(poly):
    result = _clean(poly)
    return result if _area(result) > 0 else list(reversed(result))


def _clip(poly, a, b, left=True):
    """Clip a convex polygon against a directed infinite line."""
    if not poly:
        return []
    result = []
    sign = 1 if left else -1
    for p, q in zip(poly, poly[1:] + poly[:1]):
        dp, dq = sign * _cross(a, b, p), sign * _cross(a, b, q)
        inp, inq = dp >= -EPS, dq >= -EPS
        if inp:
            result.append(p)
        if inp != inq:
            t = dp / (dp - dq)
            result.append((p[0] + t * (q[0] - p[0]), p[1] + t * (q[1] - p[1])))
    result = _clean(result)
    return result if len(result) >= 3 and abs(_area(result)) > EPS else []


def _intersection(subject, cutter):
    result = list(subject)
    cutter = _ccw(cutter)
    for a, b in zip(cutter, cutter[1:] + cutter[:1]):
        result = _clip(result, a, b)
        if not result:
            break
    return result


def _subtract(subject, cutter):
    """Return nonoverlapping convex pieces of subject minus convex cutter."""
    remaining = list(subject)
    result = []
    cutter = _ccw(cutter)
    for a, b in zip(cutter, cutter[1:] + cutter[:1]):
        outside = _clip(remaining, a, b, False)
        if outside:
            result.append(outside)
        remaining = _clip(remaining, a, b)
        if not remaining:
            break
    return result


def _on_segment(p, a, b):
    return abs(_cross(a, b, p)) <= EPS and all(min(a[i], b[i]) - EPS <= p[i] <= max(a[i], b[i]) + EPS for i in (0, 1))


def _segments_touch(a, b, c, d):
    s = [_cross(a, b, c), _cross(a, b, d), _cross(c, d, a), _cross(c, d, b)]
    return (s[0] * s[1] < -EPS * EPS and s[2] * s[3] < -EPS * EPS) or any(
        (_on_segment(p, x, y)) for p, x, y in ((a, c, d), (b, c, d), (c, a, b), (d, a, b))
    )


def _inside(p, ring, boundary=True):
    inside = False
    for a, b in zip(ring, ring[1:] + ring[:1]):
        if _on_segment(p, a, b):
            return boundary
        if (a[1] > p[1]) != (b[1] > p[1]):
            x = a[0] + (p[1] - a[1]) * (b[0] - a[0]) / (b[1] - a[1])
            if x > p[0]:
                inside = not inside
    return inside


def _rings_touch(a, b):
    return any(_segments_touch(p, q, r, s) for p, q in zip(a, a[1:] + a[:1]) for r, s in zip(b, b[1:] + b[:1]))


def _triangulate(poly):
    remaining = _ccw(poly)
    triangles = []
    while len(remaining) > 3:
        for i, b in enumerate(remaining):
            a, c = remaining[i - 1], remaining[(i + 1) % len(remaining)]
            if _cross(a, b, c) <= EPS:
                continue
            others = [p for j, p in enumerate(remaining) if j not in {(i - 1) % len(remaining), i, (i + 1) % len(remaining)}]
            if any(_cross(a, b, p) >= -EPS and _cross(b, c, p) >= -EPS and _cross(c, a, p) >= -EPS for p in others):
                continue
            triangles.append([a, b, c])
            del remaining[i]
            break
        else:
            raise ValueError("Cannot triangulate the footprint; check self-intersections and short edges")
    triangles.append(remaining)
    return triangles


def _keys(data, allowed, label):
    if not isinstance(data, dict):
        raise ValueError(f"{label} must be an object")
    unknown = set(data) - set(allowed)
    if unknown:
        raise ValueError(f"Unknown {label} field(s): {', '.join(sorted(unknown))}")


def _number(value, label, minimum=0, allow_equal=False):
    if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value):
        raise ValueError(f"{label} must be a finite number")
    if value < minimum or (not allow_equal and value == minimum):
        op = "at least" if allow_equal else "greater than"
        raise ValueError(f"{label} must be {op} {minimum}")
    return float(value)


def _integer(value, label, minimum, maximum):
    if isinstance(value, bool) or not isinstance(value, int) or not minimum <= value <= maximum:
        raise ValueError(f"{label} must be an integer in [{minimum}, {maximum}]")
    return value


def _point(value, label):
    if not isinstance(value, (list, tuple)) or len(value) != 2:
        raise ValueError(f"{label} must be [x, y]")
    result = []
    for v in value:
        if isinstance(v, bool) or not isinstance(v, (int, float)) or not math.isfinite(v) or abs(v) > 100000:
            raise ValueError(f"{label} coordinates must be finite meters within +/-100000")
        result.append(float(v))
    return result


def _ring(value, label):
    if not isinstance(value, list):
        raise ValueError(f"{label} must be an array of [x, y] points")
    ring = [_point(p, f"{label}[{i}]") for i, p in enumerate(value)]
    if len(ring) > 1 and ring[-1] == ring[0]:
        ring.pop()
    if not 3 <= len(ring) <= 256:
        raise ValueError(f"{label} must have 3 to 256 vertices")
    if abs(_area(ring)) <= EPS:
        raise ValueError(f"{label} must enclose a nonzero area")
    edges = list(zip(ring, ring[1:] + ring[:1]))
    for i, (a, b) in enumerate(edges):
        if math.dist(a, b) < 1e-4:
            raise ValueError(f"{label} has a duplicate vertex or very short edge at {i}")
        for j in range(i + 1, len(edges)):
            if j in {i + 1, (i - 1) % len(edges)}:
                continue
            if _segments_touch(a, b, *edges[j]):
                raise ValueError(f"{label} self-intersects or touches itself at edges {i}, {j}")
        prev = ring[i - 1]
        if abs(_cross(prev, a, b)) <= EPS and (a[0] - prev[0]) * (b[0] - a[0]) + (a[1] - prev[1]) * (b[1] - a[1]) <= 0:
            raise ValueError(f"{label} reverses direction at vertex {i}")
    return ring


def _offset_ring(ring, distance):
    """Miter offset; positive distance is toward the polygon interior."""
    direction = 1 if _area(ring) > 0 else -1
    lines = []
    for a, b in zip(ring, ring[1:] + ring[:1]):
        length = math.dist(a, b)
        u = ((b[0] - a[0]) / length, (b[1] - a[1]) / length)
        n = (-u[1] * direction, u[0] * direction)
        lines.append(((a[0] + n[0] * distance, a[1] + n[1] * distance), u))
    result = []
    for i, (p, u) in enumerate(lines):
        q, v = lines[i - 1]
        det = u[0] * v[1] - u[1] * v[0]
        if abs(det) <= EPS:
            result.append(list(p))
        else:
            t = ((q[0] - p[0]) * v[1] - (q[1] - p[1]) * v[0]) / det
            result.append([p[0] + t * u[0], p[1] + t * u[1]])
    return _ring(result, "wall thickness offset")


def _rectangle(a, u, n, s0, s1, d0, d1):
    return _ccw([(a[0] + u[0] * s + n[0] * d, a[1] + u[1] * s + n[1] * d) for s, d in ((s0, d0), (s1, d0), (s1, d1), (s0, d1))])


def wall_layout(spec):
    """Return convex full-height wall footprints, retaining input edge indices."""
    result = []
    for h, ring in enumerate([spec.footprint] + spec.holes):
        offset = _offset_ring(ring, spec.wall_thickness if h == 0 else -spec.wall_thickness)
        orientation = (1 if _area(ring) > 0 else -1) * (1 if h == 0 else -1)
        for i, (a, b) in enumerate(zip(ring, ring[1:] + ring[:1])):
            length = math.dist(a, b)
            u = ((b[0] - a[0]) / length, (b[1] - a[1]) / length)
            n = (-u[1] * orientation, u[0] * orientation)
            poly = _ccw([a, b, offset[(i + 1) % len(ring)], offset[i]])
            if len(poly) < 3 or any(_cross(poly[j - 1], poly[j], poly[(j + 1) % len(poly)]) < -EPS for j in range(len(poly))):
                raise ValueError("Wall thickness produces a folded wall; reduce thickness or simplify the outline")
            result.append(dict(key=i if h == 0 else f"hole:{h - 1}:{i}", polygon=poly, a=a, u=u, n=n, length=length, d0=0.0, d1=spec.wall_thickness, material=spec.wall_material))
    for i, wall in enumerate(spec.inner_walls):
        a, b = wall["start"], wall["end"]
        length = math.dist(a, b)
        u = ((b[0] - a[0]) / length, (b[1] - a[1]) / length)
        n = (-u[1], u[0])
        half = wall["thickness"] / 2
        result.append(dict(key=f"inner:{i}", polygon=_rectangle(a, u, n, 0, length, -half, half), a=a, u=u, n=n, length=length, d0=-half, d1=half, material=wall["material"]))
    return result


@dataclass
class BuildingSpec:
    name: str
    seed: int
    footprint: list
    holes: list
    levels: int
    floor_height: float
    wall_thickness: float
    slab_thickness: float
    roof_thickness: float
    fracture_size: float
    density: float
    materials: dict
    wall_material: str
    floor_material: str
    roof_material: str
    openings: list
    inner_walls: list
    schema_version: int = 1
    parts: list = field(default_factory=list)

    @classmethod
    def from_dict(cls, data):
        _keys(data, cls.__dataclass_fields__, "building")
        if "footprint" not in data:
            raise ValueError("footprint is required")
        name = data.get("name", "building")
        if not isinstance(name, str) or not name or len(name) > 80 or any(not (c.isalnum() or c in "_-") for c in name):
            raise ValueError("name must contain 1 to 80 letters, digits, underscores or hyphens")
        _integer(data.get("schema_version", 1), "schema_version", 1, 1)
        density = _number(data.get("density", 2400), "density")
        materials = {
            "concrete": {"density": density, "color": [0.55, 0.57, 0.6, 1.0], "tension_strength": 3e6, "compression_strength": 30e6, "shear_strength": 4e6, "young_modulus": 30e9, "poisson_ratio": 0.2},
            "steel": {"density": 7800.0, "color": [0.22, 0.25, 0.28, 1.0], "tension_strength": 250e6, "compression_strength": 250e6, "shear_strength": 145e6, "young_modulus": 200e9, "poisson_ratio": 0.3},
            "wood": {"density": 600.0, "color": [0.42, 0.26, 0.12, 1.0], "tension_strength": 30e6, "compression_strength": 35e6, "shear_strength": 5e6, "young_modulus": 10e9, "poisson_ratio": 0.3},
        }
        raw_materials = data.get("materials", {})
        if not isinstance(raw_materials, dict):
            raise ValueError("materials must be an object keyed by material ID")
        for key, item in raw_materials.items():
            if not isinstance(key, str) or not key:
                raise ValueError("Material IDs must be nonempty strings")
            physical_keys = ("tension_strength", "compression_strength", "shear_strength", "young_modulus", "poisson_ratio")
            _keys(item, {"density", "color"} | set(physical_keys), f"material {key}")
            color = item.get("color", materials.get(key, {}).get("color", [0.55, 0.57, 0.6, 1.0]))
            if not isinstance(color, (list, tuple)) or len(color) not in (3, 4):
                raise ValueError(f"Material {key} color must have 3 or 4 components")
            color = [_number(v, f"material {key} color", 0, True) for v in color]
            if any(v > 1 for v in color):
                raise ValueError(f"Material {key} color components must be <= 1")
            if len(color) == 3:
                color.append(1.0)
            base = materials.get(key, materials["concrete"])
            physical = {field: _number(item.get(field, base[field]), f"material {key} {field}") for field in physical_keys if field != "poisson_ratio"}
            poisson = _number(item.get("poisson_ratio", base["poisson_ratio"]), f"material {key} poisson_ratio", -1)
            if poisson >= 0.5:
                raise ValueError(f"material {key} poisson_ratio must be less than 0.5")
            physical["poisson_ratio"] = poisson
            materials[key] = {"density": _number(item.get("density", materials.get(key, {}).get("density", density)), f"material {key} density"), "color": color, **physical}
        slab = _number(data.get("slab_thickness", 0.2), "slab_thickness")
        spec = cls(
            name=name, seed=_integer(data.get("seed", 1), "seed", 0, 2**31 - 1),
            footprint=_ring(data["footprint"], "footprint"), holes=[],
            levels=_integer(data.get("levels", 1), "levels", 1, 32),
            floor_height=_number(data.get("floor_height", 3.2), "floor_height"),
            wall_thickness=_number(data.get("wall_thickness", 0.2), "wall_thickness"),
            slab_thickness=slab, roof_thickness=_number(data.get("roof_thickness", slab), "roof_thickness", 0, True),
            fracture_size=_number(data.get("fracture_size", 1.2), "fracture_size"), density=density,
            materials=materials, wall_material=data.get("wall_material", "concrete"),
            floor_material=data.get("floor_material", "concrete"), roof_material=data.get("roof_material", "concrete"), openings=[], inner_walls=[])
        if spec.slab_thickness >= spec.floor_height:
            raise ValueError("slab_thickness must be smaller than floor_height")
        for attr in ("wall_material", "floor_material", "roof_material"):
            if not isinstance(getattr(spec, attr), str) or getattr(spec, attr) not in materials:
                raise ValueError(f"{attr} must reference a declared material")
        raw_holes = data.get("holes", [])
        if not isinstance(raw_holes, list):
            raise ValueError("holes must be an array of closed rings")
        spec.holes = [_ring(ring, f"holes[{i}]") for i, ring in enumerate(raw_holes)]
        for i, hole in enumerate(spec.holes):
            if not all(_inside(p, spec.footprint, False) for p in hole) or _rings_touch(hole, spec.footprint):
                raise ValueError(f"holes[{i}] must be strictly inside the footprint")
            for other in spec.holes[:i]:
                if _rings_touch(hole, other) or _inside(hole[0], other) or _inside(other[0], hole):
                    raise ValueError("Courtyard holes must not touch, overlap or contain one another")
        raw_parts = data.get("parts", [])
        if not isinstance(raw_parts, list):
            raise ValueError("parts must be an array of convex prisms")
        if raw_parts:
            if any(data.get(key) for key in ("holes", "openings", "inner_walls")):
                raise ValueError("Explicit parts cannot be combined with holes, openings or inner_walls")
            identifiers = set()
            for i, part in enumerate(raw_parts):
                label = f"parts[{i}]"
                _keys(part, {"id", "kind", "material", "polygon", "bottom", "top"}, label)
                identifier = part.get("id")
                if (not isinstance(identifier, str) or not identifier or identifier in identifiers or
                        any(not (c.isalnum() or c in "_-") for c in identifier)):
                    raise ValueError(f"{label}.id must be a unique nonempty stable identifier")
                identifiers.add(identifier)
                kind = part.get("kind", "beam")
                if not isinstance(kind, str) or not kind:
                    raise ValueError(f"{label}.kind must be a nonempty string")
                material = part.get("material", "concrete")
                if not isinstance(material, str) or material not in materials:
                    raise ValueError(f"{label}.material must reference a declared material")
                polygon = _ccw(_ring(part.get("polygon"), f"{label}.polygon"))
                if any(_cross(polygon[j - 1], p, polygon[(j + 1) % len(polygon)]) <= EPS
                       for j, p in enumerate(polygon)):
                    raise ValueError(f"{label}.polygon must be convex")
                covered = sum(abs(_area(_intersection(polygon, triangle)))
                              for triangle in _triangulate(spec.footprint))
                if (not all(_inside(p, spec.footprint) for p in polygon) or
                        abs(covered - abs(_area(polygon))) > 1e-6):
                    raise ValueError(f"{label}.polygon must be inside footprint")
                bottom = _number(part.get("bottom"), f"{label}.bottom", 0, True)
                top = _number(part.get("top"), f"{label}.top")
                if top - bottom <= 1e-4:
                    raise ValueError(f"{label}.top must exceed bottom by more than 0.0001m")
                for other in spec.parts:
                    if (min(top, other["top"]) - max(bottom, other["bottom"]) > EPS and
                            abs(_area(_intersection(polygon, other["polygon"]))) > EPS):
                        raise ValueError(f"Explicit parts {identifier} and {other['id']} overlap in volume")
                spec.parts.append(dict(id=identifier, kind=kind, material=material,
                                       polygon=polygon, bottom=bottom, top=top))
            return spec
        inset = _offset_ring(spec.footprint, spec.wall_thickness)
        if abs(_area(inset)) >= abs(_area(spec.footprint)) - EPS or not all(_inside(p, spec.footprint, False) for p in inset):
            raise ValueError("wall_thickness consumes or folds the footprint interior")
        expanded = [_offset_ring(hole, -spec.wall_thickness) for hole in spec.holes]
        for i, hole in enumerate(expanded):
            if not all(_inside(p, inset, False) for p in hole) or _rings_touch(hole, inset):
                raise ValueError("wall_thickness leaves no clear space between courtyard and exterior walls")
            for other in expanded[:i]:
                if _rings_touch(hole, other) or _inside(hole[0], other) or _inside(other[0], hole):
                    raise ValueError("wall_thickness makes courtyard walls intersect")
        raw_inner = data.get("inner_walls", [])
        if not isinstance(raw_inner, list):
            raise ValueError("inner_walls must be an array")
        for i, wall in enumerate(raw_inner):
            _keys(wall, {"start", "end", "thickness", "material"}, f"inner_walls[{i}]")
            if "start" not in wall or "end" not in wall:
                raise ValueError(f"inner_walls[{i}] requires start and end")
            a, b = _point(wall["start"], "inner wall start"), _point(wall["end"], "inner wall end")
            if math.dist(a, b) <= 1e-4:
                raise ValueError("Inner wall endpoints must differ")
            material = wall.get("material", spec.wall_material)
            if not isinstance(material, str) or material not in materials:
                raise ValueError("Inner wall material must reference a declared material")
            for p in (a, b, [(a[0] + b[0]) / 2, (a[1] + b[1]) / 2]):
                if not _inside(p, spec.footprint) or any(_inside(p, h) for h in spec.holes):
                    raise ValueError("Inner walls must lie inside the building and outside courtyards")
            for ring in [spec.footprint] + spec.holes:
                for c, d in zip(ring, ring[1:] + ring[:1]):
                    if _segments_touch(a, b, c, d) and not (_on_segment(a, c, d) or _on_segment(b, c, d)):
                        raise ValueError("Inner wall crosses a footprint or courtyard boundary")
            if any((a == w["start"] and b == w["end"]) or (a == w["end"] and b == w["start"]) for w in spec.inner_walls):
                raise ValueError("Duplicate inner wall")
            spec.inner_walls.append({"start": a, "end": b, "thickness": _number(wall.get("thickness", spec.wall_thickness), "inner wall thickness"), "material": material})
        layout = wall_layout(spec)
        outer_count = len(spec.footprint) + sum(map(len, spec.holes))
        for i, wall in enumerate(layout):
            for other in layout[:i]:
                overlap = _intersection(wall["polygon"], other["polygon"])
                if overlap and abs(_area(overlap)) > EPS:
                    if i < outer_count:
                        raise ValueError("Wall offsets overlap; reduce wall_thickness")
                    if isinstance(other["key"], str) and other["key"].startswith("inner:") and wall["material"] != other["material"]:
                        raise ValueError("Intersecting inner walls must use the same material")
        raw_openings = data.get("openings", [])
        if not isinstance(raw_openings, list):
            raise ValueError("openings must be an array")
        by_key = {w["key"]: w for w in layout}
        for i, opening in enumerate(raw_openings):
            _keys(opening, {"wall", "level", "offset", "width", "height", "sill", "kind", "frame", "frame_width", "frame_material"}, f"openings[{i}]")
            for key in ("wall", "offset", "width", "height"):
                if key not in opening:
                    raise ValueError(f"openings[{i}] requires {key}")
            wall_key = opening["wall"]
            if isinstance(wall_key, bool) or not isinstance(wall_key, (str, int)) or wall_key not in by_key:
                raise ValueError(f"openings[{i}].wall must be an outer edge index, inner:N or hole:H:E")
            wall = by_key[wall_key]
            kind = opening.get("kind", "window")
            if not isinstance(kind, str) or kind not in {"window", "door"}:
                raise ValueError("Opening kind must be door or window")
            item = dict(wall=wall_key, level=_integer(opening.get("level", 0), "opening level", 0, spec.levels - 1),
                        offset=_number(opening["offset"], "opening offset", 0, True), width=_number(opening["width"], "opening width"),
                        height=_number(opening["height"], "opening height"), sill=_number(opening.get("sill", 0 if kind == "door" else 0.9), "opening sill", 0, True), kind=kind)
            if kind == "door" and item["sill"] != 0:
                raise ValueError("Door sill must be 0 (measured from the floor slab top)")
            if item["sill"] + item["height"] > spec.floor_height - spec.slab_thickness + EPS:
                raise ValueError("Opening extends above the story clear height")
            if item["offset"] + item["width"] > wall["length"] + EPS:
                raise ValueError("Opening extends past its wall edge")
            aperture = _rectangle(wall["a"], wall["u"], wall["n"], item["offset"], item["offset"] + item["width"], wall["d0"], wall["d1"])
            clipped = _intersection(aperture, wall["polygon"])
            if not clipped or abs(abs(_area(clipped)) - abs(_area(aperture))) > 1e-6:
                raise ValueError("Opening intersects a mitered wall corner; move it away from the corner")
            if any(other["key"] != wall_key and abs(_area(_intersection(aperture, other["polygon"]))) > EPS for other in layout):
                raise ValueError("Opening intersects another wall; move the opening or wall junction")
            for other in spec.openings:
                if other["wall"] == wall_key and other["level"] == item["level"] and min(other["offset"] + other["width"], item["offset"] + item["width"]) > max(other["offset"], item["offset"]) + EPS and min(other["sill"] + other["height"], item["sill"] + item["height"]) > max(other["sill"], item["sill"]) + EPS:
                    raise ValueError("Openings overlap")
            frame = opening.get("frame", False)
            if not isinstance(frame, bool):
                raise ValueError("Opening frame must be boolean")
            frame_width = _number(opening.get("frame_width", 0.05), "frame_width")
            if frame and frame_width * 2 >= min(item["width"], item["height"]):
                raise ValueError("frame_width must leave a clear opening")
            frame_material = opening.get("frame_material", "steel")
            if not isinstance(frame_material, str) or frame_material not in materials:
                raise ValueError("frame_material must reference a declared material")
            item.update(frame=frame, frame_width=frame_width, frame_material=frame_material)
            spec.openings.append(item)
        return spec

    def to_dict(self):
        result = copy.deepcopy({key: getattr(self, key) for key in self.__dataclass_fields__})
        if not self.parts:
            del result["parts"]
        return result


def load_spec(path):
    with Path(path).open("r", encoding="utf-8-sig") as source:
        return BuildingSpec.from_dict(json.load(source))
