"""Run with Blender --background --factory-startup --python-exit-code 1 --python this_file."""

import hashlib
import json
from pathlib import Path
import struct
import sys
import tempfile
import unittest
from unittest.mock import patch

import bmesh
import bpy
from mathutils import Vector

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from bevy_destruction import register, unregister
from bevy_destruction.bonds import generate_bonds
from bevy_destruction.fracture import mesh_metrics
from bevy_destruction.pipeline import OWNER, build_asset, export_asset
from bevy_destruction.records import Chunk
from bevy_destruction.spec import BuildingSpec
from bevy_destruction.ui import outline_from_object


def rectangle(name="integration", **changes):
    return {"name": name, "footprint": [[0, 0], [6, 0], [6, 4], [0, 4]],
            "levels": 1, "floor_height": 3, "wall_thickness": 0.2,
            "slab_thickness": 0.2, "roof_thickness": 0.2, "fracture_size": 2.0,
            "seed": 17, "openings": [
                {"wall": 0, "level": 0, "offset": 1, "width": 1, "height": 2, "sill": 0, "kind": "door"},
                {"wall": 0, "level": 0, "offset": 3.5, "width": 1, "height": 1, "sill": 1, "kind": "window"}],
            **changes}


def cube(name, minimum, maximum):
    bm = bmesh.new()
    bmesh.ops.create_cube(bm, size=1)
    for vertex in bm.verts:
        vertex.co = Vector(tuple(minimum[i] + (vertex.co[i] + 0.5) * (maximum[i] - minimum[i]) for i in range(3)))
    mesh = bpy.data.meshes.new(name)
    bm.to_mesh(mesh)
    bm.free()
    obj = bpy.data.objects.new(name, mesh)
    bpy.context.scene.collection.objects.link(obj)
    return obj


def objects_hit(objects, start, end):
    bpy.context.view_layer.update()
    for obj in objects:
        inverse = obj.matrix_world.inverted()
        origin, target = inverse @ Vector(start), inverse @ Vector(end)
        direction = target - origin
        if obj.ray_cast(origin, direction.normalized(), distance=direction.length)[0]:
            return True
    return False


def read_glb(path):
    data = Path(path).read_bytes()
    magic, version, length, json_length, json_type = struct.unpack_from("<IIIII", data)
    assert magic == 0x46546C67 and version == 2 and length == len(data) and json_type == 0x4E4F534A
    document = json.loads(data[20:20 + json_length])
    binary_length, binary_type = struct.unpack_from("<II", data, 20 + json_length)
    assert binary_type == 0x004E4942
    binary = data[28 + json_length:28 + json_length + binary_length]
    return document, binary


def positions(document, binary, accessor_index):
    accessor = document["accessors"][accessor_index]
    assert accessor["componentType"] == 5126 and accessor["type"] == "VEC3"
    view = document["bufferViews"][accessor["bufferView"]]
    offset = view.get("byteOffset", 0) + accessor.get("byteOffset", 0)
    stride = view.get("byteStride", 12)
    return [struct.unpack_from("<fff", binary, offset + i * stride) for i in range(accessor["count"])]


class AuthoringTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.directory = tempfile.TemporaryDirectory(prefix="bd_integration_")
        cls.registered_here = not hasattr(bpy.types.Scene, "bd_authoring")
        if cls.registered_here:
            register()

    @classmethod
    def tearDownClass(cls):
        if cls.registered_here:
            unregister()
        cls.directory.cleanup()

    def test_analytic_mass_properties_and_finite_contact_area(self):
        a = cube("analytic_a", (10, 0, 0), (12, 3, 4))
        b = cube("analytic_b", (12, 1, 1), (13, 2, 3))
        c = cube("point_contact", (12, 3, 4), (13, 4, 5))
        metrics = mesh_metrics(a)
        self.assertAlmostEqual(metrics["volume"], 24, places=5)
        for actual, expected in zip(metrics["center"], (11, 1.5, 2)):
            self.assertAlmostEqual(actual, expected, places=5)
        for i, expected in enumerate((50, 40, 26)):
            self.assertAlmostEqual(metrics["inertia"][i][i], expected, places=4)
        chunks = [Chunk(label, label, "wall", "concrete", obj, obj, 1, (0, 0, 0), [])
                  for label, obj in (("a", a), ("b", b), ("c", c))]
        bonds = generate_bonds(chunks)
        self.assertEqual(len(bonds), 1)
        self.assertAlmostEqual(bonds[0]["area"], 2, places=6)
        self.assertEqual(bonds[0]["normal"], [1, 0, 0])
        self.assertEqual(bonds[0]["center"], [12, 1.5, 2])
        self.assertAlmostEqual(bonds[0]["area_covariance"][1][1], 1 / 6, places=6)
        self.assertAlmostEqual(bonds[0]["area_covariance"][2][2], 2 / 3, places=6)

    def test_line_slivers_do_not_create_bonds_but_resolved_narrow_faces_do(self):
        a = cube("sliver_a", (0, 0, 0), (1, 1, 1))
        for width, expected in ((0.00001, 0), (0.0001, 1)):
            with self.subTest(width=width):
                b = cube(f"sliver_b_{width}", (1, 1 - width, 0), (2, 2 - width, 1))
                chunks = [Chunk(label, label, "wall", "concrete", obj, obj, 1, (0, 0, 0), [])
                          for label, obj in (("a", a), ("b", b))]
                bonds = generate_bonds(chunks, tolerance=0.00002)
                self.assertEqual(len(bonds), expected)
                if expected:
                    self.assertAlmostEqual(bonds[0]["area"], width, delta=2e-7)

    def test_rectangle_openings_survive_fracture_and_collision_export(self):
        previous_scale = bpy.context.scene.unit_settings.scale_length
        bpy.context.scene.unit_settings.scale_length = 0.01
        try:
            result = build_asset(BuildingSpec.from_dict(rectangle()), output_dir=self.directory.name, preview=False)
        finally:
            bpy.context.scene.unit_settings.scale_length = previous_scale
        collection = bpy.data.collections[result["collection_name"]]
        manifest = json.loads(Path(result["manifest_path"]).read_text(encoding="utf-8"))
        self.assertGreater(result["chunk_count"], 8)
        self.assertGreater(result["bond_count"], result["chunk_count"] - 1)
        self.assertGreater(result["anchor_count"], 0)
        for role in ("chunk", "collider"):
            objects = [o for o in collection.objects if o.get("bd_role") == role]
            self.assertFalse(objects_hit(objects, (1.5, -1, 1.2), (1.5, 0.5, 1.2)), role)
            self.assertFalse(objects_hit(objects, (4, -1, 1.7), (4, 0.5, 1.7)), role)
            self.assertTrue(objects_hit(objects, (2.8, -1, 1.2), (2.8, 0.5, 1.2)), role)
        self.assertEqual(manifest["mesh_sha256"], hashlib.sha256(Path(result["glb_path"]).read_bytes()).hexdigest())
        document, binary = read_glb(result["glb_path"])
        self.assertTrue(all(material.get("extras", {}).get("bd_surface_role") in {"exterior", "fracture"}
                            for material in document["materials"]))
        self.assertEqual({material["extras"]["bd_surface_role"] for material in document["materials"]},
                         {"exterior", "fracture"})
        nodes = {node["extras"]["bd_chunk_id"]: node for node in document["nodes"]
                 if "bd_chunk_id" in node.get("extras", {})}
        self.assertEqual(set(nodes), {chunk["id"] for chunk in manifest["chunks"]})
        for chunk in manifest["chunks"]:
            node = nodes[chunk["id"]]
            self.assertEqual(node["name"], chunk["id"])
            for actual, expected in zip(node.get("translation", [0, 0, 0]), chunk["position"]):
                self.assertAlmostEqual(actual, expected, places=5)
            primitives = document["meshes"][node["mesh"]]["primitives"]
            visual = [p for primitive in primitives for p in positions(document, binary, primitive["attributes"]["POSITION"])]
            hull = chunk["colliders"][0]["vertices"]
            for axis in range(3):
                self.assertAlmostEqual(min(p[axis] for p in visual), min(p[axis] for p in hull), places=4)
                self.assertAlmostEqual(max(p[axis] for p in visual), max(p[axis] for p in hull), places=4)
            self.assertGreater(chunk["mass"], 0)
            self.assertTrue(all(chunk["inertia"][i][i] > 0 for i in range(3)))

    def test_multilevel_l_shape_and_courtyard(self):
        cases = [rectangle("l_shape", footprint=[[0, 0], [7, 0], [7, 3], [3, 3], [3, 6], [0, 6]],
                           levels=2, fracture_size=3, openings=[]),
                 rectangle("courtyard", footprint=[[0, 0], [8, 0], [8, 7], [0, 7]],
                           holes=[[[3, 2], [5, 2], [5, 5], [3, 5]]], fracture_size=3, openings=[]),
                 rectangle("interior", inner_walls=[{"start": [0.2, 2], "end": [5.8, 2]}],
                           fracture_size=3, openings=[])]
        for data in cases:
            with self.subTest(name=data["name"]):
                result = build_asset(BuildingSpec.from_dict(data), preview=False)
                self.assertGreater(result["bond_count"], 0)
                collection = bpy.data.collections[result["collection_name"]]
                objects = [o for o in collection.objects if o.get("bd_role") == "collider"]
                if data["name"] == "courtyard":
                    self.assertFalse(objects_hit(objects, (4, 3.5, -1), (4, 3.5, 5)))
                if data["name"] == "l_shape":
                    self.assertFalse(objects_hit(objects, (5, 5, -1), (5, 5, 8)))

    def test_rebuild_is_deterministic_and_preserves_user_objects(self):
        sentinel = cube("user_sentinel", (100, 0, 0), (101, 1, 1))
        sentinel_data = [tuple(vertex.co) for vertex in sentinel.data.vertices]
        spec = BuildingSpec.from_dict(rectangle("repeat", fracture_size=3, openings=[]))
        first = build_asset(spec, preview=False)
        count = len(bpy.data.objects)
        second = build_asset(spec, preview=False)
        self.assertEqual(first["asset_id"], second["asset_id"])
        self.assertEqual(first["chunk_count"], second["chunk_count"])
        self.assertEqual(len(bpy.data.objects), count)
        self.assertEqual([tuple(v.co) for v in sentinel.data.vertices], sentinel_data)
        collection = bpy.data.collections[second["collection_name"]]
        self.assertEqual(collection["bd_owner"], OWNER)
        obj = next(o for o in collection.objects if o.get("bd_role") == "chunk")
        material = obj.data.materials[0]
        role = material["bd_surface_role"]
        material["bd_surface_role"] = "fracture" if role == "exterior" else "exterior"
        with self.assertRaisesRegex(ValueError, "edited"):
            export_asset(collection, self.directory.name)
        material["bd_surface_role"] = role
        obj.data.vertices[0].co.x += 0.1
        with self.assertRaisesRegex(ValueError, "edited"):
            export_asset(collection, self.directory.name)

    def test_rebuild_preserves_user_children_and_rejects_duplicate_ids(self):
        spec = BuildingSpec.from_dict(rectangle("preserve_children", fracture_size=3, openings=[]))
        result = build_asset(spec, preview=False)
        collection = bpy.data.collections[result["collection_name"]]
        child = bpy.data.collections.new("user_child_collection")
        collection.children.link(child)
        user_object = bpy.data.objects.new("user_nested_empty", None)
        child.objects.link(user_object)
        chunk = next(o for o in collection.objects if o.get("bd_role") == "chunk")
        duplicate = chunk.copy()
        collection.objects.link(duplicate)
        with self.assertRaisesRegex(ValueError, "changed"):
            export_asset(collection, self.directory.name)
        bpy.data.objects.remove(duplicate, do_unlink=True)
        bpy.context.scene.collection.objects.link(chunk)
        build_asset(spec, preview=False)
        self.assertIsNotNone(bpy.context.scene.objects.get(user_object.name))
        self.assertIsNotNone(bpy.context.scene.objects.get(chunk.name))

    def test_failed_export_preserves_previous_scene_and_published_asset(self):
        spec = BuildingSpec.from_dict(rectangle("transaction", fracture_size=3, openings=[]))
        first = build_asset(spec, output_dir=self.directory.name, preview=False)
        json_path = Path(first["manifest_path"])
        published = json_path.read_bytes()
        original_replace = Path.replace

        def fail_manifest_replace(path, target):
            if path.name.endswith(".destruction.json"):
                raise OSError("simulated manifest publish failure")
            return original_replace(path, target)

        changed = rectangle("transaction", seed=18, fracture_size=3, openings=[])
        with patch.object(Path, "replace", fail_manifest_replace):
            with self.assertRaisesRegex(OSError, "simulated"):
                build_asset(BuildingSpec.from_dict(changed), output_dir=self.directory.name, preview=False)
        self.assertEqual(json_path.read_bytes(), published)
        old_manifest = json.loads(published)
        self.assertEqual(hashlib.sha256((json_path.parent / old_manifest["mesh_file"]).read_bytes()).hexdigest(),
                         old_manifest["mesh_sha256"])
        retained = json.loads(bpy.data.collections[first["collection_name"]]["bd_manifest_json"])
        self.assertEqual(retained["asset_id"], first["asset_id"])

    def test_ui_reads_source_without_mutating_and_rejects_invalid_input(self):
        mesh = bpy.data.meshes.new("outline_source")
        mesh.from_pydata([(0, 0, 0), (6, 0, 0), (6, 4, 0), (0, 4, 0)], [], [(0, 1, 2, 3)])
        source = bpy.data.objects.new("outline_source", mesh)
        bpy.context.scene.collection.objects.link(source)
        original = [tuple(v.co) for v in mesh.vertices]
        footprint, holes = outline_from_object(source, bpy.context.evaluated_depsgraph_get())
        self.assertEqual(len(footprint), 4)
        self.assertEqual(holes, [])
        self.assertEqual(original, [tuple(v.co) for v in mesh.vertices])
        with self.assertRaises(ValueError):
            BuildingSpec.from_dict(rectangle(footprint=[[0, 0], [4, 4], [0, 4], [4, 0]]))
        with self.assertRaises(ValueError):
            BuildingSpec.from_dict(rectangle(openings=[{"wall": 0, "level": 0, "offset": 0,
                                                       "width": 99, "height": 2, "sill": 0, "kind": "door"}]))

    def test_curve_ui_generate_and_export_operators(self):
        curve = bpy.data.curves.new("ui_curve", "CURVE")
        curve.dimensions = "2D"
        spline = curve.splines.new("POLY")
        spline.points.add(3)
        for point, xy in zip(spline.points, ((0, 0), (6, 0), (6, 4), (0, 4))):
            point.co = (*xy, 0, 1)
        spline.use_cyclic_u = True
        source = bpy.data.objects.new("ui_curve", curve)
        bpy.context.scene.collection.objects.link(source)
        settings = bpy.context.scene.bd_authoring
        settings.source_mode = "OUTLINE"
        settings.outline_object = source
        settings.imported_spec_json = ""
        settings.name = "ui_curve_asset"
        settings.levels = 1
        settings.fracture_size = 3
        settings.auto_openings = True
        original = [tuple(p.co) for p in spline.points]
        self.assertEqual(bpy.ops.bd.generate(), {"FINISHED"})
        self.assertFalse(settings.has_error)
        generated = json.loads(settings.last_spec_json)
        self.assertTrue(any(o["kind"] == "door" for o in generated["openings"]))
        self.assertTrue(any(o["kind"] == "window" for o in generated["openings"]))
        self.assertEqual(original, [tuple(p.co) for p in spline.points])
        settings.export_directory = self.directory.name
        self.assertEqual(bpy.ops.bd.export_asset(), {"FINISHED"})
        self.assertTrue((Path(self.directory.name) / "ui_curve_asset.destruction.json").exists())

    def test_export_does_not_include_selected_objects_in_other_scenes(self):
        original_scene = bpy.context.scene
        outside = cube("other_scene_selected", (30, 0, 0), (31, 1, 1))
        outside.select_set(True)
        scene = bpy.data.scenes.new("isolated_export_scene")
        bpy.context.window.scene = scene
        try:
            result = build_asset(BuildingSpec.from_dict(rectangle("isolated", fracture_size=3, openings=[])),
                                 output_dir=self.directory.name, preview=False)
            document, _ = read_glb(result["glb_path"])
            mesh_nodes = [node for node in document["nodes"] if "mesh" in node]
            self.assertEqual(len(mesh_nodes), result["chunk_count"])
            self.assertTrue(all("bd_chunk_id" in node.get("extras", {}) for node in mesh_nodes))
            self.assertIsNotNone(original_scene.objects.get(outside.name))
        finally:
            bpy.context.window.scene = original_scene
            bpy.data.scenes.remove(scene)


if __name__ == "__main__":
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(AuthoringTests)
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    if not result.wasSuccessful():
        raise RuntimeError("Blender asset integration tests failed")
