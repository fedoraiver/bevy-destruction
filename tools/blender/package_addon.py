"""Create a legacy Blender add-on ZIP without caches or generated assets."""

import argparse
import zipfile
from pathlib import Path


def package_addon(destination=None):
    directory = Path(__file__).resolve().parent
    source = directory / "bevy_destruction"
    destination = Path(destination) if destination else directory.parents[1] / "output" / "blender" / "bevy_destruction-0.2.0.zip"
    destination = destination.expanduser().resolve()
    files = sorted(source.rglob("*.py"))
    required = {"__init__.py", "ui.py", "spec.py", "pipeline.py"}
    missing = required - {path.name for path in files if path.parent == source}
    if missing:
        raise FileNotFoundError(f"Add-on is incomplete; missing: {', '.join(sorted(missing))}")
    destination.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(destination, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        for path in files:
            if "__pycache__" not in path.parts:
                archive.write(path, (Path("bevy_destruction") / path.relative_to(source)).as_posix())
    return destination


def main():
    parser = argparse.ArgumentParser(description="Package the Bevy Destruction Blender add-on.")
    parser.add_argument("--output", type=Path, help="ZIP path; default: output/blender/bevy_destruction-0.2.0.zip")
    args = parser.parse_args()
    print(package_addon(args.output))


if __name__ == "__main__":
    main()
