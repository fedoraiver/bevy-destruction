"""Build the same settings file the game reads during normal startup."""

import ctypes
import json
import os
from pathlib import Path
import sys
from uuid import UUID


APP_NAME = "io.github.fedoraiver.bevy-destruction"


def windows_preferences_dir():
    folder = (ctypes.c_ubyte * 16).from_buffer_copy(
        UUID("f1b32785-6fba-4fcf-9d55-7b8e7f157091").bytes_le
    )
    get_path = ctypes.windll.shell32.SHGetKnownFolderPath
    get_path.argtypes = (ctypes.c_void_p, ctypes.c_uint32, ctypes.c_void_p,
                         ctypes.POINTER(ctypes.c_wchar_p))
    get_path.restype = ctypes.c_long
    free = ctypes.windll.ole32.CoTaskMemFree
    free.argtypes = (ctypes.c_void_p,)
    free.restype = None
    result = ctypes.c_wchar_p()
    status = get_path(ctypes.byref(folder), 0, None, ctypes.byref(result))
    try:
        if status != 0 or not result.value:
            raise OSError(f"Could not resolve LocalAppData: HRESULT {status & 0xffffffff:#010x}")
        return Path(result.value)
    finally:
        free(result)


def settings_path():
    if sys.platform == "win32":
        directory = windows_preferences_dir()
    elif sys.platform == "darwin":
        directory = Path.home() / "Library" / "Preferences"
    elif sys.platform == "linux":
        directory = Path(os.environ.get("XDG_CONFIG_HOME", ""))
        if not directory.is_absolute():
            directory = Path.home() / ".config"
    else:
        raise OSError(f"Bevy settings directory is unavailable on {sys.platform}")
    return directory / APP_NAME / "settings.toml"


class SettingsFixture:
    def __init__(self, path):
        self.path = path
        try:
            self.original = path.read_bytes()
        except FileNotFoundError:
            self.original = None

    def write(self, settings):
        self.path.parent.mkdir(parents=True, exist_ok=True)
        lines = ["[demo_settings]"]
        for key, value in settings.items():
            literal = json.dumps(value, ensure_ascii=False).replace("\x7f", r"\u007f")
            lines.append(f"{key} = {literal}")
        self.path.write_text("\n".join(lines) + "\n", encoding="utf-8")

    def restore(self):
        if self.original is None:
            self.path.unlink(missing_ok=True)
        else:
            self.path.write_bytes(self.original)
