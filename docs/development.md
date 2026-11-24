# 开发规范

适用于建筑破坏插件、桌面入口和无窗验收入口。ECS 与物理 API 以工作区锁定的 Bevy `0.20.1`、Avian `0.8.0` 为准。[工作区依赖](../Cargo.toml)

## ECS 数据与实体关系

- 用 `Entity` 标识运行时实体，以组件组合表达状态和能力。无数据的角色可以用标记组件表示，例如 `RemovedChunk`。项目代码创建的运行时实体及子实体在创建时提供可辨认的 `Name`，系统通过组件、关系和查询过滤条件选择实体。[场景与碎块组件](../src/scene/mod.rs)
- 按数据职责、访问方式和生命周期组织组件。组件可提供构造、校验和本地计算方法。由系统读取其他实体、执行交互和写回结果。同一业务状态保留明确的数据来源，区分建筑初始位姿、Avian 当前物理位姿和用于显示的变换。[建筑实例](../src/scene/mod.rs) · [刚体簇重建](../src/physics/mod.rs)
- 运行时状态优先使用 Bevy 原生类型，按归属和生命周期选择。每个实体独立的状态用 `Component`，同一 `World` 中供多个系统共享的唯一状态用 `Resource`，单个系统私有的跨次运行状态用 `Local<T>`。由资产系统管理并按句柄共享的资产用 `Asset`。资产记录、求解快照、数值中间量等可以使用普通 Rust 类型。[建筑应力状态](../src/stress_runtime/mod.rs) · [受力样本资源](../src/force_samples/mod.rs) · [受力显示缓存](../src/force_debug/mod.rs) · [破坏资产](../src/asset/mod.rs)
- 用 `ChildOf` 表达变换和显示层级，用 `ColliderOf` 表达 Avian 碰撞体的刚体归属。结构所有权用专用 `Relationship` 表达，建筑与连接使用 `BondOfBuilding`，连接与面片使用 `PatchOfBond`。Bevy 自动维护反向集合 `BuildingBonds` 和 `BondPatches`，两者都启用 `linked_spawn`。按清单查找或保持稳定遍历顺序时，业务索引应区分清单 ID 和当前 `World` 内的 `Entity`。显式维护索引的代码需要一并处理端点移除、重新归属和场景切换。[结构实体](../src/scene/mod.rs) · [碰撞体换属](../src/physics/mod.rs) · [场景切换](../src/catalog/mod.rs)

## 调度、通信与命令

- 每帧或固定步执行的运行逻辑、跨实体交互及共享状态更新，优先用 Bevy `System` 表达，并注册到相应调度。系统用 `Query`、`Res`、`Commands` 等参数声明依赖，只对需要更新的数据请求可变访问。可用互斥过滤条件、拆分系统或 `ParamSet` 管理查询冲突。纯图算法和数值计算可以由系统调用普通函数完成。[承载系统](../src/stress_runtime/mod.rs) · [连通分量](../src/structure/mod.rs)
- 启动装配放在 `Startup`，逐次应用更新的输入和表现逻辑放在 `Update` 或 `PostUpdate`，固定步模拟使用 `FixedUpdate` 或 `FixedPostUpdate` 并读取 `Time<Fixed>`。每次应用更新可运行零次或多次固定步，物理逻辑使用固定步的实际经过时间。[插件装配](../apps/game/src/lib.rs) · [过载更新](../src/stress_runtime/mod.rs)
- 同一调度内的依赖通过 `.before()`、`.after()`、`.chain()` 或已配置相对顺序的 `SystemSet` 表达。`.in_set()` 只确定成员归属，插件安装和元组注册顺序不建立系统依赖。跨调度的数据交接按实际调度阶段安排。例如，过载求解先于刚体簇重建，Avian 物理完成后才读取接触冲量并更新断键状态。[过载调度](../src/stress_runtime/mod.rs) · [簇调度](../src/physics/mod.rs) · [撞击调度](../src/projectiles/mod.rs)
- 常规实体增删及组件结构变更使用 `Commands`，后续系统在延迟命令应用后才能读到它们。Bevy 默认会为有序系统的相关延迟操作安排同步点，自定义调度或同步配置需要单独核对。即刻换属、清理旧接触并重算质量的流程可以使用独占 `&mut World`，在读取后续状态前明确刷新相关变更。[簇重建与质量更新](../src/physics/mod.rs)
- 用注册的 `Message` 进行缓冲通信，例如破坏逻辑发布 `BreakBurst`，特效系统用 `MessageReader` 读取。需要触发式响应时使用 `Event` 或 `EntityEvent` 与 Observer。`World::trigger` 立即触发，`Commands::trigger` 随命令一起应用。必须跨系统停用期保留的请求使用组件、资源或明确管理的队列。[点击破坏](../src/interaction/mod.rs) · [破坏特效](../src/vfx/mod.rs)

## 场景装配与模块

- 用 `bsn!` 和 `spawn_scene` 声明适合组合的场景、UI、初始组件及子实体。运行时交互和逐帧更新放在系统或 Observer 中，动态计算的实体可用 `Commands` 或在必要时用 `World` 创建。[演示场景](../src/scene/mod.rs) · [撞击物生成](../src/projectiles/mod.rs)
- 按功能模块保持类型和系统的归属，需要初始化 ECS 数据或注册系统的功能通过 Plugin 封装装配。`apps/game/src/lib.rs` 引入仓库 `src/` 中的功能模块，由 `DestructionPlugin` 组合建筑破坏功能。桌面和无窗入口分别配置 Bevy 的运行器、窗口和验收选项。[桌面装配](../apps/game/src/lib.rs) · [无窗入口](../apps/acceptance/src/main.rs)
- 有独立业务含义的常量保留在负责该语义的模块，名称或注释注明单位与容差。注释解释调度、数值条件及生命周期等不易从代码读出的约束。优化缓存或存储前先测量，使用变更检测或条件运行时保留对连续模拟的覆盖。[撞击参数](../src/projectiles/mod.rs) · [应力缓存](../src/stress_runtime/mod.rs)

## 检查与验证

- bevy-cicd 对 `apps/game` 运行 `entity-names` 和 `named-fields`。项目的单字段结构体使用描述用途的具名字段。[Rust CI](../.github/workflows/rust.yml)
- Rust 测试放在对应功能模块的 `tests.rs`。图、数值和资产校验优先通过对应模块测试验证。涉及真实接触、输入、渲染或应用装配的改动按影响范围运行相关无窗验收。文档改动核对源码、相对链接和命令。临时脚本、缓存及中间文件使用仓库忽略的 `tmp/`，验收产物使用相应流程的输出目录。[结构测试](../src/structure/tests.rs) · [撞击测试](../src/projectiles/tests.rs) · [验证入口](../README.md#验证) · [忽略规则](../.gitignore)

## 文档维护

- 根 README 维护项目入口、运行和验证命令。插件说明维护当前资产结构及运行机制，开发规范与测试指南维护可复用的开发方法。跨文档只保留理解当前主题所需的背景。
- 用当前行为解释接口、约束、调度和失败条件。更新或移动内容时一并检查入链、段落锚点和命令示例。
