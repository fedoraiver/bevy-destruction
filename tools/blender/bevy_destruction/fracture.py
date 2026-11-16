"""Seeded convex Voronoi fracture in Blender-local meters, without add-ons."""

import bisect
import hashlib
import math
import random

import bmesh
import bpy
from mathutils import Vector

from .records import Chunk


MAX_CHUNKS_PER_PART = 256
MAX_CHUNKS_TOTAL = 2048
INTERIOR_MATERIAL_NAME = "BD_DestructionInterior"
EXTERIOR_MATERIAL_NAME = "BD_DestructionExterior"


def _identity_transform(obj):
    # matrix_world can still be stale immediately after editing location/scale.
    matrix = (
        obj.matrix_basis
        if obj.parent is None
        else obj.evaluated_get(bpy.context.evaluated_depsgraph_get()).matrix_world
    )
    return all(
        abs(matrix[row][column] - (1.0 if row == column else 0.0))
        <= 1e-6
        for row in range(4)
        for column in range(4)
    )


def _mesh_bmesh(obj):
    if obj.type != "MESH":
        raise ValueError(f"{obj.name}: expected a mesh object")
    if not _identity_transform(obj):
        raise ValueError(f"{obj.name}: apply transforms before fracture")
    if obj.modifiers:
        raise ValueError(f"{obj.name}: apply modifiers before fracture")
    bm = bmesh.new()
    bm.from_mesh(obj.data)
    bm.normal_update()
    return bm


def _bounds(bm):
    if not bm.verts:
        raise ValueError("mesh has no vertices")
    low = tuple(min(vertex.co[axis] for vertex in bm.verts) for axis in range(3))
    high = tuple(max(vertex.co[axis] for vertex in bm.verts) for axis in range(3))
    if not all(math.isfinite(value) for value in low + high):
        raise ValueError("mesh contains non-finite coordinates")
    return low, high


def _check_closed(bm, scale):
    if len(bm.verts) < 4 or len(bm.faces) < 4:
        raise ValueError("mesh must contain a closed solid")
    if any(not edge.is_manifold for edge in bm.edges):
        raise ValueError("mesh has boundary, wire, or non-manifold edges")
    if any(edge.calc_length() <= scale * 1e-9 for edge in bm.edges):
        raise ValueError("mesh has a zero-length edge")
    if any(face.calc_area() <= scale * scale * 1e-12 for face in bm.faces):
        raise ValueError("mesh has a zero-area or numerically degenerate face")
    pending = {next(iter(bm.verts))}
    visited = set()
    while pending:
        vertex = pending.pop()
        if vertex in visited:
            continue
        visited.add(vertex)
        pending.update(edge.other_vert(vertex) for edge in vertex.link_edges)
    if len(visited) != len(bm.verts):
        raise ValueError("mesh must contain exactly one connected solid")


def _dot(a, b):
    return sum(a[axis] * b[axis] for axis in range(3))


def _cross(a, b):
    return (
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    )


def _face_normal(face):
    # Subtract a nearby origin before Newell's sum to avoid cancellation on
    # small coplanar subdivisions far from the building origin.
    origin = tuple(face.verts[0].co)
    points = [tuple(float(v.co[i]) - origin[i] for i in range(3)) for v in face.verts]
    crosses = [_cross(a, b) for a, b in zip(points, points[1:] + points[:1])]
    return Vector(tuple(math.fsum(c[i] for c in crosses) for i in range(3))).normalized()


