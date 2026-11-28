"""Verify asymmetric support failure and upper-building tipping."""

import heapq
import json
import math

from bevy_acceptance.driver import assert_that

from physics_fixture import (
    IMPACTS, STRESS, add, bodies, building_geometry, captures_at_rest, chunk_mass, close_mass,
    component, cut, expect_stress, load_manifest, paused_physics, rest_point, rotate,
)
from runtime import capture, place_camera, wait_for_building


CASES = {
    "collapse": "Retain one eccentric support path and verify strength failure, coherent tipping, and conserved mass.",
}
GEOMETRY_EPSILON = 1e-4


def chunk_bounds(chunk):
    points = [rest_point(chunk, vertex)
              for proxy in chunk["colliders"] for vertex in proxy["vertices"]]
    return [min(axis) for axis in zip(*points, strict=True)], [max(axis) for axis in zip(*points, strict=True)]


def connected(start, neighbors, allowed):
    found = set(start)
    pending = list(start)
    while pending:
        for adjacent in neighbors[pending.pop()].intersection(allowed) - found:
            found.add(adjacent)
            pending.append(adjacent)
    return found


def lightest_path(start, ends, supports, neighbors, records):
    queue = [(records[start]["mass"], start, (start,))]
    visited = set()
    while queue:
        mass, identifier, path = heapq.heappop(queue)
        if identifier in visited:
            continue
        visited.add(identifier)
        if identifier in ends:
            return mass, path
        for neighbor in neighbors[identifier].intersection(supports) - visited:
            heapq.heappush(queue, (mass + records[neighbor]["mass"], neighbor, (*path, neighbor)))
    return None


def select_support(manifest):
    records = {chunk["id"]: chunk for chunk in manifest["chunks"]}
    bounds = {identifier: chunk_bounds(chunk) for identifier, chunk in records.items()}
    floor = {identifier for identifier, chunk in records.items() if chunk["kind"] == "floor"}
    foundation_bottom = min(bounds[identifier][0][1] for identifier in floor)
    foundation = {identifier for identifier in floor
                  if abs(bounds[identifier][0][1] - foundation_bottom) < GEOMETRY_EPSILON}
    foundation_top = max(bounds[identifier][1][1] for identifier in foundation)
    upper_y = min(bounds[identifier][0][1] for identifier in floor
                  if bounds[identifier][0][1] > foundation_top + GEOMETRY_EPSILON)
    upper_floor = {identifier for identifier in floor
                   if abs(bounds[identifier][0][1] - upper_y) < GEOMETRY_EPSILON}
    supports = {identifier for identifier, chunk in records.items()
                if chunk["kind"] not in ("floor", "roof")
                and bounds[identifier][0][1] < upper_y - GEOMETRY_EPSILON
                and bounds[identifier][1][1] > foundation_top + GEOMETRY_EPSILON}
    neighbors = {identifier: set() for identifier in records}
    for bond in manifest["bonds"]:
        neighbors[bond["a"]].add(bond["b"])
        neighbors[bond["b"]].add(bond["a"])
    anchored = {entry["chunk"] for entry in manifest["anchors"] if entry["kind"] == "fixed"}
    grounded = connected(anchored.intersection(foundation), neighbors, foundation)
    starts = {identifier for identifier in supports if neighbors[identifier].intersection(grounded)
              and not neighbors[identifier].intersection(upper_floor)}
    ends = {identifier for identifier in supports if neighbors[identifier].intersection(upper_floor)}
    low = [min(bound[0][axis] for bound in bounds.values()) for axis in range(3)]
    high = [max(bound[1][axis] for bound in bounds.values()) for axis in range(3)]
    target_x = (low[0] + high[0]) / 2
    target_z = (low[2] + high[2]) / 2 - 0.5
    candidates = []
    for start in starts:
        candidate = lightest_path(start, ends, supports, neighbors, records)
        if candidate is None:
            continue
        mass, path = candidate
        assert len(path) >= 2
        lower, upper = bounds[start]
        interior = all(low[axis] + GEOMETRY_EPSILON < lower[axis]
                       and upper[axis] < high[axis] - GEOMETRY_EPSILON for axis in (0, 2))
        x, _, z = rest_point(records[start])
        candidates.append((not interior, math.hypot(x - target_x, z - target_z), mass, path))
    assert_that(candidates, "No eccentric multi-piece path connects the foundation to the upper storey")
    path = min(candidates)[3]
    return path, supports - set(path), upper_y


