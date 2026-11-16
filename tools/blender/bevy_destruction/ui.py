"""Interactive authoring panel; generation does not modify the input object."""

import json
import math
import traceback
from collections import Counter, defaultdict
from pathlib import Path

import bpy
from bpy.props import BoolProperty, EnumProperty, FloatProperty, IntProperty, PointerProperty, StringProperty
from bpy_extras.io_utils import ImportHelper

from .spec import BuildingSpec, load_spec


def _outline_object_poll(_settings, obj):
    return obj.type in {"MESH", "CURVE"}


def _area(loop):
    return sum(
        a[0] * b[1] - b[0] * a[1]
        for a, b in zip(loop, loop[1:] + loop[:1])
    ) * 0.5


def _mesh_boundary_loops(mesh, matrix):
    points = [matrix @ vertex.co for vertex in mesh.vertices]
    if not points:
        raise ValueError("轮廓没有顶点。")
    span = max(max(p.x for p in points) - min(p.x for p in points),
               max(p.y for p in points) - min(p.y for p in points), 1.0)
    if max(p.z for p in points) - min(p.z for p in points) > span * 1e-5:
        raise ValueError("轮廓必须位于水平 XY 平面；请使用无厚度的平面轮廓。")

    if mesh.polygons:
        uses = Counter()
        for polygon in mesh.polygons:
            for edge in polygon.edge_keys:
                uses[tuple(sorted(edge))] += 1
        if any(count > 2 for count in uses.values()):
            raise ValueError("轮廓网格含非流形边，请先清理网格。")
        edges = [edge for edge, count in uses.items() if count == 1]
        if any(tuple(sorted(edge.vertices)) not in uses for edge in mesh.edges):
            raise ValueError("轮廓面中混有松散边，请分离或移除松散边。")
    else:
        edges = [tuple(edge.vertices) for edge in mesh.edges]
    if not edges:
        raise ValueError("没有可用的闭合边界。请提供闭合曲线或平面网格。")

    neighbors = defaultdict(list)
    for a, b in edges:
        neighbors[a].append(b)
        neighbors[b].append(a)
    if any(len(adjacent) != 2 for adjacent in neighbors.values()):
        raise ValueError("边界必须是闭合、不分叉的环；检测到开口或分叉。")

    remaining = set(neighbors)
    loops = []
    while remaining:
        start = min(remaining)
        current, previous = start, None
        indices = []
        while True:
            if current not in remaining:
                raise ValueError("边界环发生交叉或重复，请检查轮廓。")
            remaining.remove(current)
            indices.append(current)
            adjacent = neighbors[current]
            next_vertex = adjacent[0] if adjacent[0] != previous else adjacent[1]
            previous, current = current, next_vertex
            if current == start:
                break
        if len(indices) < 3:
            raise ValueError("每个闭合边界至少需要三个顶点。")
        loops.append([[float(points[i].x), float(points[i].y)] for i in indices])
    loops.sort(key=lambda loop: abs(_area(loop)), reverse=True)
    return loops


def outline_from_object(obj, depsgraph):
    """Read evaluated horizontal loops without changing selection or source data."""
    if obj is None or obj.type not in {"MESH", "CURVE"}:
        raise ValueError("请指定一个闭合曲线或平面网格作为轮廓。")
    if obj.type == "CURVE":
        if not obj.data.splines or any(not spline.use_cyclic_u for spline in obj.data.splines):
            raise ValueError("所有轮廓曲线都必须闭合（Cyclic）。")
        if obj.data.bevel_depth or obj.data.extrude or obj.data.bevel_object:
            raise ValueError("请使用没有挤出、厚度和倒角的轮廓曲线。")
    evaluated = obj.evaluated_get(depsgraph)
    mesh = evaluated.to_mesh()
    try:
        if mesh is None:
            raise ValueError("无法读取该物体的轮廓网格。")
        loops = _mesh_boundary_loops(mesh, evaluated.matrix_world)
    finally:
        evaluated.to_mesh_clear()
    return loops[0], loops[1:]


class BD_Settings(bpy.types.PropertyGroup):
    name: StringProperty(name="建筑名称", default="building")
    source_mode: EnumProperty(
        name="输入方式",
        items=(("OUTLINE", "轮廓物体", "读取指定曲线或网格的边界"),
               ("JSON", "JSON 配置", "使用导入配置中的轮廓和门窗等设定")),
        default="OUTLINE",
    )
    outline_object: PointerProperty(name="轮廓", type=bpy.types.Object, poll=_outline_object_poll)
    auto_openings: BoolProperty(
        name="自动生成门窗", default=True,
        description="在首层最长外墙居中设置一扇门，并沿各层外墙排窗；短墙和低墙会跳过",
    )
    levels: IntProperty(name="层数", default=2, min=1)
    floor_height: FloatProperty(name="层高", default=3.0, soft_min=0.25, unit="LENGTH")
    wall_thickness: FloatProperty(name="墙厚", default=0.2, soft_min=0.01, unit="LENGTH")
    slab_thickness: FloatProperty(name="楼板厚度", default=0.2, soft_min=0.01, unit="LENGTH")
    fracture_size: FloatProperty(name="目标碎块尺度", default=1.0, soft_min=0.1, unit="LENGTH")
    seed: IntProperty(name="随机种子", default=0, min=0)
    export_directory: StringProperty(name="导出目录", subtype="DIR_PATH", default="//destruction_output/")
    imported_spec_json: StringProperty(default="", options={"HIDDEN"})
    imported_spec_path: StringProperty(default="", options={"HIDDEN"})
    last_spec_json: StringProperty(default="", options={"HIDDEN"})
    collection_name: StringProperty(default="", options={"HIDDEN"})
    status: StringProperty(default="请选择轮廓或导入 JSON 配置。", options={"HIDDEN"})
    has_error: BoolProperty(default=False, options={"HIDDEN"})