def _mass_properties(bm):
    """Integrate signed tetrahedra about a nearby origin using Python doubles."""
    origin = tuple(
        math.fsum(float(vertex.co[axis]) for vertex in bm.verts) / len(bm.verts)
        for axis in range(3)
    )
    triangles = bmesh.ops.triangulate(bm, faces=list(bm.faces))["faces"]
    volumes = []
    first = [[] for _ in range(3)]
    second = [[[] for _ in range(3)] for _ in range(3)]
    for face in triangles:
        points = [
            tuple(float(vertex.co[axis]) - origin[axis] for axis in range(3))
            for vertex in face.verts
        ]
        a, b, c = points
        volume = _dot(a, _cross(b, c)) / 6.0
        total = tuple(a[axis] + b[axis] + c[axis] for axis in range(3))
        volumes.append(volume)
        for row in range(3):
            first[row].append(volume * total[row] / 4.0)
            for column in range(3):
                second[row][column].append(
                    volume
                    * (
                        total[row] * total[column]
                        + math.fsum(point[row] * point[column] for point in points)
                    )
                    / 20.0
                )
    volume = math.fsum(volumes)
    if not math.isfinite(volume) or volume <= 0:
        raise ValueError("mesh has non-positive signed volume or inconsistent winding")
    relative_center = tuple(math.fsum(values) / volume for values in first)
    center = tuple(origin[axis] + relative_center[axis] for axis in range(3))
    covariance = [
        [
            math.fsum(second[row][column])
            - volume * relative_center[row] * relative_center[column]
            for column in range(3)
        ]
        for row in range(3)
    ]
    trace = sum(covariance[axis][axis] for axis in range(3))
    inertia = [
        [
            (trace if row == column else 0.0) - covariance[row][column]
            for column in range(3)
        ]
        for row in range(3)
    ]
    if any(not math.isfinite(value) for row in inertia for value in row):
        raise ValueError("mesh inertia is not finite")
    if any(inertia[axis][axis] <= 0 for axis in range(3)):
        raise ValueError("mesh inertia is degenerate")
    return volume, center, inertia


def mesh_metrics(obj):
    """Return density-one volume, centroidal inertia, area and local AABB.

    The input must be one closed, outward-oriented mesh with identity transform
    and applied modifiers. This function does not modify the source mesh.
    """
    bm = _mesh_bmesh(obj)
    try:
        low, high = _bounds(bm)
        scale = max(high[axis] - low[axis] for axis in range(3))
        if scale <= 0:
            raise ValueError("mesh has zero extent")
        _check_closed(bm, scale)
        area = math.fsum(face.calc_area() for face in bm.faces)
        vertex_count, face_count = len(bm.verts), len(bm.faces)
        volume, center, inertia = _mass_properties(bm)
        return {
            "volume": volume,
            "center": center,
            "inertia": inertia,
            "area": area,
            "bounds": (low, high),
            "vertex_count": vertex_count,
            "face_count": face_count,
        }
    finally:
        bm.free()


def validate_convex(obj, tolerance=None):
    """Return True for one closed outward-oriented convex solid; otherwise raise."""
    bm = _mesh_bmesh(obj)
    try:
        low, high = _bounds(bm)
        scale = max(high[axis] - low[axis] for axis in range(3))
        if scale <= 0:
            raise ValueError(f"{obj.name}: mesh has zero extent")
        _check_closed(bm, scale)
        epsilon = max(scale * 1e-5, 1e-7) if tolerance is None else tolerance
        for face in bm.faces:
            point = face.verts[0].co
            normal = _face_normal(face)
            if any(normal.dot(vertex.co - point) > epsilon for vertex in bm.verts):
                raise ValueError(f"{obj.name}: mesh is non-convex or has inconsistent normals")
        if bm.calc_volume(signed=True) <= 0:
            raise ValueError(f"{obj.name}: mesh has non-positive signed volume")
        return True
    finally:
        bm.free()


def _material(name, color):
    material = bpy.data.materials.get(name)
    if material is None:
        material = bpy.data.materials.new(name)
        material.diffuse_color = color
    material.use_nodes = True
    principled = next(
        (node for node in material.node_tree.nodes if node.type == "BSDF_PRINCIPLED"),
        None,
    )
    if principled is not None:
        principled.inputs["Base Color"].default_value = material.diffuse_color
    return material


def _source_bmesh(part):
    bm = _mesh_bmesh(part.obj)
    materials = list(part.obj.data.materials)
    if not materials:
        materials.append(_material(EXTERIOR_MATERIAL_NAME, (0.55, 0.57, 0.6, 1.0)))
    for material in materials:
        if "bd_surface_role" not in material:
            material["bd_surface_role"] = "exterior"
    interior = _material(INTERIOR_MATERIAL_NAME, (0.3, 0.29, 0.27, 1.0))
    interior["bd_surface_role"] = "fracture"
    if interior not in materials:
        materials.append(interior)
    return bm, materials, materials.index(interior)


