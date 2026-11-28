"""Check that the acceptance runner restores the original game settings."""

from contextlib import redirect_stdout
from io import StringIO
from pathlib import Path
from tempfile import TemporaryDirectory
from types import SimpleNamespace
import tomllib
import unittest
from unittest.mock import patch

import run


class ChildProcess:
    def __init__(self):
        self.running = True

    def poll(self):
        return None if self.running else 0

    def wait(self, timeout):
        if self.running:
            raise AssertionError("Settings restored before the child stopped")
        return 0


class SettingsFixtureTests(unittest.TestCase):
    def check_failed_case(self, original, error, expected_status):
        with TemporaryDirectory() as directory:
            root = Path(directory)
            settings = root / "user" / "settings.toml"
            if original is not None:
                settings.parent.mkdir()
                settings.write_bytes(original)
            scenario = run.Scenario("smoke", "rectangle", vfx=False)
            events = []
            process = ChildProcess()

            def check_fixture():
                actual = tomllib.loads(settings.read_text(encoding="utf-8"))
                self.assertEqual(actual, {"demo_settings": scenario.settings()})

            def start(game):
                check_fixture()
                game.process = process
                game.window = 1
                game.session = {"viewport": {"physical_width": 1920, "physical_height": 1080}}
                events.append("start")

            def close(game):
                check_fixture()
                process.running = False
                events.append("close")

            def fail(game, scenario, options):
                raise error

            suite = SimpleNamespace(CASES={"smoke": "exercise"}, exercise=fail)
            options = run.parse_args("smoke", [])
            with (
                patch.object(run, "settings_path", return_value=settings),
                patch.object(run, "import_module", return_value=suite),
                patch.object(run.AcceptanceGame, "start", start),
                patch.object(run.AcceptanceGame, "close", close),
                redirect_stdout(StringIO()),
            ):
                report = run.run_scenario(scenario, options, root / "output")

            self.assertEqual(report["status"], expected_status)
            self.assertEqual(events, ["start", "close"])
            self.assertFalse(process.running)
            self.assertNotIn("cleanup_errors", report)
            if original is None:
                self.assertFalse(settings.exists())
            else:
                self.assertEqual(settings.read_bytes(), original)

    def test_failure_restores_original_bytes_after_close(self):
        original = b"# User options\r\n[demo_settings]\r\nvfx = true\r\n"
        self.check_failed_case(original, RuntimeError("Exercise failed"), "failed")

    def test_keyboard_interrupt_restores_missing_file_after_close(self):
        self.check_failed_case(None, KeyboardInterrupt(), "blocked")


if __name__ == "__main__":
    unittest.main()
