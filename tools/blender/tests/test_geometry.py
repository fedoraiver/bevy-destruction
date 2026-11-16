"""Run in Blender with --background --factory-startup --python-exit-code 1."""

import copy
import json
from pathlib import Path
import sys
import unittest

import bmesh
import bpy
from mathutils import Vector

TOOL_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(TOOL_ROOT))

from bevy_destruction.geometry import generate_building
from bevy_destruction.spec import BuildingSpec, load_spec


def projected_overlap(poly_a, poly_b):
    """Independent separating-axis check; boundary-only contact is allowed."""
    for polygon in (poly_a, poly_b):
        for a, b in zip(polygon, polygon[1:] + polygon[:1]):
            axis = Vector((a[1] - b[1], b[0] - a[0])).normalized()
            values_a = [axis.dot(Vector(p)) for p in poly_a]
            values_b = [axis.dot(Vector(p)) for p in poly_b]
            if min(max(values_a), max(values_b)) - max(min(values_a), min(values_b)) <= 2e-5:
                return False
    return True


def ray_hits(parts, start, end):
    bpy.context.view_layer.update()
    origin, target = Vector(start), Vector(end)
    direction = target - origin
    return any(part.obj.ray_cast(origin, direction.normalized(), distance=direction.length)[0] for part in parts)


