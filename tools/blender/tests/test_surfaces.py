"""Run with Blender --background --factory-startup --python-exit-code 1 --python FILE."""

from pathlib import Path
import sys
import unittest

import bmesh
import bpy
from mathutils import Matrix, Vector

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from bevy_destruction.bonds import generate_bonds
from bevy_destruction.fracture import fracture_parts, mesh_metrics, validate_convex
from bevy_destruction.records import Part
from bevy_destruction.surfaces import classify_contact_surfaces


class SurfaceTests(unittest.TestCase):
    def setUp(self):
        self.collection = bpy.data.collections.new("surface_tests")
        bpy.context.scene.collection.children.link(self.collection)
        self.material = bpy.data.materials.new("surface_test_exterior")

    def tearDown(self):
        for obj in list(self.collection.objects):
            mesh = obj.data
            bpy.data.objects.remove(obj, do_unlink=True)
            bpy.data.meshes.remove(mesh)
        bpy.data.collections.remove(self.collection)
        bpy.data.materials.remove(self.material)

    def box(self, name, low, high, rotation=None):
        bm = bmesh.new()
        bmesh.ops.create_cube(bm, size=1)
        rotation = Matrix.Identity(3) if rotation is None else rotation
        for vertex in bm.verts:
            vertex.co = rotation @ Vector(tuple(low[i] + (vertex.co[i] + 0.5) * (high[i] - low[i])
                                                for i in range(3)))
        mesh = bpy.data.meshes.new(name)
        bm.to_mesh(mesh)
        bm.free()
        mesh.materials.append(self.material)
        obj = bpy.data.objects.new(name, mesh)
        self.collection.objects.link(obj)
        return Part(name, "wall", "concrete", obj)

    def prism(self, name, polygon, bottom, top):
        count = len(polygon)
        mesh = bpy.data.meshes.new(name)
        vertices = [(x, y, z) for z in (bottom, top) for x, y in polygon]
        faces = [tuple(reversed(range(count))), tuple(range(count, 2 * count))]
        faces += [(i, (i + 1) % count, (i + 1) % count + count, i + count) for i in range(count)]
        mesh.from_pydata(vertices, [], faces)
        mesh.update()
        mesh.materials.append(self.material)
        obj = bpy.data.objects.new(name, mesh)
        self.collection.objects.link(obj)
        return Part(name, "wall", "concrete", obj)

    def check_classification(self, parts, expected_areas, size=100):
        chunks = fracture_parts(parts, seed=7, target_size=size, collection=self.collection)
        before = {chunk.id: ([tuple(v.co) for v in chunk.collider.data.vertices],
                             [tuple(f.vertices) for f in chunk.collider.data.polygons],
                             mesh_metrics(chunk.obj)) for chunk in chunks}
        bonds = generate_bonds(chunks)
        classify_contact_surfaces(chunks, parts)
        after_bonds = generate_bonds(chunks)
        self.assertEqual([(b["a"], b["b"]) for b in bonds], [(b["a"], b["b"]) for b in after_bonds])
        for original, actual in zip(bonds, after_bonds):
            self.assertAlmostEqual(original["area"], actual["area"], delta=2e-5)
        areas = {part.id: 0 for part in parts}
        for chunk in chunks:
            self.assertTrue(validate_convex(chunk.obj))
            points, faces, metrics = before[chunk.id]
            self.assertEqual(points, [tuple(v.co) for v in chunk.collider.data.vertices])
            self.assertEqual(faces, [tuple(f.vertices) for f in chunk.collider.data.polygons])
            after = mesh_metrics(chunk.obj)
            self.assertAlmostEqual(metrics["volume"], after["volume"], delta=2e-6)
            self.assertAlmostEqual(metrics["area"], after["area"], delta=2e-5)
            for row in range(3):
                for column in range(3):
                    self.assertAlmostEqual(metrics["inertia"][row][column], after["inertia"][row][column], delta=2e-5)
            for face in chunk.obj.data.polygons:
                role = chunk.obj.data.materials[face.material_index].get("bd_surface_role")
                self.assertIn(role, ("exterior", "fracture"))
                if role == "fracture":
                    areas[chunk.part_id] += face.area
        for identifier, expected in expected_areas.items():
            self.assertAlmostEqual(areas[identifier], expected, delta=3e-5)
        return chunks

    def test_partial_face_keeps_exposed_area_exterior(self):
        parts = [self.box("slab", (0, 0, 0), (4, 3, 0.2)),
                 self.box("wall", (1, 1, 0.2), (3, 1.2, 2))]
        chunks = self.check_classification(parts, {"slab": 0.4, "wall": 0.4})
        slab = next(chunk for chunk in chunks if chunk.part_id == "slab")
        top_area = {}
        for face in slab.obj.data.polygons:
            if face.normal.z > 0.99:
                role = slab.obj.data.materials[face.material_index]["bd_surface_role"]
                top_area[role] = top_area.get(role, 0) + face.area
        self.assertAlmostEqual(top_area["exterior"], 11.6, places=5)
        self.assertAlmostEqual(top_area["fracture"], 0.4, places=5)

    def test_multiple_disjoint_patches_do_not_fill_gap(self):
        parts = [self.box("slab", (0, 0, 0), (4, 3, 0.2)),
                 self.box("left", (0, 0, 0.2), (1, 0.2, 2)),
                 self.box("right", (3, 0, 0.2), (4, 0.2, 2)),
                 self.box("point_only", (4, 3, 0.2), (5, 4, 1))]
        self.check_classification(parts, {"slab": 0.4, "left": 0.2, "right": 0.2, "point_only": 0})

    def test_rotated_coplanar_partial_contact(self):
        rotation = Matrix.Rotation(0.47, 3, "Z")
        parts = [self.box("a", (0, 0, 0), (2, 3, 1), rotation),
                 self.box("b", (2, 1, 0.2), (3, 2, 0.8), rotation)]
        self.check_classification(parts, {"a": 0.6, "b": 0.6})

    def test_partial_patch_survives_voronoi_subdivision(self):
        parts = [self.box("slab", (0, 0, 0), (4, 3, 0.2)),
                 self.box("wall", (1, 1, 0.2), (3, 1.2, 2))]
        source_area = sum(mesh_metrics(part.obj)["area"] for part in parts)
        chunks = self.check_classification(parts, {}, size=1.4)
        exterior_area = sum(face.area for chunk in chunks for face in chunk.obj.data.polygons
                            if chunk.obj.data.materials[face.material_index]["bd_surface_role"] == "exterior")
        self.assertAlmostEqual(exterior_area, source_area - 0.8, delta=3e-5)

    def test_surface_bake_is_deterministic(self):
        parts = [self.box("slab", (0, 0, 0), (4, 3, 0.2)),
                 self.box("wall", (1, 1, 0.2), (3, 1.2, 2))]
        signatures = []
        for _ in range(2):
            chunks = fracture_parts(parts, seed=7, target_size=1.4, collection=self.collection)
            classify_contact_surfaces(chunks, parts)
            signatures.append([(chunk.id, [tuple(v.co) for v in chunk.obj.data.vertices],
                                [(tuple(face.vertices), face.material_index) for face in chunk.obj.data.polygons])
                               for chunk in chunks])
        self.assertEqual(signatures[0], signatures[1])

    def test_shared_corner_on_triangulated_slab_contact(self):
        parts = [self.prism("slab_a", [(0, 0), (4, 0), (0, 3)], 0, 0.2),
                 self.prism("slab_b", [(4, 0), (4, 3), (0, 3)], 0, 0.2),
                 self.prism("wall", [(0, 3), (0.2, 2.8), (3.8, 2.8), (4, 3)], 0.2, 2)]
        chunks = self.check_classification(parts, {"wall": 0.76})
        source_area = sum(mesh_metrics(part.obj)["area"] for part in parts)
        exterior_area = sum(face.area for chunk in chunks for face in chunk.obj.data.polygons
                            if chunk.obj.data.materials[face.material_index]["bd_surface_role"] == "exterior")
        self.assertAlmostEqual(exterior_area, source_area - 2 * (0.76 + 1), delta=3e-5)


if __name__ == "__main__":
    result = unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(SurfaceTests))
    if not result.wasSuccessful():
        raise RuntimeError("Surface classification tests failed")
