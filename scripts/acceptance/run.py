"""Run the local windowless Bevy Destruction acceptance suites."""

import argparse
from dataclasses import dataclass
import json
import math
import os
from pathlib import Path
import struct
from subprocess import TimeoutExpired
import tempfile

from bevy_acceptance import Game, assert_that, run_suite
from bevy_acceptance.brp import BrpError, PORT


ROOT = Path(__file__).resolve().parents[2]
BINARY = "bevy-destruction-acceptance.exe" if os.name == "nt" else "bevy-destruction-acceptance"
SCENES = ("rectangle", "highrise_slab", "highrise_l", "bridge", "courtyard", "l_shape")
PROBE_SCENES = {
    "smoke": ("rectangle", "l_shape", "courtyard"),
    "physics": ("rectangle", "l_shape", "courtyard"),
    "stress": SCENES,
    "material": ("rectangle",),
    "contact": ("rectangle",),
    "force": ("rectangle",),
    "landing": ("highrise_slab",),
    "collapse": ("rectangle",),
}
SCREENSHOT_COUNTS = {
    "smoke": 12,
    "physics": 3,
    "stress": 7,
    "material": 4,
    "contact": 40,
    "force": 11,
    "landing": 4,
    "collapse": 4,
}
SCREENSHOT_SIZES = {"material": (512, 256), "contact": (512, 512)}
VIEWPORT = (1920, 1080)
FLAGS = ("no_stress", "no_vfx", "no_detail", "no_soft_particles")
METRICS = "bevy_destruction::diagnostics::RunMetrics"
MIN_PROFILE_SAMPLES = 30
NATIVE_CAMERA = "bevy_destruction::scene::DemoCamera"
NATIVE_TIMEOUT = 180
PROBE_TIMEOUT = 600


class AcceptanceGame(Game):
    def close(self):
        try:
            return super().close()
        except (ConnectionResetError, ConnectionAbortedError) as error:
            # Closing the logical window can terminate the child before its HTTP reply arrives.
            try:
                exit_code = self.process.wait(timeout=1)
            except TimeoutExpired:
                raise error from None
            if exit_code != 0:
                raise
            return super().close()


@dataclass(frozen=True)
class Scenario:
    suite: str
    scene: str | None = None
    asset: str | None = None

    @property
    def name(self):
        return self.scene if self.scene is not None else "custom-asset"

    def command_args(self, options, output):
        args = ["--scene", self.scene] if self.scene is not None else ["--asset", self.asset]
        if self.suite in PROBE_SCENES or self.suite == "profile":
            probe = "smoke" if self.suite == "profile" else self.suite
            args.extend(["--probe", probe, "--probe-output", str(output)])
        for flag in FLAGS:
            if getattr(options, flag):
                args.append("--" + flag.replace("_", "-"))
        if options.capture_replay:
            args.append("--capture-replay")
        if self.suite == "profile":
            args.append("--profile")
        return args


def scenarios(options):
    if options.suite in ("all", "baseline"):
        probes = [
            Scenario(suite, scene) for suite, scenes in PROBE_SCENES.items() for scene in scenes
        ]
        if options.suite == "all":
            return [Scenario("controls", "rectangle"), Scenario("gameplay", "rectangle"), *probes]
        return probes
    if options.asset is not None:
        return [Scenario(options.suite, asset=options.asset)]
    default_scene = PROBE_SCENES.get(options.suite, ("rectangle",))[0]
    return [Scenario(options.suite, options.scene or default_scene)]


def read_profile(game, timeout):
    def sample():
        if game.process.poll() is not None:
            return {"exited": True}
        try:
            metrics = game.resource(METRICS)
        except (BrpError, OSError):
            if game.process.poll() is not None:
                return {"exited": True}
            raise
        return metrics if metrics["wall_frame"]["count"] >= MIN_PROFILE_SAMPLES else None

    return game.wait(sample, f"{MIN_PROFILE_SAMPLES} wall-clock timing samples over BRP", timeout=timeout)


def check_screenshots(directory, suite, evidence):
    screenshots = sorted(
        path for path in directory.glob("*.png") if not path.name.startswith("rejected-")
    )
    evidence["screenshots"] = [path.name for path in screenshots]
    assert_that(len(screenshots) == SCREENSHOT_COUNTS[suite],
                f"Expected {SCREENSHOT_COUNTS[suite]} screenshots, found {len(screenshots)}")
    expected_size = SCREENSHOT_SIZES.get(suite, VIEWPORT)
    for path in screenshots:
        with path.open("rb") as png:
            header = png.read(24)
        assert_that(
            len(header) == 24 and header[:8] == b"\x89PNG\r\n\x1a\n" and header[12:16] == b"IHDR",
            f"Missing PNG header in {path.name}",
        )
        width, height = struct.unpack(">II", header[16:24])
        assert_that((width, height) == expected_size,
                    f"{path.name} is {width}×{height}, expected {expected_size[0]}×{expected_size[1]}")


def read_probe_report(directory, evidence):
    path = directory / "report.json"
    if not path.is_file():
        return None
    evidence["probe_report"] = path.name
    report = json.loads(path.read_text(encoding="utf-8"))
    evidence["errors"] = report.get("errors")
    return report


