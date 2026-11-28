"""Verify native keyboard and mouse controls on the rectangle scene."""

from dataclasses import replace
from http.client import HTTPException
import math

from bevy_acceptance import assert_that
from bevy_acceptance.brp import BrpError

from gameplay import LOWER_WALL, UPPER_LEFT_WALL, UPPER_RIGHT_WALL, fly, position
from runtime import (
    POSITION_TOLERANCE, TYPES, break_wall, camera_angles, camera_state, facing, launch_projectile,
    look_by, resource, rows, snapshot, wait_for_building,
)
from smoke import front_faces


CASES = {
    "controls.wasd": "W, A, S and D move the camera in the expected directions and stop on release.",
    "controls.mouse_look": "Relative mouse motion turns the camera in yaw and pitch and can be reversed.",
    "controls.left_click": "A left click removes a limited group of chunks from the targeted wall.",
    "controls.right_click": "A right click spawns one spherical rigid projectile.",
    "controls.key_r": "R spawns one cubical rigid projectile.",
    "controls.scene_keys": "F1 and F2 leave the active scene in place.",
    "controls.resume": "Esc pauses gameplay, and the Resume button returns to play.",
    "controls.reload": "Reload preserves the scene on error, loads another scene, and resets the same scene.",
    "controls.quit": "The Quit button closes the game successfully.",
}
VISIBILITY = "bevy_camera::visibility::Visibility"


def reset_rectangle(game, scenario):
    previous = snapshot(game)["building"]
    open_menu(game)
    click_menu(game, "Reload")
    observed = wait_for_building(game, scenario, previous)
    assert_that(observed["removed_chunks"] == 0 and observed["launched"] == 0,
                "The reloaded rectangle retained destruction or projectiles")


def named_ui(game, label, *components):
    matches = [
        row for row in rows(game, "name", *components)
        if row["components"][TYPES["name"]] == label
    ]
    assert_that(len(matches) <= 1, f"More than one UI entity named {label}")
    return matches[0] if matches else None


def menu_visible(game):
    menu = named_ui(game, "Pause menu", VISIBILITY)
    return menu is not None and menu["components"][VISIBILITY] == "Visible"


def open_menu(game):
    game.tap("Escape")
    game.wait(lambda: menu_visible(game), "the pause menu to open")


def click_menu(game, label):
    def position():
        row = named_ui(game, label, "computed_node", "ui_global_transform")
        if row:
            components = row["components"]
            if min(components[TYPES["computed_node"]]["size"]) > 0:
                return components[TYPES["ui_global_transform"]][-2:]
        return None

    game.click_at(game.wait(position, f"the {label} button layout"))


def reload_status(game):
    row = named_ui(game, "Reload status", "text")
    if not row:
        return ""
    text = row["components"][TYPES["text"]]
    return text[0] if isinstance(text, list) else text


def reload_case(game, scenario, evidence):
    original = evidence["original"] = break_wall(game, LOWER_WALL)["after"]
    open_menu(game)
    missing = "buildings/acceptance-missing.destruction.json"
    game.settings.write({**scenario.settings(), "scene": missing})
    click_menu(game, "Reload")
    evidence["failed_reload_message"] = game.wait(
        lambda: (message if (message := reload_status(game)).startswith("Asset could not load") else None),
        "a load failure to appear in the reload status",
    )
    failed = evidence["after_failure"] = snapshot(game)
    assert_that(menu_visible(game) and failed["building"] == original["building"]
                and failed["asset"] == original["asset"]
                and failed["removed_chunks"] == original["removed_chunks"],
                "A failed reload did not preserve the paused scene and its damage")

    game.settings.path.write_text("[demo_settings\n", encoding="utf-8")
    click_menu(game, "Reload")
    evidence["malformed_settings_message"] = game.wait(
        lambda: (message if (message := reload_status(game)).startswith("Invalid settings.toml") else None),
        "the settings parse error to appear in the reload status",
    )
    invalid = evidence["after_malformed_settings"] = snapshot(game)
    assert_that(menu_visible(game) and invalid["building"] == original["building"]
                and invalid["removed_chunks"] == original["removed_chunks"],
                "Malformed settings reset the paused scene")

    alternate = replace(scenario, scene="l_shape", asset=None)
    game.settings.write(alternate.settings())
    click_menu(game, "Reload")
    loaded = evidence["reloaded_scene"] = wait_for_building(game, alternate, original["building"])
    assert_that(not menu_visible(game) and loaded["removed_chunks"] == 0 and loaded["launched"] == 0,
                "Successful reload retained damage, projectiles or the pause menu")

    evidence["damage_before_reset"] = break_wall(game, front_faces(alternate)[0])
    open_menu(game)
    click_menu(game, "Reload")
    reset = evidence["reset_scene"] = wait_for_building(game, alternate, loaded["building"])
    assert_that(not menu_visible(game) and reset["removed_chunks"] == 0 and reset["launched"] == 0,
                "Reload of the same scene did not reset gameplay")