def _default_openings(data):
    """Place deterministic exterior openings while leaving corner and door piers."""
    footprint = data["footprint"]
    lengths = [math.dist(a, b) for a, b in zip(footprint, footprint[1:] + footprint[:1])]
    clear_height = data["floor_height"] - data["slab_thickness"]
    if clear_height <= 0 or not lengths:
        return []
    margin = data["wall_thickness"] + 0.15
    pier = max(0.4, data["wall_thickness"])
    longest_wall = max(range(len(lengths)), key=lambda index: lengths[index])
    usable = lengths[longest_wall] - 2 * margin
    door = None
    if usable >= 0.6 and clear_height >= 1.0:
        width = min(0.9, usable)
        door = {"wall": longest_wall, "level": 0,
                "offset": (lengths[longest_wall] - width) / 2,
                "width": width, "height": min(2.1, clear_height * 0.85),
                "sill": 0.0, "kind": "door"}
    openings = [door] if door else []
    if clear_height < 0.8:
        return openings
    for level in range(data["levels"]):
        for wall, length in enumerate(lengths):
            runs = [(margin, length - margin)]
            if level == 0 and door and wall == longest_wall:
                runs = [(margin, door["offset"] - pier),
                        (door["offset"] + door["width"] + pier, length - margin)]
            for start, end in runs:
                available = end - start
                if available < 0.6:
                    continue
                width = min(1.2, available)
                count = max(1, int((available + 0.8) / (width + 0.8)))
                gap = (available - count * width) / (count + 1)
                for index in range(count):
                    openings.append({"wall": wall, "level": level,
                                     "offset": start + gap + index * (width + gap),
                                     "width": width, "height": min(1.2, clear_height * 0.42),
                                     "sill": min(0.9, clear_height * 0.3), "kind": "window"})
    return openings


def _spec_from_settings(settings, depsgraph):
    data = json.loads(settings.imported_spec_json) if settings.imported_spec_json else {}
    if settings.source_mode == "OUTLINE":
        data["footprint"], data["holes"] = outline_from_object(settings.outline_object, depsgraph)
    elif not settings.imported_spec_json:
        raise ValueError("请先导入 JSON 配置。")
    data.update(
        name=settings.name,
        levels=settings.levels,
        floor_height=settings.floor_height,
        wall_thickness=settings.wall_thickness,
        slab_thickness=settings.slab_thickness,
        fracture_size=settings.fracture_size,
        seed=settings.seed,
    )
    data.setdefault("roof_thickness", settings.slab_thickness)
    if settings.source_mode == "OUTLINE":
        data["openings"] = _default_openings(data) if settings.auto_openings else []
    return BuildingSpec.from_dict(data)


def _report_error(operator, settings, exc):
    settings.has_error = True
    settings.status = f"失败：{exc}"
    operator.report({"ERROR"}, str(exc))
    traceback.print_exc()
    return {"CANCELLED"}


class BD_OT_UseSelectedOutline(bpy.types.Operator):
    bl_idname = "bd.use_selected_outline"
    bl_label = "使用选中轮廓"
    bl_description = "将当前活动曲线或网格指定为输入，不修改该物体"

    def execute(self, context):
        settings = context.scene.bd_authoring
        obj = context.active_object
        if obj is None or obj.type not in {"MESH", "CURVE"}:
            self.report({"ERROR"}, "请选择一个闭合曲线或平面网格。")
            return {"CANCELLED"}
        settings.outline_object = obj
        settings.source_mode = "OUTLINE"
        settings.has_error = False
        settings.status = f"输入轮廓：{obj.name}"
        return {"FINISHED"}


