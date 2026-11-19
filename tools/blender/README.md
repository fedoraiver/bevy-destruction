# Blender 建筑破坏资产工具

从平面轮廓生成有厚度的建筑、门窗开口与框、确定性的自然不规则碎块、精确复合凸碰撞代理、分面接触连接和物理数据。提供 Blender 中文面板与后台批处理入口。正式生成器默认使用新的共享分区，不需要另开实验模式；运行时碎块保持刚性。

实际验证版本：**Blender 5.2.3 LTS**。无需安装第三方 Python 包、Cell Fracture 或其他建筑扩展。

## 在 Blender 中使用

1. 在仓库根目录执行 `python tools/blender/package_addon.py`，得到 `output/blender/bevy_destruction-0.3.0.zip`。
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

可直接修改六个示例：

| 示例 | 内容 |
| --- | --- |
| [rectangle.json](examples/rectangle.json) | 矩形两层、内墙、门窗及框 |
| [l_shape.json](examples/l_shape.json) | L 形三层、内墙门、多层窗 |
| [courtyard.json](examples/courtyard.json) | 庭院两层、庭院门窗、内墙 |
| [highrise_slab.json](examples/highrise_slab.json) | 十层板式高楼，14×8 m 平面、32 m 标高 |
| [highrise_l.json](examples/highrise_l.json) | 八层 L 形高楼，14×12 m 外包络、26.4 m 标高 |
| [bridge.json](examples/bridge.json) | 28×6 m 桥面、双钢主梁、三组共六根独立混凝土桥墩 |

六个正式场景可一次重建：

```powershell
& 'C:\Program Files\Blender Foundation\Blender 5.2\blender.exe' `
  --background --factory-startup --python-exit-code 1 `
  --python tools/blender/build_catalog.py -- --output assets/buildings
```

`--asset rectangle`、`--asset highrise_slab` 等可只重建指定资产。高楼使用较粗的 3.8 / 4 m 碎块尺度；桥梁使用 3 m 尺度。逻辑碎块、凸代理和分面数量会随真实几何及种子改变，需分别统计；代理数不是刚体数，也不代表任意高层建筑的性能承诺。

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
- `fracture_size`：目标空间尺度；种子数量按源域 AABB 有效维度估计，薄墙、长梁与楼板同样会破碎。生成后按真实连通区域及尺寸复核，只能沿实际公共面合并小块，不越过门窗连接。它不是所有块严格等大的承诺；尺度越小，计算、面片和凸代理数据量越大。
- `seed`：固定输入和种子可复现结构 ID、碎块与拓扑摘要。拓扑改变后需视为新的资产版本。
- `parts`：可选的显式凸棱柱构件数组，用于桥梁等不适合从逐层围墙生成的结构。每项定义唯一 `id`、`kind`、材质 ID `material`、凸平面轮廓 `polygon` 和 Blender Z-up 的 `bottom` / `top` 标高。构件可共面接触，禁止体积重叠；全部轮廓必须位于顶层 `footprint` 内。此模式不自动生成楼板、屋顶或墙，也不能同时指定庭院、门窗或内墙。形状与高度由构件定义，面板中的楼层/墙厚参数会禁用，名称、碎块尺度和种子仍可调整。桥梁示例的桥墩之间有真实可通行净空，不是连续围墙。

所有未知字段、非有限数字、自交、过窄墙厚、重叠开口、越界门窗和无效引用都会明确报错。凹转角或过窄通道无法维持指定墙厚时，需要修改轮廓或厚度。

## 生成与导出规则

工具先生成互不体积重叠的封闭源构件，再按同一面墙、楼层或框的语义及材料组织共享源域。使用良好间隔的随机站点和非周期多尺度梯度噪声改变分区归属，在共享离散域内生成互补边界；不直接扭曲建筑外轮廓。墙体围绕同一门窗的原人工面板边界不再强制成为切缝。斜接角等可变厚度由原凸棱柱的线性厚度轮廓保留。

逻辑碎块可以非凸，通过若干精确凸子区域构造碰撞复合体；仅在体积及凸性验证允许时合并相邻子体，不能取一个跨过凹口或窗洞的总凸包。源碎块与代理进行严格闭合、单连通和正体积检查。每块显式积分体积、质心及完整惯性，并核对原源域和碰撞复合体。

采样间距固定为 `fracture_size*0.28`，不等于碎块尺度；每逻辑块代理上限为 48，当前六场景实测最多 28。正式目录计数依次为逻辑块 / 凸代理 / 父连接 / 承载面片：

| 场景 | 计数 |
| --- | --- |
| rectangle | 163 / 2143 / 524 / 3661 |
| l_shape | 269 / 3180 / 894 / 6332 |
| courtyard | 283 / 3700 / 941 / 6730 |
| highrise_slab | 228 / 2577 / 734 / 4308 |
| highrise_l | 227 / 2190 / 749 / 4529 |
| bridge | 71 / 362 / 164 / 964 |

