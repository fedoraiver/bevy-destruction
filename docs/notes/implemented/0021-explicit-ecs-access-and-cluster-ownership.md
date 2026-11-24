# ADR 0021：显式声明承载系统访问和刚体簇归属

## 状态

2026-11-23 采用并实现。补充 [ADR 0001](0001-rigid-cluster-runtime.md) 的刚体簇关系和 [ADR 0002](0002-elastic-structure-and-overload.md) 的承载运行时。

2026-11-24 将显式访问扩展至撞击断裂系统。

2026-11-24 将碎块当前所属的刚体簇统一为 Avian 的 `ColliderOf` 关系。

2026-11-24 改用建筑的 `Children` 枚举直属刚体簇。

2026-11-24 重建改用 Avian 的碰撞体换属和 Bevy 的即时层级变换。

2026-11-24 将建筑查询、即时变换和质量更新的查询状态交给独占分簇系统维护。

2026-11-24 将分簇请求迁移到建筑实体上的 `NeedsClusterRebuild` 标记。

## 背景

锚定承载系统独占整个 `World`，无法从系统签名看出它的依赖。刚体簇另外储存了一份所属建筑实体，同时已使用 `ChildOf` 建立同一个层级关系。两处归属可能在换属时产生分歧，进而影响撞击和受力样本。

## 决策与实现