def upper_motion(body, upper, upper_mass):
    identifiers = [identifier for identifier in body["chunks"] if identifier in upper]
    mass = sum(upper[identifier][1] for identifier in identifiers)
    if body["anchored"] or len(identifiers) < 2 or mass < upper_mass * 0.25:
        return None
    rest = [sum(upper[identifier][0][axis] * upper[identifier][1] for identifier in identifiers) / mass
            for axis in range(3)]
    actual = add(body["position"], rotate(body["rotation"], rest))
    normal = rotate(body["rotation"], [0, 1, 0])
    return {"body": body, "upper_mass": mass, "original_center": rest, "center": actual,
            "drop": rest[1] - actual[1], "tilt": math.acos(max(-1.0, min(1.0, normal[1])))}


def exercise(game, scenario, options):
    with game.case("collapse") as evidence:
        root = wait_for_building(game, scenario)["building"]
        manifest = load_manifest(scenario)
        building = building_geometry(game, root, manifest)
        before = expect_stress(game, root)
        assert_that(before["broken_total"] == 0 and before["reports"]
                    and all(report["valid"] for report in before["reports"]),
                    "The intact building did not have a valid self-weight solution")
        path, removed, upper_y = select_support(manifest)
        assert_that(removed, "The geometry selection found no other support to remove")
        upper = {chunk["id"]: (rest_point(chunk), chunk["mass"])
                 for chunk in manifest["chunks"] if rest_point(chunk)[1] >= upper_y}
        assert_that(upper, "The geometry selection found no upper storey")
        upper_mass = sum(mass for _, mass in upper.values())
        expected_mass = (sum(body["mass"] for body in bodies(game, root))
                         - sum(chunk_mass(game, building, identifier) for identifier in removed))
        low, high = (building["bounds"][edge] for edge in ("min", "max"))
        center = [(a + b) / 2 for a, b in zip(low, high, strict=True)]
        place_camera(game, [low[0] - 7, high[1] * 0.65, high[2] + 10],
                     [center[0], high[1] * 0.4, center[2]])
        evidence["intact"] = captures_at_rest(game, "01-intact-rectangle.png", require_scene=True)
        baseline_contacts = game.resource(IMPACTS)["contacts"]

        with paused_physics(game):
            cut(game, root, building, manifest, removed=removed)
            evidence["narrow_support"] = capture(game, "02-narrow-support.png")
        damage = {"asset": scenario.asset_path, "retained_support_path": path,
                  "removed_ids": sorted(removed), "expected_retained_mass_kg": expected_mass}
        (game.output / "damage.json").write_text(json.dumps(damage, indent=2) + "\n", encoding="utf-8")

        def tipping():
            stats = component(game, root, STRESS)
            if stats["broken_total"] <= before["broken_total"]:
                return None
            for group in bodies(game, root):
                motion = upper_motion(group, upper, upper_mass)
                if motion is None:
                    continue
                if motion["drop"] > 0.5 and group["velocity"][1] < -0.25:
                    evidence.setdefault("falling", motion)
                if motion["drop"] > 0.1 and motion["tilt"] > 0.1:
                    evidence.setdefault("tipping", motion)
            return evidence.get("falling") and evidence.get("tipping")

        game.wait(tipping, "a coherent upper section to tip and fall after support failure")
        evidence["tipping_capture"] = captures_at_rest(game, "03-bulk-tipping.png")

        def supported_contact():
            stats = game.resource(IMPACTS)
            if stats["contacts"] > baseline_contacts:
                return stats
            return None

        evidence["impacts"] = game.wait(supported_contact, "contact during the collapse")
        with paused_physics(game):
            final = bodies(game, root)
            stress = component(game, root, STRESS)
            retained = set(path).intersection(
                identifier for group in final if group["anchored"] for identifier in group["chunks"])
            assert_that(retained, "No static residual support survived the collapse")
            assert_that(stress["reports"] and all(report["valid"] for report in stress["reports"]),
                        "The surviving foundation has no valid stress solution")
            close_mass(sum(group["mass"] for group in final), expected_mass, 1e-5)
            evidence["final_capture"] = capture(game, "04-final-collapse.png")
        evidence.update(damage=damage, final_bodies=final, retained_supports=sorted(retained),
                        strength_breaks=stress["broken_total"] - before["broken_total"],
                        final_stress_reports=stress["reports"])


if __name__ == "__main__":
    from run import main

    raise SystemExit(main("collapse"))
