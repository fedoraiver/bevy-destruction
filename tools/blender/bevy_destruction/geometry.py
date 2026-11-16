"""Nonoverlapping convex construction pieces, in building-local meters/Z-up."""

import hashlib
import json

import bpy

from .records import Part
from .spec import EPS, _area, _ccw, _clip, _intersection, _rectangle, _subtract, _triangulate, wall_layout


def _floor_regions(spec):
    cells = _triangulate(spec.footprint)
    for hole in spec.holes:
        for cutter in _triangulate(hole):
            cells = [piece for cell in cells for piece in _subtract(cell, cutter)]
    return cells


def _split_at(poly, wall, distances):
    cells = [poly]
    for distance in sorted(set(distances)):
        a = (wall["a"][0] + wall["u"][0] * distance, wall["a"][1] + wall["u"][1] * distance)
        b = (a[0] + wall["n"][0], a[1] + wall["n"][1])
        cells = [piece for cell in cells for piece in (_clip(cell, a, b), _clip(cell, a, b, False)) if piece]
    return cells


def _vertical_intervals(start, end, holes):
    cursor = start
    result = []
    for lower, upper in sorted(holes):
        if lower > cursor + EPS:
            result.append((cursor, lower))
        cursor = max(cursor, upper)
    if cursor < end - EPS:
        result.append((cursor, end))
    return result


def generate_building(spec, collection):
    """Create closed convex Parts; adjacent parts may share boundary faces."""
    parts = []
    materials = {}
    for name, data in spec.materials.items():
        payload = json.dumps([spec.name, name, data], sort_keys=True, separators=(",", ":"))
        digest = hashlib.sha256(payload.encode("utf-8")).hexdigest()
        material = next((m for m in bpy.data.materials if m.get("bd_material_hash") == digest), None)
        if material is None:
            material = bpy.data.materials.new(f"BDM/{spec.name[:18]}/{name[:12]}/{digest[:16]}")
            material["bd_material_hash"] = digest
            material["bd_asset_name"] = spec.name
            material["bd_material_id"] = name
            material.diffuse_color = data["color"]
            material.use_nodes = True
            principled = next(n for n in material.node_tree.nodes if n.type == "BSDF_PRINCIPLED")
            principled.inputs["Base Color"].default_value = data["color"]
        materials[name] = material
        material["bd_surface_role"] = "exterior"

    def prism(identifier, kind, material, polygon, bottom, top):
        poly = _ccw(polygon)
        if len(poly) < 3 or abs(_area(poly)) <= EPS or top - bottom <= EPS:
            return
        count = len(poly)
        vertices = [(x, y, z) for z in (bottom, top) for x, y in poly]
        faces = [tuple(reversed(range(count))), tuple(range(count, count * 2))]
        faces.extend((i, (i + 1) % count, (i + 1) % count + count, i + count) for i in range(count))
        mesh = bpy.data.meshes.new(identifier)
        mesh.from_pydata(vertices, [], faces)
        mesh.update(calc_edges=True)
        obj = bpy.data.objects.new(identifier, mesh)
        collection.objects.link(obj)
        obj.data.materials.append(materials[material])
        obj["bd_part_id"] = identifier
        obj["bd_kind"] = kind
        obj["bd_material"] = material
        parts.append(Part(identifier, kind, material, obj))

    floor_regions = _floor_regions(spec)
    for level in range(spec.levels):
        z = level * spec.floor_height
        for i, poly in enumerate(floor_regions):
            prism(f"floor-{level:02d}-{i:04d}", "floor", spec.floor_material, poly, z, z + spec.slab_thickness)
    if spec.roof_thickness > 0:
        z = spec.levels * spec.floor_height
        for i, poly in enumerate(floor_regions):
            prism(f"roof-{i:04d}", "roof", spec.roof_material, poly, z, z + spec.roof_thickness)

    walls = wall_layout(spec)
    occupied = []
    for wall in walls:
        key = wall["key"]
        if isinstance(key, str) and key.startswith("inner:"):
            regions = [intersection for region in floor_regions if (intersection := _intersection(wall["polygon"], region))]
            for cutter in occupied:
                regions = [piece for poly in regions for piece in _subtract(poly, cutter)]
            if not regions:
                raise ValueError(f"Wall {key} has no volume after clipping existing walls")
        else:
            regions = [wall["polygon"]]
        occupied.extend(regions)
        for level in range(spec.levels):
            bottom = level * spec.floor_height + spec.slab_thickness
            top = (level + 1) * spec.floor_height
            openings = [o for o in spec.openings if o["wall"] == key and o["level"] == level]
            distances = [v for opening in openings for v in (opening["offset"], opening["offset"] + opening["width"])]
            cells = [cell for region in regions for cell in _split_at(region, wall, distances)]
            key_id = f"outer-{key}" if isinstance(key, int) else key.replace(":", "-")
            for i, cell in enumerate(cells):
                center = (sum(p[0] for p in cell) / len(cell), sum(p[1] for p in cell) / len(cell))
                s = (center[0] - wall["a"][0]) * wall["u"][0] + (center[1] - wall["a"][1]) * wall["u"][1]
                holes = [(bottom + o["sill"], bottom + o["sill"] + o["height"]) for o in openings if o["offset"] - EPS < s < o["offset"] + o["width"] + EPS]
                for j, (lower, upper) in enumerate(_vertical_intervals(bottom, top, holes)):
                    prism(f"wall-{key_id}-l{level:02d}-p{i:04d}-z{j:02d}", "wall", wall["material"], cell, lower, upper)
            for index, opening in enumerate(openings):
                if not opening["frame"]:
                    continue
                s0, s1 = opening["offset"], opening["offset"] + opening["width"]
                z0, z1 = bottom + opening["sill"], bottom + opening["sill"] + opening["height"]
                width = opening["frame_width"]
                pieces = [(s0, s1, z1 - width, z1)]
                side_bottom = z0
                if opening["kind"] == "window":
                    pieces.append((s0, s1, z0, z0 + width))
                    side_bottom += width
                pieces.extend([(s0, s0 + width, side_bottom, z1 - width), (s1 - width, s1, side_bottom, z1 - width)])
                for piece_index, (left, right, low, high) in enumerate(pieces):
                    poly = _rectangle(wall["a"], wall["u"], wall["n"], left, right, wall["d0"], wall["d1"])
                    prism(f"frame-{key_id}-l{level:02d}-o{index:03d}-{piece_index}", "frame", opening["frame_material"], poly, low, high)
    return parts
