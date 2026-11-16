"""Build an asset in Blender background mode; see --help for arguments."""

import argparse
import json
import sys
from pathlib import Path


def parse_args():
    args = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else sys.argv[1:]
    parser = argparse.ArgumentParser(description="Build and export a Bevy destruction building asset.")
    parser.add_argument("--spec", required=True, type=Path, help="Building specification JSON.")
    parser.add_argument("--output", required=True, type=Path, help="Directory for GLB and manifest JSON.")
    parser.add_argument("--save-blend", type=Path, help="Optional .blend file containing the generated preview.")
    return parser.parse_args(args)


def main():
    args = parse_args()
    package_directory = str(Path(__file__).resolve().parent)
    if package_directory not in sys.path:
        sys.path.insert(0, package_directory)

    import bpy
    from bevy_destruction.pipeline import build_asset
    from bevy_destruction.spec import load_spec

    if not bpy.app.background:
        raise RuntimeError("This entry point requires Blender --background; use the add-on panel interactively.")
    spec = load_spec(args.spec.expanduser().resolve())
    result = build_asset(spec, output_dir=args.output.expanduser().resolve(), preview=True)
    if args.save_blend is not None:
        blend_path = args.save_blend.expanduser().resolve()
        blend_path.parent.mkdir(parents=True, exist_ok=True)
        status = bpy.ops.wm.save_as_mainfile(filepath=str(blend_path))
        if "FINISHED" not in status:
            raise RuntimeError(f"Could not save Blender file: {blend_path}")
        result["blend_path"] = str(blend_path)
    print(json.dumps(result, ensure_ascii=False, indent=2), flush=True)


if __name__ == "__main__":
    main()
