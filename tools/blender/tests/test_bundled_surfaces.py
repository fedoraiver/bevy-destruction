"""Verify original-source GLB geometry and conservative whole-face material roles."""

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
    return [
        struct.unpack_from(layout, binary, offset + i * stride)
        for i in range(accessor["count"])
    ]


def _triangle_area(a, b, c):
    u = [b[i] - a[i] for i in range(3)]
    v = [c[i] - a[i] for i in range(3)]
    cross = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ]
    return math.sqrt(math.fsum(x * x for x in cross)) * 0.5


class BundledSurfaceTests(unittest.TestCase):
    def verify_asset(self, name):
        spec = BuildingSpec.from_dict(
            json.loads((ROOT / "tools/blender/examples" / (name + ".json")).read_text())
        )
        collection = bpy.data.collections.new("verify_" + name)
        bpy.context.scene.collection.children.link(collection)
        try:
            parts = generate_building(spec, collection)
            source_area = math.fsum(
                face.area for part in parts for face in part.obj.data.polygons
            )
            patches = _part_contacts(parts, 2e-5)
            buried_area = math.fsum(
                math.fsum(
                    (a - polygon[0]).cross(b - polygon[0]).dot(normal)
                    for a, b in zip(polygon, polygon[1:] + polygon[:1])
                )
                * 0.5
                for contacts in patches.values()
                for normal, polygon in contacts
            )
            directory = ROOT / "assets/buildings"
            manifest = json.loads(
                (directory / (name + ".destruction.json")).read_text()
            )
            data = (directory / manifest["mesh_file"]).read_bytes()
            self.assertEqual(hashlib.sha256(data).hexdigest(), manifest["mesh_sha256"])
            json_length = struct.unpack_from("<I", data, 12)[0]
            document = json.loads(data[20 : 20 + json_length])
            binary = data[28 + json_length :]
            self.assertEqual(
                manifest["generator"]["surface_policy"],
                "source_mesh_with_contact_overlays",
            )
            self.assertEqual(
                manifest["surface_classification_audit"]["geometry_preserved_chunks"],
                len(manifest["chunks"]),
            )
            contacts = {chunk["id"]: [] for chunk in manifest["chunks"]}
            for bond in manifest["bonds"]:
                contacts[bond["a"]].extend(bond["patches"])
                contacts[bond["b"]].extend(bond["patches"])
            areas = {"exterior": [], "fracture": []}

            def covered(points, patch):
                polygon = patch["vertices"]
                normal = patch["normal"]
                origin = polygon[0]
                dot = lambda a, b: math.fsum(x * y for x, y in zip(a, b))
                sub = lambda a, b: tuple(x - y for x, y in zip(a, b))
                cross = lambda a, b: (
                    a[1] * b[2] - a[2] * b[1],
                    a[2] * b[0] - a[0] * b[2],
                    a[0] * b[1] - a[1] * b[0],
                )
                if any(abs(dot(normal, sub(point, origin))) > 4e-5 for point in points):
                    return False
                for a, b in zip(polygon, polygon[1:] + polygon[:1]):
                    edge = sub(b, a)
                    limit = 4e-5 * math.sqrt(dot(edge, edge))
                    if any(
                        dot(normal, cross(edge, sub(point, a))) < -limit
                        for point in points
                    ):
                        return False
                return True

            for node in document["nodes"]:
                identifier = node.get("extras", {}).get("bd_chunk_id")
                if identifier is None:
                    continue
                translation = node.get("translation", [0, 0, 0])
                for primitive in document["meshes"][node["mesh"]]["primitives"]:
                    role = document["materials"][primitive["material"]]["extras"][
                        "bd_surface_role"
                    ]
                    points = _accessor(
                        document, binary, primitive["attributes"]["POSITION"]
                    )
                    indices = [
                        value[0]
                        for value in _accessor(document, binary, primitive["indices"])
                    ]
                    for offset in range(0, len(indices), 3):
                        triangle = [points[indices[offset + k]] for k in range(3)]
                        areas[role].append(_triangle_area(*triangle))
                        if role == "fracture":
                            world = [
                                tuple(
                                    point[axis] + translation[axis] for axis in range(3)
                                )
                                for point in triangle
                            ]
                            self.assertTrue(
                                any(
                                    covered(world, patch)
                                    for patch in contacts[identifier]
                                ),
                                f"{name}/{identifier}: whole fracture triangle exceeds its contact patch",
                            )
            total_area = math.fsum(value for role in areas.values() for value in role)
            actual_exterior = total_area - 2 * math.fsum(
                bond["area"] for bond in manifest["bonds"]
            )
            self.assertAlmostEqual(
                actual_exterior, source_area - buried_area, delta=source_area * 1e-6
            )
            # Partial contacts deliberately remain exterior in the base GLB;
            # runtime masks cover them without changing this closed source mesh.
            self.assertLessEqual(
                math.fsum(areas["fracture"]),
                2 * math.fsum(bond["area"] for bond in manifest["bonds"])
                + source_area * 1e-6,
            )
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

    def test_highrise_slab(self):
        self.verify_asset("highrise_slab")

    def test_highrise_l(self):
        self.verify_asset("highrise_l")

    def test_bridge(self):
        self.verify_asset("bridge")


if __name__ == "__main__":
    result = unittest.TextTestRunner(verbosity=2).run(
        unittest.defaultTestLoader.loadTestsFromTestCase(BundledSurfaceTests)
    )
    if not result.wasSuccessful():
        raise RuntimeError("Bundled surface area verification failed")
