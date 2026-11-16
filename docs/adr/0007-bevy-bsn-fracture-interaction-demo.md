# ADR 0007：BSN 建筑导入、网格材质与点击破坏演示

- 状态：提议（用户要求先给出设计方案，尚未实现）
- 日期：2026-11-17
- 关联：[ADR 0002](0002-avian-backend-and-fixed-step.md)、[ADR 0006](0006-blender-authoring-tool-and-asset-format.md)

## 背景与需求

现有 Blender 工具已输出预破碎 GLB 与结构清单，Rust 端仍为初始工程。本轮用户要求设计可交互演示：使用 BSN 语法导入建筑，外表面显示 0.2m 细格与 1m 深色粗格，固定正对建筑的相机，按 0 切换预破碎线框，左击按既定碎块删除局部网格，并产生烟尘、粒子和粗糙断面材质。

本轮以静态建筑的局部点删演示为范围。连接、质量和锚点继续加载保存；结构承载、失支撑坍塌、刚体残骸和联网属于后续运行时能力，不能把本轮点删称为已实现结构失效。

## 版本事实与 BSN 边界

- 沿用已确定的 Bevy 0.20.x + Avian 0.8.x。
- Bevy 0.20 已支持 Rust 中的 `bsn!` / `bsn_list!`，`.bsn` 文件资产格式尚未实现。本方案中的 BSN 指宏语法，不生成无法加载的 `.bsn` 文件。
- Bevy 0.20 已切换到 WESL 着色器；采用 `.wesl` 扩展 PBR，不照搬旧版 Bevy WGSL 预处理写法。
- 0.20 的 glTF primitive 材质是 `Handle<GltfMaterial>`。如需保留基础材质参数，先通过官方 `standard_material_from_gltf_material` 转换，再构建自定义扩展材质，不能将该 handle 当作 `Handle<StandardMaterial>` 使用。

上述为官方文档和固定版本源码核验，尚未在此仓库编译验证。

## 导入与实体组织

### 资产加载

新增 `DestructibleAssetLoader`，以 `.destruction.json` 为入口，校验格式、单位、引用 ID 和对应 GLB 的 SHA256，将 GLB 作为正式资产依赖。依赖完成后读取 `Gltf.named_nodes`，按 Chunk ID 定位 `GltfNode`、`GltfMesh` 及所有 primitive 的 mesh handle。

通过已加载依赖状态与待实例化队列驱动生成，不能只监听一次加载完成消息，否则先加载后创建的实例可能漏生成。加载失败应显示简洁错误，不实例化部分建筑。

### BSN 声明

用 `bsn!` 声明相机、灯光、建筑根、Chunk 和显示 primitive；动态碎块列表由 Rust 数据循环调用这些场景函数。数据清单继续使用 JSON，BSN 负责 ECS 场景组合。

```text
DemoRoot
  Camera：固定正视角
  Lights：斜向主光 + 环境补光
  BuildingRoot：实例 ID、资产 handle、存活状态
    ChunkRoot：Chunk ID、位姿、静态 Collider
      RenderPrimitive：Mesh + 外表面 Grid 材质
      RenderPrimitive：Mesh + 内部断面材质
    ...
  VfxRoot：烟尘与装饰碎屑
  Hint：左键破坏 · 0 线框
```

Chunk 根只应用清单中的质心位置和旋转，primitive 子节点使用单位变换；直接读取 glTF mesh handle，不再叠加 GLB 节点位姿。资产已经是米制 Y-up，导入时也不能再次执行 Blender 坐标转换。

建立 `ChunkId -> ChunkRoot / 全部显示子实体 / Collider / debug 边` 的映射。一个 Chunk 可以有多个 primitive，删除必须以整个 Chunk 为单位。共享资产数据不修改，`Alive/Removed` 和连接失效位于实例状态中。

本轮每块使用可独立删除的静态碰撞代理；不启用重力坍塌或失去支撑后的连锁删除。后续刚体簇表示可以通过物理接入层替换。

## 资产表面语义补充

现有 GLB 的原始材料有 `bd_material_id`，Voronoi 新切面使用 `BD_DestructionInterior`；尚无稳定的表面角色。实际模型只有 POSITION/NORMAL，未提供 UV/TANGENT。

需要在 Blender 导出材质 extras 中增加 `bd_surface_role = exterior | fracture`，导入时分别绑定 Grid 与粗糙断面材质。物理材料 ID 仍从所属 Chunk 读取，避免共享内部材质掩盖 concrete/steel 等类别。

还需处理源构件之间的埋藏接触面：当前只标记 Voronoi 新切面，墙段与墙段、楼板分区与墙体之间的接触面仍可能使用外材质。为保证所有新暴露面都正确，需要在烘焙时根据真实共面接触补丁细分表面，将接触区域标为 fracture，未覆盖区域保留 exterior。不能把只部分接触的整个楼板面都标为内部面。

