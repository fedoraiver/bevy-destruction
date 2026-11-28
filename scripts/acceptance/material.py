"""Verify that surface patterns follow an actual rendered building cluster."""

import math

from bevy_acceptance.driver import TYPES, assert_that

from render_evidence import color_spread, compare, region


CASES = {
    "material.rest_coordinates": "The same rendered facade retains its pattern when its rigid cluster and viewpoint translate together.",
    "material.negative_control": "Moving only the viewpoint visibly changes the sampled facade.",
}

FACADE = (0.38, 0.22, 0.62, 0.42)
POSITION = (4.0, 3.45, 8.0)
TARGET = (4.0, 3.45, 0.0)
CLUSTER = "bevy_destruction::physics::Cluster"


def plus(point, offset):
    return tuple(a + b for a, b in zip(point, offset, strict=True))


def move_cluster(game, entity, position):
    game.call("world.mutate_components", {
        "entity": entity, "component": TYPES["transform"], "path": "translation", "value": position,
    })

    def moved():
        pose = game.get(entity, TYPES["transform"])[TYPES["transform"]]
        return math.dist(pose["translation"], position) < 1e-4

    game.wait(moved, "anchored cluster to move to the requested transform")


def exercise(game, scenario, options):
    from runtime import capture, place_camera, wait_for_building

    wait_for_building(game, scenario)
    place_camera(game, POSITION, TARGET)
    with game.case("material.rest_coordinates") as evidence:
        def visible_facade():
            shot = capture(game, "01-facade-rest.png")
            rgb = region(shot, FACADE)
            return (shot, rgb) if color_spread(rgb) > 18 else None

        baseline, reference = game.wait(visible_facade, "rendered facade with visible surface detail")
        evidence["baseline"] = baseline
        clusters = [
            row for row in game.query(TYPES["transform"], CLUSTER)
            if row["components"][CLUSTER]["anchored"]
        ]
        assert_that(len(clusters) == 1, f"Expected one anchored facade, found {len(clusters)}")
        cluster = clusters[0]
        origin = cluster["components"][TYPES["transform"]]["translation"]

        for index, offset in enumerate(((2.125, 0.0, 0.0), (-1.375, 0.0, 1.25)), start=2):
            move_cluster(game, cluster["entity"], plus(origin, offset))
            place_camera(game, plus(POSITION, offset), plus(TARGET, offset))
            shot = capture(game, f"{index:02d}-facade-translated.png")
            delta = compare(reference, region(shot, FACADE))
            evidence[f"motion_{index}"] = {"offset_m": offset, "screenshot": shot, **delta}
            assert_that(delta["mean_rgb_error"] < 2.5 and delta["changed_pixel_fraction"] < 0.08,
                        f"Facade material changed with a rigid translation: {delta}")

    with game.case("material.negative_control") as evidence:
        place_camera(game, plus(POSITION, (0.85, 0.45, 0.0)), plus(TARGET, (0.85, 0.45, 0.0)))
        shot = capture(game, "04-facade-camera-only.png")
        delta = compare(reference, region(shot, FACADE))
        evidence.update(delta, screenshot=shot)
        assert_that(delta["mean_rgb_error"] > 3.0, "The facade comparison was insensitive to camera motion")


if __name__ == "__main__":
    from run import main

    raise SystemExit(main("material"))
