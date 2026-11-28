"""Verify loading, click damage, fracture detail and particle cleanup."""

import math
from shutil import copyfile

from bevy_acceptance import assert_that

from physics_fixture import load_manifest, point_center, rest_point
from runtime import break_wall, capture, capture_scene, resource, wait_for_building


CASES = {
    "smoke": "Load and render the scene, break a visible surface, and let effects settle.",
}
PARTICLE_LIMIT = 640
DETAIL_TRIANGLE_LIMIT = 180_000
HITS = 3


def front_faces(scenario):
    manifest = load_manifest(scenario)
    hulls = [
        (chunk["kind"], [rest_point(chunk, vertex) for vertex in collider["vertices"]])
        for chunk in manifest["chunks"] for collider in chunk["colliders"]
    ]
    axes = list(zip(*(vertex for _, hull in hulls for vertex in hull), strict=True))
    camera_side = max(axes[2])
    center = [(min(axis) + max(axis)) / 2 for axis in axes]
    candidates = []
    for kind, hull in hulls:
        face = [vertex for vertex in hull if vertex[2] > camera_side - 0.05]
        if len(face) >= 3:
            candidates.append((kind != "wall", point_center(face)))
    targets = []
    for _, point in sorted(candidates, key=lambda entry: (entry[0], math.dist(entry[1], center))):
        if all(math.dist(point, other) > 1.25 for other in targets):
            targets.append(point)
            if len(targets) == HITS:
                return targets
    raise AssertionError(f"{scenario.asset_path} has too few separated targets on its front face")


def exercise(game, scenario, options):
    with game.case("smoke") as evidence:
        initial = evidence["initial"] = wait_for_building(game, scenario)
        assert_that(initial["chunks"] > 0 and initial["live_bonds"] > 0,
                    "The loaded building has no chunks or live bonds")
        intact = evidence["intact"] = capture_scene(game, "01-intact.png")
        vfx_before = resource(game, "vfx_stats")
        evidence["damage"] = damage = []
        for point in front_faces(scenario):
            damage.append(break_wall(game, point))
            details = resource(game, "detail_stats")
            assert_that(details["triangles"] <= DETAIL_TRIANGLE_LIMIT,
                        f"Fracture detail exceeded its triangle budget: {details}")
        breached = evidence["breached"] = capture(game, "02-breached.png")

        if scenario.detail:
            details = game.wait(
                lambda: (stats if (stats := resource(game, "detail_stats"))["active_meshes"] else None),
                "exposed fracture details",
            )
            assert_that(0 < details["triangles"] <= DETAIL_TRIANGLE_LIMIT,
                        f"Fracture detail is outside its triangle budget: {details}")
        else:
            details = resource(game, "detail_stats")
            assert_that(details["active_meshes"] == 0, "Disabled detail produced fracture meshes")
        evidence["details"] = details

        if scenario.vfx:
            def settled():
                stats = evidence["effects"] = resource(game, "vfx_stats")
                spawned = sum(stats["spawned"]) > sum(vfx_before["spawned"])
                empty = not any(stats["active"]) and not any(stats["scheduled"])
                return stats if spawned and empty else None

            effects = game.wait(settled, "break particles to finish and return to the pool")
            assert_that(0 < effects["peak"] <= PARTICLE_LIMIT,
                        f"Break particles are outside their budget: {effects['peak']}")
            received = effects["received"] - vfx_before["received"]
            responses = effects["feedback_bursts"] - vfx_before["feedback_bursts"]
            assert_that(received >= HITS and responses == received,
                        f"Expected feedback for every burst after {HITS} clicks, observed {responses}/{received}")
        else:
            effects = resource(game, "vfx_stats")
            assert_that(effects["spawned"] == vfx_before["spawned"], "Disabled VFX spawned particles")
        evidence["effects"] = effects
        settled_image = evidence["settled"] = capture(game, "03-settled.png")

        if options.capture_replay:
            # Native screenshots are asynchronous, so replay records visible stages.
            replay = game.output / "replay"
            replay.mkdir(exist_ok=True)
            for index, screenshot in enumerate((intact, breached, settled_image), start=1):
                copyfile(screenshot["path"], replay / f"frame-{index:04d}.png")
            evidence["replay_frames"] = 3


if __name__ == "__main__":
    from run import main

    raise SystemExit(main("smoke"))
