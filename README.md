# Bevy Destruction

面向 Bevy 0.20 + Avian 0.8 的建筑物理破坏插件项目。

已实现 [Blender 资产工具](tools/blender/README.md)、[ADR 0007 交互演示](docs/adr/0007-bevy-bsn-fracture-interaction-demo.md)及 [ADR 0008 破坏表现](docs/adr/0008-destruction-vfx-and-exposed-edge-detail.md)：BSN 建筑导入、自由飞行相机、程序化网格、预破碎边界、点击删块、分层烟尘碎屑与真实暴露边带。

## 运行

需要 Rust 1.97.1 或以上及支持 Bevy 的图形环境。从仓库目录执行：

```powershell
cargo run
```

`WASD` 沿相机前后左右移动，鼠标转向，`Space` 沿世界竖直方向上升，左/右 `Ctrl` 下降。视角采用透视投影，加载完成后只取景一次，此后由用户自由移动。

窗口获得控制时隐藏并锁定鼠标，画面中央显示准星；左键破坏准星指向的建筑。`Tab` 释放或重新锁定鼠标；失去窗口焦点时暂停移动并释放鼠标，重新点击窗口恢复控制，这次点击不触发破坏。主键盘 `0` 或数字小键盘 `0` 切换橙色预破碎边界，`Esc` 退出。

仓库包含重新烘焙的矩形、L 形及庭院资产，切换示例：

```powershell
cargo run -- --asset buildings/l_shape.destruction.json
cargo run -- --asset buildings/courtyard.destruction.json
```

新资产由 Blender 工具导出到 `assets/buildings`，以 `.destruction.json` 为加载入口；对应 GLB 需保留在同目录，内容哈希错误会拒绝加载。运行资产根目录固定为构建时仓库的 `assets`，直接运行生成的可执行文件也使用同一目录。

本演示按已有碎块删除局部几何，每次最多 6 块；剩余建筑保持静态。结构承载、失支撑坍塌与动态刚体残骸仍属于后续功能。

破坏依次产生短促粉爆、不规则主碎屑、扩散烟尘和边缘落灰；少量主碎屑与存活建筑碰撞、反弹。烟尘具有连续淡出、主光明暗和交界深度淡出。邻块消失后，存活侧才生成带骨料材质的闭合棱角边带；完整建筑和其他实例不提前显露裂缝。边带仍使用原凸碰撞代理近似，未增加结构坍塌。

## 验证

```powershell
cargo test
cargo clippy --all-targets -- -D warnings
cargo run -- --smoke-test output/acceptance/rectangle
cargo run -- --asset buildings/l_shape.destruction.json --smoke-test output/acceptance/l_shape
cargo run -- --asset buildings/courtyard.destruction.json --smoke-test output/acceptance/courtyard
```

`--smoke-test` 会打开演示窗口，以固定 60 Hz 模拟执行加载后再次实例化、相机控制、鼠标点击、连续破坏与特效回收检查，保存截图和 `report.json`，然后退出；失败返回非零退出码。验证模式只接收预定的控制输入，正常运行使用实际时间和实际输入。截图由引擎直接生成，无需桌面自动化。

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
