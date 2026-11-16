# Blender 建筑破坏资产工具

从平面轮廓生成有厚度的建筑、门窗开口与框、确定性 Voronoi 碎块、凸碰撞代理、接触连接和物理数据。提供 Blender 中文面板与后台批处理入口。

实际验证版本：**Blender 5.2.3 LTS**。无需安装第三方 Python 包、Cell Fracture 或其他建筑扩展。

## 在 Blender 中使用

1. 在仓库根目录执行 `python tools/blender/package_addon.py`，得到 `output/blender/bevy_destruction-0.2.0.zip`。
2. 在 Blender 的偏好设置中选择插件的“从磁盘安装”，选择 ZIP 并启用 **Bevy Destruction Authoring**。
3. 在三维视图按 `N` 打开侧栏，进入 **Destruction**。
4. 选择无挤出和倒角的闭合水平曲线，或带明确边界的平面网格；点击“使用选中轮廓”。庭院采用同一物体里的内环。
5. 设置建筑名称、层数、层高、墙厚、楼板厚度、碎块尺度和种子。轮廓模式默认按规则生成门窗，较短或较低的墙会跳过无法容纳的开口。
6. 点击“生成 / 重生成”。结果位于 `BD::建筑名称` 集合；输入轮廓和其他用户物体保留。
7. 指定导出目录，点击“导出 GLB + JSON”。改参数后先重生成；直接编辑生成网格、变换、材质或复制碎块后，导出会要求重生成，避免物理数据与显示不一致。

需要精确门窗、内墙、材质或不同顶板厚度时，导入下列 JSON 示例，再调整参数。JSON 模式保留文件中的显式门窗，不自动覆盖。

轮廓按物体世界 XY 读取，生成建筑底部固定在 Z=0；场景单位显示不改变输入的米制约定。外轮廓表示外墙的外边界，墙厚向内，楼板覆盖建筑轮廓并扣除庭院。源轮廓应保持可编辑，生成后可在视口隐藏它。

## 后台批处理

以下 PowerShell 命令在仓库根目录执行；其他平台将 Blender 路径替换为对应可执行文件：

```powershell
& 'C:\Program Files\Blender Foundation\Blender 5.2\blender.exe' `
  --background --factory-startup --python-exit-code 1 `
  --python tools/blender/build_asset.py -- `
  --spec tools/blender/examples/rectangle.json `
  --output output/blender/rectangle `
  --save-blend output/blender/rectangle/rectangle.blend
