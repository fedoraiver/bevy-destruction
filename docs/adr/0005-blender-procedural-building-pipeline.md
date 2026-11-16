# ADR 0005：Blender 建筑轮廓生成、预破碎与碰撞资产流程

- 状态：部分已采用（Blender 已确定，流程提议待验证）
- 日期：2026-11-16
- 关联：[ADR 0001](0001-prefractured-graph-and-clusters.md)、[ADR 0002](0002-avian-backend-and-fixed-step.md)、[ADR 0004](0004-authoring-performance-and-delivery.md)

## 背景

用户明确使用 Blender，并要求调研能否从建筑轮廓依次生成墙体、楼板、门窗、碎块和碰撞体。目标是为 Bevy 0.20 + Avian 0.8 的运行时破坏插件准备资产。

本记录将输入理解为共面的建筑平面轮廓，具体可采用闭合曲线或明确边界的平面网格。只有外轮廓并不能确定房间、内墙、楼梯和门窗用途；这些需要额外标记或显式生成规则。

结论：Blender 具备各阶段所需的基础能力，可以组织成自动化资产流水线；尚未发现已核验、能直接输出本插件完整破坏数据的一键工具。原生几何算子、社区扩展和项目专用生成/导出逻辑需要明确区分。

## 已采用决策

- 使用 Blender 作为本项目的离线资产制作工具，SideFX 文章中的封闭资产、模块化构件和碰撞烘焙原则继续作为参考。
- 运行时运动、承载、断裂和二次撞击仍由 Bevy 插件及 Avian 处理；Blender 中的刚体配置或预模拟动画不替代这些功能。
- 本记录中的具体生成器、扩展依赖、文件格式和参数范围仍是提议，工具选择不表示完整流程已实现。

## 能力调研

| 阶段 | 已核验的基础能力 | 仍需补充的项目逻辑 |
| --- | --- | --- |
| 轮廓输入 | 曲线、平面网格、Python 数据与几何操作 | 外环/内环语义、单位、方向、共面性、闭合与自交校验 |
| 墙体 | 挤出、Solidify、布尔及 BMesh/Python | 墙厚、转角连接、凹角、墙体分段、内墙与结构标签 |
| 楼板 | Fill Curve、平面三角化、挤出与实例化 | 楼层标高、板厚、庭院和楼梯洞口，以及封底 |
| 门窗 | Mesh Boolean 或 Boolean 修改器、实例化模块 | 合法布置范围、切洞规则、洞口与梁柱避让、门框归属 |
| 碎块 | Cell Fracture 扩展；原生布尔/切分也可构造简单碎块 | 封闭性、材质与断面、种子、构件归属、稳定 ID 和连接图 |
| 凸碰撞代理 | Mesh Convex Hull、`bmesh.ops.convex_hull` | 凹碎块多凸体表示、洞口保持、复杂度限制和导出标记 |
| 自动化与导出 | Python API、后台运行、glTF 自定义属性导出 | 调度、缓存、数据清单、Avian 导入器和错误报告 |

### 原生算子的边界

- Fill Curve 生成平面几何，本身不提供建筑楼层或房间语义；楼板需要构造有效厚度和完整边界。
- Extrude Mesh 不保留被拉伸面的原始背面，因此一次面挤出不能直接当作封闭实体。最终应独立校验封闭性、连通岛和正体积。
- Solidify 适合为表面增加厚度，但复杂交汇和非均匀缩放影响结果；严格墙厚和凹角处理需要几何验证，不能仅凭修改器参数认定正确。
- Boolean 可形成门窗洞口；规则矩形开口也可以先按开口边界拆成封闭墙段。具体实现需要比较稳定性、重生成成本和破碎质量。
- Geometry Nodes 适合参数化几何与预览；Python/BMesh 适合批处理、扩展调用、结果分类、验证及自定义导出。采用混合流程是本项目的实现建议，不是 Blender 自带的完整建筑功能。

### 现成工具与依赖取舍

