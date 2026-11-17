"""Run with background factory-startup Blender and --python-exit-code 1."""

from pathlib import Path
import json
import sys
import unittest

import bpy
from mathutils import Euler

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from bevy_destruction.fracture import mesh_metrics, validate_convex
from bevy_destruction.natural_union import (
    boundary_mesh,
    cell_contacts,
    merge_convex_cells,
    mesh_data,
    volume,
)


def box(low, high):
    points = [
        (x, y, z)
        for z in (low[2], high[2])
        for y in (low[1], high[1])
        for x in (low[0], high[0])
    ]
    return [
        [points[i] for i in indices]
        for indices in [
            (0, 2, 3, 1),
            (4, 5, 7, 6),
            (0, 1, 5, 4),
            (2, 6, 7, 3),
            (0, 4, 6, 2),
            (1, 3, 7, 5),
        ]
    ]


class NaturalUnionTests(unittest.TestCase):
    def setUp(self):
        self.objects = []

    def tearDown(self):
        for obj in self.objects:
            mesh = obj.data
            bpy.data.objects.remove(obj, do_unlink=True)
            bpy.data.meshes.remove(mesh)

    def object(self, data):
        vertices, faces = data
        mesh = bpy.data.meshes.new("union-test")
        mesh.from_pydata(vertices, [], faces)
        mesh.update()
        obj = bpy.data.objects.new("union-test", mesh)
        self.objects.append(obj)
        return obj

    def test_full_face_union_merges_to_one_convex_body(self):
        cells = [box((0, 0, 0), (1, 1, 1)), box((1, 0, 0), (2, 1, 1))]
        contacts = cell_contacts(cells)
        self.assertEqual(len(contacts), 1)
        self.assertEqual(contacts[0][:2], (0, 1))
        merged = merge_convex_cells(cells)
        self.assertEqual(len(merged), 1)
        obj = self.object(boundary_mesh(merged))
        self.assertTrue(validate_convex(obj))
        metrics = mesh_metrics(obj)
        self.assertAlmostEqual(metrics["volume"], 2, places=6)
        self.assertEqual(metrics["center"], (1, 0.5, 0.5))

    def test_partial_face_union_stitches_t_junctions_and_keeps_nonconvex_volume(self):
        cells = [box((0, 0, 0), (1, 1, 1)), box((1, 0, 0), (2, 0.5, 1))]
        self.assertEqual(len(cell_contacts(cells)), 1)
        merged = merge_convex_cells(cells)
        self.assertEqual(len(merged), 2)
        metrics = mesh_metrics(self.object(boundary_mesh(merged)))
        self.assertAlmostEqual(metrics["volume"], 1.5, places=6)
        self.assertAlmostEqual(metrics["center"][0], 5 / 6, places=6)
        self.assertAlmostEqual(metrics["center"][1], 5 / 12, places=6)
        self.assertAlmostEqual(metrics["center"][2], 0.5, places=6)

    def test_edge_and_point_touch_never_connect_or_convex_merge(self):
        first = box((0, 0, 0), (1, 1, 1))
        for other in [box((1, 1, 0), (2, 2, 1)), box((1, 1, 1), (2, 2, 2))]:
            self.assertEqual(cell_contacts([first, other]), [])
            self.assertEqual(len(merge_convex_cells([first, other])), 2)

    def test_window_ring_stays_open_and_preserves_complete_inertia(self):
        cells = [
            box((x, y, 0), (x + 1, y + 1, 0.2))
            for x in range(3)
            for y in range(3)
            if (x, y) != (1, 1)
        ]
        merged = merge_convex_cells(cells)
        self.assertGreater(len(merged), 1)
        metrics = mesh_metrics(self.object(boundary_mesh(merged)))
        self.assertAlmostEqual(metrics["volume"], 1.6, places=6)
        for actual, expected in zip(metrics["center"], (1.5, 1.5, 0.1)):
            self.assertAlmostEqual(actual, expected, places=6)
        self.assertAlmostEqual(metrics["inertia"][0][0], 1.3386666667, places=6)
        self.assertAlmostEqual(metrics["inertia"][1][1], 1.3386666667, places=6)
        self.assertAlmostEqual(metrics["inertia"][2][2], 2.6666666667, places=6)
        for cell in merged:
            self.assertTrue(validate_convex(self.object(mesh_data(cell))))
            self.assertFalse(
                all(
                    -1e-7 <= self.signed_distance((1.5, 1.5, 0.1), face)
                    for face in cell
                )
            )
        self.assertAlmostEqual(sum(volume(cell) for cell in merged), 1.6, places=6)

    @staticmethod
    def signed_distance(point, face):
        from bevy_destruction.natural_union import _dot, _normal, _sub

        return -_dot(_normal(face), _sub(point, face[0]))

    def test_repeat_is_deterministic_and_reordering_keeps_material_volume(self):
        cells = [
            box((x, y, 0), (x + 1, y + 1, 0.2)) for x, y in [(0, 0), (1, 0), (0, 1)]
        ]
        self.assertEqual(merge_convex_cells(cells), merge_convex_cells(cells))
        for ordered in [cells, list(reversed(cells))]:
            metrics = mesh_metrics(
                self.object(boundary_mesh(merge_convex_cells(ordered)))
            )
            self.assertAlmostEqual(metrics["volume"], 0.6, places=6)

    def test_sampling_grid_collapses_to_one_exact_proxy(self):
        cells = [
            box((x / 4, y / 4, 0), ((x + 1) / 4, (y + 1) / 4, 0.2))
            for x in range(8)
            for y in range(6)
        ]
        merged = merge_convex_cells(cells)
        self.assertEqual(len(merged), 1)
        metrics = mesh_metrics(self.object(boundary_mesh(merged)))
        self.assertAlmostEqual(metrics["volume"], 0.6, places=6)

    def test_rotated_translated_hull_keeps_shared_canonical_boundary(self):
        rotation = Euler((0.17, -0.23, 0.37)).to_matrix()
        cells = [
            box((0, 0, 0), (0.5, 1, 0.2)),
            box((0.5, 0, 0), (1, 1, 0.2)),
            box((1, 0, 0), (1.5, 0.5, 0.2)),
        ]
        transformed = [
            [
                [
                    (
                        tuple(
                            sum(float(rotation[i][j]) * p[j] for j in range(3)) + 50
                            for i in range(3)
                        )
                    )
                    for p in face
                ]
                for face in cell
            ]
            for cell in cells
        ]
        original_points = {
            point for cell in transformed for face in cell for point in face
        }
        merged = merge_convex_cells(transformed)
        self.assertLess(len(merged), len(transformed))
        self.assertTrue(
            all(
                point in original_points
                for cell in merged
                for face in cell
                for point in face
            )
        )
        self.assertTrue(cell_contacts(merged))
        metrics = mesh_metrics(self.object(boundary_mesh(merged)))
        self.assertAlmostEqual(metrics["volume"], 0.25, places=5)

    def test_tiny_notch_is_preserved_even_below_relative_volume_budget(self):
        cells = [box((0, 0, 0), (1, 1, 1)), box((1, 0, 0), (1.001, 0.9999, 1))]
        self.assertEqual(len(merge_convex_cells(cells)), 2)

    def test_rounded_floor_partition_cancels_collapsed_backtracking_face(self):
        path = Path(__file__).parent / "fixtures" / "rounded-floor-boundary.json"
        faces = json.loads(path.read_text(encoding="utf-8"))["faces"]
        metrics = mesh_metrics(self.object(mesh_data(faces)))
        self.assertAlmostEqual(metrics["volume"], 0.8479134901250343, places=6)
        for actual, expected in zip(metrics["center"], (1.09951031, 3.08129598, 0.1)):
            self.assertAlmostEqual(actual, expected, places=6)
        self.assertAlmostEqual(metrics["inertia"][0][0], 0.29067132, places=6)
        self.assertAlmostEqual(metrics["inertia"][1][1], 0.34966558, places=6)
        self.assertAlmostEqual(metrics["inertia"][2][2], 0.63468414, places=6)
        self.assertAlmostEqual(metrics["inertia"][0][1], -0.06299143, places=6)


if __name__ == "__main__":
    if not bpy.app.background:
        raise RuntimeError("Run union tests in background Blender")
    unittest.main(argv=[__file__], verbosity=2)
