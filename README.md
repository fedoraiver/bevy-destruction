# Bevy Destruction

Bevy Destruction 是基于 Bevy 0.20.1 和 Avian 0.8.0 的建筑破坏插件。它使用 Blender 预破碎资产，在运行时模拟自重过载、撞击断裂和失去支撑后的刚体运动。内置六个演示场景，可交互破坏并查看连接受力。

![矩形楼失去部分首层支撑后倾倒](docs/images/rectangle-tipping.webp)

## 快速开始

在仓库根目录使用 Rust 1.97.1 或以上版本运行。桌面演示需要支持 Bevy 的图形环境。

```powershell
cargo run --locked
```

默认打开两层矩形楼。六个内置场景的清单都在 [assets/buildings/](assets/buildings/)。

### 桌面设置

在系统偏好目录下创建 `settings.toml`。桌面演示启动时读取此文件。编辑设置后，按 `Esc` 打开暂停菜单，选择 **Reload** 应用设置并重置场景。

| 平台 | 设置文件 |
| --- | --- |
| Windows | `%LOCALAPPDATA%\io.github.fedoraiver.bevy-destruction\settings.toml` |
| Linux | `$XDG_CONFIG_HOME/io.github.fedoraiver.bevy-destruction/settings.toml`，默认 `~/.config/io.github.fedoraiver.bevy-destruction/settings.toml` |
| macOS | `~/Library/Preferences/io.github.fedoraiver.bevy-destruction/settings.toml` |

以下示例打开板式高楼，并关闭烟尘与场景交界处的深度淡出。

```toml
[demo_settings]
scene = "buildings/highrise_slab.destruction.json"
stress = true
vfx = true
detail = true
soft_particles = false
```

缺少文件或字段时使用默认值。`scene` 填写相对 `assets/` 的 `.destruction.json` 清单路径，默认值为 `buildings/rectangle.destruction.json`。四个布尔开关默认均为 `true`，设为 `false` 可关闭对应功能。

| 设置项 | 控制功能 |
| --- | --- |
| `stress` | 锚定结构的承载求解 |
| `vfx` | 破坏特效 |
| `detail` | 破口边带 |
| `soft_particles` | 烟尘和场景交界处的深度淡出 |

### 操作

| 输入 | 动作 |
| --- | --- |
| `WASD`、鼠标 | 移动、转向 |
| `Space`、`Ctrl` | 沿世界竖直方向升降 |
| 左键、右键、`R` | 删除准星处的碎块、发射刚球、发射刚性方块 |
| `0` | 切换预破碎边界 |
| `9`、`Alt` | 切换承载率热力。开启后按住 `Alt` 查看准星处的连接载荷和接触冲量 |
| `Tab` | 释放或锁定鼠标 |
| `Esc` | 暂停或返回游戏。菜单提供 **Resume**、**Reload** 和 **Quit** |

窗口失焦会释放鼠标。重新点击窗口时，首次点击用于恢复控制。

桌面和无窗验收默认使用 1920×1080 的逻辑尺寸。

## 导入自己的建筑

使用 Blender 工具把建筑导出为 v2 清单及配套 GLB，将两者放在 `assets/buildings/`。在 `settings.toml` 的 `[demo_settings]` 中添加 `scene = "buildings/文件名.destruction.json"` 后启动。GLB 必须与清单中的文件名和内容哈希匹配。运行时从构建时工作区的 `assets/` 读取文件。[Blender 工具](tools/blender/README.md) · [v2 资产格式](tools/blender/ASSET_FORMAT.md)

## 开发文档

- [插件说明](docs/plugin.md)介绍资产、ECS 数据、刚体簇、自重承载、撞击断裂和受力诊断。
- [开发规范](docs/development.md)覆盖 ECS 数据归属、系统调度、实体装配、模块组织和修改后的验证。
- [测试指南](docs/testing.md)介绍如何为代码、资产及运行表现选择验证入口。
- [在线 ECS 架构图](https://fedoraiver.github.io/bevy-destruction/)由 bevy-cicd 从桌面应用注册的调度生成。

## 验证

从仓库根目录运行常规 Rust 检查。CI 还会对游戏包运行实体命名和具名字段规则。[Rust CI 配置](.github/workflows/rust.yml)

```powershell
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
```

### 无窗验收

需要 Python 3.11 或以上、Git 和支持 wgpu 离屏渲染的适配器。以下为 Windows PowerShell 命令。

```powershell
python -m venv tmp/acceptance-venv
.\tmp\acceptance-venv\Scripts\python.exe -m pip install -r scripts/acceptance/requirements.txt
cargo build --locked --release -p bevy-destruction-acceptance --features acceptance-test
$acceptanceExe = 'target/release/bevy-destruction-acceptance.exe'
.\tmp\acceptance-venv\Scripts\python.exe scripts/acceptance/run.py --exe $acceptanceExe
```

macOS 和 Linux 使用 `tmp/acceptance-venv/bin/python`，可执行文件省去 `.exe`。验收程序统一无参数启动。Python 脚本定义各用例的资产和功能开关，将其写入用例输出目录的 `settings.toml`，由 Bevy 原生设置插件读取。`run.py` 默认运行原生输入 `controls`、交互 `gameplay` 以及 17 个固定帧用例。`output/acceptance/run-*/` 保存汇总、用例设置、报告、日志和阶段截图。任一用例失败时命令返回非零退出码。

```powershell
.\tmp\acceptance-venv\Scripts\python.exe scripts/acceptance/gameplay.py --exe $acceptanceExe
.\tmp\acceptance-venv\Scripts\python.exe scripts/acceptance/landing.py --exe $acceptanceExe
.\tmp\acceptance-venv\Scripts\python.exe scripts/acceptance/smoke.py --capture-replay --exe $acceptanceExe
.\tmp\acceptance-venv\Scripts\python.exe scripts/acceptance/profile.py --exe $acceptanceExe
```

`run.py baseline` 单独运行固定帧矩阵。单个套件使用脚本定义的场景配置，例如 `landing.py` 加载板式高楼，`smoke.py` 加载矩形楼。可在 `Scenario` 中设置单个用例的开关，例如 `Scenario("smoke", "rectangle", vfx=False)`。[用例定义](scripts/acceptance/run.py)

`--output` 指定输出目录，`--port` 指定 BRP 的本地端口，并行运行时应分配不同端口。`--timeout` 以秒指定单次等待上限。各脚本的选项可通过 `--help` 查看。
