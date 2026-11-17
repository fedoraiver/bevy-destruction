# ADR 0009：失支撑释放、动态刚体簇与运动继承

## 状态

- 已采用；首轮实现已完成，已完成验证单独记录于下文。
- 日期：2026-11-17。
- 本文采用 [ADR 0001](0001-prefractured-graph-and-clusters.md) 与 [ADR 0002](0002-avian-backend-and-fixed-step.md) 中刚性合簇、固定锚点、显式质量与固定步拆分的子集，保留这些记录中的后续提议。本文不替代 [ADR 0003](0003-structural-loads-and-progressive-fracture.md) 的承载模型。

## 背景

ADR 0007、0008 已提供静态预破碎资产导入、点击删块、烟尘碎屑和暴露边带。原实现为每个 Chunk 创建静态刚体；删除局部几何后，失去支撑的剩余部分仍悬空，无法验证建筑图与物理后端之间的生命周期。

资产已经包含稳定的 Chunk/Bond 标识、真实接触连接、完全固定 Anchor，以及体积、质量、局部质心和完整惯性张量。首轮物理开发使用这些数据完成可独立验证的闭环：拓扑改变、失去永久支撑、生成动态簇、下落碰撞、再次拆分和休眠。连通性只判断永久支撑是否存在，不能判断仍相连的细柱是否过载。

## 已采用的决策

### 图与簇的身份

1. 每个 BuildingInstance 独立保存存活 Chunk、连接状态、拓扑脏标记和簇归属；共享资产保持不可变。
2. 只沿存活 Chunk 之间仍有效的刚性 Bond 计算连通分量。遍历和成员顺序使用稳定 ID 排序，实体编号不作为结构身份。
3. 每个连通分量对应一个 Avian 刚体。现有 Chunk 实体保留凸代理 Collider 和 `ChunkIdentity`，作为该刚体的子实体及子 Collider；射线命中仍可定位到原 Chunk。
4. 分量包含至少一个存活的 `kind=fixed` Anchor 时，簇为 Static；否则为 Dynamic。接触地面、建筑、残骸及进入休眠均不生成新 Anchor，不自动焊接，也不恢复已断 Bond。
5. 未改变成员与锚定状态的簇复用原刚体，避免每次局部破坏唤醒或重建无关残骸。仅受影响建筑重新分组；首轮未进一步缓存其局部子图。

### 固定步提交与变换

- 时间基准为 60 Hz。输入可以在 Update 收集和更新实例状态，物理拓扑统一在 `FixedPostUpdate`、`PhysicsSystems::First` 之前提交，随后由 Avian Prepare 完成变换和质量准备，再推进本步物理。
- 使用显式批处理，改变归属前先为受影响 Collider 添加 `ColliderDisabled` 并刷新操作，触发 Avian 的公开观察器清除旧 `ContactGraph` 接触和约束。仅修改 `ColliderOf` 会更新 BVH 归属，不能保证已有接触中缓存的刚体身份同步变化；带着旧身份进入求解器可能引用已经删除的刚体。
- 清除旧接触后换属，刷新延迟关系操作，再移除旧刚体；最后为存活 Collider 移除 `ColliderDisabled`、重新进入碰撞检测，保证进入求解器前每个存活 Collider 只有一个刚体归属。本轮通过公开禁用/启用生命周期处理缓存，不直接修改 Avian 内部接触或约束容器。
- 簇坐标系保留其来源建筑坐标系的原点与方向。拆分后先继承父簇当前物理位姿，Chunk 使用资产局部变换，因而不会由于重新居中而跳动；簇质心由独立的质量属性表示。
- 拆分读取物理 `Position`、`Rotation`、线速度和角速度；动态簇的渲染启用 `TransformInterpolation`。不从经过渲染插值的 Transform 推断真实物理速度。
- 点击候选面、接触边带发射面及方向通过当前 Chunk 物理位姿映射，避免残骸运动后仍在资产初始位置选取或发射烟尘。

### 质量、惯性与运动继承

质量唯一来源为资产已烘焙的存活 Chunk。簇的局部质量、质心和惯性为：

```text
M = sum(m_i)
c = sum(m_i * c_i) / M
I = sum(R_i I_i R_i^T + m_i * ((r_i dot r_i) 1 - r_i r_i^T))
r_i = c_i - c
```

`c_i` 和 `R_i` 均已转换到簇局部坐标，保留完整惯性非对角项和平行轴贡献。根刚体显式设置 `Mass`、`CenterOfMass`、`AngularInertia` 和三个 `NoAuto*` 标记，Collider 密度为零；不通过凸代理体积重新估计或重复累加质量。删除的 Chunk 不再属于新簇，也不再贡献质量或碰撞。

对来自运动父簇的子簇，在世界坐标下初始化：

```text
v_child = v_parent + omega_parent cross (c_child_world - c_parent_world)
omega_child = omega_parent
```

