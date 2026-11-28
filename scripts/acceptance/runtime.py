"""Inspect and drive the production ECS through Bevy Remote Protocol."""

import math

from bevy_acceptance.driver import TYPES as BEVY_TYPES, assert_that

from render_evidence import color_spread, region


TYPES = {
    **BEVY_TYPES,
    "demo_settings": "bevy_destruction::DemoSettings",
    "demo_camera": "bevy_destruction::scene::DemoCamera",
    "free_camera": "bevy_destruction::camera::FreeCamera",
    "chunk_identity": "bevy_destruction::scene::ChunkIdentity",
    "bond": "bevy_destruction::scene::BondInstance",
    "removed_chunk": "bevy_destruction::scene::RemovedChunk",
    "cluster": "bevy_destruction::physics::Cluster",
    "text": "bevy_ui::widget::text::Text",
    "detail_stats": "bevy_destruction::scene::DetailStats",
    "projectile": "bevy_destruction::projectiles::RigidProjectile",
    "impact_stats": "bevy_destruction::projectiles::ImpactStats",
    "vfx_stats": "bevy_destruction::vfx::VfxStats",
    "time": "bevy_time::time::Time<bevy_time::virt::Virtual>",
}
VIEWPORT = [1920, 1080]
LOOK_SENSITIVITY = 0.002
ANGLE_TOLERANCE = 0.002
POSITION_TOLERANCE = 0.003
BREAK_LIMIT = 6


def resource(game, name):
    return game.resource(TYPES.get(name, name))


def rows(game, *names, with_=()):
    return game.query(*(TYPES.get(name, name) for name in names),
                      with_=tuple(TYPES.get(name, name) for name in with_))


def camera_state(game):
    cameras = rows(game, "transform", "free_camera", with_=("demo_camera",))
    assert_that(len(cameras) == 1, f"Expected one gameplay camera, observed {len(cameras)}")
    row, = cameras
    transform = row["components"][TYPES["transform"]]
    free_camera = row["components"][TYPES["free_camera"]]
    return {
        "camera": row["entity"],
        "camera_initialized": free_camera["initialized"],
        "translation": transform["translation"],
        "rotation": transform["rotation"],
    }


def snapshot(game):
    chunks = rows(game, "chunk_identity")
    buildings = {chunk["components"][TYPES["chunk_identity"]]["building"] for chunk in chunks}
    assert_that(len(buildings) <= 1, f"Expected one building, observed {len(buildings)}")
    building = next(iter(buildings), None)
    settings = resource(game, "demo_settings")
    camera = camera_state(game)
    if not chunks:
        for row in rows(game, "name", "text"):
            components = row["components"]
            if components[TYPES["name"]] == "Building status":
                hint = components[TYPES["text"]]
                hint = hint[0] if isinstance(hint, list) else hint
                assert_that(not hint.startswith("Building error:"), hint)

    impact = resource(game, "impact_stats")
    projectiles = [row["components"][TYPES["projectile"]]["shape"]
                   for row in rows(game, "projectile")]
    return {
        **camera,
        "asset": settings["scene"],
        "building": building,
        "chunks": len(chunks),
        "ready": camera["camera_initialized"] and bool(rows(game, with_=("cluster",))),
        "removed_chunks": len(rows(game, with_=("removed_chunk",))),
        "live_bonds": sum(row["components"][TYPES["bond"]]["active"] for row in rows(game, "bond")),
        "launched": impact["launched"],
        "contacts": impact["contacts"],
        "sphere_contacts": impact.get("sphere_contacts", 0),
        "cube_contacts": impact.get("cube_contacts", 0),
        "broken_bonds": impact["broken_bonds"],
        "spheres": projectiles.count("Sphere"),
        "cubes": projectiles.count("Cube"),
    }


def wait_for_state(game, predicate, description, read=snapshot):
    def sample():
        observed = read(game)
        return observed if predicate(observed) else None

    return game.wait(sample, description)


def wait_for_building(game, scenario, previous=None):
    return wait_for_state(
        game,
        lambda observed: (
            observed["asset"] == scenario.asset_path
            and observed["ready"]
            and observed["building"] != previous
        ),
        f"{scenario.asset_path} and the camera to be ready",
    )


def camera_angles(observed):
    x, y, z, w = observed["rotation"]
    forward_x = -2.0 * (x * z + w * y)
    forward_y = -2.0 * (y * z - w * x)
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
    yaw, pitch = camera_angles(camera_state(game))
    game.mouse_motion(delta)
    return wait_for_state(
        game,
        lambda observed: facing(
            observed, yaw - delta[0] * LOOK_SENSITIVITY, pitch - delta[1] * LOOK_SENSITIVITY,
        ),
        f"camera to consume relative mouse motion {delta}",
        read=camera_state,
    )


def aim_at(game, target):
    before = camera_state(game)
    dx, dy, dz = (point - origin for point, origin in zip(target, before["translation"], strict=True))
    target_yaw = math.atan2(-dx, -dz)
    target_pitch = math.atan2(dy, math.hypot(dx, dz))
    yaw, pitch = camera_angles(before)
    return look_by(game, (
        -angle_difference(target_yaw, yaw) / LOOK_SENSITIVITY,
        (pitch - target_pitch) / LOOK_SENSITIVITY,
    ))


def place_camera(game, translation, target=None):
    camera = camera_state(game)["camera"]
    game.call("world.mutate_components", {
        "entity": camera,
        "component": TYPES["transform"],
        "path": "translation",
        "value": list(translation),
    })
    wait_for_state(
        game, lambda observed: math.dist(observed["translation"], translation) <= POSITION_TOLERANCE,
        "the gameplay camera to observe the BRP fixture position", read=camera_state,
    )
    if target is not None:
        aim_at(game, target)
    return snapshot(game)


def break_wall(game, point):
    aim_at(game, point)
    before = snapshot(game)
    with game.hold_button("Left"):
        pass
    after = wait_for_state(game, lambda observed: observed["removed_chunks"] > before["removed_chunks"],
                           "a native left click to remove wall chunks")
    removed = after["removed_chunks"] - before["removed_chunks"]
    assert_that(1 <= removed <= BREAK_LIMIT, f"One click removed {removed} chunks")
    assert_that(after["live_bonds"] < before["live_bonds"], "Removed chunks retained their live bonds")
    return {"target": point, "removed": removed, "before": before, "after": after}


def launch_projectile(game, shape, point):
    aim_at(game, point)
    before = snapshot(game)
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


def capture(game, filename):
    screenshot = game.screenshot(filename)
    assert_that(screenshot["dimensions"] == VIEWPORT, f"{filename} did not capture the 1920x1080 viewport")
    return screenshot


def scene_is_visible(screenshot):
    # Ignore the crosshair and HUD when checking that scene geometry is visible.
    return color_spread(region(screenshot, (0.25, 0.125, 0.75, 0.485), step=32)) >= 16


def capture_scene(game, filename):
    def sample():
        screenshot = capture(game, filename)
        return screenshot if scene_is_visible(screenshot) else None

    return game.wait(sample, "scene geometry to appear in the capture")
