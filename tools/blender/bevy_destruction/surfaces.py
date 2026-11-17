"""Mark buried source-part contact patches without moving solid boundaries."""

import bmesh
import bpy
from mathutils import Vector

from .bonds import _bounds, _clip, _face_contacts, _faces, _polygon_area
from .fracture import INTERIOR_MATERIAL_NAME, _material, mesh_metrics, validate_convex


def _part_contacts(parts, tolerance):
    entries = [(part, _bounds(part.obj), _faces(part.obj)) for part in parts]
    entries.sort(key=lambda entry: (entry[1][0][0], entry[0].id))
    result = {part.id: [] for part in parts}
    active = []
    for part_b, bounds_b, faces_b in entries:
        active = [
            entry for entry in active if entry[1][1][0] >= bounds_b[0][0] - tolerance
        ]
        for part_a, bounds_a, faces_a in active:
            if any(
                bounds_a[1][i] < bounds_b[0][i] - tolerance
                or bounds_b[1][i] < bounds_a[0][i] - tolerance
                for i in (1, 2)
            ):
                continue
            for normal, origin, tangent, bitangent, overlap in _face_contacts(
                faces_a, faces_b, tolerance
            ):
                points = []
                for p in overlap:
                    point = origin + tangent * p[0] + bitangent * p[1]
                    if not points or (point - points[-1]).length > 1e-7:
                        points.append(point)
                if len(points) > 1 and (points[0] - points[-1]).length <= 1e-7:
                    points.pop()
                if len(points) < 3:
                    continue
                result[part_a.id].append((normal, points))
                result[part_b.id].append((-normal, list(reversed(points))))
        active.append((part_b, bounds_b, faces_b))
    return result


def _coplanar_faces(bm, normal, origin, exterior_slots, tolerance):
    return [
        face
        for face in bm.faces
        if face.material_index in exterior_slots
        and face.normal.dot(normal) >= 1 - 1e-5
        and all(
            abs(normal.dot(vertex.co - origin)) <= tolerance for vertex in face.verts
        )
    ]


def _inside(point, patch, normal, tolerance):
    return all(
        normal.dot((b - a).cross(point - a)) >= -tolerance * (b - a).length
        for a, b in zip(patch, patch[1:] + patch[:1])
    )


def _overlaps(face, patch, normal, tolerance):
    origin = patch[0]
    tangent = (patch[1] - origin).normalized()
    bitangent = normal.cross(tangent)
    subject = [
        ((v.co - origin).dot(tangent), (v.co - origin).dot(bitangent))
        for v in face.verts
    ]
    boundary = [((p - origin).dot(tangent), (p - origin).dot(bitangent)) for p in patch]
    overlap = _clip(subject, boundary, tolerance * tolerance)
    return len(overlap) >= 3 and abs(_polygon_area(overlap)) > tolerance * tolerance


def classify_contact_surfaces(chunks, parts, tolerance=2e-5):
    """Split only render faces along actual source-Part overlaps.

    Cutting each polygon's edges through BMesh also splits adjacent edge loops,
    retaining a closed manifold. Colliders and the previously integrated mass
    properties remain untouched. Voronoi faces already carry the fracture role.
    """
    contacts = _part_contacts(parts, tolerance)
    for chunk in chunks:
        patches = contacts[chunk.part_id]
        if not patches:
            continue
        obj = chunk.obj
        exterior_slots = {
            i
            for i, material in enumerate(obj.data.materials)
            if material.get("bd_surface_role") == "exterior"
        }
        interior = _material(INTERIOR_MATERIAL_NAME, (0.3, 0.29, 0.27, 1.0))
        interior["bd_surface_role"] = "fracture"
        if interior.name not in obj.data.materials:
            obj.data.materials.append(interior)
        interior_index = obj.data.materials.find(interior.name)
        bm = bmesh.new()
        try:
            bm.from_mesh(obj.data)
            bm.normal_update()
            changed = False
            for normal, patch in patches:
                faces = _coplanar_faces(bm, normal, patch[0], exterior_slots, tolerance)
                if not any(_overlaps(face, patch, normal, tolerance) for face in faces):
                    continue
                for a, b in zip(patch, patch[1:] + patch[:1]):
                    faces = _coplanar_faces(
                        bm, normal, patch[0], exterior_slots, tolerance
                    )
                    cutter = (b - a).cross(normal).normalized()
                    faces = [
                        face
                        for face in faces
                        if min(cutter.dot(v.co - a) for v in face.verts) < -tolerance
                        and max(cutter.dot(v.co - a) for v in face.verts) > tolerance
                    ]
                    if not faces:
                        continue
                    geometry = list(
                        dict.fromkeys(
                            faces
                            + [edge for face in faces for edge in face.edges]
                            + [vertex for face in faces for vertex in face.verts]
                        )
                    )
                    bmesh.ops.bisect_plane(
                        bm,
                        geom=geometry,
                        dist=1e-7,
                        plane_co=a,
                        plane_no=cutter,
                        clear_outer=False,
                        clear_inner=False,
                    )
                for face in _coplanar_faces(
                    bm, normal, patch[0], exterior_slots, tolerance
                ):
                    if _inside(face.calc_center_median(), patch, normal, 1e-7):
                        face.material_index = interior_index
                changed = True
            if changed:
                bmesh.ops.remove_doubles(bm, verts=list(bm.verts), dist=1e-6)
                bmesh.ops.dissolve_degenerate(bm, edges=list(bm.edges), dist=1e-7)
                bm.normal_update()
                bm.to_mesh(obj.data)
                obj.data.update()
        finally:
            bm.free()
        if changed:
            validate_convex(obj)
            actual = mesh_metrics(obj)
            if abs(actual["volume"] - chunk.volume) > max(chunk.volume * 1e-5, 1e-10):
                raise ValueError(
                    f"{chunk.id}: surface classification changed the solid volume"
                )
            if (Vector(actual["center"]) - Vector(chunk.center)).length > tolerance:
                raise ValueError(
                    f"{chunk.id}: surface classification changed the center of mass"
                )


