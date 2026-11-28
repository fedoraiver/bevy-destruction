"""Drive the rectangle demo through native keyboard and mouse input."""

import math
import time

from bevy_acceptance import assert_that

from runtime import (
    POSITION_TOLERANCE, break_wall, camera_angles, camera_state, capture, capture_scene,
    launch_projectile, place_camera, wait_for_building, wait_for_state,
)


# Interior points on the rectangle's front collision hulls and exterior mesh.
LOWER_WALL = (6.2, 1.55, 0.0)
UPPER_LEFT_WALL = (1.5, 4.65, 0.0)
UPPER_RIGHT_WALL = (6.1, 4.65, 0.0)
FIXTURE_CAMERA = (4.0, 3.45, 9.0)

CASES = {
    "gameplay.approach_and_break": "Fly toward the building, aim, and break the lower wall.",
    "gameplay.projectile_impacts": "Fire both projectile shapes and observe contacts and bond fractures.",
}


def position(observed):
    return observed["translation"]


def fly(game, key, distance=0.15):
    before = camera_state(game)
    yaw, pitch = camera_angles(before)
    forward = (-math.sin(yaw) * math.cos(pitch), math.sin(pitch), -math.cos(yaw) * math.cos(pitch))
    right = (math.cos(yaw), 0.0, -math.sin(yaw))
    direction = {
        "KeyW": forward,
        "KeyA": tuple(-value for value in right),
        "KeyS": tuple(-value for value in forward),
        "KeyD": right,
    }[key]

    def travel(observed):
        return tuple(end - start for end, start in zip(position(observed), position(before), strict=True))

    def along(displacement):
        return sum(offset * axis for offset, axis in zip(displacement, direction, strict=True))

    with game.hold_key(key):
        held = wait_for_state(game, lambda observed: along(travel(observed)) >= distance,
                              f"{key} to move the camera at least {distance} m", read=camera_state)
    released = camera_state(game)
    time.sleep(0.05)
    stopped = camera_state(game)
    assert_that(math.dist(position(released), position(stopped)) <= POSITION_TOLERANCE,
                f"Camera continued moving after release of {key}")
    displacement = travel(stopped)
    projected = tuple(along(displacement) * axis for axis in direction)
    assert_that(math.dist(displacement, projected) <= POSITION_TOLERANCE,
                f"{key} moved the camera away from the intended axis")
    return {"before": position(before), "held": position(held), "released": position(stopped)}


def observe_impact(game, launch):
    before = launch["before"]
    contact_count = f"{launch['shape']}_contacts"
    return wait_for_state(
        game,
        lambda observed: (
            observed[contact_count] > before[contact_count]
            and observed["broken_bonds"] > before["broken_bonds"]
        ),
        f"a {launch['shape']} contact and structural bond fracture at {launch['target']}",
    )


def exercise(game, scenario, options):
    with game.case("gameplay.approach_and_break") as evidence:
        evidence["initial"] = wait_for_building(game, scenario)
        evidence["fixture"] = place_camera(game, FIXTURE_CAMERA)
        evidence["flight"] = fly(game, "KeyW", distance=0.5)
        evidence["intact_png"] = capture_scene(game, "01-intact-rectangle.png")
        evidence["left_click"] = break_wall(game, LOWER_WALL)
        evidence["breach_png"] = capture(game, "02-breached-rectangle.png")

    with game.case("gameplay.projectile_impacts") as evidence:
        sphere = evidence["sphere"] = launch_projectile(game, "sphere", UPPER_LEFT_WALL)
        sphere["impact"] = observe_impact(game, sphere)
        cube = evidence["cube"] = launch_projectile(game, "cube", UPPER_RIGHT_WALL)
        cube["impact"] = observe_impact(game, cube)
        evidence["damaged_png"] = capture(game, "03-projectile-damage.png")


if __name__ == "__main__":
    from run import main

    raise SystemExit(main("gameplay"))
