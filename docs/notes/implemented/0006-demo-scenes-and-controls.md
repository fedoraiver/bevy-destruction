# ADR 0006：演示场景与交互

## 状态

已采用并实现。2026-11-17 采用 BSN 场景组合、自由视角和正式场景目录。

2026-11-24 修正发射瞄准使用上一帧相机位姿的问题。

2026-11-24 将点击破坏射线迁移到 Bevy 消息。

2026-11-24 将场景切换的撞击状态重置迁移到 Bevy Observer。

2026-11-24 点击破坏的有限边最近点计算改用 Bevy `Segment3d`。

2026-11-24 调试边线绘制由 Bevy 调度条件控制。

2026-11-28 桌面和无窗验收的启动设置迁移到 Bevy 原生 `SettingsPlugin`。

## 背景

演示需要沿外墙、室内及近景观察建筑，并通过实际碰撞位置触发局部破坏。高楼和离散桥墩用于观察不同尺度、平面和支撑路径下的表现。

## 决策与实现

- 用 Bevy 的 Rust `bsn!` 宏组合相机、灯光和碎块显示实体。读取 v2 清单及指定 GLB，待校验和依赖就绪后按 Chunk ID 建立实例；同一资产可在加载完成后继续实例化。资产契约见 [ADR 0005](0005-v2-asset-contract.md)。
- 相机采用 60° 竖直视场的透视投影，裁剪距离为 0.05 至 500 m，切换场景后按建筑包围盒取景一次。WASD 沿相机方向移动，Space/Ctrl 沿世界竖直方向升降，输入归一化后以 6 m/s 移动。鼠标相对位移按 0.002 rad/px 更新，俯仰限制为 ±89°。Tab 切换鼠标捕获，失焦后暂停控制，恢复捕获的点击仅恢复控制。
- 主键盘及数字小键盘的 `0` 切换橙色预破碎边线。线条取自焊接重复顶点后的真实折角与边界，跳过共面三角化对角线，保留深度遮挡。绘制系统通过 `run_if` 读取开关，在变换传播后绘制。`9` 切换受力热力，按住 `Alt` 详查准星目标，见 [ADR 0009](0009-structural-diagnostics.md)。最初由 `Esc` 退出，现改为打开暂停菜单。
- 左键通过中央准星查询最近的存活建筑 Collider；命中块必删，扩展范围仅限该块的直接连接邻块。半径 0.4 m、深度带 0.08 m、法线点积至少 0.7，每次最多删除 6 块。立即更新显示、存活状态和相邻连接，Collider 换属由 [ADR 0001](0001-rigid-cluster-runtime.md) 的固定步统一处理。表面和碎屑分别见 [ADR 0007](0007-fracture-surface-rendering.md)、[ADR 0008](0008-destruction-effects-and-rims.md)。
- 右键发射刚性球，`R` 发射刚性方块。发射输入在相机移动后用 Bevy 的 `TransformHelper` 计算当前层级位姿，以该位姿确定发射起点和方向。撞击物的刚体参数及真实接触破坏见 [ADR 0003](0003-collision-driven-fracture.md)。
- `DamageRequest` 携带一条点击破坏射线。玩家输入先发送消息，`apply_damage` 在同一次 `Update` 依序读取。冒烟探针在消费系统运行后发送的批量射线由下一次 `Update` 读取。

| 原场景快捷键 | 场景 | 内容 |
| --- | --- | --- |
| F1 | `rectangle` | 矩形两层楼 |
| F2 | `highrise_slab` | 十层板式高楼 |
| F3 | `highrise_l` | 八层 L 形高楼 |
| F4 | `bridge` | 28 m 桥面、双钢主梁、六根独立桥墩 |
| F5 | `courtyard` | 有庭院的两层楼 |
| F6 | `l_shape` | L 形三层楼 |

桌面和无窗入口通过 Bevy 原生 `SettingsPlugin` 从系统偏好目录读取 `settings.toml` 的 `[demo_settings]`，在运行前应用 `DemoSettings`。当前使用 `scene` 路径及暂停菜单重载场景。架构导出使用内置设置，Python 验收暂时替换默认设置文件。[场景与暂停菜单](0024-scene-settings-and-pause-menu.md) · [无窗验收](0025-python-driven-acceptance.md)


切换场景时，目录系统在销毁旧建筑和撞击物之后、生成新建筑之前，用 `Commands::trigger` 排入 `ResetProjectiles`。撞击物插件的 Observer 清空上一场景的发射请求、接触工况和计数。延迟命令在建筑实例化和相机更新前应用，同帧的新发射输入随后进入队列。

## 备选方案与取舍

