"""Sample native frame durations from the continuously running offscreen game."""

import math
from statistics import fmean, median
import time

from bevy_acceptance import assert_that

from runtime import resource, wait_for_building


CASES = {
    "profile": "Collect wall-clock frame samples from Bevy after the scene is ready.",
}
SAMPLE_COUNT = 30


def exercise(game, scenario, options):
    with game.case("profile") as evidence:
        evidence["initial"] = wait_for_building(game, scenario)
        frame_durations = []
        observed = resource(game, "time")["elapsed_secs_f64"]
        start = time.monotonic()

        def collect():
            nonlocal observed
            clock = resource(game, "time")
            now = clock["elapsed_secs_f64"]
            if now > observed:
                ms = clock["delta_secs_f64"] * 1000
                assert_that(math.isfinite(ms) and ms > 0, f"Invalid frame duration {ms}")
                frame_durations.append(ms)
                observed = now
            return len(frame_durations) == SAMPLE_COUNT

        game.wait(collect, f"{SAMPLE_COUNT} distinct frame duration samples")
        average_ms = fmean(frame_durations)
        evidence.update(
            sample_wall_seconds=time.monotonic() - start,
            offscreen_fps=1000.0 / average_ms,
            wall_frame={
                "samples": len(frame_durations),
                "average_ms": average_ms,
                "median_ms": median(frame_durations),
                "maximum_ms": max(frame_durations),
            },
        )


if __name__ == "__main__":
    from run import main

    raise SystemExit(main("profile"))
