"""Drive the rectangle demo through native keyboard and mouse input."""

import math
from pathlib import Path
import zlib

from bevy_acceptance.driver import TYPES, assert_that


GAMEPLAY_STATE = "bevy_destruction::acceptance::GameplayState"
RECTANGLE = "buildings/rectangle.destruction.json"
LOOK_SENSITIVITY = 0.002
ANGLE_TOLERANCE = 0.002
POSITION_TOLERANCE = 0.003
BREAK_LIMIT = 6
VIEWPORT = [1920, 1080]

# Interior points on the rectangle's front collision hulls and exterior mesh.
LOWER_WALL = (6.2, 1.55, 0.0)
UPPER_LEFT_WALL = (1.5, 4.65, 0.0)
UPPER_RIGHT_WALL = (6.1, 4.6, 0.0)
FIXTURE_CAMERA = (4.0, 3.45, 9.0)

CONTROL_CASES = {
    "controls.wasd": "W, A, S and D move the camera in the expected directions and stop on release.",
    "controls.mouse_look": "Relative mouse motion turns the camera in yaw and pitch and can be reversed.",
    "controls.left_click": "A left click removes a limited group of chunks from the targeted wall.",
    "controls.right_click": "A right click spawns one spherical rigid projectile.",
    "controls.key_r": "R spawns one cubical rigid projectile.",
}

GAMEPLAY_CASES = {
    "gameplay.approach_and_break": "From a BRP camera fixture, fly toward the building, aim and break the lower wall.",
    "gameplay.projectile_impacts": "Aim at two intact upper wall panels, fire both projectile shapes and observe structural impacts.",
}


def state(game):
    observed = game.resource(GAMEPLAY_STATE)
    error = observed["load_error"]
    assert_that(error is None, f"Could not load {observed['asset']}: {error}")
    return observed


def wait_for_state(game, predicate, description):
    def sample():
        observed = state(game)
        return observed if predicate(observed) else None

    return game.wait(sample, description)


def wait_for_rectangle(game, previous_building=None):
    return wait_for_state(
        game,
        lambda observed: (
            observed["asset"] == RECTANGLE
            and observed["ready"]
            and observed["camera_initialized"]
            and observed["building"] != previous_building
        ),
        "the rectangle building and camera to be ready",
    )


def reset_rectangle(game):
    before = wait_for_rectangle(game)
    game.tap("F1")
    after = wait_for_rectangle(game, previous_building=before["building"])
    assert_that(after["removed_chunks"] == 0 and after["launched"] == 0,
                "Reloaded rectangle retained destruction or projectiles")
    return after


def position(observed):
    return observed["translation"]


def camera_angles(observed):
    x, y, z, w = observed["rotation"]
    forward_x = -2.0 * (x * z + w * y)
    forward_y = 2.0 * (w * x - y * z)
    forward_z = 2.0 * (x * x + y * y) - 1.0
    return (
        math.atan2(-forward_x, -forward_z),
        math.atan2(forward_y, math.hypot(forward_x, forward_z)),
    )


def angle_difference(target, current):
    return math.remainder(target - current, math.tau)


def facing(observed, yaw, pitch):
    actual_yaw, actual_pitch = camera_angles(observed)
    return (
        abs(angle_difference(yaw, actual_yaw)) < ANGLE_TOLERANCE
        and abs(pitch - actual_pitch) < ANGLE_TOLERANCE
    )


def look_by(game, delta):
    before = state(game)
    yaw, pitch = camera_angles(before)
    game.mouse_motion(delta)
    return wait_for_state(
        game,
        lambda observed: (
            observed["frame"] > before["frame"]
            and facing(
                observed,
                yaw - delta[0] * LOOK_SENSITIVITY,
                pitch - delta[1] * LOOK_SENSITIVITY,
            )
        ),
        f"camera to consume relative mouse motion {delta}",
    )


def aim_at(game, point):
    before = state(game)
    dx, dy, dz = (target - origin for target, origin in zip(point, position(before), strict=True))
    target_yaw = math.atan2(-dx, -dz)
    target_pitch = math.atan2(dy, math.hypot(dx, dz))
    yaw, pitch = camera_angles(before)
    delta = (
        -angle_difference(target_yaw, yaw) / LOOK_SENSITIVITY,
        (pitch - target_pitch) / LOOK_SENSITIVITY,
    )
    return look_by(game, delta)


def fly(game, key, distance=0.15):
    before = state(game)
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
                              f"{key} to move the camera at least {distance} m")
    released = state(game)
    stopped = wait_for_state(game, lambda observed: observed["frame"] >= released["frame"] + 2,
                             f"two application frames after releasing {key}")
    assert_that(math.dist(position(released), position(stopped)) <= POSITION_TOLERANCE,
                f"Camera continued moving after release of {key}")
    displacement = travel(stopped)
    projected = tuple(along(displacement) * axis for axis in direction)
    assert_that(math.dist(displacement, projected) <= POSITION_TOLERANCE,
                f"{key} moved the camera away from the intended axis")
    return {"before": position(before), "held": position(held), "released": position(stopped)}