整场景 glTF 实例化后改造需要额外处理节点变换、显示分组和运行时组件。直接读取网格资源并用 BSN 组合场景，可按清单显式维护 Chunk 与 Collider 的对应关系。透视自由飞行提供进深反馈，便于进入建筑检查门窗、正面和背面。

相机输入和发射运行在 `Update`，读取缓存的 `GlobalTransform` 要等到 `PostUpdate` 传播后才反映本帧运动。即时读取层级位姿使发射沿用现有输入顺序及固定步队列。

点击射线曾经写入手工队列。消费者每次 `Update` 都运行，Bevy 消息的跨帧缓冲覆盖玩家输入和冒烟探针两种时序。撞击物发射请求由固定步消费，需跨越任意次数未运行固定步的 `Update`，继续使用耐久队列。

撞击状态重置曾以独占 `World` 命令执行。Observer 通过 `ResMut` 声明并更新所属资源，仍在场景切换的同一延迟命令队列执行。

点击候选面先按投影点和半空间容差判断面内命中。投影超出三角形时，以 Bevy `Segment3d::closest_point` 查询三条有限边并取最近点。候选深度沿 Avian 命中法线测量。

桌面入口曾通过 `--scene`、`--asset` 和 `--no-*` 接收逐次运行的参数。TOML 文件便于在多次启动之间复用设置。Bevy 负责文件定位、类型解析和默认值装配。桌面入口通过 `cargo run --locked` 启动。

## 验证与未决问题

- 回归覆盖资产加载后再次实例化、自由相机和鼠标捕获、调试边线、中央射线、受击表层选取及背墙保护，以及显示与 Collider 的生命周期。
- 发射回归在同一 `Update` 先平移和转动带父实体的相机再触发发射，检查起点和方向采用本帧位姿。测试和矩形场景 `controls` 无窗验收通过。
- 点击请求回归通过，验证在消费系统之后发送的射线下一次 `Update` 才生效，并且只产生一次破坏反馈。矩形场景 `controls` 无窗验收通过。冒烟测试覆盖批量射线。
- 场景重置回归通过，验证旧发射、接触和统计清零，并保留同一 `Update` 的新发射输入。矩形场景 `controls` 无窗验收覆盖真实 F 键切换场景，已通过。
- 邻面距离回归通过，覆盖边中部、有限边端点、面内容差以及倾斜面的命中轴深度。矩形场景 `smoke` 无窗验收通过。
- 最终正式矩形在引擎中通过点击、重复实例化及偏侧支撑倾倒。细长代理修复提交 `dfd8205` 的六场景 Collider 构造回归通过，正式 L 形通过关闭 VFX 的交互回放。该次默认 VFX 的 L 形回放通过加载及首击，粒子回收检查失败。
- 初次迁移桌面设置时，矩形楼的旧版 `smoke` 验收在四个开关关闭时通过。报告确认四项均为 `false`，首击粒子数和边带三角形数均为零。
- 初次迁移时，设置回归通过，覆盖隔离 TOML 中的资产与四个开关、缺失文件、部分字段和探针参数隔离。该次无窗验收的设置加载与产物验证见 [ADR 0022](0022-script-driven-acceptance-scenarios.md#验证要求与结果)。
- 当前相机为固定默认速度与灵敏度的无碰撞自由飞行。行走或相机碰撞需单独设计与验证。

## 来源

[运行方式](../../../README.md)、[场景目录](../../../src/catalog/mod.rs)、[场景组合](../../../src/scene/mod.rs)、[相机](../../../src/camera/mod.rs)和[局部交互](../../../src/interaction/mod.rs)。

[发射系统](../../../src/projectiles/mod.rs) · [Bevy 变换传播](https://docs.rs/bevy/0.20.1/bevy/transform/components/struct.Transform.html#transform-and-globaltransform)

[点击请求](../../../src/interaction/mod.rs) · [Bevy 消息缓冲](https://docs.rs/bevy/latest/bevy/ecs/message/struct.Messages.html)

[场景切换](../../../src/catalog/mod.rs) · [Bevy Observer](https://docs.rs/bevy/latest/bevy/ecs/prelude/struct.Observer.html)

[Bevy 有限边最近点](https://docs.rs/bevy/latest/bevy/shape/struct.Segment3d.html#method.closest_point)

[线框绘制](../../../src/debug/mod.rs) · [Bevy 调度条件](https://docs.rs/bevy/latest/bevy/ecs/schedule/trait.IntoScheduleConfigs.html#method.run_if)

[桌面装配](../../../apps/game/src/lib.rs) · [Bevy 0.20.1 设置插件源码](https://github.com/bevyengine/bevy/blob/v0.20.1/crates/bevy_settings/src/lib.rs)