class GeometryTests(unittest.TestCase):
    def setUp(self):
        self.collections = []

    def tearDown(self):
        for collection in self.collections:
            for obj in list(collection.objects):
                mesh = obj.data
                bpy.data.objects.remove(obj, do_unlink=True)
                if mesh.users == 0:
                    bpy.data.meshes.remove(mesh)
            bpy.data.collections.remove(collection)

    def build(self, spec):
        collection = bpy.data.collections.new("geometry-test")
        bpy.context.scene.collection.children.link(collection)
        self.collections.append(collection)
        return generate_building(spec, collection)

    def check_parts(self, parts):
        prisms = []
        total_volume = 0
        identifiers = set()
        for part in parts:
            self.assertNotIn(part.id, identifiers)
            identifiers.add(part.id)
            self.assertTrue(part.obj.matrix_world.is_identity)
            bm = bmesh.new()
            try:
                bm.from_mesh(part.obj.data)
                bm.normal_update()
                self.assertTrue(all(edge.is_manifold for edge in bm.edges), part.id)
                volume = bm.calc_volume(signed=True)
                self.assertGreater(volume, 1e-8, part.id)
                total_volume += volume
                for face in bm.faces:
                    self.assertTrue(all(face.normal.dot(v.co - face.verts[0].co) <= 2e-5 for v in bm.verts), part.id)
            finally:
                bm.free()
            vertices = [tuple(v.co) for v in part.obj.data.vertices]
            bottom = min(v[2] for v in vertices)
            top = max(v[2] for v in vertices)
            polygon = [(v[0], v[1]) for v in vertices if abs(v[2] - bottom) < 1e-6]
            for old_id, old_poly, old_bottom, old_top in prisms:
                if min(top, old_top) - max(bottom, old_bottom) > 2e-5:
                    self.assertFalse(projected_overlap(polygon, old_poly), f"Volume overlap: {part.id}, {old_id}")
            prisms.append((part.id, polygon, bottom, top))
        return total_volume

    def test_defaults_canonical_round_trip_and_physical_materials(self):
        spec = BuildingSpec.from_dict({"footprint": [[0, 0], [8, 0], [8, 6], [0, 6]]})
        self.assertEqual(BuildingSpec.from_dict(json.loads(json.dumps(spec.to_dict()))).to_dict(), spec.to_dict())
        for material in spec.materials.values():
            for key in ("density", "tension_strength", "compression_strength", "shear_strength", "young_modulus"):
                self.assertGreater(material[key], 0)
            self.assertGreater(material["poisson_ratio"], -1)
            self.assertLess(material["poisson_ratio"], 0.5)

    def test_invalid_specs_are_rejected(self):
        base = {"footprint": [[0, 0], [8, 0], [8, 6], [0, 6]]}
        cases = [
            {"typo": 1}, {"schema_version": True}, {"schema_version": 2},
            {"density": float("nan")}, {"levels": True},
            {"footprint": [[0, 0], [2, 2], [0, 2], [2, 0]]},
            {"wall_thickness": 4},
            {"holes": [[[7, 5], [9, 5], [9, 7], [7, 7]]]},
            {"holes": [[[0, 1], [1, 1], [1, 2], [0, 2]]]},
            {"materials": {"x": {"poisson_ratio": 0.5}}},
            {"materials": {"x": {"unknown": 1}}},
            {"openings": [{"wall": 0, "offset": 0, "width": 1, "height": 2}]},
            {"openings": [{"wall": 0, "offset": 1, "width": 1, "height": 8}]},
            {"openings": [{"wall": False, "offset": 1, "width": 1, "height": 2}]},
            {"openings": [{"wall": 0, "offset": 1, "width": 1, "height": 2, "kind": "door", "sill": 1}]},
            {"inner_walls": [{"start": [1, 1], "end": [1, 1]}]},
            {"openings": [{"wall": 0, "offset": 1, "width": 2, "height": 1}, {"wall": 0, "offset": 2, "width": 2, "height": 1}]},
        ]
        for case in cases:
            with self.subTest(case=case), self.assertRaises(ValueError):
                BuildingSpec.from_dict({**copy.deepcopy(base), **case})

    def test_three_examples_are_closed_convex_disjoint_and_preserve_volume(self):
        # Analytic totals include floor/roof slabs, wall bands, openings and frames.
        expected_volumes = {"rectangle": 65.697, "l_shape": 121.1882, "courtyard": 161.627072}
        for name, expected in expected_volumes.items():
            with self.subTest(example=name):
                spec = load_spec(TOOL_ROOT / "examples" / f"{name}.json")
                parts = self.build(spec)
                self.assertAlmostEqual(self.check_parts(parts), expected, delta=1e-3)
                self.assertEqual(BuildingSpec.from_dict(spec.to_dict()).to_dict(), spec.to_dict())
                self.assertTrue(any(part.id.startswith("wall-inner-") for part in parts))
                self.assertTrue(any(part.kind == "roof" for part in parts))

    def test_doors_windows_frames_inner_walls_and_courtyard_remain_open(self):
        rectangle = self.build(load_spec(TOOL_ROOT / "examples" / "rectangle.json"))
        self.assertFalse(ray_hits(rectangle, (1.6, -0.5, 1.2), (1.6, 0.5, 1.2)))
        self.assertTrue(ray_hits(rectangle, (1.025, -0.5, 1.2), (1.025, 0.5, 1.2)))
        self.assertFalse(ray_hits(rectangle, (3.7, -0.5, 5.0), (3.7, 0.5, 5.0)))
        self.assertTrue(ray_hits(rectangle, (3.025, -0.5, 5.0), (3.025, 0.5, 5.0)))
        self.assertTrue(ray_hits(rectangle, (3.5, 4.0, 1.2), (4.5, 4.0, 1.2)))
        l_shape = self.build(load_spec(TOOL_ROOT / "examples" / "l_shape.json"))
        self.assertFalse(ray_hits(l_shape, (2.0, 2.0, 1.2), (3.6, 2.0, 1.2)))
        self.assertFalse(ray_hits(l_shape, (8.0, 6.0, -1), (8.0, 6.0, 11)))
        courtyard = self.build(load_spec(TOOL_ROOT / "examples" / "courtyard.json"))
        self.assertFalse(ray_hits(courtyard, (6.0, 5.0, -1), (6.0, 5.0, 8.0)))
        self.assertFalse(ray_hits(courtyard, (6.0, 2.4, 1.5), (6.0, 3.4, 1.5)))

    def test_material_cache_reuses_content_without_mutating_prior_assets(self):
        data = {"name": "material_cache_test", "footprint": [[0, 0], [4, 0], [4, 4], [0, 4]]}
        old_parts = self.build(BuildingSpec.from_dict(data))
        old_material = old_parts[0].obj.data.materials[0]
        old_color = tuple(old_material.diffuse_color)
        for node in old_material.node_tree.nodes:
            if node.type == "BSDF_PRINCIPLED":
                node.name = "Renamed shader node"
        count = len(bpy.data.materials)
        repeated = self.build(BuildingSpec.from_dict(data))
        self.assertEqual(len(bpy.data.materials), count)
        self.assertEqual(repeated[0].obj.data.materials[0], old_material)
        changed = self.build(BuildingSpec.from_dict({**data, "materials": {"concrete": {"color": [0.1, 0.2, 0.3, 1]}}}))
        self.assertNotEqual(changed[0].obj.data.materials[0], old_material)
        self.assertEqual(tuple(old_material.diffuse_color), old_color)


if __name__ == "__main__":
    unittest.main(argv=[__file__], verbosity=2)
