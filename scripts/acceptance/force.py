"""Verify self-weight, color thresholds, native inspection, and physical impulse readouts."""

import math

from bevy_acceptance.driver import assert_that

from physics_fixture import (
    CHUNK, IDENTITY, VIRTUAL_TIME, add, bodies, component, cross, paused_physics,
    point_center, rotate, scale,
)
from render_evidence import color_spread, compare, region


CASES = {
    "force.overview": "The 9 key shows current self-weight without casting a pick ray or drawing an arrow.",
    "force.heat": "Real self-weight reaches the amber and red thresholds and colors source meshes.",
    "force.pick": "Holding Alt distinguishes a visible facade from empty space and release clears inspection.",
    "force.impulse": "A native projectile leads to a contact readout equal to an Avian impulse sample.",
    "force.break_and_clear": "Broken connections disappear from inspection and disabling heat clears its state.",
}

CAPTURE = "bevy_destruction::force_samples::ForceCapture"
VIEW = "bevy_destruction::force_debug::ForceViewStats"
SAMPLES = "bevy_destruction::force_samples::ForceSamples"
IMPACT = "bevy_destruction::projectiles::ImpactStats"
STRESS = "bevy_destruction::stress_runtime::StressState"
STRESS_SETTINGS = "bevy_destruction::stress_runtime::StressSettings"
GRAVITY = "avian3d::dynamics::integrator::Gravity"
TAG = "bevy_mesh::components::MeshTag"
BOND = "bevy_destruction::scene::BondInstance"
POSITION = (4.0, 3.45, 8.0)
FACADE = (4.0, 3.45, 0.0)
TARGET = (1.5, 4.65, 0.0)
SURFACE = (0.12, 0.21, 0.88, 0.91)


def wait_view(game, predicate, description, timeout=None):
    def sample():
        view = game.resource(VIEW)
        return view if predicate(view) else None

    return game.wait(sample, description, timeout)


def set_resource_field(game, resource, path, value):
    game.call("world.mutate_resources", {"resource": resource, "path": path, "value": value})


def mesh_tags(game):
    return {row["components"][TAG]["value"] for row in game.query(TAG)}


def update_heat(game, gravity, scale, tag):
    set_resource_field(game, GRAVITY, ".0", [axis * scale for axis in gravity])
    return game.wait(lambda: tag in mesh_tags(game) and game.resource(VIEW),
                     f"the real force shader mesh tag to reach level {tag}")


def matching_contact(game, previous):
    view = game.resource(VIEW)
    inspected = view["inspected"]
    if not inspected or inspected["impulse_ns"] is None:
        return None
    impulse = inspected["impulse_ns"]
    building, chunk = inspected["chunk"]
    samples = game.resource(SAMPLES)["impulses"]
    observed = [
        math.hypot(*sample["impulse"])
        for sample in samples if sample["building"] == building and sample["chunk"] == chunk
    ]
    impact = game.resource(IMPACT)
    if impulse > 0 and impact["contacts"] > previous and any(
        math.isclose(impulse, actual, rel_tol=1e-6) for actual in observed
    ):
        return {"inspection": inspected, "contact_samples_ns": observed, "impacts": impact}
    return None


def contact_poses(game, previous, eye):
    body_by_chunk = {}
    chunk_entities = {
        (identity["building"], identity["id"]): row["entity"]
        for row in game.query(IDENTITY)
        if (identity := row["components"][IDENTITY])
    }
    impulses = [sample for sample in game.resource(SAMPLES)["impulses"]
                if math.hypot(*sample["impulse"]) > 0.0]
    if game.resource(IMPACT)["contacts"] <= previous or not impulses:
        return []
    for building in {sample["building"] for sample in impulses}:
        for body in bodies(game, building):
            body_by_chunk.update(((building, chunk), body) for chunk in body["chunks"])
    poses = []
    for key in dict.fromkeys((sample["building"], sample["chunk"]) for sample in reversed(impulses)):
        body = body_by_chunk.get(key)
        entity = chunk_entities.get(key)
        if entity is None or body is None:
            continue
        chunk = component(game, entity, CHUNK)
        if not chunk["alive"]:
            continue
        faces = []
        for triangle in chunk["triangles"]:
            points = [add(body["position"], rotate(body["rotation"], vertex)) for vertex in triangle]
            a, b, c = points
            perpendicular = cross(add(b, scale(a, -1)), add(c, scale(a, -1)))
            area = math.hypot(*perpendicular)
            if area < 1e-5:
                continue
            normal = scale(perpendicular, 1 / area)
            center = point_center(points)
            toward_eye = add(eye, scale(center, -1))
            visible_side = sum(axis * offset for axis, offset in zip(normal, toward_eye, strict=True))
            if visible_side > 0:
                position = add(center, scale(normal, 1.2))
                if abs(normal[1]) > 0.98:
                    position[0] += 0.15
                faces.append((math.dist(center, eye), position, center))
        poses.extend(sorted(faces)[:2])
    return sorted(poses)


