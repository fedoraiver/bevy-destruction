# ADR 0022：由独立脚本驱动无窗验收场景

## 状态

2026-11-23 采用并实现。更新 [ADR 0018](0018-windowless-acceptance.md) 的无窗验收场景选择方式。

2026-11-28 随[演示设置迁移](0006-demo-scenes-and-controls.md)采用原生 TOML 设置文件，供各用例加载并作为验收产物保存。

后续由 [ADR 0026](0026-python-driven-acceptance.md) 更新用例职责和设置恢复方式。此记录保留迁移前实现和验证。

## 背景

验收二进制先前直接解析场景、资产、探针、输出路径和性能参数。不同用例需要不同的输入、断言和报告，同时可共用一个无窗 Bevy 应用入口。

## 决策与实现

- `controls.py`、`gameplay.py`、八个固定帧探针脚本和 `profile.py` 提供独立入口。Python 驱动的 `Scenario` 定义各用例的场景和开关，`run.py` 提供单项、基线和完整验收。
- 每个用例的输出目录保存独立的 `settings.toml`，其中的 `[demo_settings]` 记录资产和 `stress`、`vfx`、`detail`、`soft_particles`。二进制无参数启动，从 `BEVY_DESTRUCTION_ACCEPTANCE_SCENARIO` 的 JSON 读取探针类型 `probe`、绝对输出目录 `output`、`capture_replay` 和 `profile`。Rust 校验启动参数，使用同一个 Bevy `SettingsPlugin` 从输出目录同步加载 `DemoSettings`，再安装游戏插件。
- 固定步断言、阶段截图、回放和性能测量由探针执行。冒烟报告将开关记在 `settings` 对象中，顶层保存特效与破口边带的指标。

## 备选方案与取舍

每个探针使用单独的 Rust 二进制会重复构建和维护无窗装配。通过 BRP 在运行中选择探针需要额外协调初始资产、固定时钟和渲染管线配置。脚本传递启动配置，允许共用二进制并在初始化前确定探针设置。资产与开关曾通过环境 JSON 传递。将它们写入用例输出目录后，验收与桌面共用 Bevy 的解析和默认值规则，并可从产物检查每次运行的输入。环境变量只传递启动探针所需的元数据。

## 验证要求与结果

检查独立脚本和汇总矩阵，确认报告中的进程命令只有可执行文件。运行原生交互、固定帧基线和性能验收，确认报告、截图和退出码继续按原断言生成。

2026-11-23 的两组原生交互、17 个固定帧用例和性能脚本均通过。20 次运行的报告都记录了无参数启动。无效场景配置在应用初始化前被拒绝，进程返回非零退出码。

TOML 迁移后，`controls`、`smoke` 和 `landing` 验收均通过。核对了各用例的设置文件与无参数启动记录，`landing` 报告确认加载板式高楼。矩形楼 `smoke` 从含中文和空格的输出目录加载设置，报告中四个开关均为 `false`，首击粒子数和边带三角形数均为零。脚本启动配置缺少设置文件时，Rust 在应用初始化前返回错误。

## 未决问题

引入需要首帧前初始化的新探针时，应核对它期望的时钟、渲染与资产状态。

## 来源

独立脚本的组织参考 `D:/demo` 的 `6425fd3`。[无窗入口](../../../apps/acceptance/src/main.rs) · [探针装配](https://github.com/fedoraiver/bevy-destruction/blob/3d535f2020d40f4243a7ce5d4ee5943e85bbf1e9/src/acceptance/mod.rs) · [Python 驱动](../../../scripts/acceptance/run.py)
