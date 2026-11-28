"""Run the local windowless Bevy Destruction acceptance suites."""

import argparse
from dataclasses import dataclass
from importlib import import_module
import json
import math
import os
from pathlib import Path
from subprocess import TimeoutExpired
import tempfile

from bevy_acceptance import Game, assert_that, run_suite
from bevy_acceptance.brp import PORT

from runtime import TYPES, VIEWPORT, resource
from settings_file import SettingsFixture, settings_path


ROOT = Path(__file__).resolve().parents[2]
BINARY = "bevy-destruction-acceptance.exe" if os.name == "nt" else "bevy-destruction-acceptance"
SCENES = ("rectangle", "highrise_slab", "highrise_l", "bridge", "courtyard", "l_shape")
BASELINE_SCENES = {
    "smoke": ("rectangle", "l_shape", "courtyard"),
    "physics": ("rectangle", "l_shape", "courtyard"),
    "stress": SCENES,
    "material": ("rectangle",),
    "contact": ("rectangle",),
    "force": ("rectangle",),
    "landing": ("highrise_slab",),
    "collapse": ("rectangle",),
}
DEFAULT_TIMEOUT = 180


class AcceptanceGame(Game):
    def __init__(self, scenario, args, report, *, port):
        environment = {"BEVY_DESTRUCTION_ACCEPTANCE_UNCAPPED": "1" if scenario.suite == "profile" else "0"}
        super().__init__(scenario.suite, args, report, cwd=ROOT, port=port, env=environment,
                         camera_component=TYPES["demo_camera"])
        self.scenario = scenario
        self.settings = None

    def prepare_settings(self):
        self.settings = SettingsFixture(settings_path())
        self.report["settings_file"] = str(self.settings.path)
        self.settings.write(self.scenario.settings())

    def check_startup(self):
        expected = self.scenario.settings()
        actual = resource(self, "demo_settings")
        assert_that(actual == expected, f"Game loaded settings {actual!r}, expected {expected!r}")

    def restart(self):
        self.close()
        self.start()

    def close(self):
        try:
            return super().close()
        except (ConnectionResetError, ConnectionAbortedError) as error:
            # The window may close before Bevy can send the HTTP acknowledgement.
            try:
                exit_code = self.process.wait(timeout=1)
            except TimeoutExpired:
                raise error from None
            if exit_code != 0:
                raise
            return super().close()

    def cleanup(self):
        errors = super().cleanup()
        if self.settings is not None:
            try:
                if self.process is not None and self.process.poll() is None:
                    raise RuntimeError("Game is still running")
                self.settings.restore()
            except (Exception, KeyboardInterrupt) as error:
                errors.append(f"Restoring settings: {type(error).__name__}: {error}")
        return errors


@dataclass(frozen=True)
class Scenario:
    suite: str
    scene: str | None = None
    asset: str | None = None
    stress: bool = True
    vfx: bool = True
    detail: bool = True
    soft_particles: bool = True

    @property
    def name(self):
        return self.scene if self.asset is None else "custom-asset"

    @property
    def asset_path(self):
        return self.asset if self.asset is not None else f"buildings/{self.scene}.destruction.json"

    def settings(self):
        return {
            "scene": self.asset_path,
            "stress": self.stress,
            "vfx": self.vfx,
            "detail": self.detail,
            "soft_particles": self.soft_particles,
        }


def baseline_scenario(suite, scene):
    return Scenario(suite, scene, vfx=suite not in ("material", "contact"))


def scenarios(options):
    if options.suite in ("all", "baseline"):
        baseline = [
            baseline_scenario(suite, scene) for suite, scenes in BASELINE_SCENES.items() for scene in scenes
        ]
        if options.suite == "all":
            return [Scenario("controls", "rectangle"), Scenario("gameplay", "rectangle"), *baseline]
        return baseline
    scene = BASELINE_SCENES.get(options.suite, ("rectangle",))[0]
    return [baseline_scenario(options.suite, scene)]


def run_scenario(scenario, options, output):
    suite = import_module(scenario.suite)

    def exercise(game):
        window = game.session["viewport"]
        actual = [window["physical_width"], window["physical_height"]]
        assert_that(actual == VIEWPORT, f"Acceptance window has size {actual}, expected {VIEWPORT}")
        suite.exercise(game, scenario, options)

    report = {
        "settings": scenario.settings(),
        "checks": {
            name: {"expected": expected, "status": "not_run", "evidence": {}}
            for name, expected in suite.CASES.items()
        },
    }
    args = argparse.Namespace(exe=options.exe, output=output, timeout=options.timeout or DEFAULT_TIMEOUT)
    game = AcceptanceGame(scenario, args, report, port=options.port)
    run_suite(game, exercise, prepare=game.prepare_settings)
    return report


def parse_args(suite=None, argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    if suite is None:
        parser.add_argument(
            "suite", nargs="?", default="all",
            choices=("all", "baseline", *BASELINE_SCENES, "controls", "gameplay", "profile"),
            help="all runs the baseline matrix plus controls and gameplay. Run profile separately.",
        )
    else:
        parser.set_defaults(suite=suite)
    parser.add_argument("--exe", type=Path, default=ROOT / "target" / "debug" / BINARY)
    parser.add_argument("--output", type=Path, default=ROOT / "output" / "acceptance")
    parser.add_argument("--port", type=int, default=PORT)
    parser.add_argument("--timeout", type=float, help=f"Timeout per wait in seconds (default {DEFAULT_TIMEOUT})")
    parser.add_argument("--capture-replay", action="store_true")
    args = parser.parse_args(argv)
    if not 1 <= args.port <= 65535:
        parser.error("--port must be in 1..65535")
    if args.timeout is not None and (not math.isfinite(args.timeout) or args.timeout <= 0):
        parser.error("--timeout must be finite and positive")
    if args.capture_replay and args.suite != "smoke":
        parser.error("--capture-replay requires the smoke suite")
    args.exe = args.exe.resolve()
    args.output = args.output.resolve()
    return args


def main(suite=None):
    options = parse_args(suite)
    options.output.mkdir(parents=True, exist_ok=True)
    run_dir = Path(tempfile.mkdtemp(prefix="run-", dir=options.output))
    results = []
    selected = scenarios(options)
    for scenario in selected:
        relative_output = Path(scenario.suite) / scenario.name
        print(f"Running {scenario.suite} on {scenario.asset_path}", flush=True)
        report = run_scenario(scenario, options, run_dir / relative_output)
        result = {
            "suite": scenario.suite,
            "scene": scenario.scene,
            "asset": scenario.asset,
            "status": report["status"],
            "report": (relative_output / f"{scenario.suite}-acceptance.json").as_posix(),
        }
        if scenario.suite == "profile" and report["passed"]:
            evidence = report["checks"]["profile"]["evidence"]
            fps = evidence["offscreen_fps"]
            samples = evidence["wall_frame"]["samples"]
            result.update(offscreen_fps=fps, samples=samples)
            print(f"Offscreen FPS {fps:.2f} over {samples} wall-clock frame samples", flush=True)
        results.append(result)
        if report.get("error") == "KeyboardInterrupt":
            break

    passed = len(results) == len(selected) and all(result["status"] == "passed" for result in results)
    summary = run_dir / "summary.json"
    summary.write_text(json.dumps({"passed": passed, "results": results}, indent=2) + "\n", encoding="utf-8")
    print(f"{'Passed' if passed else 'Failed'} acceptance run. Summary at {summary}")
    return 0 if passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
