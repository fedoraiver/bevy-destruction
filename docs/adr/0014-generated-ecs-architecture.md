# ADR 0014：生成式 ECS 架构站点

## 状态

2026-11-22 采用。替代 [ADR 0013](0013-shared-bevy-cicd.md) 的静态架构索引发布步骤。项目二进制中的调度导出入口与依赖由 [ADR 0016](0016-shared-schedule-export.md) 替代。现行分析包与 Pages 触发条件见 [ADR 0019](0019-workspace-app-entrypoints.md)。

## 背景

调度注册、系统访问、关系和源码会随实现变化。架构站点需要展示当前修订产生的证据。已有 SVG 随所属 ADR 维护，用于说明设计意图。

## 决策与实现

- 架构工作流使用 bevy-cicd 的生成流程，从当前项目收集 rustc 事实，导出原生 Linux 目标的 Bevy 调度，生成并校验同一提交的站点。
- Cargo 功能 `debugdump` 提供 `--dump-schedules DIR`。导出使用正常的插件装配，并在插件完成准备后读取主应用和子应用的调度。它不执行 Startup、游戏循环和渲染线程。
- `bevy-schedule-export` 跟踪 bevy-cicd 的默认分支。`Cargo.lock` 记录当前构建所用的提交，通过 `cargo update -p bevy-schedule-export` 同步上游更新。
- 当时在 PR 生成并校验站点。默认分支支持 Pages 发布，上线后核对首页与提交证明。

## 备选方案与取舍

既有 SVG 能解释选定流程，编译事实和注册快照覆盖自动发现的调度与 ECS 对象。后者需要锁定 nightly、图形生成工具和 Linux 虚拟显示环境。

## 验证要求

生成的站点须包含实际主应用和渲染子应用的调度、可追溯源码与当前提交的发布证明。站点校验及部署证明随 Actions 运行保存。

## 未决问题

快照记录装配后的注册结构。Startup 之后追加的注册和运行轨迹仍需其他诊断。

## 来源

[bevy-cicd 架构生成工作流](https://github.com/fedoraiver/bevy-cicd)。
