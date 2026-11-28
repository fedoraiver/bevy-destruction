"""Verify that the severed upper tower fractures when it lands."""

from bevy_acceptance.driver import assert_that

from physics_fixture import (
    BOND, IMPACTS, bodies, building_geometry, captures_at_rest, chunk_mass, close_mass, component,
    cut, load_manifest, paused_physics, rest_point,
)
from runtime import capture, place_camera, wait_for_building


CASES = {
    "landing": "Release the upper tower and verify structural ground impacts, fragmentation, and mass conservation.",
}


def exercise(game, scenario, options):
    with game.case("landing") as evidence:
        root = wait_for_building(game, scenario)["building"]
        manifest = load_manifest(scenario)
        building = building_geometry(game, root, manifest)
        low, high = (building["bounds"][edge] for edge in ("min", "max"))
        height = high[1] - low[1]
        assert_that(height >= 12, "The landing fixture needs an intact tower at least 12 m tall")
        cut_height = (low[1] + high[1]) / 2
        upper = {chunk["id"] for chunk in manifest["chunks"] if rest_point(chunk)[1] >= cut_height}
        lower = set(building["chunks"]) - upper
        assert_that(upper and lower, "The tower cut selected only one half")
        expected_mass = sum(chunk_mass(game, building, identifier) for identifier in upper)
        evidence.update(cut_height=cut_height, initial_upper_mass=expected_mass,
                        upper_chunk_count=len(upper), removed_chunk_count=len(lower))
        internal = {bond["id"] for bond in manifest["bonds"] if {bond["a"], bond["b"]} <= upper}
        upper_storey = {bond["id"] for bond in manifest["bonds"]
                        if bond["id"] in internal and bond["center"][1] > cut_height + 4}
        assert_that(upper_storey, "No internal bond lies above the contact storey")
        center = [(a + b) / 2 for a, b in zip(low, high, strict=True)]
        place_camera(game, [high[0] + height * 0.6, height * 0.7, high[2] + height],
                     [center[0], height * 0.4, center[2]])
        evidence["intact"] = captures_at_rest(game, "01-intact-tower.png", require_scene=True)
        initial = game.resource(IMPACTS)
        with paused_physics(game):
            cut(game, root, building, manifest, removed=lower)

        def free_fall():
            groups = bodies(game, root)
            if len(groups) != 1 or groups[0]["anchored"] or groups[0]["velocity"][1] >= -0.1:
                return None
            return groups[0]

        evidence["released"] = game.wait(free_fall, "an intact, unanchored upper tower in free fall")
        close_mass(evidence["released"]["mass"], expected_mass)
        assert_that(set(evidence["released"]["chunks"]) == upper, "The cut changed the upper tower's membership")
        assert_that(game.resource(IMPACTS)["broken_bonds"] == initial["broken_bonds"],
                    "The tower fractured before making contact")
        evidence["free_fall"] = captures_at_rest(game, "02-free-fall.png")

        def hit_ground():
            stats = game.resource(IMPACTS)
            return stats if stats["structural_solves"] > initial["structural_solves"] else None

        evidence["contact"] = game.wait(hit_ground, "a structure solve caused by ground impact")
        evidence["ground_impact"] = captures_at_rest(game, "03-ground-impact.png")

        def live_bonds():
            return {value["id"] for row in game.query(BOND)
                    if (value := row["components"][BOND])["active"]}

        def fragmented():
            groups = bodies(game, root)
            if not groups or max(len(group["chunks"]) for group in groups) + 4 >= len(upper):
                return None
            return groups if upper_storey - live_bonds() else None

        game.wait(fragmented, "the tower to split and break upper-storey connections")
        with paused_physics(game):
            groups = bodies(game, root)
            active = live_bonds()
            sizes = sorted((len(group["chunks"]) for group in groups), reverse=True)
            close_mass(sum(group["mass"] for group in groups), expected_mass, 1e-5)
            assert_that(sizes[0] >= 2 and active.intersection(internal),
                        "Ground impact atomized the entire tower")
            assert_that(upper_storey - active, "Ground impact broke no connection above the contact storey")
            stats = game.resource(IMPACTS)
            assert_that(stats["structural_failures"] == 0
                        and stats["max_force_balance_ratio"] <= 1e-6
                        and stats["max_moment_balance_ratio"] <= 1e-6
                        and stats["max_gauge_reaction_ratio"] <= 1e-5,
                        "The dynamic structure solve failed the equilibrium checks")
            evidence["impact_fracture"] = capture(game, "04-impact-fracture.png")
        evidence.update(final_cluster_sizes=sizes, final_sections=groups, impacts=stats,
                        retained_internal_bonds=len(active.intersection(internal)),
                        internal_bonds_broken=sorted(upper_storey - active))


if __name__ == "__main__":
    from run import main

    raise SystemExit(main("landing"))
