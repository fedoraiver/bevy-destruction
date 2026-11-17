# Bevy Destruction

面向 Bevy 0.20 + Avian 0.8 的建筑物理破坏插件项目。

已实现建筑资产导入、自由飞行、点击破坏、分层烟尘与暴露破口、失支撑刚体下落，并接入六自由度连接应力、自重承载、渐进过载断裂与刚性撞击物。动态大块落地时会把接触载荷传到内部连接，逐处断裂并在分离后卸载，保留成组残骸；程序化网格与断面材质随碎块运动。受损窄支撑的稳健求解和破坏范围见 [ADR 0016](docs/adr/0016-damaged-structure-solver-and-support-release.md)、[ADR 0017](docs/adr/0017-progressive-impact-unloading.md)。

## 运行

需要 Rust 1.97.1 或以上及支持 Bevy 的图形环境。从仓库目录执行：

```powershell
cargo run
cargo run -- --scene highrise_slab
cargo run -- --scene bridge
```

`WASD` 沿相机前后左右移动，鼠标转向，`Space` 沿世界竖直方向上升，左/右 `Ctrl` 下降。视角采用透视投影，加载完成后只取景一次，此后由用户自由移动。

窗口获得控制时隐藏并锁定鼠标，画面中央显示准星。左键删除准星指向的局部碎块；右键发射刚性球，`R` 发射刚性方块。`Tab` 释放或重新锁定鼠标；失去窗口焦点时暂停移动并释放鼠标，重新点击窗口恢复控制，这次点击不触发破坏。主键盘 `0` 或数字小键盘 `0` 切换橙色预破碎边界，`Esc` 退出。

`F1` 至 `F6` 切换场景并重新取景；也可用 `--scene` 指定启动场景：

| 按键 | 场景名 | 内容 |
| --- | --- | --- |
| F1 | `rectangle` | 矩形两层建筑 |
| F2 | `highrise_slab` | 十层板式高楼 |
| F3 | `highrise_l` | 八层 L 形高楼 |
| F4 | `bridge` | 28 m 桥面、双钢主梁、六根独立桥墩 |
| F5 | `courtyard` | 庭院两层建筑 |
| F6 | `l_shape` | L 形三层建筑 |

两种撞击物均为有限质量 8 t、初速 55 m/s 的动态刚体，会受重力及碰撞影响并交换动量，自身保持刚性且不破碎。球半径为 0.65 m，方块边长为 1.2 m；同时最多保留 12 个，寿命为 20 秒。

新资产由 [Blender 工具](tools/blender/README.md)导出到 `assets/buildings`，以 `.destruction.json` 为加载入口；自定义文件通过 `--asset buildings/name.destruction.json` 加载。对应 GLB 需保留在同目录，内容哈希错误会拒绝加载。运行资产根目录固定为构建时仓库的 `assets`，直接运行生成的可执行文件也使用同一目录。

应力计算默认启用：对仍与地基连接的结构求解自重和外加载荷，恢复连接拉、压、剪应力及弯曲、扭转影响，持续过载会积累损伤并断开连接。HUD 显示最大承载率、过载断键数、无效求解数量和毫米级弹性位移估计；位移估计用于承载诊断，可见建筑仍按刚体簇运动。受损窄支撑需要时使用稀疏因子与补偿残差修正，保留 `1e-8` 默认精度；不合格结果仍标为无效并重试，全部无效时显示 `unavailable (retrying)`，不显示零承载率。`--no-stress` 可关闭此承载路径，保留直接点击与撞击破坏。

左键每次最多删除 6 个碎块。失去全部固定锚点的连通部分会转为动态残骸，在重力下下落、与地面和建筑碰撞并休眠；运动中仍可点击拆分并继承原速度。多块动态残骸的快速碰撞会扣除整体刚体加速度，使用平衡后的接触载荷求解内部连接力与力矩；完整自由落体不会因此自行损坏。静态建筑和单块仍采用局部冲量断裂路径。接触或休眠不会生成永久锚点，物理冲量不重复施加。当前采用小变形弹性网络和 10 ms 冲量标定时间，未模拟完整应力波、大变形或可见挠曲。

动态撞击每断开一个连接，都在当前图上重新判断后续失效；原簇分离后释放原撞击事件，不继续执行旧整簇断裂名单。后续损伤由新的真实接触触发。偏侧残余支撑可通过接触使上部自然转动；完全失去支撑的直立结构也可能先垂直落下，不人为添加倾倒力矩。

默认开发构建针对结构求解与碰撞依赖启用优化，并缓存未变化的锚定结构。此前板楼初始应力求解优化约 `357 → 30 ms`，相同 300 步无图形落地测试约 `10.04 → 2.30 s`；这些是 [ADR 0015](docs/adr/0015-structural-and-collision-runtime-performance.md) 阶段的测量，不是渲染帧率保证。当前稳健求解增加原始单元残差校验，测量和验证边界见 ADR 0016。

破坏依次产生短促粉爆、不规则主碎屑、扩散烟尘和边缘落灰；少量主碎屑与存活建筑碰撞、反弹。烟尘具有连续淡出、主光明暗和交界深度淡出。邻块消失或连接断开后，暴露侧生成带骨料材质的闭合棱角边带，并随所属碎块运动；完整建筑和其他实例不提前显露裂缝。碰撞仍使用资产的凸代理近似，新增边带不改变碰撞形状。

