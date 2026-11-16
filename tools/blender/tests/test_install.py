"""Install the packaged add-on; run only with BLENDER_USER_SCRIPTS set to a test directory."""

import os
from pathlib import Path

import addon_utils
import bpy


root = Path(__file__).resolve().parents[3]
scripts = os.environ.get("BLENDER_USER_SCRIPTS")
if not scripts:
    raise RuntimeError("Set BLENDER_USER_SCRIPTS to an isolated test directory before running this check")
scripts = Path(scripts).resolve()
actual = Path(bpy.utils.user_resource("SCRIPTS", create=True)).resolve()
if actual != scripts:
    raise RuntimeError(f"Refusing installation outside the test directory: {actual}")

package = root / "output/blender/bevy_destruction-0.2.0.zip"
result = bpy.ops.preferences.addon_install(filepath=str(package))
assert "FINISHED" in result
module = addon_utils.enable("bevy_destruction", default_set=False, persistent=False)
assert module is not None
assert Path(module.__file__).resolve().is_relative_to(scripts)
assert hasattr(bpy.types.Scene, "bd_authoring")
assert bpy.ops.bd.generate.get_rna_type().name
addon_utils.disable("bevy_destruction", default_set=False)
assert not hasattr(bpy.types.Scene, "bd_authoring")
print("PASS: packaged add-on installs, registers and unregisters in isolated scripts directory")