def _sites(part, count, seed):
    """Sample uniformly inside convex volume, using weighted tetrahedra."""
    bm = _mesh_bmesh(part.obj)
    try:
        reference = sum((vertex.co for vertex in bm.verts), Vector()) / len(bm.verts)
        triangles = bmesh.ops.triangulate(bm, faces=list(bm.faces))["faces"]
        tetrahedra, cumulative = [], []
        total = 0.0
        for face in triangles:
            points = tuple(vertex.co.copy() for vertex in face.verts)
            a, b, c = (point - reference for point in points)
            volume = float(a.dot(b.cross(c))) / 6.0
            if volume > 0:
                total += volume
                cumulative.append(total)
                tetrahedra.append((reference.copy(),) + points)
        if total <= 0:
            raise ValueError(f"{part.id}: cannot sample a non-positive volume")
        digest = hashlib.sha256(f"voronoi-v1\0{seed}\0{part.id}".encode("utf-8")).digest()
        rng = random.Random(int.from_bytes(digest, "big"))
        sites = []
        for _ in range(count):
            tetrahedron = tetrahedra[bisect.bisect_left(cumulative, rng.random() * total)]
            weights = [rng.expovariate(1.0) for _ in range(4)]
            weight_sum = sum(weights)
            sites.append(
                sum(
                    (point * (weight / weight_sum) for point, weight in zip(tetrahedron, weights)),
                    Vector(),
                )
            )
        return sites
    finally:
        bm.free()


def _clip_cell(source, sites, index, interior_index, epsilon):
    bm = source.copy()
    try:
        site = sites[index]
        for other_index, other in enumerate(sites):
            if other_index == index:
                continue
            normal = other - site
            if normal.length <= epsilon * 10:
                raise ValueError(f"Voronoi seeds {index} and {other_index} are numerically coincident; change seed")
            normal.normalize()
            point = (site + other) * 0.5
            distances = [normal.dot(vertex.co - point) for vertex in bm.verts]
            if max(distances) <= epsilon:
                continue
            if min(distances) >= -epsilon:
                raise ValueError(f"Voronoi cell {index} became empty or thinner than clipping tolerance")
            bmesh.ops.bisect_plane(
                bm,
                geom=list(bm.verts) + list(bm.edges) + list(bm.faces),
                dist=epsilon,
                plane_co=point,
                plane_no=normal,
                clear_outer=True,
                clear_inner=False,
            )
            boundary = [edge for edge in bm.edges if edge.is_boundary]
            if boundary:
                caps = bmesh.ops.holes_fill(bm, edges=boundary, sides=0)["faces"]
                if not caps:
                    raise ValueError(f"Voronoi cell {index}: failed to cap a cut")
                for face in caps:
                    face.material_index = interior_index
            if not bm.faces:
                raise ValueError(f"Voronoi cell {index}: clipping produced an empty cell")
        bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
        return bm
    except Exception:
        bm.free()
        raise


def _object_from_bmesh(bm, name, collection, materials, created_objects, created_meshes):
    mesh = bpy.data.meshes.new(name)
    created_meshes.append(mesh)
    bm.to_mesh(mesh)
    mesh.update()
    for material in materials:
        mesh.materials.append(material)
    obj = bpy.data.objects.new(name, mesh)
    created_objects.append(obj)
    collection.objects.link(obj)
    return obj


def _collider(obj, chunk_id, collection, created_objects, created_meshes):
    # A clipped convex solid already is its convex hull. Re-running QHull on
    # collinear clipping vertices can introduce invalid microscopic facets.
    mesh = obj.data.copy()
    mesh.name = f"BD_collider_{chunk_id}"
    mesh.materials.clear()
    created_meshes.append(mesh)
    collider = bpy.data.objects.new(f"BD_collider_{chunk_id}", mesh)
    created_objects.append(collider)
    collection.objects.link(collider)
    collider.hide_render = True
    collider.display_type = "WIRE"
    collider["bd_role"] = "collider"
    collider["bd_chunk_id"] = chunk_id
    validate_convex(collider)
    return collider


