# 破坏资产格式 v2

读取 `<name>.destruction.json`，校验 `schema == "bevy_destruction_asset"`、`schema_version == 2`，再校验同目录 `mesh_file` 的 SHA256 与 `mesh_sha256` 一致。运行时只接受 v2；旧资产需要重新烘焙。格式不匹配或网格校验失败时应拒绝加载。`specification.schema_version` 是独立的制作输入版本，当前仍为 1，不等于本资产版本。

## 坐标与标识

所有运行时数据使用米、右手系、Y-up。Blender `(x,y,z)` 转为 `(x,z,-y)`。GLB 使用同一转换；`specification` 保留原始制作输入的 Blender XY/Z-up 约定，不能再当运行时数据使用。

`asset_id` 是规范化结构数据、制作参数和工具版本的 SHA256。`mesh_sha256` 是实际 GLB 字节摘要，独立于结构标识。相同资产的节点、实体和容器顺序都不应代替稳定 ID。修改输入或升级生成算法后，消费者应通过资产标识检测不匹配。

## 顶层字段

| 字段 | 含义 |
| --- | --- |
| `schema` / `schema_version` | 格式名和版本 |
| `generator` | 工具名与版本 |
| `name` / `asset_id` | 资产名与结构内容标识 |
| `coordinates` | `units=meters`、`up=+Y`、`handedness=right` |
| `specification` | 完整、规范化的制作输入，用于重新生成 |
| `materials` | 质量密度、颜色、强度与线弹性原型参数 |
| `chunks` | 碎块显示与物理数据 |
| `bonds` | 具有真实面积的连接 |
| `anchors` | 与世界固定支撑关联的碎块 |
| `mesh_file` / `mesh_sha256` | 此清单对应的显示资产 |
| `authoring_audit` | 各共享源域的配置尝试、连通碎块尺度、凸代理成本及体积、质心、完整惯性误差 |
| `contact_quantization_audit` | float32 后退化为点、线或无法解析二维截面的微接触统计，含明确排除的边接触面积与宽度 |
| `glb_contact_audit` | 实际 GLB 承载面覆盖检查及有限 float32 边界微修正，记录连接、面片、减少的接触面积、最大顶点位移和 ULP 预算 |
| `surface_classification_audit` | 显示分区表示、几何边闭合、最终材质面积误差与原边界距离；不删除材料体积 |

## Chunk

- `id`、`part_id`、`kind`、`material`：碎块身份、源构件身份、构件类别、材料表索引。
- `node`：对应 GLB 节点名称，等于碎块 ID；该节点同时包含 `extras.bd_chunk_id` 和 `extras.bd_part_id`。
- `position`：碎块质心在资产空间的位置，三维数组。
- `rotation`：xyzw 四元数，当前为单位旋转。
- `volume`、`mass`：体积 m³、质量 kg，质量由材料密度计算。
- `center_of_mass`：刚体局部质心，当前为零向量；网格已经以真实质心重定位。
- `inertia`：关于质心的完整 3×3 张量，单位 kg·m²，使用行数组；不能忽略非对角项后声称保留完整惯性。
- `colliders`：一个或多个 `type=convex_hull` 代理，`vertices` 是刚体局部坐标的顶点。非凸逻辑碎块由不覆盖开口的多个凸子体组成，子体不增加逻辑 Chunk 或结构自由度。导入时构造复合凸碰撞体，并使用给定质量属性，避免再次自动累加密度造成质量重复。不能对非凸显示网格整体取一个凸包而封住凹口或门窗。

GLB 仅包含显示碎块，不包含代理网格、源轮廓或其他场景物体。显示局部顶点与碰撞局部顶点处于同一坐标系。内部切面可由独立材质原语表示；多个 primitive 仍属于同一 Chunk。

消费者不能从 GLB 材质原语推断物理连接；物理形状使用 `colliders`，结构连接使用 Bond/Anchor。

GLB 材质 `extras.bd_surface_role` 必须明确为 `exterior` 或 `fracture`。`generator.surface_policy` 当前固定为 `source_mesh_with_contact_overlays`：制作器只在一个规范化接触面片完整覆盖源面时将整面标为 `fracture`，其余源面保留 `exterior`，不修改顶点、边、面或拓扑。部分接触由运行时精确接触遮罩负责，不进行离线布尔切面或 CDT 重网格化。外表面材质继续携带 `extras.bd_material_id`，物理类别仍以 Chunk 的 `material` 为准，不能从共享断面材质推断。