该步骤只做共面划分、三角化和材质语义，不移动物理边界；重新校验封闭性、体积、碰撞、连接与导出哈希。现有 Bond 仅保存面积、中心、法线和二阶矩，不能据此准确重建每个接触补丁，分类时需保留或重新计算真实多边形重叠。

最终验收资产应重新烘焙。旧资产可按内部材质名兼容原有 Voronoi 切面，但不能因此宣称旧模型的全部跨构件暴露面已经正确。

## 网格材质

外表面采用基于米制位置的程序化材质，无需 UV。所有碎块共用统一建筑坐标原点和方向，不能按各碎块质心重新起格；本演示建筑根保持单位缩放。

| 参数 | 提议默认值 |
| --- | --- |
| 小格边长 | 0.2m，用户明确要求 |
| 大格边长 | 1.0m，即每 5 小格，用户明确要求 |
| 小格线宽 | 约 0.8 屏幕像素，细且较浅 |
| 大格线宽 | 约 1.8 屏幕像素，更粗且更深 |
| 表面底色 | 浅灰，避免强反光掩盖格线 |
| 抗锯齿 | 使用屏幕导数控制线边平滑，远处稳定退化 |

格距按米，线宽按屏幕像素以保证固定展示距离下可读。1m 线与小格线重合时以大格样式覆盖，不重复叠加变黑。

墙面使用由建筑竖直方向和未扰动几何法线构成的面内正交基；水平楼板使用建筑水平轴。这样斜向竖墙的面内格距也是真实 0.2m，而不是把世界投影距离误当作表面边长。对共面碎块保持同一基与相位。

采用 `ExtendedMaterial<StandardMaterial, ...>` 保留 PBR 光照。网格与断面可为同一扩展的两种模式或两个明确材质类型，具体拆分在实现时以代码清晰度和共享绑定验证。

## 固定相机与显示

- 采用正交相机，严格正对建筑正立面；对当前资产，前立面为 Blender y=0 对应的运行时 +Z 侧，相机朝 -Z。
- 相机高度对齐建筑包围盒中心，不采用等角或斜俯视角；禁用鼠标旋转、平移和滚轮缩放。
- 初次加载按包围盒及窗口宽高比自动取景，约留 15% 边距；窗口尺寸变化时调整投影范围，方向始终固定。粒子不会触发相机追踪或缩放。
- 使用斜向主光和适量环境补光，让凹凸断面和建筑深度可读；烟尘为半透明，不作为点击目标。

## 0 键调试线框

主键盘 `Digit0` 切换，兼容数字小键盘 `Numpad0`；按下沿触发一次，持续按住不反复切换。

普通模式保持完整 Grid 外观。调试模式叠加橙色预破碎边界，保留表面和深度遮挡，不默认透视全部内部线。

导入时合并同一 Chunk 的全部 primitive，在位置容差下焊接重复顶点，构建三角面邻接；移除同一平面内的三角化对角线，保留真正折角与边界。边数据缓存，删除碎块同步移除其线，不每帧重新分析网格。

首选 Gizmos 或专用线段绘制，保留深度测试并施加轻微深度偏移。普通 GPU triangle wireframe 会显示三角化对角线，且 polygon line 模式存在后端限制，不作为用户所需 fracture 线框的默认实现。

## 左键破坏

1. 由固定 Camera 将鼠标位置转换为世界射线，使用 Avian 查询最近存活的建筑 Collider；忽略地面、特效和已删除状态。
2. 命中 Chunk 必定删除。默认可扩展到命中点附近、同一受击表层的直接邻接碎块，建议半径 0.4m、上限 6 块；具体值为可调演示参数。
3. 扩展选择以真实表面距离和受击面深度带约束，不能沿射线穿过前墙连删背墙，也不能只按碎块质心距离选择。
4. 将目标立即标记 Removed，删除/隐藏它的全部显示 primitive 与 debug 边，取消实例内 incident Bond，并在物理安全边界移除 Collider。查询过滤同步排除 Removed，避免固定步数据尚未更新时再次命中旧碰撞体。
5. 在真实命中点和暴露面产生特效。剩余碎块保留原有封闭面，其朝向被删块的内部面自然暴露，不再生成平面封住新洞。

所有操作基于导入的预定义碎块；本轮不运行网格切割，也不因为图失支撑而让整栋楼坍塌。

## 断面与 VFX

断面使用材料物理类别对应的程序化颗粒噪声、颜色变化、高粗糙度和高度梯度法线扰动。混凝土使用厘米级骨料与更细孔隙两层尺度；法线扰动带来近景凹凸感。

这是材质层面的凹凸，实际预切轮廓、接触图、质量和碰撞不变。仅提高 roughness 不足以表达凹凸，也不在此轮对相邻切面独立位移，以免完整建筑提前露缝。