def fracture_parts(parts, *, seed: int, target_size: float, collection) -> list[Chunk]:
    """Fracture closed convex Parts into actual seeded Voronoi intersections.

    Site count is ceil(prod(max(1, AABB_extent / target_size))), preserving a
    meaningful planar scale for thin walls/slabs. It is a count estimate, not
    a bound on individual cell dimensions. Oversized requests fail explicitly.
    All output meshes stay in building coordinates with identity transforms;
    inertia is about the chunk centroid at density one, in Blender Z-up axes.
    Source objects are untouched. On failure, newly created output is removed.
    """
    if type(seed) is not int:
        raise ValueError("fracture seed must be an integer")
    if not math.isfinite(target_size) or target_size <= 0:
        raise ValueError("target_size must be a finite positive length in meters")
    if not isinstance(collection, bpy.types.Collection):
        raise ValueError("fracture output collection must be a Blender Collection")
    parts = list(parts)
    if len({part.id for part in parts}) != len(parts):
        raise ValueError("source Part IDs must be unique")
    plans = []
    for part in parts:
        validate_convex(part.obj)
        metrics = mesh_metrics(part.obj)
        low, high = metrics["bounds"]
        extents = [high[axis] - low[axis] for axis in range(3)]
        count = max(1, math.ceil(math.prod(max(1.0, extent / target_size) for extent in extents) - 1e-7))
        if count > MAX_CHUNKS_PER_PART:
            raise ValueError(
                f"{part.id}: target_size={target_size} requests {count} cells; "
                f"limit is {MAX_CHUNKS_PER_PART}. Increase target_size or split the source Part"
            )
        plans.append((part, metrics, count, max(extents)))
    total = sum(plan[2] for plan in plans)
    if total > MAX_CHUNKS_TOTAL:
        raise ValueError(f"fracture requests {total} cells; total limit is {MAX_CHUNKS_TOTAL}; increase target_size")

    created_objects, created_meshes, chunks = [], [], []
    try:
        for part, original, count, scale in plans:
            sites = _sites(part, count, seed)
            source, materials, interior_index = _source_bmesh(part)
            part_chunks = []
            try:
                for index in range(count):
                    chunk_id = f"{part.id}:voronoi:{seed}:{index:04d}"
                    cell = _clip_cell(source, sites, index, interior_index, max(scale * 1e-7, 1e-8))
                    try:
                        obj = _object_from_bmesh(
                            cell, f"BD_chunk_{chunk_id}", collection, materials, created_objects, created_meshes
                        )
                    finally:
                        cell.free()
                    obj["bd_role"] = "chunk"
                    obj["bd_chunk_id"] = chunk_id
                    obj["bd_part_id"] = part.id
                    validate_convex(obj)
                    metrics = mesh_metrics(obj)
                    minimum_volume = max(original["volume"] / count * 1e-5, 1e-12)
                    low, high = metrics["bounds"]
                    if metrics["volume"] < minimum_volume or min(high[axis] - low[axis] for axis in range(3)) < scale * 1e-7:
                        raise ValueError(f"{chunk_id}: numerically degenerate/sliver cell; change seed or increase target_size")
                    collider = _collider(obj, chunk_id, collection, created_objects, created_meshes)
                    collider_metrics = mesh_metrics(collider)
                    if abs(collider_metrics["volume"] - metrics["volume"]) > max(metrics["volume"] * 1e-4, 1e-10):
                        raise ValueError(f"{chunk_id}: convex proxy changed the solid volume")
                    part_chunks.append(
                        Chunk(
                            id=chunk_id,
                            part_id=part.id,
                            kind=part.kind,
                            material=part.material,
                            obj=obj,
                            collider=collider,
                            volume=metrics["volume"],
                            center=metrics["center"],
                            inertia=metrics["inertia"],
                        )
                    )
            finally:
                source.free()
            final_volume = math.fsum(chunk.volume for chunk in part_chunks)
            tolerance = max(original["volume"] * 1e-4, 1e-10)
            if abs(final_volume - original["volume"]) > tolerance:
                raise ValueError(
                    f"{part.id}: Voronoi volume not conserved: {final_volume:.9g} vs "
                    f"{original['volume']:.9g} (tolerance {tolerance:.3g})"
                )
            chunks.extend(part_chunks)
        return chunks
    except Exception:
        for obj in reversed(created_objects):
            bpy.data.objects.remove(obj, do_unlink=True)
        for mesh in created_meshes:
            if mesh.users == 0:
                bpy.data.meshes.remove(mesh)
        raise