def break_wall(game, point):
    before = aim_at(game, point)
    with game.hold_button("Left"):
        pass
    after = wait_for_state(game, lambda observed: observed["removed_chunks"] > before["removed_chunks"],
                           "a native left click to remove wall chunks")
    removed = after["removed_chunks"] - before["removed_chunks"]
    assert_that(1 <= removed <= BREAK_LIMIT, f"One click removed {removed} chunks")
    assert_that(after["live_bonds"] < before["live_bonds"], "Removed chunks retained their live bonds")
    return {"target": point, "removed": removed, "before": before, "after": after}


def launch_projectile(game, shape, point):
    before = aim_at(game, point)
    if shape == "sphere":
        with game.hold_button("Right"):
            pass
    else:
        game.tap("KeyR")
    launched = wait_for_state(game, lambda observed: observed["launched"] > before["launched"],
                              f"one new {shape} projectile")
    assert_that(launched["launched"] == before["launched"] + 1, "One input launched multiple projectiles")
    count = f"{shape}s"
    assert_that(launched[count] == before[count] + 1, f"New projectile has no {shape} collider")
    return {"shape": shape, "target": point, "before": before, "launched": launched}


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


def place_camera(game, observed, translation):
    assert_that(observed["camera"] is not None, "The initialized scene has no camera entity")
    game.call("world.mutate_components", {
        "entity": observed["camera"],
        "component": TYPES["transform"],
        "path": "translation",
        "value": list(translation),
    })
    return wait_for_state(
        game,
        lambda sample: (
            sample["frame"] > observed["frame"]
            and sample["camera_initialized"]
            and math.dist(position(sample), translation) <= POSITION_TOLERANCE
        ),
        "the gameplay camera to observe the BRP fixture position",
    )


def capture(game, filename):
    screenshot = game.screenshot(filename)
    assert_that(screenshot["dimensions"] == VIEWPORT, f"{filename} did not capture the 1920×1080 viewport")
    return screenshot


def scene_is_visible(screenshot):
    # The pinned SDK encodes screenshots as RGBA8 PNGs with unfiltered rows.
    png = Path(screenshot["path"]).read_bytes()
    offset = 8
    image = bytearray()
    while offset < len(png):
        length = int.from_bytes(png[offset:offset + 4], "big")
        if png[offset + 4:offset + 8] == b"IDAT":
            image.extend(png[offset + 8:offset + 8 + length])
        offset += length + 12
    rows = zlib.decompress(image)
    width, height = screenshot["dimensions"]
    stride = 4 * width + 1
    assert_that(len(rows) == height * stride and not any(rows[::stride]),
                "Expected unfiltered RGBA8 screenshot rows from the SDK")

    background = rows[1:4]
    # Sample the upper center above both the crosshair and the HUD. The scene
    # can be ECS-ready while the render pipeline still draws only the HUD.
    changed = 0
    for y in range(height // 8, height // 2 - 16, 32):
        for x in range(width // 4, width * 3 // 4, 32):
            pixel = y * stride + 1 + x * 4
            changed += rows[pixel:pixel + 3] != background
    return changed >= 16


def capture_intact(game):
    def sample():
        screenshot = capture(game, "01-intact-rectangle.png")
        return screenshot if scene_is_visible(screenshot) else None

    return game.wait(sample, "the intact rectangle geometry to appear in the capture")


def exercise_controls(game):
    with game.case("controls.wasd") as evidence:
        wait_for_rectangle(game)
        for key in ("KeyW", "KeyA", "KeyS", "KeyD"):
            evidence[key] = fly(game, key)

    with game.case("controls.mouse_look") as evidence:
        before = state(game)
        yaw, pitch = camera_angles(before)
        evidence["before"] = before
        evidence["turned"] = look_by(game, (125.0, -75.0))
        evidence["restored"] = look_by(game, (-125.0, 75.0))
        assert_that(facing(evidence["restored"], yaw, pitch), "Mouse motion did not restore the original aim")
        assert_that(math.dist(position(before), position(evidence["restored"])) <= POSITION_TOLERANCE,
                    "Mouse motion translated the camera")

    with game.case("controls.left_click") as evidence:
        reset_rectangle(game)
        evidence.update(break_wall(game, LOWER_WALL))

    with game.case("controls.right_click") as evidence:
        reset_rectangle(game)
        evidence.update(launch_projectile(game, "sphere", UPPER_LEFT_WALL))

    with game.case("controls.key_r") as evidence:
        reset_rectangle(game)
        evidence.update(launch_projectile(game, "cube", UPPER_RIGHT_WALL))


def exercise_gameplay(game):
    with game.case("gameplay.approach_and_break") as evidence:
        evidence["initial"] = wait_for_rectangle(game)
        evidence["fixture"] = place_camera(game, evidence["initial"], FIXTURE_CAMERA)
        evidence["flight"] = fly(game, "KeyW", distance=0.5)
        evidence["intact_png"] = capture_intact(game)
        evidence["left_click"] = break_wall(game, LOWER_WALL)
        evidence["breach_png"] = capture(game, "02-breached-rectangle.png")

    with game.case("gameplay.projectile_impacts") as evidence:
        sphere = evidence["sphere"] = launch_projectile(game, "sphere", UPPER_LEFT_WALL)
        sphere["impact"] = observe_impact(game, sphere)
        cube = evidence["cube"] = launch_projectile(game, "cube", UPPER_RIGHT_WALL)
        cube["impact"] = observe_impact(game, cube)
        evidence["damaged_png"] = capture(game, "03-projectile-damage.png")