def exercise_probe(game, suite, capture_replay, timeout):
    with game.case(suite) as evidence:
        profile = read_profile(game, timeout) if suite == "profile" else None
        game.wait(lambda: game.process.poll() is not None, "Rust probe completion", timeout=timeout)
        exit_code = game.process.returncode
        evidence["exit_code"] = exit_code
        report = read_probe_report(game.output, evidence)
        assert_that(exit_code == 0, f"Rust probe exited with code {exit_code}")
        assert_that(report is not None, "Rust probe did not write report.json")
        assert_that(report.get("errors") == [], f"Rust probe reported errors {report.get('errors')!r}")

        if profile is not None:
            assert_that(not profile.get("exited"), "Probe exited before supplying timing samples over BRP")
            evidence["brp_sample"] = profile
            observed = profile["wall_frame"]
            total_ms = observed["total_ms"]
            assert_that(math.isfinite(total_ms) and total_ms > 0,
                        "BRP must report positive wall-clock time")
            wall = report["metrics"]["wall_frame"]
            average_ms = wall["average_ms"]
            assert_that(wall["samples"] >= observed["count"],
                        "Final profile report contains fewer samples than the BRP snapshot")
            assert_that(math.isfinite(average_ms) and average_ms > 0,
                        "Profile must report a positive wall-clock frame duration")
            evidence.update(
                brp_fps=observed["count"] * 1000 / total_ms,
                offscreen_fps=1000 / average_ms,
                wall_frame=wall,
            )
            return

        check_screenshots(game.output, suite, evidence)
        if suite == "collapse":
            assert_that((game.output / "damage.json").is_file(), "Missing collapse damage.json")
            evidence["damage"] = "damage.json"
        if capture_replay:
            frames = sum(1 for _ in (game.output / "replay").glob("frame-*.png"))
            assert_that(frames > 0 and frames == report["replay_frames"], "Incomplete smoke replay")
            evidence["replay_frames"] = frames


def run_scenario(scenario, options, output):
    suite = scenario.suite
    native_input = suite in ("controls", "gameplay")
    if native_input:
        from gameplay import CONTROL_CASES, GAMEPLAY_CASES, exercise_controls, exercise_gameplay

        cases, run_checks = {
            "controls": (CONTROL_CASES, exercise_controls),
            "gameplay": (GAMEPLAY_CASES, exercise_gameplay),
        }[suite]
    else:
        cases = {suite: "The Rust probe completes and writes its report and expected artifacts."}

        def run_checks(game):
            exercise_probe(game, suite, options.capture_replay, options.timeout or PROBE_TIMEOUT)

    def exercise(game):
        window = game.session["viewport"]
        actual = (window["physical_width"], window["physical_height"])
        assert_that(actual == VIEWPORT, f"Acceptance window has size {actual}, expected {VIEWPORT}")
        run_checks(game)

    report = {"checks": {
        name: {"expected": expected, "status": "not_run", "evidence": {}}
        for name, expected in cases.items()
    }}
    args = argparse.Namespace(exe=options.exe, output=output, timeout=options.timeout or NATIVE_TIMEOUT)
    game = AcceptanceGame(suite, args, report, cwd=ROOT, port=options.port,
                          command_args=scenario.command_args(options, output), camera_component=NATIVE_CAMERA)
    run_suite(game, exercise)
    if not native_input and report["checks"][suite]["status"] == "not_run" and not report["passed"]:
        evidence = report["checks"][suite]["evidence"]
        try:
            read_probe_report(output, evidence)
        except (OSError, ValueError) as error:
            evidence["probe_report_error"] = str(error)
        if evidence:
            result = output / f"{suite}-acceptance.json"
            result.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
    return report


def parse_args():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "suite", nargs="?", default="all",
        choices=("all", "baseline", *PROBE_SCENES, "controls", "gameplay", "profile"),
        help="all runs the baseline matrix plus controls and gameplay. Run profile separately.",
    )
    source = parser.add_mutually_exclusive_group()
    source.add_argument("--scene", choices=SCENES)
    source.add_argument("--asset", help="Asset path relative to the Bevy assets directory")
    parser.add_argument("--exe", type=Path, default=ROOT / "target" / "debug" / BINARY)
    parser.add_argument("--output", type=Path, default=ROOT / "output" / "acceptance")
    parser.add_argument("--port", type=int, default=PORT)
    parser.add_argument(
        "--timeout", type=float,
        help=f"Timeout per wait in seconds, defaults to {NATIVE_TIMEOUT} for startup/native input and {PROBE_TIMEOUT} for scripted probes",
    )
    parser.add_argument("--capture-replay", action="store_true")
    for flag in FLAGS:
        parser.add_argument("--" + flag.replace("_", "-"), action="store_true")
    args = parser.parse_args()
    if not 1 <= args.port <= 65535:
        parser.error("--port must be in 1..65535")
    if args.timeout is not None and (not math.isfinite(args.timeout) or args.timeout <= 0):
        parser.error("--timeout must be finite and positive")
    if args.capture_replay and args.suite != "smoke":
        parser.error("--capture-replay requires the smoke suite")
    if args.suite not in (*PROBE_SCENES, "profile"):
        if args.scene is not None or args.asset is not None or any(getattr(args, flag) for flag in FLAGS):
            parser.error("scene, asset and --no-* overrides require a single Rust probe or profile")
    args.exe = args.exe.resolve()
    args.output = args.output.resolve()
    return args


def main():
    options = parse_args()
    options.output.mkdir(parents=True, exist_ok=True)
    run_dir = Path(tempfile.mkdtemp(prefix="run-", dir=options.output))
    results = []
    selected = scenarios(options)
    for scenario in selected:
        relative_output = Path(scenario.suite) / scenario.name
        print(f"Running {scenario.suite} on {scenario.scene or scenario.asset}", flush=True)
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