- **Building Tools**：作者资料支持从自定义平坦且已填面的 floorplan 生成楼层、墙、楼板和门窗等构件，可参考其规则。仓库明确声明 Blender 4.0 兼容；没有本项目在 Blender 5.x 上的实测结果，不能视为已验证依赖，也不能假定生成完整室内及破坏数据。
- **Home Builder 5**：官方扩展平台的作者说明提供房间、楼板、天花及门窗自动开洞，当前 5.2.4 更新记录声明 Blender 5.2 兼容。它面向室内和参数化家具，可作为房间与开口工具参考；尚未安装或验证其输入轮廓、封闭破碎体及本插件数据导出，不因平台兼容声明就认定满足完整流程。
- **Cell Fracture**：官方扩展页列出 0.2.1、Blender 4.2 LTS 及更新版本，同时注明有限支持。它需要作为扩展获取，不能默认仍为内置或已经安装；实际版本兼容性和后台调用需验证。
- Cell Fracture 历史官方源码提供操作符和按所选 Mesh 处理的接口，说明可用脚本组织操作；尚未据此证明当前扩展包接口完全一致。原物体与凸 Voronoi 单元求交后的碎块也不一定凸，仍需检查。
- **近似凸分解**：CoACD 可以作为复杂凹碎块的候选，尚未选作依赖或验证 Blender Python 兼容性。V-HACD 作者仓库已标注停止维护，不能把它作为默认受维护方案。

初期建议直接掌握轮廓、构件语义和导出逻辑，按需要采用扩展。不先把完整流水线绑定到大型建筑扩展或未经验证的插件接口。

## 自动生成流程提议

```text
外轮廓 + 可选内环/内墙线 + 楼层参数 + 门窗规则/标记
  -> 输入校验与构件语义
  -> 具有厚度的墙 / 梁柱 / 楼板 / 基础
  -> 门窗开口与附着细节
  -> 应用几何结果并检查封闭性
  -> 按构件预破碎
  -> 逐碎块凸代理或多个凸代理
  -> 连接界面 / 材料 / 锚点 / 质量与稳定 ID
  -> 网格资产 + 结构清单
  -> Bevy 导入、创建 Avian 碰撞体并验证
```

### 输入与首轮范围

先支持无自交的闭合矩形和 L 形轮廓，约定其代表外墙外边界，并显式输入层数、层高、墙厚、板厚和门窗布置规则。此输入约定属于提议，尚未作为资产格式实现。

外墙厚度向内构造，楼板范围与墙的衔接单独定义。凹角、窄通道、重合边和过短墙段需要检测。庭院需要显式内环，楼板不得填满；内墙和楼梯需要额外输入或预定义模板，不能凭外轮廓推断为真实设计。

初期生成独立且有语义的墙段、梁柱和楼板，再分别破碎，避免将带空腔的整栋建筑当作实心块切分。门窗参数必须避开非法边界；附着细节应切分或关联到正确碎块。

### 碰撞与连接数据

- 给 Blender 对象设置刚体 `CONVEX_HULL` 只是 Blender 物理配置，不等于导出了实际凸代理。应生成可识别的代理网格或明确的顶点数据，由 Bevy 导入器创建 Collider。
- 单个凸包会填平凹槽或门窗洞口。对于凹碎块，应细分为若干凸块或做经验证的凸分解，再组合成同一个 Chunk 的碰撞体；不得牺牲洞口通行来减少代理数量。
- GLB 可携带普通代理网格和自定义 `extras` 标记，但不会自动变成 Avian Collider 或结构 Bond。结构清单或自定义扩展仍需定义 Chunk/代理对应、连接位置与法线、截面、强度、锚点和质量数据。
- 首选候选仍为 GLB 加独立结构清单；具体 schema 和格式未锁定。代理应在导入后转为碰撞，不参与常规显示。
- 稳定 ID 从源构件身份与明确的生成规则产生，不能依赖临时实体、面索引或导出顺序。固定种子只能帮助复现同一输入；改变拓扑后仍可能需要新资产版本和迁移规则。

## 备选方案与取舍

- **全 Geometry Nodes**：参数化预览直接，但扩展调用、资产清单、逐对象处理和稳定数据维护更适合用脚本协调。
- **全 Python/BMesh**：便于确定性批处理和验证，但交互式编辑体验需要额外工具；可先作为受控原型，再补节点或面板。
- **直接采用建筑扩展**：能加速外观建模，需要验证室内、封闭体、版本兼容及构件语义；不能省略破坏专用数据生成。
- **直接导出 Blender 刚体仿真**：适合预模拟演出，不能满足玩家在 Bevy 运行时改变支撑并触发坍塌的目标。

## 验证与未决问题

文档和源码核验说明各阶段具备基础能力，完整自动生成器、当前 Cell Fracture 扩展、凸分解及 Bevy 导入尚未完成端到端验证。

### 本机局部实测

2026-11-16，使用本机 **Blender 5.2.3 LTS** 的独立后台进程，以 `--background --factory-startup` 运行临时脚本；未连接或修改用户当前 Blender 场景，未安装扩展或保存 `.blend`。