def exercise(game, scenario, options):
    with game.case("controls.wasd") as evidence:
        wait_for_building(game, scenario)
        for key in ("KeyW", "KeyA", "KeyS", "KeyD"):
            evidence[key] = fly(game, key)

    with game.case("controls.mouse_look") as evidence:
        before = camera_state(game)
        yaw, pitch = camera_angles(before)
        evidence["before"] = before
        evidence["turned"] = look_by(game, (125.0, -75.0))
        evidence["restored"] = look_by(game, (-125.0, 75.0))
        assert_that(facing(evidence["restored"], yaw, pitch), "Mouse motion did not restore the original aim")
        assert_that(math.dist(position(before), position(evidence["restored"])) <= POSITION_TOLERANCE,
                    "Mouse motion translated the camera")

    with game.case("controls.left_click") as evidence:
        evidence.update(break_wall(game, LOWER_WALL))

    with game.case("controls.right_click") as evidence:
        reset_rectangle(game, scenario)
        evidence.update(launch_projectile(game, "sphere", UPPER_LEFT_WALL))

    with game.case("controls.key_r") as evidence:
        reset_rectangle(game, scenario)
        evidence.update(launch_projectile(game, "cube", UPPER_RIGHT_WALL))


    with game.case("controls.scene_keys") as evidence:
        initial = snapshot(game)
        for key in ("F1", "F2"):
            game.tap(key)
            observed = evidence[key] = snapshot(game)
            assert_that(observed["building"] == initial["building"] and observed["asset"] == initial["asset"],
                        f"{key} changed the active scene")

    with game.case("controls.resume") as evidence:
        open_menu(game)
        paused = evidence["paused"] = resource(game, "time")
        assert_that(paused["context"]["paused"], "Opening the pause menu did not freeze virtual time")
        camera = camera_state(game)
        with game.hold_key("KeyW"):
            held = camera_state(game)
        assert_that(held["translation"] == camera["translation"], "The camera moved while paused")
        click_menu(game, "Resume")
        game.wait(lambda: not menu_visible(game), "the Resume button to close the pause menu")
        resumed = evidence["resumed"] = resource(game, "time")
        assert_that(not resumed["context"]["paused"], "Resume left virtual time paused")
        open_menu(game)
        game.tap("Escape")
        game.wait(lambda: not menu_visible(game), "Escape to close the pause menu")

    with game.case("controls.reload") as evidence:
        reload_case(game, scenario, evidence)

    with game.case("controls.quit") as evidence:
        open_menu(game)
        try:
            click_menu(game, "Quit")
        except (BrpError, OSError, HTTPException):
            # The process may exit before the driver verifies button release.
            if game.process.poll() != 0:
                raise
        code = evidence["exit_code"] = game.process.wait(timeout=15)
        assert_that(code == 0, f"The Quit button exited with code {code}")


if __name__ == "__main__":
    from run import main

    raise SystemExit(main("controls"))