运行时在源三角形的同一次片元着色中，按规范化 `patches` 的实际有限区域选择断面材质。候选接触与多边形缓冲按资产缓存共享，坐标随原碎块运动；部分接触之外仍显示 grid。不得通过独立共面显示层、关闭遮挡测试或移动物理顶点处理材质覆盖。允许为显示顶点附加每三角形元数据，但实际位置、法线及三角形几何、拾取、碰撞体和质量保持。原始 GLB 源网格在材质分类前后的几何摘要必须相同。消费端实现见 [ADR 0007](../../docs/adr/0007-fracture-surface-rendering.md)；现有 `generator.surface_policy` 是制作端源网格保留政策的标识，不决定 GPU 绘制方式，无需为此重新导出资产。

## Bond

- `id`：由排序后的两个 Chunk ID 生成的稳定连接 ID。
- `a`、`b`：两端 Chunk ID，按字典序排列。
- `area`：接触区域总面积，单位 m²；点接触和边接触不产生 Bond。
- `center`：面积加权的接触中心，资产空间坐标。
- `normal`：各面片面积加权法线的归一化，仅作父连接几何元数据，不能替代各面片方向。加权法线近乎抵消时拒绝导出。
- `area_covariance`：未归一化的接触面积二阶矩，`Q = integral((p-center)(p-center)^T dA)`，单位 m⁴，3×3 行数组。v2 父连接保存完整三维积分，不先投影到一个平均平面；不要把它当作质量惯性张量。
- 共面接触覆盖先合并，再从最终 float32 顶点检查每个承载面片：正面积、最小平面宽度严格大于 0.00002 m，且二维切平面二阶矩可解析。即使窄片的张量为正，也不能将不超过 20 µm 的边接触当作承载面。被排除的边接触面积、宽度和数量写入 `contact_quantization_audit`；源碎块、碰撞体和材料体积不改变。
- 接触共面性只在实际凸重叠域内检查，容差为 20 µm。小面片的序列化法线可能带有角度噪声，不能用相邻大面远处的顶点否定局部有限接触；真正分离的接触仍被拒绝。
- GLB 导出后以实际局部顶点和节点平移重建 float32 世界坐标，复核两端显示三角形对面片的覆盖。超过运行时误差门槛一半的缺口只能作有界微修正：裁剪到两端实际凸支撑域，最终 float32 候选必须位于裁剪半空间内；每个新顶点距离原顶点不得超过 min(4 ULP, 40 µm)，面积减少不得超过位移与周长推导的壳层预算。仅在真实并集仍为凸形时去除三角剖分对角线，不填凹口或孔洞。重新计算面片与父连接全部元数据、审核双端覆盖、更新资产 ID，较大缺口拒绝发布。`generator.contact_export_policy` 为 `actual_glb_footprint`。此过程不改 GLB、源几何、碰撞代理或质量。
- `state`：初始为 `rigid`。运行时损伤不写回共享资产。

每个 `(a,b)` 只允许一个父 Bond。v2 每条连接必须有非空 `patches` 数组；各 Patch 是平面、凸、闭合顺序的多边形，包含：

- `vertices`：资产空间 Y-up 顶点，按从 a 指向 b 的面法线逆时针排列；不重复首点。
- `area`、`center`、`normal`、`area_covariance`：与父字段相同的单位与定义，但只描述该平面片。指标从最终序列化 float32 顶点重新积分，不能保留量化前另一份几何的指标。

父面积为 `sum(A_f)`，父中心为 `sum(A_f*c_f)/sum(A_f)`，父完整二阶矩为 `sum(Q_f + A_f*(c_f-c)*(c_f-c)^T)`；父法线为 `normalize(sum(A_f*n_f))`。面片应位于两端真实显示边界和碰撞代理范围内，不得重复覆盖同一接触面积。运行时按面片方向、中心、面积和二阶矩组装及恢复应力；父连接仍作为稳定图身份和断裂状态。

材料强度由两端材料表读取；具体异材质界面规则和断裂准则由结构求解层决定，本格式不把几何接触等同于已标定的工程焊缝。

## Anchor

`chunk` 指碎块 ID，`kind=fixed`，`position` 为资产空间的地面参考点。当前由碎块接触 Blender Z=0（运行时 Y=0）的底面规则生成。消费者应将其作为世界支撑边界，而不是给该 Chunk 添加额外可见网格。

## 发布顺序

先完整生成并校验临时 GLB，按完整内容 SHA 命名并发布，再原子替换 JSON。旧 JSON 因而始终引用旧的有效 GLB。消费者只以 JSON 作为发布入口，不扫描目录猜测最新 GLB；历史未被引用的网格清理由使用者决定。
