# ADR 0019：独立应用入口与 ECS 架构分析包

## 状态

2026-11-23 采用。替代 [ADR 0011](0011-rust-project-layout.md) 的二进制入口位置、[ADR 0016](0016-shared-schedule-export.md) 的根包装配和 Pages 触发方式，并更新 [ADR 0017](0017-bevy-compiler-rules.md) 与 [ADR 0018](0018-windowless-acceptance.md) 的包范围。

独立架构分析包的决定由 [ADR 0020](0020-game-architecture-export.md) 替代。

## 背景

桌面和无窗验收需要不同的窗口、输入和渲染配置。原先由同一个游戏库构造两种 App，架构分析也选择仓库根包。Cargo workspace 可以为每种用途提供独立入口，并继续复用现有的游戏模块和资产目录。

## 决策与实现

- 仓库根清单只管理 workspace 和共用依赖。`apps/game` 提供 `bevy-destruction`，也是默认成员。`apps/acceptance` 提供 `bevy-destruction-acceptance`，仍以 `acceptance-test` 显式启用。两者的 `main` 各自创建 Bevy App、配置引擎并安装共用业务插件。入口继续使用仓库根的 `assets/`。
- `tools/ecs-architecture` 编译同一份 `src/` 游戏代码，并在其装配模块提供 `build_app()`。根清单使用 `[workspace.metadata.bevy-architecture]` 的 `package` 选择分析包。bevy-cicd 以整个仓库为源码与资源根，按该包提取编译事实，并用共享启动器导出主应用和渲染子应用的调度。
- 交互快照、验收相机和性能资源通过 Bevy 的 `Reflect` 自动注册，保留现有 BRP 类型路径。`RunMetrics` 和 `Samples` 的字段均参与反射。墙钟时间戳与渲染测量去重状态放在采样系统的 `Local`，BRP 与 JSON 报告继续使用原有字段。
- Architecture Pages 只在推送到 `master` 时构建、校验和发布，继续在同一次运行内交接产物。Rust CI 触发条件沿用现有配置，编译器规则显式检查 `apps/game`。

## 备选方案与取舍

保留根游戏包可以继续使用原有的工作流定位规则。虚拟 workspace 把默认桌面入口与分析目标分开，需要共享工作流支持按包名选择 workspace 成员。分析包编译共用源码，使实体、组件和调度仍可归入同一分析目标，也使分析入口可以独立于桌面与验收二进制演进。

## 验证要求

- 检查 workspace 成员、默认成员、分析包元数据以及根目录和不同工作目录下的资源解析。运行锁定的 Rust 回归、格式与 Clippy，并核对桌面和验收入口的命令行。
- 运行交互、固定帧与性能验收，确认自动注册能读到相机、游戏快照和完整的性能样本。核对性能样本去重、最终报告字段和离屏帧率。
- 启动共享分析器的提取验证，核对选中的分析包、源码路径、主应用和渲染子应用快照。通过 `master` 推送检查站点生成、发布证明和线上页面。

## 未决问题

迁移分支的验证结果随 PR 记录。Pages 发布以合并后 `master` 的工作流结果为准。

## 来源

[bevy-cicd 的虚拟 workspace 分析包配置](https://github.com/fedoraiver/bevy-cicd/blob/1f760e6cbad20fa9c0749201b513f0c0cdfdf954/README.md)。