def inspect_contact(game, before, place_camera, capture):
    speed = game.resource(VIRTUAL_TIME)["context"]["relative_speed"]
    set_resource_field(game, VIRTUAL_TIME, "context.relative_speed", 0.12)
    try:
        for shots, target in enumerate((TARGET, (6.1, 4.6, 0.0), (3.4, 1.65, 0.0)), start=1):
            eye = (target[0], target[1], 4.5)
            place_camera(game, eye, target)
            with game.hold_button("Right"):
                pass
            try:
                game.wait(lambda: game.resource(SAMPLES)["impulses"],
                          "the native shot to produce physical contact samples", timeout=6)
            except TimeoutError:
                continue
            with paused_physics(game), game.hold_key("AltLeft", logical_key="Alt"):
                for _, position, center in contact_poses(game, before, eye)[:6]:
                    place_camera(game, position, center)
                    try:
                        observed = game.wait(lambda: matching_contact(game, before),
                                             "Alt to read the sampled contact surface", timeout=1.2)
                    except TimeoutError:
                        continue
                    return {**observed, "shot": shots, "screenshot": capture(game, "04-force-impulse.png")}
    finally:
        set_resource_field(game, VIRTUAL_TIME, "context.relative_speed", speed)
    raise AssertionError("No sampled projectile contact impulse reached the inspected surface")


def active_bonds(game):
    return {
        row["components"][BOND]["id"]
        for row in game.query(BOND)
        if row["components"][BOND]["active"]
    }


def heat_hues(pixels):
    amber = red = 0
    for r, g, b in pixels:
        if r >= 0.9 * g and g > 1.4 * b and g - b > 18:
            amber += 1
        elif r > 1.35 * max(g, b) and r - g > 20:
            red += 1
    return {"amber": amber, "red": red}


