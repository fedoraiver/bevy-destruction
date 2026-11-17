"""Run in factory-startup background Blender; no production assets are changed."""

import hashlib
import json
from pathlib import Path
import sys
import unittest

import bpy

sys.path.insert(0, str(Path(__file__).resolve().parent))
import curved_partition as prototype


def signature(groups):
    return hashlib.sha256(
        json.dumps(groups, sort_keys=True, separators=(",", ":")).encode()
    ).hexdigest()


class CurvedPartitionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if not bpy.app.background:
            raise RuntimeError("Curved partition tests require background Blender")
        prototype.select_configuration(prototype.SEED, 1.4, 0.1)
        cls.sites, cls.field = prototype.SITES, prototype.FIELD_SEED
        cls.configuration = dict(prototype.CURRENT_CONFIG)

    def setUp(self):
        if not bpy.app.background:
            raise RuntimeError("Curved partition tests require background Blender")
        prototype.SITES, prototype.FIELD_SEED = self.sites, self.field
        prototype.CURRENT_CONFIG = dict(self.configuration)
        prototype.OUT.mkdir(parents=True, exist_ok=True)

    def tearDown(self):
        prototype.SITES, prototype.FIELD_SEED = self.sites, self.field

    def test_same_seed_repeats_and_changed_seed_changes_partition(self):
        original = signature(prototype.partition(0.2, True)[0])
        self.assertEqual(original, signature(prototype.partition(0.2, True)[0]))
        prototype.SITES, prototype.FIELD_SEED = prototype.seeded_values(
            prototype.SEED + 1
        )
        self.assertNotEqual(original, signature(prototype.partition(0.2, True)[0]))

    def test_boundary_only_mesh_preserves_area_and_reduces_geometry(self):
        original, _, _ = prototype.partition(0.1, True)
        simplified = prototype.retriangulate_boundaries(original)
        self.assertEqual(set(original), set(simplified))
        for label, polygons in original.items():
            self.assertAlmostEqual(
                sum(prototype.area(p) for p in polygons),
                sum(prototype.area(p) for p in simplified[label]),
                places=7,
            )
        self.assertLess(
            sum(len(p) for p in simplified.values()),
            sum(len(p) for p in original.values()) * 0.45,
        )

    def test_curved_closed_chunks_exact_compounds_and_window_clearance(self):
        report, _ = prototype.build_variant(0.2, True)
        self.assertEqual(report["errors"], [])
        self.assertLess(report["volume_relative_error"], 1e-6)
        self.assertLess(report["source_inertia_relative_error"], 1e-6)
        self.assertLess(report["max_proxy_overlap_window_area_m2"], 1e-9)
        self.assertEqual(len(report["window_clearance_rays"]), 9)
        self.assertTrue(
            all(not ray["hit_proxy"] for ray in report["window_clearance_rays"])
        )
        self.assertGreater(
            max(i["max_distance_from_average_plane_m"] for i in report["interfaces"]),
            0.005,
        )
        minimum, maximum = (
            factor * prototype.TARGET_SIZE**2 for factor in prototype.AREA_FACTORS
        )
        for quality in report["size_normalization"]["qualities"].values():
            self.assertGreaterEqual(quality["area_m2"], minimum)
            self.assertLessEqual(quality["area_m2"], maximum)
            self.assertLessEqual(quality["aabb_diagonal_m"], 3 * prototype.TARGET_SIZE)
            self.assertLessEqual(quality["aabb_aspect_ratio"], prototype.MAX_ASPECT)
            self.assertGreaterEqual(
                quality["compactness_4pi_area_over_perimeter2"],
                prototype.MIN_COMPACTNESS,
            )

    def test_tiny_component_transfers_all_area_to_a_real_face_neighbor(self):
        groups = {
            0: [[(0, 0), (0.1, 0), (0.1, 0.1), (0, 0.1)]],
            1: [[(0.1, 0), (1, 0), (1, 1), (0.1, 1), (0.1, 0.1)]],
        }
        normalized, log = prototype.normalize_chunk_sizes(groups)
        self.assertEqual(set(normalized), {1})
        self.assertEqual(len(log["merges"]), 1)
        self.assertEqual(log["merges"][0]["from_component"], 0)
        self.assertEqual(log["merges"][0]["to_component"], 1)
        self.assertAlmostEqual(log["merges"][0]["transferred_area_m2"], 0.01)
        self.assertAlmostEqual(
            log["area_before_m2"],
            log["area_after_m2"],
            places=12,
        )

    def test_tiny_component_cannot_jump_across_a_gap(self):
        groups = {
            0: [[(0, 0), (0.1, 0), (0.1, 0.1), (0, 0.1)]],
            1: [[(2, 0), (3, 0), (3, 1), (2, 1)]],
        }
        with self.assertRaisesRegex(ValueError, "no face-adjacent merge"):
            prototype.normalize_chunk_sizes(groups)

    def test_random_sites_are_well_spaced_and_stay_outside_the_window(self):
        import math

        sites = prototype.SITES
        self.assertEqual(len(sites), round(22.08 / prototype.TARGET_SIZE**2))
        self.assertGreaterEqual(
            min(math.dist(a, b) for i, a in enumerate(sites) for b in sites[:i]),
            0.65 * prototype.TARGET_SIZE,
        )
        self.assertTrue(all(not (2.4 < x < 3.6 and 1.2 < z < 2.8) for x, z in sites))

    def test_repeated_render_selects_original_chunks_without_display_copies(self):
        mesh = bpy.data.meshes.new("render-selector-test")
        source = bpy.data.objects.new("render-selector-test-chunk-0", mesh)
        source["variant"] = "render-selector-test"
        source["prototype_chunk"] = 0
        displayed = source.copy()
        try:
            self.assertEqual(
                prototype.original_chunk_objects("render-selector-test"), [source]
            )
        finally:
            bpy.data.objects.remove(displayed, do_unlink=True)
            bpy.data.objects.remove(source, do_unlink=True)
            bpy.data.meshes.remove(mesh)


if __name__ == "__main__":
    if not bpy.app.background:
        raise RuntimeError("Use --background --factory-startup")
    unittest.main(argv=[__file__], verbosity=2)