def _triangulated_role_areas(mesh, interior_index):
    import math

    mesh.calc_loop_triangles()
    areas = [[], []]
    for triangle in mesh.loop_triangles:
        a, b, c = [tuple(mesh.vertices[index].co) for index in triangle.vertices]
        u = tuple(b[i] - a[i] for i in range(3))
        v = tuple(c[i] - a[i] for i in range(3))
        cross = (
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        )
        role = int(
            mesh.polygons[triangle.polygon_index].material_index == interior_index
        )
        areas[role].append(math.sqrt(math.fsum(value * value for value in cross)) * 0.5)
    return [math.fsum(values) for values in areas]


def classify_bond_surfaces(chunks, bonds, tolerance=2e-5):
    """Mark only wholly covered source faces; runtime masks own partial contacts.

    Bonds are the final canonical v2 records in asset Y-up coordinates. No mesh
    vertex, face, or edge is inserted, moved, merged, removed, or triangulated.
    """
    import hashlib
    import json
    import math
    from .natural_union import _normal

    def dot(a, b):
        return math.fsum(x * y for x, y in zip(a, b))

    def sub(a, b):
        return tuple(x - y for x, y in zip(a, b))

    def cross(a, b):
        return (
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        )

    def geometry(mesh):
        return (
            [tuple(vertex.co) for vertex in mesh.vertices],
            [tuple(face.vertices) for face in mesh.polygons],
        )

    patches = {chunk.id: [] for chunk in chunks}
    for bond in bonds:
        for patch in bond["patches"]:
            polygon = [(point[0], -point[2], point[1]) for point in patch["vertices"]]
            for identifier, points in (
                (bond["a"], polygon),
                (bond["b"], list(reversed(polygon))),
            ):
                normal = _normal(points)
                edges = [
                    (a, sub(b, a)) for a, b in zip(points, points[1:] + points[:1])
                ]
                low = [min(point[axis] for point in points) for axis in range(3)]
                high = [max(point[axis] for point in points) for axis in range(3)]
                patches[identifier].append((normal, points[0], edges, low, high))
    for chunk in chunks:
        mesh = chunk.obj.data
        before = geometry(mesh)
        exterior = next(
            index
            for index, material in enumerate(mesh.materials)
            if material.get("bd_surface_role") == "exterior"
        )
        interior = _material(INTERIOR_MATERIAL_NAME, (0.3, 0.29, 0.27, 1.0))
        interior["bd_surface_role"] = "fracture"
        if interior.name not in mesh.materials:
            mesh.materials.append(interior)
        interior_index = mesh.materials.find(interior.name)
        covered = 0
        for face in mesh.polygons:
            face.material_index = exterior
            points = [tuple(mesh.vertices[index].co) for index in face.vertices]
            normal = _normal(points)
            low = [min(point[axis] for point in points) for axis in range(3)]
            high = [max(point[axis] for point in points) for axis in range(3)]
            for patch_normal, origin, edges, patch_low, patch_high in patches[chunk.id]:
                if dot(normal, patch_normal) < 1 - 1e-5:
                    continue
                if any(
                    low[axis] < patch_low[axis] - 1e-9
                    or high[axis] > patch_high[axis] + 1e-9
                    for axis in range(3)
                ):
                    continue
                if any(
                    abs(dot(patch_normal, sub(point, origin))) > tolerance
                    for point in points
                ):
                    continue
                if all(
                    dot(patch_normal, cross(edge, sub(point, a)))
                    >= -1e-10 * math.sqrt(dot(edge, edge))
                    for a, edge in edges
                    for point in points
                ):
                    face.material_index = interior_index
                    covered += 1
                    break
        if geometry(mesh) != before:
            raise ValueError(
                f"{chunk.id}: whole-face material assignment changed source geometry"
            )
        fingerprint = hashlib.sha256(
            json.dumps(before, separators=(",", ":"), allow_nan=False).encode()
        ).hexdigest()
        chunk.obj["bd_surface_partition_audit"] = json.dumps(
            {
                "source_geometry_preserved": True,
                "source_asset_space_geometry_sha256": fingerprint,
                "vertex_count": len(mesh.vertices),
                "face_count": len(mesh.polygons),
                "uniform_interior_faces": covered,
                "canonical_contact_patches": len(patches[chunk.id]),
                "final_surface_role_area_m2": _triangulated_role_areas(
                    mesh, interior_index
                ),
                "final_surface_role_area_drift_m2": 0.0,
                "maximum_original_vertex_surface_distance_m": 0.0,
            },
            sort_keys=True,
        )