VFX 分为短促的向外碎屑爆发、缓慢上升扩散的烟尘和快速消散的细粉。烟尘使用柔边公告板，装饰碎屑使用少量共享网格；固定种子可用于重放验收。首版采用 Bevy 原生 ECS/共享材质和网格，不依赖尚未核验 0.20 兼容性的额外粒子库。

特效有总数、生命周期和连续点击预算，不无限累积；装饰粒子不加入建筑连接图，也不创建永久物理障碍。除预算外，不额外同步删除已存在的建筑碎块来制造视觉效果。

## 建议代码边界与交付顺序

| 模块 | 责任 |
| --- | --- |
| `asset` | 结构清单/GLB 依赖、校验与稳定 ID 绑定 |
| `scene` | BSN 场景、Chunk 实例、固定相机与灯光 |
| `material` | 米制 Grid、WESL PBR 扩展及断面 |
| `interaction` | 射线命中、局部选择、实例删块事务 |
| `debug` | fracture 边缓存、0 键开关与深度线绘制 |
| `vfx` | 受预算控制的烟尘、细粉与装饰碎屑 |

先补资产表面角色及跨构件接触分类；随后依次提交 BSN 导入/固定相机、Grid/调试边、点击删除/碰撞同步、断面/VFX，各小功能验证后独立提交。

## 备选方案与取舍

- 运行时任意切割：能生成更自由的破口，但超出本轮按预定义碎块删除的目标，并增加碰撞重建和封口成本；采用已有预破碎资产。
- 直接实例化完整 glTF 场景后遍历改造：可以保留更多原场景关系，但需要额外处理节点位姿、primitive 分组和运行时组件补挂；本轮使用 glTF 原始资源加 BSN 声明，保持 Chunk 身份和删除边界明确。
- 为每块烘焙独立网格纹理：依赖 UV，容易跨块错位；本轮采用统一建筑坐标下的程序化 Grid。
- 原生三角线框与实际切面位移：前者含多余对角线，后者可能让完整建筑提前露缝；分别采用几何边提取和材质法线凹凸。若之后要求断面轮廓本身起伏，再单独设计匹配的切面几何与碰撞更新。

## 验收与未决问题

1. 导入矩形、L 形与庭院资产，Chunk 数、primitive 数、位姿和碰撞与 Blender 输出一致；错误哈希拒绝加载。
2. 墙和楼板格距精确为 0.2m/1m，跨碎块连续；大线更粗更深，远处无明显锯齿或闪烁。
3. 相机严格正对建筑且覆盖整个模型，普通鼠标移动和滚轮不改变视角。
4. 0 键单次切换；普通模式不露预破碎线，调试模式可看边界且无三角化对角线。
5. 点击只破坏命中位置附近的预定义碎块，所有 primitive、Collider、debug 边和实例状态同步移除；一次点击不穿透前墙删除背墙。
6. 删除相邻不同 Part 的碎块后，所有新暴露接触区域为粗糙断面，原来可见的外表面仍为 Grid；门窗洞口保持通行。
7. 烟尘和粒子从命中处出现、按时消失；连续点击后对象数量受上限控制。
8. 在完整资产上固定视角截图、切换调试截图和点击后截图进行视觉比较，并实际编译和运行；此 ADR 本身不代表任何运行时功能已完成。

BSN 组合、glTF 新材质类型、WESL 绑定与后端线绘制仍需在锁定补丁版本上编译验证。上述默认颜色、线宽、破坏半径和粒子参数是提议，可在视觉验收时调整。

## 参考资料

- [Bevy 0.20 BSN 宏文档](https://docs.rs/bevy/0.20.0/bevy/scene/prelude/macro.bsn.html)、[BSN 场景与 .bsn 边界](https://docs.rs/bevy/0.20.0/bevy/scene/index.html)
- [Bevy v0.20.0 glTF 资产类型](https://github.com/bevyengine/bevy/blob/v0.20.0/crates/bevy_gltf/src/assets.rs)、[glTF 到 PBR 材质转换](https://github.com/bevyengine/bevy/blob/v0.20.0/crates/bevy_pbr/src/gltf.rs)
- [递归依赖加载消息](https://docs.rs/bevy_asset/0.20.0/bevy_asset/enum.AssetEvent.html)
- [官方扩展材质示例](https://github.com/bevyengine/bevy/blob/v0.20.0/examples/shader/extended_material.rs)、[Bevy 0.20 WESL 说明](https://bevy.org/news/bevy-0-20/#wesl-shaders)
- [Camera 射线 API](https://docs.rs/bevy_camera/0.20.0/bevy_camera/struct.Camera.html#method.viewport_to_world)、[Avian 0.8 空间查询](https://docs.rs/avian3d/0.8.0/avian3d/spatial_query/struct.SpatialQuery.html)
- [官方 Wireframe 示例及后端限制](https://github.com/bevyengine/bevy/blob/v0.20.0/examples/3d/wireframe.rs)、[Gizmos 示例](https://github.com/bevyengine/bevy/blob/v0.20.0/examples/gizmos/3d_gizmos.rs)
