# ADR 0020：从桌面游戏包导出 ECS 架构

## 状态

2026-11-23 采用并实现。替代 [ADR 0019](0019-workspace-app-entrypoints.md) 中的独立架构分析包。

## 背景

架构分析和桌面入口先前分别装配 Bevy，并通过两个 Cargo 包编译同一份插件源码。分析结果应反映桌面入口实际安装的插件和调度。

## 决策与实现

工作区元数据选择 `apps/game` 的 `bevy-destruction` 包，包元数据指定 `build_app`。`apps/game/src/lib.rs` 引入仓库 `src/` 中的功能模块，并为桌面运行和共享 bevy-cicd 启动器提供同一装配函数。导出使用默认演示选项，桌面入口使用解析后的参数。无窗验收继续从独立包选择窗口、输入和渲染配置。[工作区元数据](../../../Cargo.toml) · [桌面包元数据](../../../apps/game/Cargo.toml) · [装配入口](../../../apps/game/src/lib.rs)

## 备选方案与取舍

独立分析包便于试验不同配置，但需要维护单独的清单和条件编译。将导出入口放在桌面包，可共用实例化路径。导出函数成为库入口的一部分，适配 bevy-cicd 要求的无参 App 入口。

## 验证要求与结果

确认 Cargo 仍默认选择桌面应用，无窗包可使用 `acceptance-test` 编译。验证共享导出器选择桌面包，获取主应用和渲染子应用调度，并能检查生成的架构站点。

已检查锁定的 Cargo 元数据、桌面选项与无窗构建。共享启动器的运行快照包含主应用、RenderApp 和 RenderExtractApp 的 27 个调度。主应用的固定步含刚体簇重建系统。

## 未决问题

完整编译事实提取及站点校验依赖共享分析工作流。线上 Pages 发布结果以 `master` 推送后的 GitHub Actions 运行记录为准。

## 来源

参考 `D:/demo` 的 `6425fd3`，其中 workspace 选择普通游戏包，游戏库提供 App 工厂。当前仓库使用的共享导出流程见 [Architecture Pages](../../../.github/workflows/architecture-pages.yml)。