首次从静态锚定簇释放时继承零速度，之后由重力和接触驱动。删除几何是玩法上的质量移除；只有没有质量损失的自由拆分适用完整动量守恒比较，不能把删块后的剩余系统宣称为封闭系统。

### 碰撞与表现参数

- 固定地面为 `400 m × 0.2 m × 400 m` 盒体，中心 `Y=-0.1 m`，顶面 `Y=0`。
- 地面及簇摩擦系数设为 `0.7`，恢复系数设为 `0.0`。重力沿世界负 Y，使用 Avian 当前默认 `9.81 m/s²`。
- 每个 Chunk 沿用资产中的凸代理；动态簇由可识别子 Collider 组合。边带不增加碰撞几何。
- 点击仍按原规则删除最多 6 个局部 Chunk 并断开其关联连接；剩余部分是否下落由图和锚点决定。未断连接的剩余部分作为整体运动。
- 邻块删除或 Bond 断开时更新暴露边带；边带挂在 Chunk 下随其运动，缓存几何保持资产空间定义。

## 已实现范围

- `src/structure.rs`：稳定连通分量、锚点分类、显式质量/质心/完整惯性聚合、速度场继承。
- `src/physics.rs`：固定步前批量建立、复用及销毁簇，子 Collider 换属，动态插值，永久地面。
- 场景、点击和边带系统：拓扑脏标记、簇归属、运动中的选取与特效发射面、断键暴露。
- `--physics-smoke-test`：资产及簇就绪后预热 150 帧，让渲染管线就绪；选择屋顶相连两块，清除与这两块直接相邻且未锚定的屋顶或上部墙体 Chunk，保留所有固定 Anchor。清除 ID 写入报告的 `clearance_chunks`，以区分烟测夹具准备与实际被验证的双块质量。然后断开双块外围连接，验证真实重力下落；在半空中断开内部连接，检查拆分、质量与速度连续性，最终检查在实际支撑面上停止并休眠、锚定建筑静止和有限数值，保存阶段截图及报告。支撑面可以是下层楼板；永久 `Y=0` 地面碰撞由独立物理运行时测试覆盖。清除相邻几何用于排除凸代理接触夹持；图连接断开本身不表示不存在物理接触支撑。
- 烟测在 `FixedPostUpdate` 中、`rebuild_clusters` 之后且 `PhysicsSystems::First` 之前严格检查拆分连续性：世界质心距离误差不超过 `1e-4 m`，旋转四元数逐分量误差不超过 `1e-5`，线速度及角速度向量距离误差不超过 `1e-4`；结果写入 `split_pose_and_velocity_preserved`。该边界处本步重力与接触尚未求解，能直接判断拆簇初始化。若在后续物理步后按纯自由落体外推比较，合法接触会改变速度并导致误报，因此不将后续运动误差混入拆分连续性判据。
- `--smoke-test` 继续承担原资产、相机、点击、特效回收与多实例回归。两个验证模式互斥。

本轮未实现结构应力、载荷重分配、细柱过载失效、碰撞冲量驱动的二次断裂、弱连接、接触形成临时承载、网络同步或存档。重力下落和碰撞是刚体模拟，不代表承载求解已经完成。

## 备选方案与取舍

- **每个 Chunk 独立动态刚体并加固定关节**：易于表示局部相对运动，但完整建筑阶段就引入大量活动刚体和约束。当前采用刚性图合簇；弱连接另行设计。
- **每个簇使用单个不可识别 compound Collider**：实体更少，但现有射线与未来撞击输入需要可靠的 Chunk 映射。当前保留 Chunk Collider 实体，其数量仍需计入性能成本。
- **一开始实现 ADR 0003 的完整承载与撞击断裂**：物理与结构求解故障会混在一起。先验证确定的拓扑释放管线，之后独立实现载荷和接触反馈；当前连通判定不能替代承载模型。
- **按凸代理密度自动计算质量**：接入方便，但会把碰撞近似误差带入资产质量。当前显式汇总烘焙属性，代价是需要维护并验证聚合公式与后端计算属性一致。
- **每次拆分把原点移动到新质心**：可缩短局部坐标，但需重算所有 Chunk 变换。当前保留来源坐标系，质心和惯性独立表达；远离原点的局部坐标精度仍需后续规模测试。

## 验证要求与已完成验证

### 必须验证

- 图遍历顺序不影响成员和锚点分类；失去最后 Anchor 才释放，多实例互不修改。
- 资产质量求和、质心和完整惯性与独立计算一致，旋转 Chunk 及非零局部质心包含在验证中。
- 自由旋转簇拆分保持 Chunk 世界位姿、速度场、总质量、线动量和角动量；直接删块场景单独检查剩余质量。
- 物理准备阶段前的 Collider 归属与计算质量正确，无被删除碰撞、同一步重复碰撞或旧刚体残留；未改变分量保持原实体。
- 释放后真实下落，接触实际支撑面后停止并休眠；独立测试覆盖永久 `Y=0` 地面碰撞，建筑烟测允许下层楼板等几何承接。休眠残骸仍为 Dynamic，不成为 Anchor。锚定建筑保持静止。
- 运动残骸仍可射线选中，点击位置与新特效位置正确，断键边带跟随所属 Chunk。
- 原静态交互与特效验证在矩形、L 形和庭院三个资产上继续通过；物理烟测分别覆盖三个资产。

