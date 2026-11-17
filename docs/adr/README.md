# 架构决策记录

本目录保存 Bevy 建筑实时物理破坏插件的设计决策、重要设定及验证边界。

## 状态说明

- **提议（待原型验证）**：当前设计方向，尚未视为最终选型或完成实现。
- **部分已采用**：仅记录中明确指出的条目已作出决定，其余条目仍为提议；实现和验证进度需分别说明。
- **已采用**：已作出的设计决策；实现和验证进度仍需单独说明。
- **已被取代**：保留历史，并链接替代它的新记录。

截至 2026-11-17，六个场景的自然不规则分块、复合凸代理与 v2 多面片接触已导出，正式资产和图形验收进度见 ADR 0020、0023。Rust 模块及测试已统一为目录布局，旧资产兼容已移除。运行时包含六自由度准静态承载、渐进断裂、有限质量刚性撞击物、惯性平衡的动态簇撞击分析，以及连接内力/真实接触冲量视图。材质随碎块运动，性能测量的条件与历史边界见 ADR 0013 至 0019。建筑仍按刚体簇运动；用户明确不需要显示弹性或弯曲变形，完整瞬态应力波不属于当前实现。

## 索引

| 编号 | 主题 | 状态 |
| --- | --- | --- |
| [0001](0001-prefractured-graph-and-clusters.md) | 预破碎资产、结构连接图与刚体簇 | 部分已采用（刚性图、锚点与动态簇已实现） |
| [0002](0002-avian-backend-and-fixed-step.md) | Avian 物理后端与固定步进 | 部分已采用（固定步与动态簇接入已实现） |
| [0003](0003-structural-loads-and-progressive-fracture.md) | 结构承载、连接损伤与渐进断裂 | 部分已采用（六自由度界面、自重与过载已实现，详见 0010） |
| [0004](0004-authoring-performance-and-delivery.md) | 资产制作、性能边界与分阶段交付 | 提议（待原型验证） |
| [0005](0005-blender-procedural-building-pipeline.md) | Blender 建筑轮廓生成、预破碎与碰撞资产流程 | 部分已采用（Blender 已确定，流程提议待验证） |
| [0006](0006-blender-authoring-tool-and-asset-format.md) | Blender 资产工具实现与导出格式 | 已采用（已实现，验证范围见记录） |
| [0007](0007-bevy-bsn-fracture-interaction-demo.md) | BSN 建筑导入、网格材质与点击破坏演示 | 已采用（已实现，验证范围见记录） |
| [0008](0008-destruction-vfx-and-exposed-edge-detail.md) | 破坏 VFX 的冲击感与暴露破口边缘细节 | 部分已采用（分层 VFX 与暴露边带已实现，验证范围见记录） |
| [0009](0009-support-release-and-dynamic-clusters.md) | 失支撑释放、动态刚体簇与运动继承 | 已采用（已实现，验证范围见记录） |
| [0010](0010-elastic-interface-stress-and-overload-fracture.md) | 六自由度界面应力、自重承载与过载断裂 | 已采用（已实现，验证范围见记录） |
| [0011](0011-building-catalog-and-rigid-impact-scenarios.md) | 高层建筑、桥梁资产与刚性撞击场景 | 已采用（已实现，验证范围见记录） |
| [0012](0012-resolvable-contact-sections.md) | 可分辨接触截面与资产重烘焙 | 已采用（已实现，验证范围见记录） |
| [0013](0013-object-attached-procedural-surfaces.md) | 随碎块运动的程序化网格与断面材质 | 已采用（已实现并完成图像对照） |
| [0014](0014-inertial-cluster-impact-fracture.md) | 惯性平衡的动态簇撞击与内部断裂 | 已采用（已实现，验证范围见记录） |
| [0015](0015-structural-and-collision-runtime-performance.md) | 结构缓存、求解与碎片碰撞性能 | 已采用（已实现，测量范围见记录） |
| [0016](0016-damaged-structure-solver-and-support-release.md) | 受损结构稳健求解与窄支撑释放 | 已采用（已实现，验证范围见记录） |
| [0017](0017-progressive-impact-unloading.md) | 逐连接撞击断裂与分离卸载 | 已采用（已实现，验证范围见记录） |
| [0018](0018-natural-irregular-prefracture.md) | 自然不规则预破碎与碎块尺度 | 历史调研与原型，正式实现见 0020 |
| [0019](0019-performance-hot-paths.md) | 求解热点、撞击输入复用与烟尘渲染性能 | 已采用，验证与测量范围见记录 |
| [0020](0020-production-irregular-fracture.md) | 正式不规则分块与多面片承载接口 | 已实现，63 项制作测试及六目录核心检查通过；落地限制见 0024 |
| [0021](0021-structural-force-visualization.md) | 连接内力与真实接触冲量视图 | 已实现，最终矩形五阶段图形验收通过 |
| [0022](0022-rust-module-layout.md) | Rust 模块目录、集中测试与 v2-only 资产 | 已采用并实现，验证范围见记录 |
| [0023](0023-source-mesh-and-contact-surface-overlays.md) | 保留源网格与真实接触面的断面显示 | 已实现，六目录覆盖及最终矩形点击/受力/倾倒检查通过 |
| [0024](0024-fresh-impact-event-rebinding.md) | 独立接触事件绑定与落地时序诊断 | 新事件绑定已实现；大块落地耦合未解决，失败实验未采用 |

后续设计记录按编号追加；相关决策改变时同步维护本索引和原记录。
