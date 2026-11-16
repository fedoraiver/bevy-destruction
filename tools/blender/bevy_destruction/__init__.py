"""Blender authoring tools for Bevy destruction assets."""

bl_info = {
    "name": "Bevy Destruction Authoring",
    "author": "Bevy Destruction contributors",
    "version": (0, 2, 0),
    "blender": (5, 2, 0),
    "location": "View3D > Sidebar > Destruction",
    "description": "Build and export prefractured buildings for Bevy",
    "category": "Object",
}


def register():
    from . import ui

    ui.register()


def unregister():
    from . import ui

    ui.unregister()
