# 架构决策记录

这里记录现行设计、取舍和验证范围。运行方法和最近一次验证见[项目 README](../../README.md)。

**当前限制。** 正式板式高楼上部落地后破坏偏轻，严格落地回归仍失败。独立接触事件的首次绑定已通过聚焦回归。两项验证的区别和诊断见 [ADR 0003](0003-collision-driven-fracture.md)。

## 结构与物理

| ADR | 决策 |
| --- | --- |
| [0001](0001-rigid-cluster-runtime.md) | 预破碎结构与刚体簇运行时 |
| [0002](0002-elastic-structure-and-overload.md) | 六自由度结构承载与过载断裂 |
| [0003](0003-collision-driven-fracture.md) | 碰撞驱动的断裂与事件卸载 |

## 资产制作

| ADR | 决策 |
| --- | --- |
| [0004](0004-blender-asset-pipeline.md) | Blender 制作与自然不规则预破碎 |
| [0005](0005-v2-asset-contract.md) | v2 破坏资产与规范接触面 |

## 交互、渲染与诊断

| ADR | 决策 |
| --- | --- |
| [0006](0006-demo-scenes-and-controls.md) | 演示场景与交互 |
| [0007](0007-fracture-surface-rendering.md) | 源网格、随物体移动的材质与断面着色 |
| [0008](0008-destruction-effects-and-rims.md) | 分层破坏特效、共享烟尘与暴露边带 |
| [0009](0009-structural-diagnostics.md) | 受力样本、热力总览与局部详查 |

## 性能与工程布局

| ADR | 决策 |
| --- | --- |
| [0010](0010-runtime-performance.md) | 结构求解与碰撞性能 |
| [0011](0011-rust-project-layout.md) | Rust 模块与测试布局 |

新增决策从 0012 开始顺序编号。整理前的编号、已被替代的方案和阶段性证据可在提交 `d462e4d` 中查阅。
