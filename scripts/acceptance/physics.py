"""Verify cluster detachment, falling, splitting, and sleep."""

from math import dist

from bevy_acceptance.driver import assert_that

from physics_fixture import (
    bodies, building_geometry, captures_at_rest, chunk_mass, close_mass, cut,
    load_manifest, paused_physics, point_center, rest_point, sleeping_entities,
)
from runtime import capture, place_camera, wait_for_building


CASES = {
    "physics": "Release a bonded roof pair, split it in free fall, and verify mass and sleep.",
}


def roof_pair(manifest):
    anchors = {anchor["chunk"] for anchor in manifest["anchors"]}
    roof = {chunk["id"]: chunk for chunk in manifest["chunks"]
            if chunk["kind"] == "roof" and chunk["id"] not in anchors}
    choices = [(bond["a"], bond["b"]) for bond in manifest["bonds"]
               if bond["a"] in roof and bond["b"] in roof]
    assert_that(choices, "The fixture has no bonded non-anchored roof pair")
    center = point_center([chunk["position"] for chunk in roof.values()])
    pair = set(min(choices, key=lambda ids:
                   (dist(point_center([roof[identifier]["position"] for identifier in ids]), center), ids)))
    clearance = {
        bond[external]
        for bond in manifest["bonds"]
        for internal, external in (("a", "b"), ("b", "a"))
        if bond[internal] in pair and bond[external] not in pair and bond[external] not in anchors
    }
    detach = {bond["id"] for bond in manifest["bonds"] if (bond["a"] in pair) != (bond["b"] in pair)}
    split = {bond["id"] for bond in manifest["bonds"] if {bond["a"], bond["b"]} == pair}
    return pair, clearance, detach, split


def exercise(game, scenario, options):
    with game.case("physics") as evidence:
        root = wait_for_building(game, scenario)["building"]
        manifest = load_manifest(scenario)
        building = building_geometry(game, root, manifest)
        pair, clearance, detach, split = roof_pair(manifest)
        center = point_center([rest_point(chunk) for chunk in manifest["chunks"] if chunk["id"] in pair])
        expected = sum(chunk_mass(game, building, identifier) for identifier in pair)
        evidence.update(selected_chunks=sorted(pair), clearance_chunks=sorted(clearance), expected_mass_kg=expected)
        place_camera(game, [center[0] + 5, center[1] + 3, center[2] + 12], center)
        evidence["supported"] = captures_at_rest(game, "01-supported.png", require_scene=True)

        with paused_physics(game):
            cut(game, root, building, manifest, removed=clearance, broken=detach)

        def selected_bodies():
            return [body for body in bodies(game, root) if pair.intersection(body["chunks"])]

        def released():
            selected = selected_bodies()
            if len(selected) == 1 and not selected[0]["anchored"]:
                assert_that(set(selected[0]["chunks"]) == pair, "The detached pair retained neighbor chunks")
                return selected[0]
            return None

        evidence["released"] = game.wait(released, "one unsupported rigid roof pair")
        close_mass(evidence["released"]["mass"], expected)

        def falling():
            body = released()
            return body if body and body["center"][1] < center[1] - 0.10 and body["velocity"][1] < 0 else None

        game.wait(falling, "the still-connected roof pair to fall")
        with paused_physics(game):
            evidence["falling"] = capture(game, "02-falling.png")
            cut(game, root, building, manifest, broken=split)

        def detached():
            selected = selected_bodies()
            if (len(selected) == 2 and {tuple(body["chunks"]) for body in selected} == {(item,) for item in pair}
                    and all(body["kind"] == "Dynamic" and not body["anchored"] for body in selected)):
                return selected
            return None

        evidence["split"] = game.wait(detached, "two independent dynamic roof chunks")
        close_mass(sum(body["mass"] for body in evidence["split"]), expected)

        def settled():
            selected = detached()
            asleep = sleeping_entities(game)
            return selected if selected and all(body["entity"] in asleep for body in selected) else None

        evidence["settled"] = game.wait(settled, "both detached chunks to fall asleep on the ground")
        assert_that(all(body["center"][1] >= -0.1 and dist(body["velocity"], [0, 0, 0]) < 0.1
                        for body in evidence["settled"]), "Detached chunks failed to settle above ground")
        assert_that(all(body["kind"] == "Static" and dist(body["position"], [0, 0, 0]) < 0.0001
                        for body in bodies(game, root) if body["anchored"]), "The supported building moved")
        evidence["settled_capture"] = captures_at_rest(game, "03-settled.png")


if __name__ == "__main__":
    from run import main

    raise SystemExit(main("physics"))