## 验证

当前已通过 82 项 Rust 测试；此前资产流程已通过 28 项 Blender 测试及六个场景的应力/刚性撞击图形验收。验证范围分别见 [ADR 0010](docs/adr/0010-elastic-interface-stress-and-overload-fracture.md)、[ADR 0013 材质附着](docs/adr/0013-object-attached-procedural-surfaces.md)、[ADR 0016 受损支撑](docs/adr/0016-damaged-structure-solver-and-support-release.md)、[ADR 0017 选择性撞击断裂](docs/adr/0017-progressive-impact-unloading.md)。

```powershell
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo run -- --material-smoke-test output/acceptance/material-attached
cargo run -- --scene rectangle --collapse-smoke-test output/acceptance/support-tipping
cargo run -- --scene highrise_slab --landing-smoke-test output/acceptance/selective-landing-highrise_slab
cargo run -- --scene rectangle --stress-smoke-test output/acceptance/stress-rectangle
cargo run -- --scene highrise_slab --stress-smoke-test output/acceptance/stress-highrise_slab
cargo run -- --scene highrise_l --stress-smoke-test output/acceptance/stress-highrise_l
cargo run -- --scene bridge --stress-smoke-test output/acceptance/stress-bridge
cargo run -- --scene courtyard --stress-smoke-test output/acceptance/stress-courtyard
cargo run -- --scene l_shape --stress-smoke-test output/acceptance/stress-l_shape
```

`--stress-smoke-test` 先检查所选真实资产的完整自重求解，再用具有明确 SI 质量和截面的双柱混凝土梁检查支撑移除、仍连通时的过载、自动断键、下落与休眠，最后发射球和方块验证真实碰撞、撞击断裂与撞击物保持完整。输出 7 张阶段截图和 `report.json`，失败返回非零退出码。验证以固定 60 Hz 接收预定输入，截图由引擎直接生成；正常运行使用实际时间和输入。

原交互与刚体簇路径保留独立验证入口：

```powershell
cargo run -- --smoke-test output/acceptance/rectangle
cargo run -- --asset buildings/l_shape.destruction.json --smoke-test output/acceptance/l_shape
cargo run -- --asset buildings/courtyard.destruction.json --smoke-test output/acceptance/courtyard
cargo run -- --physics-smoke-test output/acceptance/physics
cargo run -- --asset buildings/l_shape.destruction.json --physics-smoke-test output/acceptance/physics_l_shape
cargo run -- --asset buildings/courtyard.destruction.json --physics-smoke-test output/acceptance/physics_courtyard
```

`--smoke-test` 会打开演示窗口，以固定 60 Hz 模拟执行加载后再次实例化、相机控制、鼠标点击、连续破坏与特效回收检查，保存截图和 `report.json`，然后退出；失败返回非零退出码。验证模式只接收预定的控制输入，正常运行使用实际时间和实际输入。截图由引擎直接生成，无需桌面自动化。

`--physics-smoke-test` 独立检查屋顶双块失支撑下落、运动中再次拆分、质量/位姿/速度继承及接触后休眠，保存 3 张阶段截图和报告。各验证模式互斥；原表现对照可加 `--no-stress`。

`--material-smoke-test` 比较真实材质在平移、旋转前后的图像，并加入世界锁定负对照，输出四张对照图和报告。`--landing-smoke-test` 暖机后移除高楼下半部分，使完整上半部分仅受重力落地；检查接触前不损坏、内部楼层断键、保留成组结构、质量保留、有限状态和力/力矩平衡，输出四张阶段图及含簇成员/空间范围的报告。建议使用 `highrise_slab` 或 `highrise_l`。

`--collapse-smoke-test` 使用默认矩形楼，移除首层 61 块并保留偏侧门框，检查强度断裂后大块上部下降、倾斜及其与静态残墙的真实接触冲量，不注入速度或力矩。输出四张阶段图、可回放的 `damage.json` 和数值/质量报告。

验证包含 0.05 / 0.15 / 0.4 / 1 / 3 秒分阶段画面，以及烟尘散尽后的破口近景。可用以下参数做同路径比较：

```powershell
cargo run -- --smoke-test output/vfx-replay --capture-replay
cargo run -- --smoke-test output/geometry-only --no-vfx
cargo run -- --smoke-test output/baseline --no-vfx --no-detail --no-soft-particles
cargo run --release -- --smoke-test output/profile --profile
```

`--capture-replay` 另存 30fps 的 PNG 序列到 `replay`。`--profile` 关闭截图，记录实际墙钟帧间隔、VFX 生成/更新、边带缓存/更新及渲染通道 CPU/GPU 时间（GPU 项取决于设备支持）；固定模拟时间不作为帧率测量。`--no-detail` 单独关闭新增边带，`--no-soft-particles` 关闭深度预通道和交界淡出。粒子总量限 640，边带显示几何限 180,000 三角形；这些数量上限不代表 60fps 保证。

Blender 资产测试与插件使用方法见[工具说明](tools/blender/README.md)。

- [资产格式](tools/blender/ASSET_FORMAT.md)
- [设计决策](docs/adr/README.md)