- 锚定承载通过 `Query`、`Res`、`TransformHelper` 和 `MessageWriter` 声明访问。首次求解后通过 `Commands` 添加应力状态。过载判定及反馈仍先于本固定步刚体簇重建，断裂反馈使用重建前位姿。
- 刚体簇的所属建筑由 `ChildOf` 提供。撞击工况、受力显示和验收夹具沿用这条关系。碎块碰撞体仍通过 Avian 的 `ColliderOf` 关联刚体簇。
- 分簇从建筑的直属 `Children` 选出带 `Cluster` 的实体。旧簇按其成员中的首个稳定碎块 ID 排序，以原有顺序清理。验收探针同样通过结构层级查询建筑的簇及就绪状态。
- 分簇计划从 `ColliderOf` 读取旧归属，包括逻辑上已删除但尚待解除碰撞关系的碎块。提交时同步解除已删除碎块的碰撞和归属，存活碎块通过 `ChildOf` 换属。Avian 的 `ColliderHierarchyPlugin` 更新 `ColliderOf`，重建在销毁旧簇和聚合新质量前刷新该关系。点击后到下一固定步前，结构状态和碰撞归属分别反映各自的消费时点。首次分簇前的断裂反馈使用建筑位姿。
- 刚体簇重建在独占系统中同步清理旧接触、更新碰撞体归属、重新聚合质量并赋予子簇速度。该顺序依赖中途刷新和读取即时结果。
- 重建通过 Bevy 的 `QueryState` 查询建筑，通过 `SystemState<TransformHelper>` 获取传播前的即时位姿。碰撞体换属和启用后刷新命令，再通过 `SystemState<MassPropertyHelper>` 调用 Avian 同步计算新簇质量并继承父簇运动。
- 场景装配、点击破坏、承载断键和撞击断键在对应建筑上插入 `NeedsClusterRebuild`。重建通过 `With<NeedsClusterRebuild>` 查询待处理建筑，成功后移除标记。资产或包围盒尚不可用时保留标记等待重试。承载请求在同一固定步重建前应用，撞击请求在下一固定步消费。
- 标记的 `on_insert` 钩子将 `BuildingInstance` 标为已变更，标记已经存在时再次插入也会执行该钩子。重建提交时同样将建筑标为已变更，以维持承载缓存和静态受力诊断的失效时点。请求的添加与移除可以发生在同一固定步，诊断通过保留下来的时间戳判断历史结果是否仍有效。[承载缓存](0010-runtime-performance.md#决策与实现)
- 撞击系统通过 `Query`、`Res` 和 `ResMut` 声明组件与资源访问。它在 `PhysicsSystems::Last` 后按顺序读取已求解的接触、执行动态求解、在共同预算内提交断键，并通过 `MessageWriter` 发送反馈。下一固定步的分簇消费断键结果。[接触与断裂规则](0003-collision-driven-fracture.md)

## 备选方案与取舍

保留独占承载可以直接插入组件并读取任意实体，但系统依赖会留在函数体中。显式系统参数使访问可检查，新增应力组件的可见性由排序和延迟命令应用保证。将刚体簇归属留在自定义字段便于直接读取，使用 `ChildOf` 则与层级和生命周期共享同一数据源。

撞击曾通过独占访问临时移出工况资源。普通系统可以通过可变资源借用直接更新工况。读取、求解和提交在同一系统内顺序执行，保持每步求解和断键预算，避免引入阶段间的额外缓冲。

`ChunkInstance` 曾另存当前簇实体。直接查询 `ColliderOf` 可让撞击、受力诊断、断裂反馈和分簇读取同一份归属。调用方继续按自己的消费阶段校验存活状态和簇实体。`Cluster.chunks` 提供稳定顺序的结构成员快照，用于工况绑定和求解。

`BuildingInstance.clusters` 曾保存有序的簇实体列表，重建需要在层级更新后同步写入该列表。由 `ChildOf` 和 `Children` 维护的直接关系统一提供建筑与簇的双向查询。分簇只在处理旧簇时从成员 ID 恢复排序，普通子实体通过 `Cluster` 过滤。

重建曾同时写 `ChildOf` 和 `ColliderOf`，并自行遍历建筑的变换层级。Avian 按最近的刚体祖先维护存活 Collider 的归属，Bevy 提供传播前的即时位姿。复用两者使分簇只需写入父子关系并在清理前刷新，再按当前建筑层级定位簇。

质量更新曾通过 `run_system_cached_with` 注册并运行额外系统。独占系统使用 Bevy 持久化的查询状态直接借用 Avian 质量更新参数，延迟命令仍在明确的重建阶段刷新。

原先重建遍历建筑并检查 `BuildingInstance.topology_dirty`，断裂系统仅为写该字段就需要可变的建筑查询。使用稀疏存储的标记后，断裂系统可以只读建筑，重建以组件查询过滤。边带刷新继续使用自己的 `RefreshDamageDetails` 标记，因为它的消费阶段是 `Update`。

## 验证要求与结果

覆盖首次分簇前断裂及反馈、同一固定步的状态可见性、应力缓存和失败重试。检查子簇换属、跨建筑样本隔离、父撞击卸载和独立接触事件首次绑定。运行锁定的工作区 Rust 回归。

撞击访问调整须验证真实接触、动态与局部预算、关闭采样时的断裂选择、父工况卸载和子簇首次绑定。无窗验收覆盖实际冲量采样和板式高楼落地。

碎块归属回归覆盖首次分簇前、逻辑移除后至提交前、解除归属后及存活碎块换属，并复用旧接触清理、跨建筑隔离和子簇动量回归。

建筑簇查询回归覆盖已有普通子实体但尚未就绪、保留簇与新生簇同时可查、旧簇清理，以及跨建筑隔离。

工作区 147 项 Rust 回归通过。现有测试同时验证了分簇前断裂反馈的位置和时点、子簇的所属建筑及旧受力样本的失效。启用验收特性的 Clippy 和格式检查通过。

2026-11-24 用工作区锁定的依赖编译并通过启用验收特性的 150 项 Rust 回归。Clippy 和格式检查通过。矩形场景 `gameplay`、`force` 和板式高楼场景 `landing` 无窗验收通过。

碎块归属迁移通过启用验收特性的 150 项工作区 Rust 回归、格式和 Clippy 检查。矩形场景 `physics`、`stress`、`force`、`collapse`、`gameplay` 和板式高楼场景 `landing` 无窗验收均通过。

建筑簇查询迁移通过启用验收特性的 150 项工作区 Rust 回归、格式和 Clippy 检查。矩形场景 `physics`、`force`、`collapse`、`gameplay` 和板式高楼场景 `landing` 无窗验收均通过。

碰撞体换属复用现有存活碎块重绑、旧接触清理、质量及动量回归。新增用例覆盖父实体平移和旋转后、变换尚未传播时的首次分簇。启用验收特性的 152 项工作区 Rust 回归、格式和 Clippy 检查通过，矩形场景 `physics` 无窗验收通过。

查询状态迁移通过首次生成、运动中拆分、质量更新和旧接触清理回归。启用验收特性的 154 项工作区 Rust 测试、格式和 Clippy 检查通过。矩形场景 `physics` 和板式高楼场景 `landing` 无窗验收通过。

重建标记回归通过，覆盖重复插入、边界或资产暂缺后的重试、同一步的承载断键以及下一步的撞击重建。启用验收特性的 157 项工作区 Rust 测试、格式和 Clippy 检查通过。矩形场景 `smoke`、`stress`、`force`、`physics`、`collapse` 和板式高楼场景 `landing` 无窗验收通过。

## 未决问题

场景结构变更仍有多阶段的独占流程。之后的重构需保留接触、断键、重建和反馈之间的消费时点。

[ADR 0023](0023-ecs-structure-entities.md) 记录后续的结构图 ECS 实体化。

## 来源

[承载系统](../../../src/stress_runtime/mod.rs) · [断裂反馈](../../../src/interaction/mod.rs) · [刚体簇](../../../src/physics/mod.rs) · [撞击处理](../../../src/projectiles/mod.rs) · [受力显示](../../../src/force_debug/mod.rs)

[Avian 碰撞体层级](https://docs.rs/avian3d/latest/avian3d/struct.PhysicsPlugins.html#plugins) · [Bevy 层级变换](https://docs.rs/bevy/latest/bevy/prelude/struct.TransformHelper.html)

[Bevy 查询状态](https://docs.rs/bevy/latest/bevy/ecs/query/struct.QueryState.html) · [Avian 即时质量更新](https://docs.rs/avian3d/latest/avian3d/dynamics/rigid_body/mass_properties/components/struct.Mass.html#mass-updates)

[Bevy 组件插入钩子](https://docs.rs/bevy/latest/bevy/ecs/lifecycle/struct.ComponentHooks.html#method.on_insert)