```

`--save-blend` 可省略。失败时输出具体错误，配合 `--python-exit-code 1` 返回非零退出码。后台入口只允许在后台 Blender 使用；在交互窗口中使用插件面板。

成功后生成：

- `rectangle.<完整内容SHA256>.glb`：显示网格和内外表面材质。
- `rectangle.destruction.json`：引用该 GLB，包含碰撞、连接、锚点和质量属性。
- 可选的 `.blend`：便于打开检查的场景文件。

GLB 以内容摘要命名，JSON 最后原子替换。导出失败时，上次发布的 JSON 和它引用的 GLB 保持有效。历史 GLB 可供旧清单继续引用，工具不会自动删除它们。

## 输入配置

可直接修改三个示例：

| 示例 | 内容 |
| --- | --- |
| [rectangle.json](examples/rectangle.json) | 矩形两层、内墙、门窗及框 |
| [l_shape.json](examples/l_shape.json) | L 形三层、内墙门、多层窗 |
| [courtyard.json](examples/courtyard.json) | 庭院两层、庭院门窗、内墙 |

最简配置：

```json
{
  "name": "my_building",
  "footprint": [[0, 0], [6, 0], [6, 4], [0, 4]],
  "levels": 1,
  "floor_height": 3.0,
  "wall_thickness": 0.2,
  "slab_thickness": 0.2,
  "fracture_size": 2.0,
  "seed": 7,
  "openings": [
    {"wall": 0, "level": 0, "kind": "door", "offset": 1.0,
     "width": 1.0, "height": 2.1, "frame": true},
    {"wall": 0, "level": 0, "kind": "window", "offset": 3.5,
     "width": 1.2, "height": 1.0, "sill": 1.0}
  ]
}
```

- `footprint`：外环顶点，不必重复末尾点；顺/逆时针均可，边索引保持输入顺序。
- `holes`：庭院内环数组，必须严格位于外环内且不能相交或嵌套。
- `levels`、`floor_height`、`slab_thickness`、`roof_thickness`：楼层参数，米制；顶板厚度为 0 可不生成顶板。第 n 层楼板从 `n * floor_height` 向上增厚，墙从板顶延伸至下一层标高。
- `openings[].wall`：外墙边索引；`inner:N` 指第 N 面内墙，`hole:H:E` 指第 H 个庭院的第 E 条边。索引与层数均从 0 开始。
- `offset`：沿输入边方向，从起点到开口起点的距离。`sill` 从当层楼板顶面量起；门的窗台高度必须为 0。
- `frame`、`frame_width`、`frame_material`：可选框；窗框含四边，门框含两侧与顶部，留出内部通行空间。
- `inner_walls`：`start`、`end` 平面坐标，可覆盖 `thickness` 与 `material`。相交内墙按构件顺序分区，异材质交汇需明确拒绝以避免隐含覆盖。
- `materials`：以 ID 为键的材质表，支持 `density`、`color`、`tension_strength`、`compression_strength`、`shear_strength`、`young_modulus`、`poisson_ratio`。密度单位 kg/m³，强度和弹性模量单位 Pa。
- `wall_material`、`floor_material`、`roof_material`：各类构件的材质 ID；内置 concrete、steel、wood。默认物理值用于游戏原型标定，不是工程承载保证，木材暂按各向同性近似。
- `fracture_size`：目标空间尺度；种子数量按凸构件 AABB 有效维度估计，薄墙与楼板同样会破碎。它不是最大碎块直径；尺度越小，计算与数据量越大。
- `seed`：固定输入和种子可复现结构 ID、碎块与拓扑摘要。拓扑改变后需视为新的资产版本。

所有未知字段、非有限数字、自交、过窄墙厚、重叠开口、越界门窗和无效引用都会明确报错。凹转角或过窄通道无法维持指定墙厚时，需要修改轮廓或厚度。

## 生成与导出规则

工具先将构件分区为互不体积重叠的封闭凸实体，再在每个实体内部以固定种子生成真实三维 Voronoi 单元。Voronoi 切面赋 `bd_surface_role=fracture`，原始表面赋 `exterior`。跨构件共面接触使用真实重叠多边形细分显示表面，只把被接触覆盖的区域改为 `fracture`；例如楼板上墙底以外的区域仍为 `exterior`。GLB 三角化保留这些材质边界；拆除相邻构件后可以显示对应断面。

此分类只改显示网格的面划分和材质，不重新计算或移动凸碰撞代理、质量、质心和连接。切分后的数值重复点在 0.000001 米内焊接，并检查封闭性、凸性、体积与质心。碰撞代理是通过封闭性和凸性验证的碎块独立网格副本，避免对凹物体整体包凸包而封住门窗。

连接来自实际共面接触区域，不按距离猜测；共边或共点不会形成有面积的连接。接触容差为 0.00002 米。底部接触 Z=0 的碎块导出固定锚点，工具检查所有碎块是否可通过连接到达锚点。

重生成只替换当前场景中对应的成功建筑集合，出错保留旧结果。用户加入的物体、子集合及另行链接到其他集合的物体会保留。

完整格式见 [ASSET_FORMAT.md](ASSET_FORMAT.md)。本工具生成资产；Bevy/Avian 导入与运行时承载、坍塌、联网仍由运行时插件实现。

## 验证

```powershell
$blenderExecutable = 'C:\Program Files\Blender Foundation\Blender 5.2\blender.exe'
& $blenderExecutable --background --factory-startup --python-exit-code 1 --python tools/blender/tests/test_geometry.py
& $blenderExecutable --background --factory-startup --python-exit-code 1 --python tools/blender/tests/test_fracture.py
& $blenderExecutable --background --factory-startup --python-exit-code 1 --python tools/blender/tests/test_asset_tool.py
& $blenderExecutable --background --factory-startup --python-exit-code 1 --python tools/blender/tests/test_surfaces.py
& $blenderExecutable --background --factory-startup --python-exit-code 1 --python tools/blender/tests/test_bundled_surfaces.py
```

测试覆盖几何封闭与凸性、独立重叠检查、门窗通行、解析体积与惯性、碎块重组、种子复现、连接面积和截面矩、部分接触与斜向接触的内外表面面积、GLB 材质角色及清单坐标一致性、跨场景隔离、UI 曲线生成与导出、用户数据保留，以及失败发布回滚。

`tests/test_install.py` 另用于安装包验收，必须将 `BLENDER_USER_SCRIPTS` 指向独立测试目录后在后台执行；未设置隔离目录时会拒绝运行，避免改动常用插件目录。它检查 ZIP 安装、启用、面板注册及停用。

## 当前边界

- 验证范围为米制小型建筑，单构件最多 256 个 Voronoi 单元、单建筑最多 2048；超限会报错，不能静默减少碎块。
- 极端细长、微小或高坐标精度不足的几何可能被拒绝；请加大碎块尺度、调整种子或清理轮廓。不会丢弃失败碎块假装成功。
- 支持平直竖墙、水平楼板与平顶；房间规划、楼梯和斜屋顶不自动推断。
- 当前是独立资产烘焙工具，尚无 Geometry Nodes 实时预览、任意外部模型凹分解或运行时网格切割。
- 输出为带内外表面材质的几何；未生成建筑纹理和 UV 展开方案。
