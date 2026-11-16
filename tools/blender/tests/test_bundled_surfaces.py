"""Verify baked demo GLB surface areas against source geometry and contacts."""

import hashlib
import json
import math
from pathlib import Path
import struct
import sys
import unittest

import bpy

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from bevy_destruction.geometry import generate_building
from bevy_destruction.spec import BuildingSpec
from bevy_destruction.surfaces import _part_contacts


ROOT = Path(__file__).resolve().parents[3]


def _accessor(document, binary, index):
    accessor = document["accessors"][index]
    view = document["bufferViews"][accessor["bufferView"]]
    offset = view.get("byteOffset", 0) + accessor.get("byteOffset", 0)
    component = {5126: "f", 5123: "H", 5125: "I", 5121: "B"}[accessor["componentType"]]
    count = {"VEC3": 3, "SCALAR": 1}[accessor["type"]]
    layout = "<" + component * count
    stride = view.get("byteStride", struct.calcsize(layout))
    return [struct.unpack_from(layout, binary, offset + i * stride) for i in range(accessor["count"])]


def _triangle_area(a, b, c):
    u = [b[i] - a[i] for i in range(3)]
    v = [c[i] - a[i] for i in range(3)]
    cross = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]]
    return math.sqrt(math.fsum(x * x for x in cross)) * 0.5


class BundledSurfaceTests(unittest.TestCase):
    def verify_asset(self, name):
        spec = BuildingSpec.from_dict(json.loads((ROOT / "tools/blender/examples" / (name + ".json")).read_text()))
        collection = bpy.data.collections.new("verify_" + name)
        bpy.context.scene.collection.children.link(collection)
        try:
            parts = generate_building(spec, collection)
            source_area = math.fsum(face.area for part in parts for face in part.obj.data.polygons)
            patches = _part_contacts(parts, 2e-5)
            buried_area = math.fsum(
                math.fsum((a - polygon[0]).cross(b - polygon[0]).dot(normal)
                          for a, b in zip(polygon, polygon[1:] + polygon[:1])) * 0.5
                for contacts in patches.values() for normal, polygon in contacts)
            directory = ROOT / "assets/buildings"
            manifest = json.loads((directory / (name + ".destruction.json")).read_text())
            data = (directory / manifest["mesh_file"]).read_bytes()
            self.assertEqual(hashlib.sha256(data).hexdigest(), manifest["mesh_sha256"])
            json_length = struct.unpack_from("<I", data, 12)[0]
            document = json.loads(data[20:20 + json_length])
            binary = data[28 + json_length:]
            triangles = []
            for mesh in document["meshes"]:
                for primitive in mesh["primitives"]:
                    role = document["materials"][primitive["material"]]["extras"]["bd_surface_role"]
                    if role != "exterior":
                        continue
                    points = _accessor(document, binary, primitive["attributes"]["POSITION"])
                    indices = [value[0] for value in _accessor(document, binary, primitive["indices"])]
                    triangles.extend(_triangle_area(*(points[indices[j + k]] for k in range(3)))
                                     for j in range(0, len(indices), 3))
            actual = math.fsum(triangles)
            self.assertAlmostEqual(actual, source_area - buried_area, delta=source_area * 1e-6)
        finally:
            for obj in list(collection.objects):
                mesh = obj.data
                bpy.data.objects.remove(obj, do_unlink=True)
                bpy.data.meshes.remove(mesh)
            bpy.data.collections.remove(collection)

    def test_rectangle(self):
        self.verify_asset("rectangle")

    def test_l_shape(self):
        self.verify_asset("l_shape")

    def test_courtyard(self):
        self.verify_asset("courtyard")


if __name__ == "__main__":
    result = unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(BundledSurfaceTests))
    if not result.wasSuccessful():
        raise RuntimeError("Bundled surface area verification failed")
