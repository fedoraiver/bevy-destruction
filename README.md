# Bevy Destruction

面向 Bevy 0.20 + Avian 0.8 的建筑物理破坏插件项目。

已实现 [Blender 资产工具](tools/blender/README.md)和 [ADR 0007 交互演示](docs/adr/0007-bevy-bsn-fracture-interaction-demo.md)：BSN 建筑导入、固定正视相机、0.2m/1m 程序化网格、预破碎边界、点击删块、粗糙断面与烟尘碎屑。

## 运行

需要 Rust 1.97.1 或以上及支持 Bevy 的图形环境。从仓库目录执行：

```powershell
cargo run
```

左键点击建筑破坏；主键盘 `0` 或数字小键盘 `0` 切换橙色预破碎边界；`Esc` 退出。相机始终正对建筑，窗口变化时自动取景。

仓库包含重新烘焙的矩形、L 形及庭院资产，切换示例：

```powershell
cargo run -- --asset buildings/l_shape.destruction.json
cargo run -- --asset buildings/courtyard.destruction.json
```

新资产由 Blender 工具导出到 `assets/buildings`，以 `.destruction.json` 为加载入口；对应 GLB 需保留在同目录，内容哈希错误会拒绝加载。运行资产根目录固定为构建时仓库的 `assets`，直接运行生成的可执行文件也使用同一目录。

本演示按已有碎块删除局部几何，每次最多 6 块；剩余建筑保持静态。结构承载、失支撑坍塌与动态刚体残骸仍属于后续功能。

## 验证

```powershell
cargo test
cargo clippy --all-targets -- -D warnings
cargo run -- --smoke-test output/acceptance/rectangle
cargo run -- --asset buildings/l_shape.destruction.json --smoke-test output/acceptance/l_shape
cargo run -- --asset buildings/courtyard.destruction.json --smoke-test output/acceptance/courtyard
```

`--smoke-test` 会打开演示窗口，以固定 60 Hz 模拟执行加载后再次实例化、鼠标点击、连续破坏与特效回收检查，保存普通/调试/破坏/预算压力/消散后五张截图和 `report.json`，然后退出；失败返回非零退出码。正常运行使用实际时间。截图由引擎直接生成，无需桌面自动化。

Blender 资产测试与插件使用方法见[工具说明](tools/blender/README.md)。

- [资产格式](tools/blender/ASSET_FORMAT.md)
- [设计决策](docs/adr/README.md)