def exercise(game, scenario, options):
    from runtime import aim_at, capture, place_camera, snapshot, wait_for_building

    ready = wait_for_building(game, scenario)
    place_camera(game, POSITION, FACADE)
    with game.case("force.overview") as evidence:
        def intact():
            shot = capture(game, "01-force-normal.png")
            return shot if color_spread(region(shot, SURFACE)) > 20 else None

        baseline = game.wait(intact, "the force fixture facade to render")
        assert_that(not game.resource(CAPTURE)["enabled"], "Heat was enabled before keyboard input")
        game.tap("Digit9")
        overview = wait_view(game, lambda view: view["static_bonds"] > 0 and view["max_force_n"] > 0,
                             "the self-weight overview to contain positive force")
        assert_that(overview["inspected"] is None and overview["force_arrows"] == overview["pick_queries"] == 0,
                    "The ordinary overview performed inspection")
        evidence.update(baseline=baseline, overview=overview)

    with game.case("force.heat") as evidence:
        state = game.get(ready["building"], STRESS)[STRESS]
        utilization = state["max_utilization"]
        assert_that(math.isfinite(utilization) and utilization > 0.0, "Expected a valid self-weight baseline")
        gravity = game.resource(GRAVITY)
        damage_seconds = game.resource(STRESS_SETTINGS)["damage_seconds"]
        # Hold the real over-capacity surface long enough to capture its shader output.
        set_resource_field(game, STRESS_SETTINGS, "damage_seconds", 30.0)
        with_load = {}
        try:
            for threshold, target, color in ((1, 0.68, "amber"), (2, 1.12, "red")):
                scale = target / utilization
                view = update_heat(game, gravity, scale, threshold)
                assert_that(view["tinted_chunks"] > 0, f"No loaded chunks were tinted {color}")
                shot = capture(game, f"02-force-heat-{color}.png")
                with_load[color] = {"gravity_scale": scale, "view": view, "screenshot": shot}
        finally:
            set_resource_field(game, GRAVITY, ".0", gravity)
            set_resource_field(game, STRESS_SETTINGS, "damage_seconds", damage_seconds)
        baseline_pixels = region(baseline, SURFACE)
        baseline_hues = heat_hues(baseline_pixels)
        evidence["baseline_hues"] = baseline_hues
        for color, sample in with_load.items():
            pixels = region(sample["screenshot"], SURFACE)
            hues = heat_hues(pixels)
            delta = compare(baseline_pixels, pixels)
            evidence[color] = {**sample, "heat_hues": hues, "image_difference": delta}
            assert_that(hues[color] > baseline_hues[color] + 10,
                        f"The {color} heat tags produced no visible {color} surfaces")

    with game.case("force.pick") as evidence:
        aim_at(game, TARGET)
        with game.hold_key("AltLeft", logical_key="Alt"):
            visible = wait_view(game, lambda view: view["inspected"] is not None and view["force_arrows"] == 1,
                                "Alt to select the visible wall and draw its force arrow")
            evidence["visible"] = visible
            evidence["screenshot"] = capture(game, "03-force-inspect.png")
            aim_at(game, (4.0, 19.0, 0.0))
            missed = wait_view(game, lambda view: view["pick_queries"] == 1 and view["inspected"] is None,
                               "the same inspection ray to reject empty space")
            assert_that(missed["force_arrows"] == 0, "A ray miss retained a force arrow")
            evidence["empty_space"] = missed
        wait_view(game, lambda view: view["pick_queries"] == 0 and view["inspected"] is None,
                  "Alt release to clear inspection")

    with game.case("force.impulse") as evidence:
        previous = game.resource(IMPACT)["contacts"]
        evidence.update(inspect_contact(game, previous, place_camera, capture))

    with game.case("force.break_and_clear") as evidence:
        place_camera(game, (6.2, 1.55, 6.0), (6.2, 1.55, 0.0))
        before = snapshot(game)
        with game.hold_button("Left"):
            pass

        def removed():
            after = snapshot(game)
            return after if after["removed_chunks"] > before["removed_chunks"] else None

        after = game.wait(removed, "the hovered front wall to break")
        assert_that(after["live_bonds"] < before["live_bonds"], "The removed wall retained its active bonds")
        evidence["remaining_forces"] = measured_bonds(game, aim_at)
        evidence["after_break"] = after
        game.tap("Digit9")
        cleared = wait_view(game, lambda view: not any(view.values()), "force overlay counters to clear")
        captured = game.resource(SAMPLES)
        assert_that(not game.resource(CAPTURE)["enabled"] and not captured["dynamic"] and not captured["impulses"],
                    "Disabling heat retained force samples")
        assert_that(not (mesh_tags(game) & {1, 2}), "Disabled heat left mesh tags painted")
        evidence.update(cleared=cleared, screenshot=capture(game, "05-force-clear.png"))


def measured_bonds(game, aim_at):
    active = active_bonds(game)
    with game.hold_key("AltLeft", logical_key="Alt"):
        for target in (TARGET, (3.25, 4.6, 0.0), (2.0, 2.1, 0.0)):
            aim_at(game, target)
            try:
                current = wait_view(game, lambda view: view["inspected"] and view["inspected"]["bonds"],
                                    "inspection of a surviving loaded connection", timeout=1.5)
            except TimeoutError:
                continue
            visible = current["inspected"]["bonds"]
            assert_that(all(readout["id"] in active for readout in visible), "Force inspection retained a broken bond")
            return visible
    raise AssertionError("No surviving loaded connection reached the inspector after damage")


if __name__ == "__main__":
    from run import main

    raise SystemExit(main("force"))
