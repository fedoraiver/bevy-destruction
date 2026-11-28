"""Verify fracture shading and occlusion around a native wall opening."""

from bevy_acceptance.driver import assert_that

from render_evidence import color_spread, compare, region


CASES = {
    "contact.exposed_surface": "A real click exposes shaded contact surfaces and rebuilds fracture detail.",
    "contact.surviving_occlusion": "Surviving wall pixels stay in front outside the opening over repeated renders.",
}

POSITION = (6.2, 1.55, 6.0)
TARGET = (6.2, 1.55, 0.0)
OPENING = (0.46, 0.43, 0.54, 0.57)
NEIGHBOR = (0.19, 0.16, 0.35, 0.31)
DETAIL_STATS = "bevy_destruction::scene::DetailStats"


def exercise(game, scenario, options):
    from runtime import capture, place_camera, snapshot, wait_for_building

    state = wait_for_building(game, scenario)
    place_camera(game, POSITION, TARGET)
    with game.case("contact.exposed_surface") as evidence:
        def visible_wall():
            shot = capture(game, "01-contact-intact.png")
            return shot if color_spread(region(shot, NEIGHBOR)) > 12 else None

        baseline = game.wait(visible_wall, "contact fixture facade to render")
        opening_before = region(baseline, OPENING)
        neighbor_before = region(baseline, NEIGHBOR)
        before_detail = game.resource(DETAIL_STATS)
        with game.hold_button("Left"):
            pass

        def damaged():
            current = snapshot(game)
            return current if current["removed_chunks"] > state["removed_chunks"] else None

        after = game.wait(damaged, "a native wall click to remove the front shell")
        assert_that(after["live_bonds"] < state["live_bonds"], "Front shell retained its original bonds")

        def visible_detail():
            stats = game.resource(DETAIL_STATS)
            return stats if stats["triangles"] > before_detail["triangles"] else None

        detail = game.wait(visible_detail, "fracture detail to appear along the surviving contact edges")
        result = capture(game, "02-contact-exposed.png")
        exposed = region(result, OPENING)
        delta = compare(opening_before, exposed)
        evidence.update(delta, removed=after["removed_chunks"] - state["removed_chunks"],
                        fracture_triangles=detail["triangles"], baseline=baseline, exposed=result)
        assert_that(delta["changed_pixel_fraction"] > 0.25, "The contact opening did not change visible pixels")
        assert_that(color_spread(exposed) > 22, "The opening revealed no shaded depth or fracture detail")

    with game.case("contact.surviving_occlusion") as evidence:
        repeat = capture(game, "03-contact-exposed-repeat.png")
        surviving = region(result, NEIGHBOR)
        delta = compare(neighbor_before, surviving)
        temporal = compare(surviving, region(repeat, NEIGHBOR))
        evidence.update(delta, repeat=repeat, temporal=temporal,
                        opening_changed_fraction=compare(opening_before, exposed)["changed_pixel_fraction"])
        assert_that(delta["changed_pixel_fraction"] < 0.20, "The intact wall no longer occludes the interior")
        assert_that(temporal["changed_pixel_fraction"] < 0.02, "Intact surface flickered near the contact opening")


if __name__ == "__main__":
    from run import main

    raise SystemExit(main("contact"))
