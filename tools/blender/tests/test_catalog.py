"""Verify explicit bridge parts and the baked high-rise/bridge catalog."""

import copy
import hashlib
import json
import math
from pathlib import Path
import sys
import unittest

import bpy

TOOL_ROOT = Path(__file__).resolve().parents[1]
ROOT = TOOL_ROOT.parents[1]
sys.path.insert(0, str(TOOL_ROOT))
sys.path.insert(0, str(Path(__file__).resolve().parent))

from bevy_destruction.geometry import generate_building
from bevy_destruction.spec import BuildingSpec, load_spec
from test_geometry import GeometryTests, ray_hits


class CatalogTests(unittest.TestCase):
    def test_specs_without_explicit_parts_keep_their_normalized_content(self):
        for name in ("rectangle", "l_shape", "courtyard"):
            with self.subTest(asset=name):
                spec = load_spec(TOOL_ROOT / "examples" / (name + ".json"))
                self.assertNotIn("parts", spec.to_dict())
                manifest = json.loads(
                    (
                        ROOT / "assets" / "buildings" / (name + ".destruction.json")
                    ).read_text()
                )
                self.assertEqual(
                    json.loads(json.dumps(spec.to_dict())), manifest["specification"]
                )

    def test_explicit_parts_reject_overlap_invalid_material_and_concavity(self):
        base = json.loads((TOOL_ROOT / "examples" / "bridge.json").read_text())
        cases = []
        overlap = copy.deepcopy(base)
        overlap["parts"].append({**overlap["parts"][0], "id": "duplicate-volume"})
        cases.append(overlap)
        material = copy.deepcopy(base)
        material["parts"][0]["material"] = "missing"
        cases.append(material)
        concave = copy.deepcopy(base)
        concave["parts"][0]["polygon"] = [[0, 0], [6, 0], [6, 4], [3, 2], [0, 4]]
        cases.append(concave)
        identifiers = copy.deepcopy(base)
        identifiers["parts"][1]["id"] = identifiers["parts"][0]["id"]
        cases.append(identifiers)
        mixed = copy.deepcopy(base)
        mixed["openings"] = [{"wall": 0, "offset": 2, "width": 1, "height": 1}]
        cases.append(mixed)
        for data in cases:
            with self.subTest(parts=data["parts"]), self.assertRaises(ValueError):
                BuildingSpec.from_dict(data)

    def test_bridge_has_discrete_piers_real_air_spans_and_expected_volume(self):
        spec = load_spec(TOOL_ROOT / "examples" / "bridge.json")
        helper = GeometryTests()
        helper.setUp()
        try:
            parts = helper.build(spec)
            expected = math.fsum(
                abs(
                    sum(
                        a[0] * b[1] - b[0] * a[1]
                        for a, b in zip(
                            p["polygon"], p["polygon"][1:] + p["polygon"][:1]
                        )
                    )
                )
                * 0.5
                * (p["top"] - p["bottom"])
                for p in spec.parts
            )
            self.assertAlmostEqual(helper.check_parts(parts), expected, delta=1e-3)
            self.assertEqual(sum(p.kind == "pier" for p in parts), 6)
            self.assertEqual(sum(p.kind == "beam" for p in parts), 2)
            self.assertFalse(ray_hits(parts, (7, -1, 2), (7, 7, 2)))
            self.assertFalse(ray_hits(parts, (21, -1, 2), (21, 7, 2)))
            self.assertTrue(ray_hits(parts, (7, 3, 10), (7, 3, 0)))
            self.assertTrue(ray_hits(parts, (1.5, -1, 2), (1.5, 2, 2)))
            self.assertEqual(
                BuildingSpec.from_dict(spec.to_dict()).to_dict(), spec.to_dict()
            )
        finally:
            helper.tearDown()

    def test_catalog_bakes_have_connected_anchors_finite_mass_and_checked_meshes(self):
        for name in (
            "rectangle",
            "l_shape",
            "courtyard",
            "highrise_slab",
            "highrise_l",
            "bridge",
        ):
            with self.subTest(asset=name):
                manifest = json.loads(
                    (
                        ROOT / "assets" / "buildings" / (name + ".destruction.json")
                    ).read_text()
                )
                self.assertEqual(manifest["schema_version"], 2)
                self.assertEqual(
                    BuildingSpec.from_dict(manifest["specification"]).to_dict(),
                    load_spec(TOOL_ROOT / "examples" / (name + ".json")).to_dict(),
                )
                chunks = {c["id"]: c for c in manifest["chunks"]}
                self.assertGreater(len(chunks), 20)
                self.assertLessEqual(len(chunks), 1000)
                self.assertEqual(len(chunks), len(manifest["chunks"]))
                self.assertTrue(
                    all(
                        c["mass"] > 0 and math.isfinite(c["mass"])
                        for c in chunks.values()
                    )
                )
                self.assertTrue(
                    all(
                        all(c["inertia"][i][i] > 0 for i in range(3))
                        for c in chunks.values()
                    )
                )
                neighbors = {identifier: set() for identifier in chunks}
                for bond in manifest["bonds"]:
                    self.assertGreater(bond["area"], 0)
                    self.assertTrue(bond["patches"])
                    neighbors[bond["a"]].add(bond["b"])
                    neighbors[bond["b"]].add(bond["a"])
                anchors = {a["chunk"] for a in manifest["anchors"]}
                self.assertTrue(anchors)
                visited = set(anchors)
                pending = list(anchors)
                while pending:
                    for other in neighbors[pending.pop()]:
                        if other not in visited:
                            visited.add(other)
                            pending.append(other)
                self.assertEqual(visited, set(chunks))
                points = [
                    c["position"][1] + v[1]
                    for c in chunks.values()
                    for proxy in c["colliders"]
                    for v in proxy["vertices"]
                ]
                self.assertAlmostEqual(min(points), 0, delta=2e-5)
                self.assertTrue(
                    all(1 <= len(c["colliders"]) <= 48 for c in chunks.values())
                )
                if name.startswith("highrise"):
                    self.assertGreater(max(points), 26)
                elif name == "bridge":
                    self.assertAlmostEqual(max(points), 7.45, delta=2e-5)
                    self.assertEqual(
                        {chunks[identifier]["kind"] for identifier in anchors}, {"pier"}
                    )
                    self.assertEqual(
                        len({chunks[identifier]["part_id"] for identifier in anchors}),
                        6,
                    )
                mesh = ROOT / "assets" / "buildings" / manifest["mesh_file"]
                self.assertEqual(
                    hashlib.sha256(mesh.read_bytes()).hexdigest(),
                    manifest["mesh_sha256"],
                )


if __name__ == "__main__":
    result = unittest.TextTestRunner(verbosity=2).run(
        unittest.defaultTestLoader.loadTestsFromTestCase(CatalogTests)
    )
    if not result.wasSuccessful():
        raise RuntimeError("Physics catalog verification failed")