class BD_OT_ImportSpec(bpy.types.Operator, ImportHelper):
    bl_idname = "bd.import_spec"
    bl_label = "导入 JSON 配置"
    filename_ext = ".json"
    filter_glob: StringProperty(default="*.json", options={"HIDDEN"})

    def execute(self, context):
        settings = context.scene.bd_authoring
        try:
            spec = load_spec(Path(self.filepath))
            data = spec.to_dict()
            for key in ("name", "levels", "floor_height", "wall_thickness", "slab_thickness", "fracture_size", "seed"):
                setattr(settings, key, data[key])
            settings.imported_spec_json = json.dumps(data, ensure_ascii=False)
            settings.imported_spec_path = self.filepath
            settings.source_mode = "JSON"
            settings.has_error = False
            settings.status = f"已载入配置：{Path(self.filepath).name}"
        except Exception as exc:
            return _report_error(self, settings, exc)
        return {"FINISHED"}


class BD_OT_Generate(bpy.types.Operator):
    bl_idname = "bd.generate"
    bl_label = "生成 / 重生成"
    bl_description = "保留输入与其他物体，只替换本工具拥有的对应建筑集合"

    def execute(self, context):
        settings = context.scene.bd_authoring
        try:
            from .pipeline import build_asset

            spec = _spec_from_settings(settings, context.evaluated_depsgraph_get())
            result = build_asset(spec, output_dir=None, preview=True)
            settings.collection_name = result["collection_name"]
            settings.last_spec_json = json.dumps(spec.to_dict(), ensure_ascii=False)
            settings.has_error = False
            settings.status = (f"已生成：{result['chunk_count']} 碎块 / "
                               f"{result['collider_count']} 碰撞体 / {result['bond_count']} 连接")
            self.report({"INFO"}, settings.status)
            if settings.source_mode == "OUTLINE" and settings.auto_openings and not spec.to_dict()["openings"]:
                self.report({"WARNING"}, "墙体过短或过低，未能自动放置门窗。")
            for warning in result.get("warnings", []):
                self.report({"WARNING"}, str(warning))
        except Exception as exc:
            return _report_error(self, settings, exc)
        return {"FINISHED"}


class BD_OT_Export(bpy.types.Operator):
    bl_idname = "bd.export_asset"
    bl_label = "导出 GLB + JSON"
    bl_description = "导出上次成功生成的建筑；修改参数后请先重生成"

    @classmethod
    def poll(cls, context):
        settings = getattr(context.scene, "bd_authoring", None)
        return settings is not None and bpy.data.collections.get(settings.collection_name) is not None

    def execute(self, context):
        settings = context.scene.bd_authoring
        try:
            from .pipeline import export_asset

            if not settings.export_directory.strip():
                raise ValueError("请填写导出目录。")
            directory = Path(bpy.path.abspath(settings.export_directory)).expanduser().resolve()
            collection = bpy.data.collections.get(settings.collection_name)
            if collection is None:
                raise ValueError("上次生成的建筑集合不存在，请重新生成。")
            result = export_asset(collection, directory)
            settings.has_error = False
            settings.status = f"已导出：{directory}"
            self.report({"INFO"}, f"GLB：{result['glb_path']}；JSON：{result['manifest_path']}")
        except Exception as exc:
            return _report_error(self, settings, exc)
        return {"FINISHED"}


class BD_PT_Authoring(bpy.types.Panel):
    bl_label = "Bevy 建筑破坏资产"
    bl_idname = "BD_PT_authoring"
    bl_space_type = "VIEW_3D"
    bl_region_type = "UI"
    bl_category = "Destruction"

    def draw(self, context):
        layout = self.layout
        settings = context.scene.bd_authoring
        layout.prop(settings, "source_mode")
        if settings.source_mode == "OUTLINE":
            layout.prop(settings, "outline_object")
            layout.operator("bd.use_selected_outline", icon="EYEDROPPER")
            layout.label(text="水平闭合轮廓；世界 XY，底部 Z = 0")
            layout.prop(settings, "auto_openings")
            if settings.auto_openings:
                layout.label(text="首层最长边设门，各层外墙排窗")
                layout.label(text="门宽≤0.9 m；窗宽/高≤1.2 m")
        elif settings.imported_spec_path:
            layout.label(text=Path(settings.imported_spec_path).name, icon="FILE_TEXT")
        layout.operator("bd.import_spec", icon="IMPORT")
        layout.separator()
        for key in ("name", "levels", "floor_height", "wall_thickness", "slab_thickness", "fracture_size", "seed"):
            layout.prop(settings, key)
        layout.operator("bd.generate", icon="MOD_EXPLODE")
        layout.separator()
        layout.prop(settings, "export_directory")
        layout.operator("bd.export_asset", icon="EXPORT")
        layout.label(text="导出上次生成结果；改参数后先重生成")
        box = layout.box()
        box.label(text=settings.status, icon="ERROR" if settings.has_error else "INFO")


CLASSES = (BD_Settings, BD_OT_UseSelectedOutline, BD_OT_ImportSpec, BD_OT_Generate, BD_OT_Export, BD_PT_Authoring)


def register():
    for cls in CLASSES:
        bpy.utils.register_class(cls)
    bpy.types.Scene.bd_authoring = PointerProperty(type=BD_Settings)


def unregister():
    del bpy.types.Scene.bd_authoring
    for cls in reversed(CLASSES):
        bpy.utils.unregister_class(cls)
