"""Run with Blender --background --factory-startup --python-exit-code 1 --python FILE."""

from pathlib import Path
import sys
import unittest
from unittest.mock import patch

import bmesh
import bpy
from mathutils import Euler, Matrix, Vector

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from bevy_destruction.fracture import (
    INTERIOR_MATERIAL_NAME,
    fracture_parts,
    mesh_metrics,
    validate_convex,
)
from bevy_destruction.records import Part


class FractureTests(unittest.TestCase):
    def setUp(self):
        self.collection = bpy.data.collections.new("fracture_tests")
        bpy.context.scene.collection.children.link(self.collection)
        self.material = bpy.data.materials.get("fracture_test_exterior")
        if self.material is None:
            self.material = bpy.data.materials.new("fracture_test_exterior")

    def tearDown(self):
        for obj in list(self.collection.objects):
            mesh = obj.data
            bpy.data.objects.remove(obj, do_unlink=True)
            if mesh.users == 0:
                bpy.data.meshes.remove(mesh)
        bpy.data.collections.remove(self.collection)

    def mesh(self, name, vertices, faces):
        mesh = bpy.data.meshes.new(name)
        mesh.from_pydata(vertices, [], faces)
        mesh.update()
        mesh.materials.append(self.material)
        obj = bpy.data.objects.new(name, mesh)
        self.collection.objects.link(obj)
        return obj

    def box(self, name, size, center=(0, 0, 0), rotation=None):
        bm = bmesh.new()
        bmesh.ops.create_cube(bm, size=1)
        rotation = Matrix.Identity(3) if rotation is None else rotation
        for vertex in bm.verts:
            vertex.co = rotation @ Vector(
                tuple(vertex.co[axis] * size[axis] for axis in range(3))
            ) + Vector(center)
        mesh = bpy.data.meshes.new(name)
        bm.to_mesh(mesh)
        bm.free()
        mesh.materials.append(self.material)
        obj = bpy.data.objects.new(name, mesh)
        self.collection.objects.link(obj)
        return obj

    def prism(self, name, polygon, bottom, top):
        n = len(polygon)
        vertices = [(x, y, z) for z in (bottom, top) for x, y in polygon]
        faces = [tuple(reversed(range(n))), tuple(range(n, n * 2))]
        faces += [(i, (i + 1) % n, (i + 1) % n + n, i + n) for i in range(n)]
        return self.mesh(name, vertices, faces)

    def fracture(self, parts, seed=27, size=1.5):
        return fracture_parts(
            parts, seed=seed, target_size=size, collection=self.collection
        )

    def assert_solids(self, chunks):
        for chunk in chunks:
            self.assertIsNot(chunk.obj, chunk.collider)
            self.assertIsNot(chunk.obj.data, chunk.collider.data)
            self.assertTrue(validate_convex(chunk.obj))
            self.assertTrue(validate_convex(chunk.collider))
            self.assertGreater(chunk.volume, 0)
            self.assertAlmostEqual(
                mesh_metrics(chunk.collider)["volume"], chunk.volume, delta=1e-7
            )

    @staticmethod
    def signature(chunks):
        return [
            (
                chunk.id,
                [tuple(vertex.co) for vertex in chunk.obj.data.vertices],
                [(tuple(face.vertices), face.material_index) for face in chunk.obj.data.polygons],
            )
            for chunk in chunks
        ]

    def test_thin_parts_volume_materials_and_seed_reproducibility(self):
        parts = [
            Part("wall", "wall", "concrete", self.box("wall", (6, 0.18, 3))),
            Part("slab", "floor", "concrete", self.box("slab", (6, 4, 0.2))),
        ]
        first = self.fracture(parts)
        self.assert_solids(first)
        self.assertGreater(len(first), 10)
        for part in parts:
            source = mesh_metrics(part.obj)
            chunks = [chunk for chunk in first if chunk.part_id == part.id]
            self.assertAlmostEqual(
                sum(chunk.volume for chunk in chunks),
                source["volume"],
                delta=source["volume"] * 1e-5,
            )
            exterior_area = sum(
                face.area
                for chunk in chunks
                for face in chunk.obj.data.polygons
                if face.material_index == 0
            )
            self.assertAlmostEqual(exterior_area, source["area"], delta=source["area"] * 1e-5)
        self.assertEqual(self.signature(first), self.signature(self.fracture(parts)))
        changed = self.fracture(parts, seed=28)
        self.assertNotEqual(
            [entry[1] for entry in self.signature(first)],
            [entry[1] for entry in self.signature(changed)],
        )
        self.assertTrue(all(chunk.obj.data.materials[0] == self.material for chunk in first))
        self.assertTrue(any(face.material_index == 1 for chunk in first for face in chunk.obj.data.polygons))
        interior = bpy.data.materials[INTERIOR_MATERIAL_NAME]
        self.assertTrue(interior.use_nodes)
        principled = next(node for node in interior.node_tree.nodes if node.type == "BSDF_PRINCIPLED")
        self.assertEqual(tuple(principled.inputs["Base Color"].default_value), tuple(interior.diffuse_color))
        self.assertEqual(
            len([material for material in bpy.data.materials if material.name.startswith(INTERIOR_MATERIAL_NAME)]),
            1,
        )

    def test_translated_rotated_box_inertia_and_fragment_reassembly(self):
        rotation = Euler((0.2, -0.35, 0.7)).to_matrix()
        source = self.box("rotated_box", (2, 1, 1.5), (10, -7, 3), rotation)
        metrics = mesh_metrics(source)
        self.assertAlmostEqual(metrics["volume"], 3.0, delta=2e-6)
        for axis, value in enumerate((10, -7, 3)):
            self.assertAlmostEqual(metrics["center"][axis], value, delta=1e-6)
        expected = rotation @ Matrix.Diagonal((0.8125, 1.5625, 1.25)) @ rotation.transposed()
        for row in range(3):
            for column in range(3):
                self.assertAlmostEqual(metrics["inertia"][row][column], expected[row][column], delta=3e-6)
        chunks = self.fracture([Part("rotated_box", "wall", "concrete", source)], size=1.3)
        self.assert_solids(chunks)
        rebuilt = [[0.0] * 3 for _ in range(3)]
        for chunk in chunks:
            displacement = [chunk.center[i] - metrics["center"][i] for i in range(3)]
            squared_distance = sum(value * value for value in displacement)
            for row in range(3):
                for column in range(3):
                    rebuilt[row][column] += chunk.inertia[row][column] + chunk.volume * (
                        (squared_distance if row == column else 0) - displacement[row] * displacement[column]
                    )
        for row in range(3):
            for column in range(3):
                self.assertAlmostEqual(rebuilt[row][column], metrics["inertia"][row][column], delta=1e-5)

    def test_tetrahedron_mass_properties_and_fracture(self):
        source = self.mesh(
            "tetrahedron",
            [(0, 0, 0), (2, 0, 0), (0, 3, 0), (0, 0, 4)],
            [(0, 2, 1), (0, 1, 3), (1, 2, 3), (2, 0, 3)],
        )
        metrics = mesh_metrics(source)
        self.assertAlmostEqual(metrics["volume"], 4.0)
        self.assertEqual(metrics["center"], (0.5, 0.75, 1.0))
        expected = [[3.75, 0.3, 0.4], [0.3, 3.0, 0.6], [0.4, 0.6, 1.95]]
        for row in range(3):
            for column in range(3):
                self.assertAlmostEqual(metrics["inertia"][row][column], expected[row][column])
        chunks = self.fracture([Part("tetra", "wall", "concrete", source)], seed=42, size=1.4)
        self.assert_solids(chunks)
        self.assertAlmostEqual(sum(chunk.volume for chunk in chunks), 4.0, delta=1e-5)

    def test_regression_elevated_triangular_slab_seed_7(self):
        # The rectangle example formerly failed while re-hulling cell 0005.
        source = self.prism("elevated_floor", [(0, 6), (0, 0), (8, 0)], 3.2, 3.4)
        chunks = self.fracture(
            [Part("floor-01-0000", "floor", "concrete", source)], seed=7, size=1.5
        )
        self.assertEqual(len(chunks), 22)
        self.assert_solids(chunks)
        self.assertAlmostEqual(
            sum(chunk.volume for chunk in chunks), mesh_metrics(source)["volume"], delta=1e-5
        )

    def test_reject_unsupported_inputs_and_roll_back_slivers(self):
        source = self.box("unit_box", (1, 1, 1))
        part = Part("unit", "wall", "concrete", source)
        before_objects, before_meshes = set(bpy.data.objects), set(bpy.data.meshes)
        with self.assertRaisesRegex(ValueError, "requests.*limit"):
            self.fracture([part], size=0.01)
        source.location.x = 2
        with self.assertRaisesRegex(ValueError, "apply transforms"):
            self.fracture([part])
        source.location.x = 0
        self.assertEqual(set(bpy.data.objects), before_objects)
        self.assertEqual(set(bpy.data.meshes), before_meshes)

        concave = self.prism(
            "concave", [(0, 0), (2, 0), (2, 1), (1, 1), (1, 2), (0, 2)], 0, 1
        )
        with self.assertRaisesRegex(ValueError, "non-convex"):
            self.fracture([Part("concave", "wall", "concrete", concave)])

        before_objects, before_meshes = set(bpy.data.objects), set(bpy.data.meshes)
        # Place two sites near one boundary: the first cell succeeds, then a
        # genuine thin second cell must fail and remove all newly made output.
        sites = [Vector((0.499997, 0, 0)), Vector((0.499999, 0, 0))]
        with patch("bevy_destruction.fracture._sites", return_value=sites):
            with self.assertRaisesRegex(ValueError, "sliver"):
                self.fracture([part], size=0.8)
        self.assertEqual(set(bpy.data.objects), before_objects)
        self.assertEqual(set(bpy.data.meshes), before_meshes)
        self.assertEqual(len(self.collection.objects), 2)


if __name__ == "__main__":
    unittest.main(argv=[__file__], verbosity=2)
