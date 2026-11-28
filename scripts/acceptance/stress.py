"""Verify self-weight, overload fracture, and rigid impactors."""

import math

from bevy_acceptance.driver import assert_that

from physics_fixture import (
    BOND, CHUNK, GRAVITY, IMPACTS, LINEAR, MASS, PROJECTILE, STRESS,
    STRESS_SETTINGS, bodies, building_geometry, captures_at_rest, close_mass, component, cross, expect_stress,
    load_manifest, mass_from, mutate_resource, paused_physics, point_center, scale,
)
from runtime import place_camera, wait_for_building


CASES = {
    "stress.self_weight": "The intact building solves self-weight below its strength limits.",
    "stress.overload": "Higher self-weight produces progressive damage before breaking a live connection.",
    "stress.rigid_impacts": "Both native projectile inputs launch rigid bodies that survive structural impacts.",
}


def set_gravity(game, vector):
    game.call("world.insert_resources", {"resource": GRAVITY, "value": vector})


def closest_wall_point(game, root, building, fraction):
    low, high = (building["bounds"][edge] for edge in ("min", "max"))
    desired = [low[0] + (high[0] - low[0]) * fraction,
               low[1] + (high[1] - low[1]) * 0.55, high[2]]
    entities = {building["chunks"][identifier]
                for body in bodies(game, root) if body["anchored"]
                for identifier in body["chunks"]}
    candidates = []
    for row in game.query(CHUNK):
        chunk = row["components"][CHUNK]
        if row["entity"] not in entities or not chunk["alive"]:
            continue
        for a, b, c in chunk["triangles"]:
            normal = cross([v - w for v, w in zip(b, a, strict=True)],
                           [v - w for v, w in zip(c, a, strict=True)])
            if normal[2] <= 0.95 * math.hypot(*normal):
                continue
            point = point_center([a, b, c])
            if point[2] >= high[2] - 0.25:
                candidates.append(point)
    assert_that(candidates, "No intact front wall is available for a rigid impact")
    return min(candidates, key=lambda point: math.dist(point, desired))


def projectile_impact(game, root, building, shape, fraction, filename):
    target = closest_wall_point(game, root, building, fraction)
    place_camera(game, [target[0], target[1], target[2] + 6], target)
    before = game.resource(IMPACTS)
    contacts = f"{shape}_contacts"
    existing = {row["entity"] for row in game.query(with_=(PROJECTILE,))}
    if shape == "sphere":
        with game.hold_button("Right"):
            pass
    else:
        game.tap("KeyR")

    def impacted():
        after = game.resource(IMPACTS)
        if (after[contacts] <= before[contacts]
                or after["broken_bonds"] <= before["broken_bonds"]):
            return None
        rows = [row for row in game.query(MASS, LINEAR, PROJECTILE)
                if row["entity"] not in existing]
        if len(rows) != 1:
            return None
        values = rows[0]["components"]
        assert_that(values[PROJECTILE]["shape"].lower() == shape,
                    f"Native {shape} input spawned {values[PROJECTILE]['shape']}")
        if not all(math.isfinite(value) for value in values[LINEAR]):
            return None
        if abs(values[LINEAR][2] + 55.0) <= 1.0:
            return None
        return after, mass_from(values[MASS]), values[LINEAR]

    result, mass, velocity = game.wait(impacted, f"the {shape} to fracture a wall bond without breaking")
    assert_that(result["launched"] == before["launched"] + 1, "One input launched more than one impactor")
    close_mass(mass, 8000.0, 2e-5)
    return {"after": result, "mass_kg": mass, "velocity": velocity, "target": target,
            "screenshot": captures_at_rest(game, filename)}


def exercise(game, scenario, options):
    root = wait_for_building(game, scenario)["building"]
    manifest = load_manifest(scenario)
    with game.case("stress.self_weight") as evidence:
        baseline = expect_stress(game, root)
        utilization = baseline["max_utilization"]
        assert_that(baseline["reports"] and all(report["valid"] for report in baseline["reports"])
                    and baseline["broken_total"] == 0 and 0 < utilization < 1,
                    "The intact building failed its self-weight solve")
        evidence.update(reports=baseline["reports"], max_utilization=utilization,
                        screenshot=captures_at_rest(game, "01-intact-selfweight.png", require_scene=True))

    with game.case("stress.overload") as evidence:
        building = building_geometry(game, root, manifest)
        normal_gravity = game.resource(GRAVITY)
        with paused_physics(game):
            mutate_resource(game, STRESS_SETTINGS, "damage_seconds", 3.0)
            mutate_resource(game, STRESS_SETTINGS, "immediate_utilization", 100.0)
            set_gravity(game, scale(normal_gravity, 2.5 / utilization))

        def progressive():
            stress = component(game, root, STRESS)
            for identifier, bond in stress["bonds"].items():
                if bond["valid"] and bond["stress"]["utilization"] > 1 and 0 < bond["damage"] < 1:
                    if component(game, building["bonds"][identifier], BOND)["active"]:
                        return {"bond": identifier, **bond}
            return None

        evidence["connected_overload"] = game.wait(progressive, "progressive overload while the bond remains active")
        evidence["overload_capture"] = captures_at_rest(game, "02-overloaded-connection.png")

        def fractured():
            stress = component(game, root, STRESS)
            return stress if stress["broken_total"] > baseline["broken_total"] else None

        final = game.wait(fractured, "a link to fail after accumulating overload damage")
        assert_that(final["failures"] == baseline["failures"],
                    "The building's static solver failed while applying the overload")
        evidence["fractured_bonds"] = final["broken_total"]
        set_gravity(game, normal_gravity)
        evidence["fracture_capture"] = captures_at_rest(game, "03-structural-fracture.png")

    with game.case("stress.rigid_impacts") as evidence:
        game.restart()
        root = wait_for_building(game, scenario)["building"]
        building = building_geometry(game, root, manifest)
        evidence["sphere"] = projectile_impact(game, root, building, "sphere", 0.3, "04-rigid-sphere-impact.png")
        evidence["cube"] = projectile_impact(game, root, building, "cube", 0.7, "05-rigid-cube-impact.png")


if __name__ == "__main__":
    from run import main

    raise SystemExit(main("stress"))
