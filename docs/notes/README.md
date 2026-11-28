# 设计记录

这里记录设计决策、取舍与验证范围。已落地的决策及其历史版本见下方索引，尚未落地的方案见[提案目录](proposed/README.md)。各条记录的状态说明其采用、实现和验证进度。运行方法见[项目 README](../../README.md)。

板式高楼的无窗口落地验证覆盖楼板和墙体分簇、上层内部断裂及质量守恒。验证方法和历史基线见 [ADR 0003](implemented/0003-collision-driven-fracture.md)。

三张流程图分别说明[固定步与断裂反馈](implemented/0001-rigid-cluster-runtime.md#固定步和碰撞)、[离线预破碎与 v2 发布](implemented/0004-blender-asset-pipeline.md#决策与实现)和[受力样本到热力视图](implemented/0009-structural-diagnostics.md#总览拾取与详查)。

## 已实现的决策

### 结构与物理

| ADR | 决策 |
| --- | --- |
| [0001](implemented/0001-rigid-cluster-runtime.md) | 预破碎结构与刚体簇运行时 |
| [0002](implemented/0002-elastic-structure-and-overload.md) | 六自由度结构承载与过载断裂 |
| [0003](implemented/0003-collision-driven-fracture.md) | 碰撞驱动的断裂与事件卸载 |
| [0012](implemented/0012-density-derived-mass-properties.md) | Avian 按碰撞体与密度计算质量属性 |
| [0021](implemented/0021-explicit-ecs-access-and-cluster-ownership.md) | 承载系统的显式访问与刚体簇归属 |
| [0023](implemented/0023-ecs-structure-entities.md) | 运行时结构图的 ECS 实体 |

### 资产制作

| ADR | 决策 |
| --- | --- |
| [0004](implemented/0004-blender-asset-pipeline.md) | Blender 制作与自然不规则预破碎 |
| [0005](implemented/0005-v2-asset-contract.md) | v2 破坏资产与规范接触面 |

### 交互、渲染与诊断

| ADR | 决策 |
| --- | --- |
| [0006](implemented/0006-demo-scenes-and-controls.md) | 演示场景与交互 |
| [0007](implemented/0007-fracture-surface-rendering.md) | 源网格、随物体移动的材质与断面着色 |
| [0008](implemented/0008-destruction-effects-and-rims.md) | 分层破坏特效、共享烟尘与暴露边带 |
| [0009](implemented/0009-structural-diagnostics.md) | 受力样本、热力总览与局部详查 |
| [0025](implemented/0025-scene-settings-and-pause-menu.md) | 场景设置和暂停菜单 |

### 性能与工程布局

| ADR | 决策 |
| --- | --- |
| [0010](implemented/0010-runtime-performance.md) | 结构求解与碰撞性能 |
| [0011](implemented/0011-rust-project-layout.md) | Rust 模块与测试布局 |
| [0013](implemented/0013-shared-bevy-cicd.md) | 共享 Bevy 持续集成与架构发布 |
| [0014](implemented/0014-generated-ecs-architecture.md) | 生成式 ECS 架构站点 |
| [0015](implemented/0015-follow-bevy-cicd-main.md) | 跟踪 bevy-cicd 主分支 |
| [0016](implemented/0016-shared-schedule-export.md) | 共享工作流运行 Bevy 调度导出 |
| [0017](implemented/0017-bevy-compiler-rules.md) | 在 Rust CI 中自动运行 Bevy 编译器规则 |
| [0018](implemented/0018-windowless-acceptance.md) | 独立运行无窗验收 |
| [0019](implemented/0019-workspace-app-entrypoints.md) | 独立应用入口与 ECS 架构分析包 |
| [0020](implemented/0020-game-architecture-export.md) | 从桌面游戏包导出 ECS 架构 |
| [0022](implemented/0022-script-driven-acceptance-scenarios.md) | 由独立脚本驱动无窗验收场景 |
| [0024](implemented/0024-rust-analyzer-background-work.md) | 限制 Rust Analyzer 保存检查并行度和 VFS 扫描范围 |
| [0026](implemented/0026-python-driven-acceptance.md) | Python 驱动无窗验收 |

两个目录共用连续编号，下一个编号为 0027。整理前的编号、已被替代的方案和阶段性证据可在提交 `d462e4d` 中查阅。
