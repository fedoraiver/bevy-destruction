# Bevy Destruction

面向 Bevy 0.20 + Avian 0.8 的建筑物理破坏插件项目。

已实现建筑资产导入、自由飞行、点击破坏、分层烟尘与暴露破口、失支撑刚体下落，并接入六自由度连接应力、自重承载、渐进过载断裂与刚性撞击物。动态大块落地时会把接触载荷传到内部连接，按接触工况处理断裂并在分离后卸载，保留成组残骸；程序化网格与断面材质随碎块运动。受损窄支撑的稳健求解和破坏范围见 [ADR 0002](docs/adr/0002-elastic-structure-and-overload.md)和 [ADR 0003](docs/adr/0003-collision-driven-fracture.md)。

## 演示画面

![矩形楼上部绕残余支撑倾倒](docs/images/rectangle-tipping.webp)

矩形楼的大部分首层支撑移除后，上部以刚体簇倾倒。完整楼体、窄支撑和倾倒阶段见[矩形楼失支撑过程](docs/adr/0002-elastic-structure-and-overload.md#矩形楼失支撑过程)。

## 运行

需要 Rust 1.97.1 或以上及支持 Bevy 的图形环境。从仓库目录执行：

```powershell
cargo run
cargo run -- --scene highrise_slab
cargo run -- --scene bridge
```

桌面演示窗口和无窗验收画布的默认逻辑尺寸均为 1920×1080。

`WASD` 沿相机前后左右移动，鼠标转向，`Space` 沿世界竖直方向上升，左/右 `Ctrl` 下降。视角采用透视投影，加载完成后只取景一次，此后由用户自由移动。

窗口获得控制时隐藏并锁定鼠标，画面中央显示准星。左键删除准星指向的局部碎块；右键发射刚性球，`R` 发射刚性方块。`Tab` 释放或重新锁定鼠标；失去窗口焦点时暂停移动并释放鼠标，重新点击窗口恢复控制，这次点击不触发破坏。`0` 切换橙色预破碎边界，`9` 切换受力热力（均支持数字小键盘），`Esc` 退出。开启热力后按住 `Alt`，查看准星指向的碎块。

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

新资产由 [Blender 工具](tools/blender/README.md)导出到 `assets/buildings`，以 `.destruction.json` 为加载入口；自定义文件通过 `--asset buildings/name.destruction.json` 加载。只接受 v2 资产，连接必须具有真实接触多边形，显示材质必须具有明确的表面角色；旧资产需重新导出。对应 GLB 需保留在同目录，内容哈希错误会拒绝加载。运行资产根目录固定为构建时仓库的 `assets`，直接运行生成的可执行文件也使用同一目录。

六个正式场景已替换为自然不规则分块。随机种子点与非周期扰动决定弯折边界，并约束碎块大小；每块由精确复合凸代理参与碰撞，多面片连接保留各自方向、面积及截面量。源网格、洞口、质量、质心和完整惯性均纳入导出检查，仍按刚体簇运动，不显示弹性或弯曲形变。算法、尺寸及代理上限见 [ADR 0004](docs/adr/0004-blender-asset-pipeline.md)。

`9` 受力热力默认关闭。开启后，每块按有效相邻连接中最高的承载率 U 着色。低于 50% 或暂无有效结果保留原色，达到 50% 为琥珀色，达到 100% 为红色。颜色随原网格和断面纹理显示，墙后碎块由正常深度遮挡。完好建筑负载较低时可能只出现图例。

按住 `Alt` 瞄准可见碎块，面板列出承载率最高的三条有效连接及合力 F、力矩 M、承载率 U。准星附近的一支箭头表示第一条连接作用在该块上的力方向，大小看面板数值。撞击结果标为 `Impact J/10 ms`，实际法向接触冲量 J 单独以 N·s 显示。松开 `Alt` 收起详查。取舍与验证见 [ADR 0009](docs/adr/0009-structural-diagnostics.md)。

应力计算默认启用：对仍与地基连接的结构求解自重和外加载荷，恢复连接拉、压、剪应力及弯曲、扭转影响，持续过载会积累损伤并断开连接。HUD 显示最大承载率、过载断键数、无效求解数量和毫米级弹性位移估计；位移估计用于承载诊断，可见建筑仍按刚体簇运动。受损窄支撑需要时使用稀疏因子与补偿残差修正，保留 `1e-8` 默认精度；不合格结果仍标为无效并重试，全部无效时显示 `unavailable (retrying)`，不显示零承载率。`--no-stress` 可关闭此承载路径，保留直接点击与撞击破坏。

左键每次最多删除 6 个碎块。失去全部固定锚点的连通部分会转为动态残骸，在重力下下落、与地面和建筑碰撞并休眠；运动中仍可点击拆分并继承原速度。多块动态残骸的快速碰撞会扣除整体刚体加速度，使用平衡后的接触载荷求解内部连接力与力矩；完整自由落体不会因此自行损坏。静态建筑和单块仍采用局部冲量断裂路径。接触或休眠不会生成永久锚点，物理冲量不重复施加。当前采用小变形弹性网络和 10 ms 冲量标定时间，未模拟完整应力波、大变形或可见挠曲。

动态撞击每断开一个连接，都在当前图上重新判断后续失效；原簇分离后释放原撞击事件，不继续执行旧整簇断裂名单。后续损伤由新的真实接触触发。偏侧残余支撑可通过接触使上部自然转动；完全失去支撑的直立结构也可能先垂直落下，不人为添加倾倒力矩。

默认开发构建针对结构求解与碰撞依赖启用优化，并缓存未变化的锚定结构。此前板楼初始应力求解优化约 `357 → 30 ms`，相同 300 步无图形落地测试约 `10.04 → 2.30 s`；测量对应更换不规则分块前的旧资产，范围见 [ADR 0010](docs/adr/0010-runtime-performance.md)。当前稳健求解增加原始单元残差校验，见 [ADR 0002](docs/adr/0002-elastic-structure-and-overload.md)。

进一步优化复用同一撞击事件的平衡载荷，直接读取 v2 面片并移除旧截面表构建，跳过补偿计算中精确为零的系数；烟尘使用共享材质和固定大小的实例缓冲。替换分块前的独立程序交替对照中，同一 L 形高楼落地场景平均墙钟帧耗时约 `74.24 → 53.53 ms`，四次运行的破坏结果一致。这是历史几何的测量，不能用来推算新复合代理场景的帧率；测量范围见 [ADR 0010](docs/adr/0010-runtime-performance.md)。

破坏依次产生短促粉爆、不规则主碎屑、扩散烟尘和边缘落灰；少量主碎屑与存活建筑碰撞、反弹。烟尘具有连续淡出、主光明暗和交界深度淡出。源三角形按规范接触区域直接选择 grid 或粗糙断面材质，邻块移走后断面自然显露；不再以共面显示层竞争深度，避免两种材质闪烁。暴露侧另有可选的闭合骨料边带，两者都不改变碰撞或质量，见 [断面着色决策](docs/adr/0007-fracture-surface-rendering.md)和[边带决策](docs/adr/0008-destruction-effects-and-rims.md)。

桌面入口位于 `src/main.rs`，无窗验收入口位于 `apps/acceptance`，共用的游戏装配位于 `src/lib.rs`。模块使用 `src/<module>/mod.rs`，模块测试集中于相邻 `tests.rs`。布局与 v2 导入规则分别见 [ADR 0011](docs/adr/0011-rust-project-layout.md)和 [ADR 0005](docs/adr/0005-v2-asset-contract.md)。

## 验证

面向 `master` 的 PR、`master` 推送和手动启动都会运行 Rust CI，包括 bevy-cicd 的 `entity-names`（实体命名）和 `named-fields`（具名字段）检查。[ADR 0017](docs/adr/0017-bevy-compiler-rules.md)

独立的 Pages 工作流通过 bevy-cicd 构建并运行调度导出入口，用 Rust 编译事实和实际注册的 Bevy 调度生成站点。[在线 ECS 架构图](https://fedoraiver.github.io/bevy-destruction/) · [ADR 0016](docs/adr/0016-shared-schedule-export.md)

```powershell
cargo test --locked --workspace
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

迁移前的验证覆盖 63 项 Blender 测试和插件安装/启停、六个正式 v2 场景的 GLB 校验和自重平衡、矩形楼破坏和倾倒、受力详查的 11 阶段截图、断面着色的 40 张 GPU 对照图，以及 147 项 Rust 回归。各次验证的资产快照与范围见[设计决策索引](docs/adr/README.md)。

### 本地无窗验收

需要 Rust 1.97.1 或以上、Python 3.11 或以上，以及支持 wgpu 离屏渲染的适配器。在仓库根目录执行以下命令。Python 和 Rust 依赖固定在同一个 `bevy-acceptance-test` 修订。

```powershell
python -m venv tmp/acceptance-venv
.\tmp\acceptance-venv\Scripts\python.exe -m pip install -r scripts/acceptance/requirements.txt
cargo build --locked --release -p bevy-destruction-acceptance --features acceptance-test
$acceptanceExe = 'target/release/bevy-destruction-acceptance.exe'
.\tmp\acceptance-venv\Scripts\python.exe scripts/acceptance/run.py --exe $acceptanceExe
```

macOS 或 Linux 使用 `tmp/acceptance-venv/bin/python`，可执行文件路径省略 `.exe`。默认 `all` 先运行 `controls` 和 `gameplay` 两组交互验收，再运行 17 个固定帧基线。`controls` 通过 Bevy 原生键鼠消息验证 WASD、鼠标转向、左键破坏、右键刚球与 R 键方块。`gameplay` 经 BRP 放置初始相机，随后飞近矩形楼、瞄准、破坏底层墙面并发射两种撞击物，读取破坏、接触与断键状态。

`baseline` 仅运行固定帧基线。各探针在应用进程中以固定 60 Hz 模拟步骤验证原有断言。

| 用例 | 基线场景 | 验证内容 |
| --- | --- | --- |
| `smoke` | `rectangle`、`l_shape`、`courtyard` | 重复实例化、相机控制、局部点击破坏、边带与粒子预算，以及最后一次出尘后的粒子回收。每次保存 12 张阶段图，涵盖破坏后 0.05、0.15、0.4、1、3 秒及烟尘散尽的近景。 |
| `physics` | `rectangle`、`l_shape`、`courtyard` | 屋顶双块失支撑后下落、运动中再次拆分、质量与位姿速度继承、接触后休眠。每次保存 3 张。 |
| `stress` | 六个正式场景 | 真实自重求解，双柱梁失去支撑后的渐进过载、自动断键、下落与休眠，刚球与方块碰撞断裂且保持刚体完整。每次保存 7 张。 |
| `material` | `rectangle` | 材质随构件刚体平移和旋转保持一致，世界锁定材质作为负对照。保存 4 张 512×256 图。 |
| `contact` | `rectangle` | 纯断面参考、旧共面显示层与单次着色对照，覆盖视角、深度预通道、接触范围外 grid 和前方遮挡。保存 40 张 512×512 图。 |
| `force` | `rectangle` | 自重内力、断键、刚球接触冲量、关闭清空，以及热力阈值、遮挡、窗洞拾取、实例隔离、松开 Alt 和原色恢复。保存 11 张。 |
| `landing` | `highrise_slab` | 移除下半部分后检查接触前完整、内部楼层碰撞断键、成组残骸、质量保留、有限状态以及力和力矩平衡。保存 4 张及簇成员和空间范围报告，也可指定 `highrise_l`。 |
| `collapse` | `rectangle` | 从资产几何选取偏侧支撑，移除其他首层支撑后检查上部下降、倾斜及与残墙接触的实际冲量。保存 4 张和可回放的 `damage.json`。 |

每次运行在 `output/acceptance/run-*` 下生成 `summary.json`、各用例的验收报告和进程日志。固定帧探针同时写出 `report.json` 和阶段截图。失败返回非零退出码。可用 `--output`、`--exe`、`--port` 和 `--timeout` 指定输出目录、可执行文件、回环 BRP 端口和单次等待的秒数。启动和输入等待默认限时 180 秒，固定帧探针默认限时 600 秒。并行运行时为每个进程指定不同端口。单个固定帧探针或 `profile` 可用 `--scene` 或 `--asset` 选择场景，并可传入 `--no-stress`、`--no-vfx`、`--no-detail` 或 `--no-soft-particles`。

```powershell
.\tmp\acceptance-venv\Scripts\python.exe scripts/acceptance/run.py gameplay --exe $acceptanceExe
.\tmp\acceptance-venv\Scripts\python.exe scripts/acceptance/run.py stress --scene bridge --exe $acceptanceExe
.\tmp\acceptance-venv\Scripts\python.exe scripts/acceptance/run.py smoke --scene rectangle --capture-replay --exe $acceptanceExe
.\tmp\acceptance-venv\Scripts\python.exe scripts/acceptance/run.py profile --exe $acceptanceExe
```

`smoke --capture-replay` 另存每模拟秒 30 帧的 PNG 序列到 `replay`。`profile` 连续运行离屏更新并关闭截图，使用实际墙钟帧间隔计算离屏帧率。报告还记录 VFX、边带缓存和渲染通道的耗时，GPU 项取决于设备支持。架构和验证范围见 [ADR 0018](docs/adr/0018-windowless-acceptance.md)。

`--no-detail` 关闭可选边带，保留真实接触面断面材质。`--no-soft-particles` 关闭深度预通道和交界淡出。用例检查 640 个粒子和 180,000 个边带三角形的预算。

Blender 资产测试与插件使用方法见[工具说明](tools/blender/README.md)。

- [资产格式](tools/blender/ASSET_FORMAT.md)
- [设计决策](docs/adr/README.md)
