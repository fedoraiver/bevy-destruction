"""Mark buried source-part contact patches without moving solid boundaries."""

import bmesh
from mathutils import Vector

from .bonds import _bounds, _clip, _face_contacts, _faces, _polygon_area
from .fracture import INTERIOR_MATERIAL_NAME, _material, mesh_metrics, validate_convex


def _part_contacts(parts, tolerance):
    entries = [(part, _bounds(part.obj), _faces(part.obj)) for part in parts]
    entries.sort(key=lambda entry: (entry[1][0][0], entry[0].id))
    result = {part.id: [] for part in parts}
    active = []
    for part_b, bounds_b, faces_b in entries:
        active = [entry for entry in active if entry[1][1][0] >= bounds_b[0][0] - tolerance]
        for part_a, bounds_a, faces_a in active:
            if any(bounds_a[1][i] < bounds_b[0][i] - tolerance or
                   bounds_b[1][i] < bounds_a[0][i] - tolerance for i in (1, 2)):
                continue
            for normal, origin, tangent, bitangent, overlap in _face_contacts(faces_a, faces_b, tolerance):
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
    return [face for face in bm.faces if face.material_index in exterior_slots and
            face.normal.dot(normal) >= 1 - 1e-5 and
            all(abs(normal.dot(vertex.co - origin)) <= tolerance for vertex in face.verts)]


def _inside(point, patch, normal, tolerance):
    return all(normal.dot((b - a).cross(point - a)) >= -tolerance * (b - a).length
               for a, b in zip(patch, patch[1:] + patch[:1]))


def _overlaps(face, patch, normal, tolerance):
    origin = patch[0]
    tangent = (patch[1] - origin).normalized()
    bitangent = normal.cross(tangent)
    subject = [((v.co - origin).dot(tangent), (v.co - origin).dot(bitangent)) for v in face.verts]
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
        exterior_slots = {i for i, material in enumerate(obj.data.materials)
                          if material.get("bd_surface_role") == "exterior"}
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
                    faces = _coplanar_faces(bm, normal, patch[0], exterior_slots, tolerance)
                    cutter = (b - a).cross(normal).normalized()
                    faces = [face for face in faces
                             if min(cutter.dot(v.co - a) for v in face.verts) < -tolerance and
                             max(cutter.dot(v.co - a) for v in face.verts) > tolerance]
                    if not faces:
                        continue
                    geometry = list(dict.fromkeys(faces + [edge for face in faces for edge in face.edges] +
                                                  [vertex for face in faces for vertex in face.verts]))
                    bmesh.ops.bisect_plane(bm, geom=geometry, dist=1e-7,
                                           plane_co=a, plane_no=cutter,
                                           clear_outer=False, clear_inner=False)
                for face in _coplanar_faces(bm, normal, patch[0], exterior_slots, tolerance):
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
                raise ValueError(f"{chunk.id}: surface classification changed the solid volume")
            if (Vector(actual["center"]) - Vector(chunk.center)).length > tolerance:
                raise ValueError(f"{chunk.id}: surface classification changed the center of mass")