| 测试 | 结果 | 覆盖边界 |
| --- | --- | --- |
| 矩形轮廓生成构件 | 生成 4 面墙和 1 块楼板；均封闭、正体积，无零面积面或零长度边 | 只测一层矩形，不代表任意凹轮廓或交汇处理已完成 |
| 原生门窗开洞 | EXACT Boolean 后墙体体积依次约为 3.60、3.18、2.94 立方米，符合预期；洞口射线由命中变为通行，剩余墙体仍命中 | 验证洞口几何，不包含门扇、窗框和装饰生成 |
| 原生分块 | 对另一个独立封闭块，用 4 个矩形单元做 Boolean INTERSECT，得到 4 个封闭且正体积的碎块；总量在 `1e-5` 立方米容差内守恒 | 这是简单原生布尔切分，不是 Cell Fracture 或 Voronoi 测试 |
| 凸代理网格 | 用 `bmesh.ops.convex_hull` 生成 4 个封闭、正体积且包含对应碎块顶点的代理 | 只测简单碎块；没有验证凹碎块分解或破碎后门窗洞口保持 |

进程退出码为 0，以上检查通过。开洞墙体与分块源块是不同对象，因此本次并未打通“带门窗墙体 → 破碎 → 保持洞口的凸代理”的完整链路。本次 factory-startup 环境没有注册 Cell Fracture 操作符；没有运行该扩展，也未测试 Geometry Nodes 建筑网络、刚体模拟、导出、Bond 或承载求解。局部成功不代表完整建筑生成器、生产资产或性能预算已经验证。

### 下一步验收

首个完整验证件应为一层建筑，至少包含门洞、窗洞和楼板；之后增加 L 形、庭院、多层和内墙：

1. 输入轮廓或参数改变后可以重生成，保留源构件语义和人工覆盖。
2. 墙、板和碎块均无非法开放边、零体积或意外独立岛；截面材质和断面正确。
3. 门洞射线和通行检查通过，窗洞碰撞不被凸包填满。
4. 生成的凸代理与对应碎块一致，记录数量、几何误差和烘焙耗时。
5. 连接只出现在真实接触界面，跨墙/板/柱连接可检查并人工修正。
6. 在 Bevy 中重建碰撞、质量和连接，比较导出前后的位置、数量及 ID；重新导出不会静默错配。

Blender 具体支持版本、Cell Fracture 使用方式、凸分解依赖、默认破碎尺度、缓存失效规则与性能预算均待确定。

## 参考资料

- [Blender Geometry Nodes](https://docs.blender.org/manual/en/latest/modeling/geometry_nodes/introduction.html)
- [Fill Curve](https://docs.blender.org/manual/en/5.3/modeling/geometry_nodes/curve/operations/fill_curve.html)、[Extrude Mesh](https://docs.blender.org/manual/en/5.1/modeling/geometry_nodes/mesh/operations/extrude_mesh.html)、[Mesh Boolean](https://docs.blender.org/manual/en/5.0/modeling/geometry_nodes/mesh/operations/mesh_boolean.html)
- [Solidify 修改器](https://docs.blender.org/manual/en/5.0/modeling/modifiers/generate/solidify.html)
- [Building Tools 作者仓库](https://github.com/ranjian0/building_tools)、[使用说明](https://github.com/ranjian0/building_tools/wiki/Usage)
- [Home Builder 5 扩展](https://extensions.blender.org/add-ons/home-builder-5/)、[版本记录](https://extensions.blender.org/add-ons/home-builder-5/versions/)
- [Cell Fracture 扩展](https://extensions.blender.org/add-ons/cell-fracture/)、[历史操作符源码](https://github.com/blender/blender-addons/blob/main/object_fracture_cell/__init__.py)、[历史几何处理源码](https://github.com/blender/blender-addons/blob/main/object_fracture_cell/fracture_cell_setup.py)
- [BMesh API](https://docs.blender.org/api/current/bmesh.ops.html)、[Blender 刚体碰撞设置](https://docs.blender.org/manual/en/5.0/physics/rigid_body/properties/collisions.html)
- [CoACD 作者仓库](https://github.com/SarahWeiii/CoACD)、[V-HACD 作者仓库及维护说明](https://github.com/kmammou/v-hacd)
- [Blender glTF 导出与自定义属性](https://docs.blender.org/manual/en/latest/addons/scene_gltf2.html)

在线资料于 2026-11-16 核验；不同版本手册用于确认对应算子的职责，不代表已验证全部列出的 Blender 版本。扩展页面兼容声明、历史源码和本机实测的证据范围分别记录，不相互替代。
