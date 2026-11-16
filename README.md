# Bevy Destruction

面向 Bevy 0.20 + Avian 0.8 的建筑物理破坏插件项目。

目前已实现 [Blender 资产工具](tools/blender/README.md)：轮廓生成建筑、门窗、Voronoi 预破碎、凸碰撞代理、连接图及 GLB/JSON 导出。包含中文 Blender 面板、后台入口和三个示例。

- [资产格式](tools/blender/ASSET_FORMAT.md)
- [设计决策](docs/adr/README.md)

Rust 运行时仍是初始工程，尚未实现资产导入和动态破坏系统。
