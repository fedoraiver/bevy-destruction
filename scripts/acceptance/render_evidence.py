"""Read sampled RGB evidence from SDK screenshots without optional image libraries."""

import struct
import zlib
from pathlib import Path

from bevy_acceptance.driver import assert_that


def region(screenshot, bounds, *, step=4):
    """Sample a proportional viewport rectangle as RGB triples."""
    png = Path(screenshot["path"]).read_bytes()
    assert_that(png.startswith(b"\x89PNG\r\n\x1a\n"), "Invalid screenshot PNG")
    width, height = screenshot["dimensions"]
    blocks = bytearray()
    cursor = 8
    while cursor < len(png):
        size, kind = struct.unpack_from(">I4s", png, cursor)
        if kind == b"IDAT":
            blocks.extend(png[cursor + 8:cursor + 8 + size])
        cursor += size + 12
    raw = zlib.decompress(blocks)
    stride = width * 4 + 1
    assert_that(len(raw) == height * stride and not any(raw[::stride]),
                "Expected the SDK's unfiltered RGBA8 screenshot rows")
    left, top, right, bottom = bounds
    return [
        tuple(raw[offset:offset + 3])
        for y in range(round(top * height), round(bottom * height), step)
        for x in range(round(left * width), round(right * width), step)
        for offset in (1 + y * stride + x * 4,)
    ]


def color_spread(pixels):
    """Reject screenshots containing only one color in the measured region."""
    return max(sum(pixel) for pixel in pixels) - min(sum(pixel) for pixel in pixels)


def compare(reference, current):
    errors = [
        sum(abs(a - b) for a, b in zip(before, after, strict=True))
        for before, after in zip(reference, current, strict=True)
    ]
    return {
        "mean_rgb_error": sum(errors) / (3 * len(errors)),
        "changed_pixel_fraction": sum(delta > 36 for delta in errors) / len(errors),
    }