### 已完成验证

- 已通过 5 项结构测试：替代连接路径与剩余锚点、遍历/输入顺序无关性、旋转局部质心与惯性、平行轴贡献、自由旋转拆分的世界线动量与角动量。
- 已通过 5 项物理运行时测试：移除支撑后的碰撞体换属及多实例隔离、运动簇世界位姿与角速度场继承、重力下落/地面碰撞/休眠及休眠后再拆分、运动后点击候选面与发射面坐标、与邻近静态建筑保持接触时的旧接触清理与新归属。
- 已通过 `cargo test`：49 项通过，0 项失败；其中包含上述结构与物理运行时测试。
- 已通过 `cargo clippy --all-targets -- -D warnings` 和 `cargo fmt --check`。
- 矩形、L 形和庭院三个资产的物理烟测及原交互回归均通过，六份最终 `report.json` 的 `errors` 均为空。三个物理报告中的 `released_as_one_cluster`、`fell`、`split_into_two_clusters`、`split_pose_and_velocity_preserved` 和 `slept` 均为 `true`，各运行 930 个验证帧；阶段截图已导出并完成目视检查。

| 资产 | Chunk 数 | 物理烟测报告 | 原交互回归报告 | 结果 |
| --- | ---: | --- | --- | --- |
| 矩形 | 255 | `output/acceptance/physics-rectangle/report.json` | `output/acceptance/static-rectangle/report.json` | 两条路径均通过 |
| L 形 | 482 | `output/acceptance/physics-l_shape/report.json` | `output/acceptance/static-l_shape/report.json` | 两条路径均通过 |
| 庭院 | 353 | `output/acceptance/physics-courtyard/report.json` | `output/acceptance/static-courtyard/report.json` | 两条路径均通过 |

物理烟测确认了实际下落和静止支撑，而非仅确认刚体类型切换。例如矩形屋顶两块的世界质心从约 `Y=6.50 m` 下落至 `Y=3.50 m` 的下层楼板附近并休眠；L 形和庭院分别由楼板或墙体等实际几何承接，不将这些结果解释为全部落到永久地面 `Y=0`。原交互回归继续覆盖加载、多实例、相机控制、点击破坏及粒子回收。上述结果不构成承载求解、碰撞驱动断裂、高速 CCD 或大规模性能结论。

## 未决问题

- ADR 0003 承载模型、材料强度标定与失效预算如何接入当前拓扑事务。
- 接触冲量的采样阶段、子步累计与冲击阈值，以及撞击反馈不得重复注入物理能量。
- 大型动态簇、薄墙、堆叠、高速撞击的稳定性、CCD 需求、活跃残骸和 Collider 数量预算。
- 当前整栋受影响建筑的重新遍历及质量重算成本，是否需要按原簇划分缓存；尚无 60fps 性能承诺。
- 非单位缩放、远离建筑原点的数值精度，以及跨建筑连接和移动支撑。

## 来源

- 本仓库 [ADR 0001](0001-prefractured-graph-and-clusters.md)、[ADR 0002](0002-avian-backend-and-fixed-step.md)、[ADR 0003](0003-structural-loads-and-progressive-fracture.md) 与 [资产格式 v1](../../tools/blender/ASSET_FORMAT.md)。
- 本地已锁定 `avian3d-0.8.0` 源码：`src/schedule/mod.rs`、`src/physics_transform/mod.rs`、`src/collision/collider/collider_hierarchy/{plugin.rs,mod.rs}`、`src/collision/collider/collider_transform/plugin.rs`、`src/collision/narrow_phase/mod.rs`、`src/collider_tree/update.rs`、`src/dynamics/rigid_body/mass_properties/{mod.rs,system_param.rs}`、`src/interpolation.rs`。接触清理核对了 narrow phase 的禁用观察器，BVH 换属核对了 collider tree 的关系观察器；接口与观察器顺序以此版本为依据，不将后续版本行为视为已验证。
- 对应固定版本接口：[PhysicsSystems](https://docs.rs/avian3d/0.8.0/avian3d/schedule/enum.PhysicsSystems.html)、[ColliderOf](https://docs.rs/avian3d/0.8.0/avian3d/collision/collider/collider_hierarchy/struct.ColliderOf.html)、[MassPropertyHelper](https://docs.rs/avian3d/0.8.0/avian3d/dynamics/rigid_body/mass_properties/struct.MassPropertyHelper.html)。本轮实现以本地源码核对为准。