完整当前资产 ID 及验收记录见 [ADR 0004](../../docs/adr/0004-blender-asset-pipeline.md)，GLB 文件名以各正式清单为准。

制作端保留 `source_mesh_with_contact_overlays` 政策标识：只给被规范化接触面片完整覆盖的源面赋 `fracture`，其余保留 `exterior`。材质分类不增加、移动、删除或重排顶点与面，并核对分类前后几何摘要一致；没有离线接触布尔切面或 CDT 路径。运行时现直接在源三角形着色中按接触区域选择粗糙断面，取消独立共面显示层及材质深度偏置，消除与 grid 的竞争；无需重导出既有 v2 资产。物理网格、拾取和质量不受影响。消费者契约和着色实现见 [ADR 0005](../../docs/adr/0005-v2-asset-contract.md)和 [ADR 0007](../../docs/adr/0007-fracture-surface-rendering.md)。

导出还以实际 GLB 三角形复核接触覆盖。只有 float32 舍入范围内的边界缺口允许向内修正接触面片，随后重算面积、质心、方向和截面矩；记录逐面片位移、减少的接触面积及 ULP 预算。较大不一致拒绝发布，显示网格和物理体积不变。

连接来自实际公共面，不按距离猜测；共边或共点不形成有面积的连接。资产只输出和接受 v2：同一碎块对只有一个父连接，内部保存方向可不同的凸平面片及真实顶点，各片的面积、中心和二阶矩从最终 float32 顶点重新计算。不能把曲面平均成一个平面代替承载数据。底部接触 Z=0 的碎块导出固定锚点，工具检查所有碎块能经连接到达锚点。旧清单需要重新烘焙。

重生成只替换当前场景中对应的成功建筑集合，出错保留旧结果。用户加入的物体、子集合及另行链接到其他集合的物体会保留。

完整格式见 [ASSET_FORMAT.md](ASSET_FORMAT.md)。本工具生成资产；Bevy/Avian 导入与运行时承载、坍塌、联网仍由运行时插件实现。

## 验证

```powershell
$blenderExecutable = 'C:\Program Files\Blender Foundation\Blender 5.2\blender.exe'
& $blenderExecutable --background --factory-startup --python-exit-code 1 --python tools/blender/tests/test_geometry.py
& $blenderExecutable --background --factory-startup --python-exit-code 1 --python tools/blender/tests/test_fracture.py
& $blenderExecutable --background --factory-startup --python-exit-code 1 --python tools/blender/tests/test_natural_pipeline.py
& $blenderExecutable --background --factory-startup --python-exit-code 1 --python tools/blender/tests/test_natural_union.py
& $blenderExecutable --background --factory-startup --python-exit-code 1 --python tools/blender/tests/test_asset_tool.py
& $blenderExecutable --background --factory-startup --python-exit-code 1 --python tools/blender/tests/test_surfaces.py
& $blenderExecutable --background --factory-startup --python-exit-code 1 --python tools/blender/tests/test_bundled_surfaces.py
& $blenderExecutable --background --factory-startup --python-exit-code 1 --python tools/blender/tests/test_catalog.py
```

测试覆盖几何封闭与凸性、独立重叠检查、门窗通行、解析体积与惯性、碎块重组、种子复现、连接面积和截面矩、部分接触与斜向接触的内外表面面积、GLB 材质角色及清单坐标一致性、跨场景隔离、UI 曲线生成与导出、用户数据保留，以及失败发布回滚。

2026-11-17 在 Blender 5.2.3 后台执行以上八个模块的 63 个不同测试，全部通过；其中正式六场景的外壳面积检查保留原 `1e-6` 相对门槛。重复导出还检查最终 ID 与 JSON 字节稳定；相同源碎块的接触重算不改变代理、质量、质心、惯性或锚点。

`tests/test_install.py` 另用于安装包验收，必须将 `BLENDER_USER_SCRIPTS` 指向独立测试目录后在后台执行；未设置隔离目录时会拒绝运行，避免改动常用插件目录。它检查 ZIP 安装、启用、面板注册及停用。

## 当前边界

自然不规则边界和碎块尺度控制最初的隔离工具见[薄墙原型](prototypes/README.md)。正式生成器使用共享分区、复合凸碰撞和多面片连接，见 [ADR 0004](../../docs/adr/0004-blender-asset-pipeline.md)。

- 每个共享源域最多 256 个初始站点和 12000 个微单元、每个逻辑碎块最多 48 个凸代理、单资产最多 2048 个逻辑碎块。配置最多尝试 12 个确定性种子，超限或无法保持几何时明确报错，不能静默减少碎块。
- 极端细长、微小或高坐标精度不足的几何可能被拒绝；请加大碎块尺度、调整种子或清理轮廓。不会丢弃失败碎块假装成功。
- 支持平直竖墙、水平楼板与平顶；房间规划、楼梯和斜屋顶不自动推断。
- 当前是独立资产烘焙工具，尚无 Geometry Nodes 实时预览、任意外部模型凹分解或运行时网格切割。
- 输出为带内外表面材质的几何；未生成建筑纹理和 UV 展开方案。
