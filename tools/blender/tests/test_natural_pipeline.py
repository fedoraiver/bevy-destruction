"""Production natural fracture/v2 checks; run in isolated background Blender."""

import json
import math
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

import bpy

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
sys.path.insert(0, str(Path(__file__).resolve().parent))
from bevy_destruction.geometry import generate_building
from bevy_destruction.natural import fracture_parts
from bevy_destruction.fracture import mesh_metrics, validate_convex
from bevy_destruction.pipeline import (
    build_asset,
    export_asset,
    _patch_metrics,
    _aggregate_patches,
    _manifest,
    _finalize_surface_audit,
)
from bevy_destruction.spec import BuildingSpec
from bevy_destruction.bonds import _clip, merge_coplanar_polygons, generate_bonds
from bevy_destruction.bonds import _face_contacts
from mathutils import Vector
from bevy_destruction.records import Chunk
from bevy_destruction.surfaces import classify_bond_surfaces, _material
import bevy_destruction.natural as natural
from bevy_destruction.contact_export import Footprint, clip_polygon, correct_contacts
from test_asset_tool import rectangle, objects_hit, cube


class NaturalPipelineTests(unittest.TestCase):
    def test_small_contact_plane_check_uses_overlap_not_remote_vertices(self):
        a = [
            (
                Vector((0, 1e-4, 1)).normalized(),
                [Vector(p) for p in ((0, 0, 0), (1e-4, 0, 0), (0, 1e-4, 0))],
            )
        ]
        b = [
            (
                Vector((0, 0, -1)),
                [Vector(p) for p in ((-1, -1, 0), (0, 1, 0), (1, -1, 0))],
            )
        ]
        contacts = list(_face_contacts(a, b, 2e-5))
        self.assertEqual(len(contacts), 1)
        self.assertGreater(abs(natural._area(contacts[0][-1])), 4.9e-9)
        separated = [
            (normal, [point + Vector((0, 0, 5e-5)) for point in points])
            for normal, points in b
        ]
        self.assertEqual(list(_face_contacts(a, separated, 2e-5)), [])

    def test_export_contact_clip_uses_actual_glb_and_rejects_large_repairs(self):
        import copy

        fixture = json.loads(
            (Path(__file__).parent / "fixtures/glb-contact-rounding.json").read_text()
        )
        manifest = {
            "bonds": [
                {"id": "rounded", "a": "a", "b": "b", "patches": [fixture["patch"]]}
            ]
        }
        triangles = {key: fixture[key] for key in ("a", "b")}
        original = copy.deepcopy(manifest)
        audit = correct_contacts(
            manifest, triangles, _patch_metrics, _aggregate_patches
        )
        self.assertEqual(audit["corrected_patches"], 1)
        patch = manifest["bonds"][0]["patches"][0]
        self.assertGreater(patch["area"], 4.9e-7)
        self.assertLess(patch["area"], 5.0e-7)
        self.assertLessEqual(audit["maximum_vertex_adjustment_m"], 1e-6)
        self.assertEqual(len(patch["vertices"]), 3)
        plane = Footprint(patch)
        for key, orientation in (("a", 1), ("b", -1)):
            support = plane.supports(triangles[key], orientation)
            from bevy_destruction.contact_export import _area

            covered = sum(
                abs(_area(clip_polygon(plane.boundary, boundary)))
                for boundary in support
            )
            self.assertAlmostEqual(covered, patch["area"], delta=patch["area"] * 1e-6)
        unchanged_triangles = copy.deepcopy(triangles)
        # A genuinely missing interface must reject rather than shrink to fit.
        displaced = {
            "a": [
                [(x, y - 0.1, z) for x, y, z in triangle] for triangle in triangles["a"]
            ],
            "b": triangles["b"],
        }
        with self.assertRaises(ValueError):
            correct_contacts(original, displaced, _patch_metrics, _aggregate_patches)
        self.assertEqual(triangles, unchanged_triangles)
        large_patch = _patch_metrics(
            [(1e6, 0, 0), (1e6 + 1, 0, 0), (1e6 + 1, 1, 0), (1e6, 1, 0)]
        )
        large = {
            "bonds": [{"id": "large", "a": "a", "b": "b", "patches": [large_patch]}]
        }
        support_a = [
            ((1e6 + 0.0625, 0, 0), (1e6 + 1, 0, 0), (1e6 + 1, 1, 0)),
            ((1e6 + 0.0625, 0, 0), (1e6 + 1, 1, 0), (1e6 + 0.0625, 1, 0)),
        ]
        support_b = [
            ((1e6, 0, 0), (1e6 + 1, 1, 0), (1e6 + 1, 0, 0)),
            ((1e6, 0, 0), (1e6, 1, 0), (1e6 + 1, 1, 0)),
        ]
        with self.assertRaises(ValueError):
            correct_contacts(
                large,
                {"a": support_a, "b": support_b},
                _patch_metrics,
                _aggregate_patches,
            )

    def test_positive_tensor_does_not_make_subresolution_edges_load_bearing(self):
        audit = {"edge_contact_area_m2": 0.0, "maximum_edge_contact_width_m": 0.0}
        narrow = [[0, 0, 0], [0.2, 0, 0], [0.2, 2e-6, 0], [0, 2e-6, 0]]
        self.assertIsNone(
            _patch_metrics(narrow, allow_edge_contact=True, edge_audit=audit)
        )
        self.assertAlmostEqual(audit["edge_contact_area_m2"], 4e-7, places=14)
        self.assertLessEqual(audit["maximum_edge_contact_width_m"], 2e-5)
        resolved = [[0, 0, 0], [0.2, 0, 0], [0.2, 8e-5, 0], [0, 8e-5, 0]]
        self.assertGreater(_patch_metrics(resolved)["area"], 0)

    def test_whole_face_roles_preserve_geometry_and_defer_partial_contacts(self):
        material = _material("source-policy-exterior", (0.7, 0.7, 0.7, 1))
        material["bd_surface_role"] = "exterior"
        objects = [
            cube("source-policy-left", (0, 0, 0), (2, 1, 1)),
            cube("source-policy-right", (2, 0.2, 0.1), (3, 0.8, 0.9)),
        ]
        try:
            chunks = []
            before = []
            for obj in objects:
                obj.data.materials.append(material)
                metrics = mesh_metrics(obj)
                chunks.append(
                    Chunk(
                        obj.name,
                        obj.name,
                        "wall",
                        "concrete",
                        obj,
                        obj,
                        metrics["volume"],
                        metrics["center"],
                        metrics["inertia"],
                    )
                )
                before.append(
                    (
                        [tuple(v.co) for v in obj.data.vertices],
                        [tuple(face.vertices) for face in obj.data.polygons],
                    )
                )
            spec = BuildingSpec.from_dict(
                {
                    "name": "source-policy",
                    "footprint": [[0, 0], [3, 0], [3, 1], [0, 1]],
                    "levels": 1,
                }
            )
            manifest = _manifest(spec, chunks, generate_bonds(chunks, detailed=True))
            classify_bond_surfaces(chunks, manifest["bonds"])
            _finalize_surface_audit(manifest, chunks)
            self.assertEqual(
                manifest["generator"]["surface_policy"],
                "source_mesh_with_contact_overlays",
            )
            self.assertEqual(
                manifest["surface_classification_audit"]["geometry_preserved_chunks"], 2
            )
            for chunk, geometry in zip(chunks, before):
                self.assertEqual(
                    (
                        [tuple(v.co) for v in chunk.obj.data.vertices],
                        [tuple(face.vertices) for face in chunk.obj.data.polygons],
                    ),
                    geometry,
                )
                self.assertAlmostEqual(
                    mesh_metrics(chunk.obj)["volume"], chunk.volume, places=7
                )
            areas = [
                sum(
                    face.area
                    for face in obj.data.polygons
                    if obj.data.materials[face.material_index].get("bd_surface_role")
                    == "fracture"
                )
                for obj in objects
            ]
            self.assertAlmostEqual(areas[0], 0.0)
            self.assertAlmostEqual(areas[1], 0.48, places=6)
            self.assertAlmostEqual(manifest["bonds"][0]["area"], 0.48, places=6)
        finally:
            for obj in objects:
                mesh = obj.data
                bpy.data.objects.remove(obj, do_unlink=True)
                if mesh.users == 0:
                    bpy.data.meshes.remove(mesh)

    def test_floor_fracture_is_independent_of_artificial_source_diagonal(self):
        signatures = []
        panels = [
            [[[0, 0], [4, 0], [4, 3]], [[0, 0], [4, 3], [0, 3]]],
            [[[0, 0], [4, 0], [0, 3]], [[4, 0], [4, 3], [0, 3]]],
        ]
        for polygons in panels:
            spec = BuildingSpec.from_dict(
                {
                    "name": "source-diagonal",
                    "footprint": [[0, 0], [4, 0], [4, 3], [0, 3]],
                    "levels": 1,
                    "parts": [
                        {
                            "id": f"floor-00-{index:04d}",
                            "kind": "floor",
                            "material": "concrete",
                            "polygon": polygon,
                            "bottom": 0,
                            "top": 0.2,
                        }
                        for index, polygon in enumerate(polygons)
                    ],
                }
            )
            collection = bpy.data.collections.new("natural-source-diagonal")
            bpy.context.scene.collection.children.link(collection)
            try:
                chunks = fracture_parts(
                    generate_building(spec, collection),
                    seed=7,
                    target_size=1.8,
                    collection=collection,
                    spec=spec,
                )
                signatures.append(
                    [
                        (
                            chunk.id,
                            round(chunk.volume, 6),
                            tuple(round(value, 6) for value in chunk.center),
                            sorted(
                                tuple(round(value, 6) for value in vertex.co)
                                for collider in chunk.collision_objects
                                for vertex in collider.data.vertices
                            ),
                        )
                        for chunk in chunks
                    ]
                )
            finally:
                for obj in list(collection.objects):
                    mesh = obj.data
                    bpy.data.objects.remove(obj, do_unlink=True)
                    if mesh.users == 0:
                        bpy.data.meshes.remove(mesh)
                bpy.data.collections.remove(collection)
        self.assertEqual(signatures[0], signatures[1])

    def test_real_l_wall_removes_internal_depth_panel_seams(self):
        from bevy_destruction.spec import load_spec

        spec = load_spec(
            Path(__file__).resolve().parents[1] / "examples" / "l_shape.json"
        )
        collection = bpy.data.collections.new("natural-l-depth-union")
        bpy.context.scene.collection.children.link(collection)
        try:
            parts = [
                part
                for part in generate_building(spec, collection)
                if part.id.startswith("wall-inner-0-l00")
            ]
            expected = sum(mesh_metrics(part.obj)["volume"] for part in parts)
            chunks = fracture_parts(
                parts,
                seed=spec.seed,
                target_size=spec.fracture_size,
                collection=collection,
                spec=spec,
            )
            audit = json.loads(collection["bd_natural_audit"])[0]
            self.assertEqual(audit["attempt"], 0)
            self.assertAlmostEqual(
                sum(chunk.volume for chunk in chunks), expected, delta=expected * 1e-5
            )
            self.assertLessEqual(
                max(len(chunk.collision_objects) for chunk in chunks), 48
            )
            self.assertLess(audit["source_inertia_relative_error"], 1e-5)
        finally:
            for obj in list(collection.objects):
                mesh = obj.data
                bpy.data.objects.remove(obj, do_unlink=True)
                if mesh.users == 0:
                    bpy.data.meshes.remove(mesh)
            bpy.data.collections.remove(collection)

    def test_serialization_retry_removes_failed_attempt_objects_and_meshes(self):
        spec = BuildingSpec.from_dict(
            {
                "name": "natural-retry",
                "footprint": [[0, 0], [2, 0], [2, 1], [0, 1]],
                "levels": 1,
                "parts": [
                    {
                        "id": "prism",
                        "kind": "wall",
                        "material": "concrete",
                        "polygon": [[0, 0], [2, 0], [2, 0.2], [0, 0.2]],
                        "bottom": 0,
                        "top": 2,
                    }
                ],
            }
        )
        collection = bpy.data.collections.new("natural-retry-test")
        bpy.context.scene.collection.children.link(collection)
        mesh_count = len(bpy.data.meshes)
        original = natural._object
        calls = 0

        def fail_one_proxy(*args, **kwargs):
            nonlocal calls
            calls += 1
            if calls == 2:
                raise ValueError("injected serialized proxy failure")
            return original(*args, **kwargs)

        try:
            parts = generate_building(spec, collection)
            with patch.object(natural, "_object", side_effect=fail_one_proxy):
                chunks = fracture_parts(
                    parts, seed=7, target_size=1.4, collection=collection, spec=spec
                )
            audit = json.loads(collection["bd_natural_audit"])[0]
            self.assertIn("injected serialized proxy failure", audit["rejected"])
            expected = (
                len(parts)
                + len(chunks)
                + sum(len(chunk.collision_objects) for chunk in chunks)
            )
            self.assertEqual(len(collection.objects), expected)
            self.assertEqual(len(bpy.data.meshes) - mesh_count, expected)
            self.assertAlmostEqual(sum(chunk.volume for chunk in chunks), 0.8, places=5)
        finally:
            for obj in list(collection.objects):
                mesh = obj.data
                bpy.data.objects.remove(obj, do_unlink=True)
                if mesh.users == 0:
                    bpy.data.meshes.remove(mesh)
            bpy.data.collections.remove(collection)

    def test_coplanar_contact_tiles_merge_without_filling_a_concave_notch(self):
        tiles = [
            [(0, 0, 0), (1, 0, 0), (1, 1, 0), (0, 1, 0)],
            [(1, 0, 0), (2, 0, 0), (2, 1, 0), (1, 1, 0)],
        ]
        merged = merge_coplanar_polygons(tiles)
        self.assertEqual(len(merged), 1)
        self.assertAlmostEqual(_patch_metrics(merged[0])["area"], 2.0)
        # The third square makes an L: a total hull would add a false 0.5m².
        tiles.append([(0, 1, 0), (1, 1, 0), (1, 2, 0), (0, 2, 0)])
        remaining = merge_coplanar_polygons(tiles)
        self.assertEqual(len(remaining), 2)
        self.assertAlmostEqual(
            sum(_patch_metrics(poly)["area"] for poly in remaining), 3.0
        )

    def test_serialized_outline_removes_only_microscopic_backtracking_vertex(self):
        vertices = [
            (0, 0, 0),
            (1, 0, 0),
            (1, 1, 0),
            (0.99999994, 0.99999988, 0),
            (0, 1, 0),
        ]
        patch = _patch_metrics(vertices)
        self.assertEqual(len(patch["vertices"]), 4)
        self.assertTrue(all(point in vertices for point in patch["vertices"]))
        self.assertAlmostEqual(patch["area"], 1.0)

    def test_tolerant_contact_clip_never_extrapolates_outside_subject(self):
        subject = [(0.2, -0.00005), (0.8, -0.0002), (0.5, -0.0003)]
        actual = _clip(
            subject, [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)], 0.0001
        )
        for x, y in actual:
            self.assertGreaterEqual(x, 0.2)
            self.assertLessEqual(x, 0.8)
            self.assertGreaterEqual(y, -0.0003)
            self.assertLessEqual(y, -0.00005)

    def test_tolerant_source_clip_never_extrapolates_outside_subject(self):
        from bevy_destruction.spec import _clip as clip_source

        subject = [(0.0, -5e-9), (10.0, -5e-9), (10.0, -3e-8), (0.0, -3e-8)]
        # The true clip has no area. Extrapolation used to invent a strip up to y=0.
        self.assertEqual(clip_source(subject, (0, 0), (1, 0)), [])

    def test_f32_patch_geometry_and_full_nonplanar_parent_moments(self):
        first = _patch_metrics(
            [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]]
        )
        second = _patch_metrics(
            [[1.0, 0.0, 0.0], [1.0, 0.0, 1.0], [1.0, 1.0, 1.0], [1.0, 1.0, 0.0]]
        )
        parent = _aggregate_patches([first, second])
        self.assertAlmostEqual(parent["area"], 2.0)
        for actual, expected in zip(parent["center"], (0.75, 0.5, 0.25)):
            self.assertAlmostEqual(actual, expected)
        self.assertGreater(abs(parent["area_covariance"][0][2]), 0.1)
        self.assertAlmostEqual(sum(v * v for v in parent["normal"]), 1.0)

    def test_profiled_wedge_and_compounds_preserve_source_mass_properties(self):
        spec = BuildingSpec.from_dict(
            {
                "name": "wedge-profile",
                "footprint": [[0, 0], [3, 0], [3, 1], [0, 1]],
                "levels": 1,
                "fracture_size": 1.4,
                "parts": [
                    {
                        "id": "wedge",
                        "kind": "wall",
                        "material": "concrete",
                        "polygon": [[0, 0], [3, 0], [2, 0.2]],
                        "bottom": 0,
                        "top": 2,
                    }
                ],
            }
        )
        collection = bpy.data.collections.new("natural-profile-test")
        bpy.context.scene.collection.children.link(collection)
        try:
            parts = generate_building(spec, collection)
            expected = sum(mesh_metrics(part.obj)["volume"] for part in parts)
            chunks = fracture_parts(
                parts, seed=7, target_size=1.4, collection=collection, spec=spec
            )
            self.assertAlmostEqual(
                sum(chunk.volume for chunk in chunks), expected, delta=expected * 1e-4
            )
            self.assertTrue(all(len(chunk.collision_objects) <= 48 for chunk in chunks))
            for chunk in chunks:
                self.assertGreater(chunk.volume, 0)
                self.assertTrue(
                    all(math.isfinite(v) for row in chunk.inertia for v in row)
                )
                for collider in chunk.collision_objects:
                    self.assertTrue(validate_convex(collider))
                self.assertAlmostEqual(
                    sum(
                        mesh_metrics(collider)["volume"]
                        for collider in chunk.collision_objects
                    ),
                    chunk.volume,
                    delta=expected * 1e-5,
                )
        finally:
            for obj in list(collection.objects):
                mesh = obj.data
                bpy.data.objects.remove(obj, do_unlink=True)
                if mesh.users == 0:
                    bpy.data.meshes.remove(mesh)
            bpy.data.collections.remove(collection)

    def test_formal_v2_export_keeps_openings_and_serializes_all_compound_shapes(self):
        with tempfile.TemporaryDirectory(prefix="natural_v2_") as directory:
            result = build_asset(
                BuildingSpec.from_dict(rectangle("natural-window-v2")),
                output_dir=directory,
                preview=False,
            )
            manifest = json.loads(Path(result["manifest_path"]).read_text())
            self.assertEqual(manifest["schema_version"], 2)
            self.assertGreater(result["collider_count"], result["chunk_count"])
            self.assertEqual(
                result["collider_count"],
                sum(len(chunk["colliders"]) for chunk in manifest["chunks"]),
            )
            self.assertTrue(all(bond["patches"] for bond in manifest["bonds"]))
            collection = bpy.data.collections[result["collection_name"]]
            for role in ("chunk", "collider"):
                objects = [
                    obj for obj in collection.objects if obj.get("bd_role") == role
                ]
                self.assertFalse(objects_hit(objects, (1.5, -1, 1.2), (1.5, 0.5, 1.2)))
                self.assertFalse(objects_hit(objects, (4.0, -1, 1.7), (4.0, 0.5, 1.7)))
                self.assertTrue(objects_hit(objects, (2.8, -1, 1.2), (2.8, 0.5, 1.2)))
            for bond in manifest["bonds"]:
                aggregate = _aggregate_patches(bond["patches"])
                self.assertAlmostEqual(aggregate["area"], bond["area"], places=7)
                for patch in bond["patches"]:
                    recovered = _patch_metrics(patch["vertices"])
                    self.assertAlmostEqual(
                        recovered["area"], patch["area"], delta=patch["area"] * 1e-6
                    )
            collider = next(
                obj for obj in collection.objects if obj.get("bd_role") == "collider"
            )
            collider.data.vertices[0].co.x += 0.01
            with self.assertRaisesRegex(ValueError, "edited"):
                export_asset(collection, directory)


if __name__ == "__main__":
    if not bpy.app.background:
        raise RuntimeError("Use background factory-startup Blender")
    arguments = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
    unittest.main(argv=[__file__, *arguments], verbosity=2)
