"""Bake the physics catalog in Blender background mode using checked-in specs."""

import argparse
import json
from pathlib import Path
import sys


def main():
    arguments = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=Path(__file__).resolve().parents[2] / "assets" / "buildings")
    parser.add_argument("--asset", choices=("highrise_slab", "highrise_l", "bridge"), action="append")
    args = parser.parse_args(arguments)
    root = Path(__file__).resolve().parent
    sys.path.insert(0, str(root))
    import bpy
    from bevy_destruction.pipeline import build_asset
    from bevy_destruction.spec import load_spec

    if not bpy.app.background:
        raise RuntimeError("Use Blender --background for catalog baking")
    for name in args.asset or ("highrise_slab", "highrise_l", "bridge"):
        result = build_asset(load_spec(root / "examples" / (name + ".json")),
                             output_dir=args.output.expanduser().resolve(), preview=False)
        print(json.dumps(result, indent=2), flush=True)


if __name__ == "__main__":
    main()
