# ADR 0026：Python 驱动无窗验收

## 状态

2026-11-28 采用，2026-11-29 实现。替代 [ADR 0022](0022-script-driven-acceptance-scenarios.md) 的 Rust 固定帧探针装配。

2026-11-29 将共享设置整理到独立模块，并明确游戏 BRP 支持与无窗验收入口的 Cargo 功能归属。

2026-11-29 将 Rust 插件和 Python SDK 的依赖声明切换到上游 `master` 分支。

## 背景

无窗用例可借助原生 BRP 查询 ECS、注入输入和截图。项目原先在 Rust 中维护测试阶段并写报告，Python 负责启动和检查输出。

## 决策与实现

- `apps/acceptance` 独立声明 Bevy 装配，直接安装各功能插件，通过原生设置插件读取与桌面相同的设置文件，并配置 `AcceptancePlugin`、离屏相机、逻辑窗口与运行器。
- `apps/acceptance` 声明 `acceptance-test` Cargo 功能并启用游戏库的 `brp` 功能。后者为脚本查询和修改游戏 ECS 数据提供反射信息与观测统计。[游戏包功能](../../../apps/game/Cargo.toml) · [无窗验收包功能](../../../apps/acceptance/Cargo.toml)
- 共享 `DemoSettings` 定义在 `src/settings.rs`，沿用 `bevy_destruction::DemoSettings` 作为 BRP 类型路径。[共享设置类型](../../../src/settings.rs) · [Python BRP 调用](../../../scripts/acceptance/runtime.py)
- Python 选择场景和开关，为无窗进程写入默认 `settings.toml`。关闭子进程后恢复原始字节，原文件缺失时删除夹具文件。
- 各用例通过生产组件、资源和消息验证游戏。Python 控制动作、截图、报告和取证顺序。需要精确验证求解前状态的断言由模块内 Rust 回归覆盖。
- Rust 插件和 Python SDK 共用 `bevy-acceptance-test` 的上游 `master` 分支。Cargo 将当前解析的插件提交记入 `Cargo.lock`，Python 的需求文件在安装时解析分支。[Rust 依赖](../../../apps/acceptance/Cargo.toml) · [Python 依赖](../../../scripts/acceptance/requirements.txt) · [Cargo 锁文件](../../../Cargo.lock)

## 备选方案与取舍

固定帧 Rust 探针可在两道物理系统之间采样，但会把测试编排带入应用。通过 BRP 等待可观测状态，Python 可以复用实际游戏输入与截图。连续运行中的 BRP 查询和截图会跨越更新，报告按所观察到的状态记证据。性能用例让无窗运行器连续更新，并采样 Bevy 时钟。

跟踪同一上游分支可以沿用插件与 SDK 的配套更新。两者各自解析依赖，更新后需运行无窗验收确认兼容性。

## 验证要求与结果

切换到上游分支后，Cargo 与新建 Python 环境均解析到 `317abe0`。`controls`、`smoke` 和两项 Python 设置恢复测试通过。

功能归属调整后，Cargo 特性检查确认单独构建桌面游戏时使用默认特性，无窗入口的 `acceptance-test` 启用游戏库的 `brp`。`controls`、`gameplay` 和 `smoke` 均通过既有 BRP 类型路径读取共享设置并完成验收。

- 两组原生输入用例、17 个场景组合和独立性能用例均通过。冒烟回放生成建筑完整、破开和稳定后三个阶段的截图。
- 设置恢复回归通过。用例失败时复原已有文件的原始字节，通过 Ctrl+C 中断时恢复文件缺失的状态，并验证关闭子进程后才复原文件。完整矩阵运行后也核对了默认设置文件的原始状态。
- 迁移验收时的 145 项工作区 Rust 回归通过，包括需要在同一物理更新内采样的断言。
- 入口改为直接装配后，143 项 Rust 回归、`controls`、默认 `smoke` 和四个开关全关的 `smoke` 通过。无窗用例结束后核对了默认设置文件的原始状态。

## 未决问题

上游 BRP 当前的状态读取与截图分别处理。需要严格关联渲染帧号的验证仍依赖可控步进能力。

## 来源

[无窗装配](../../../apps/acceptance/src/main.rs) · [Python 验收入口](../../../scripts/acceptance/run.py) · [上游跟踪分支](https://github.com/fedoraiver/bevy-acceptance-test/tree/master) · [迁移时使用的上游插件](https://github.com/fedoraiver/bevy-acceptance-test/blob/4a50239b27fd6c05e49ce7662832d718e094d95c/src/lib.rs)
