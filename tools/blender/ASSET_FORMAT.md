# 破坏资产格式 v1

读取 `<name>.destruction.json`，校验 `schema == "bevy_destruction_asset"`、`schema_version == 1`，再校验同目录 `mesh_file` 的 SHA256 与 `mesh_sha256` 一致。格式不匹配或网格校验失败时应拒绝加载。

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

## Chunk

- `id`、`part_id`、`kind`、`material`：碎块身份、源构件身份、构件类别、材料表索引。
- `node`：对应 GLB 节点名称，等于碎块 ID；该节点同时包含 `extras.bd_chunk_id` 和 `extras.bd_part_id`。
- `position`：碎块质心在资产空间的位置，三维数组。
- `rotation`：xyzw 四元数，当前为单位旋转。
- `volume`、`mass`：体积 m³、质量 kg，质量由材料密度计算。
- `center_of_mass`：刚体局部质心，当前为零向量；网格已经以真实质心重定位。
- `inertia`：关于质心的完整 3×3 张量，单位 kg·m²，使用行数组；不能忽略非对角项后声称保留完整惯性。
- `colliders`：当前每块一个 `type=convex_hull` 代理，`vertices` 是刚体局部坐标的顶点。导入时构造实际凸碰撞体，并使用给定质量属性，避免再次自动累加密度造成质量重复。

GLB 仅包含显示碎块，不包含代理网格、源轮廓或其他场景物体。显示局部顶点与碰撞局部顶点处于同一坐标系。内部切面可由独立材质原语表示；多个 primitive 仍属于同一 Chunk。

生成器 0.2.0 起，GLB 材质 `extras.bd_surface_role` 明确为 `exterior` 或 `fracture`。`fracture` 同时覆盖 Voronoi 新切面与跨源构件的真实埋藏接触区域；部分接触的面按接触多边形细分，未覆盖部分保留 `exterior`。外表面材质继续携带 `extras.bd_material_id`，物理类别仍应以 Chunk 的 `material` 为准，不能从共享断面材质推断。旧资产可按材质名 `BD_DestructionInterior` 识别 Voronoi 面，但没有完整跨构件语义，需要重新烘焙。

## Bond

- `id`：由排序后的两个 Chunk ID 生成的稳定连接 ID。
- `a`、`b`：两端 Chunk ID，按字典序排列。
- `area`：接触区域总面积，单位 m²；点接触和边接触不产生 Bond。
- `center`：面积加权的接触中心，资产空间坐标。
- `normal`：从 a 指向 b 的单位法线，资产空间坐标。
- `area_covariance`：未归一化的接触面积二阶矩，`Q = integral((p-center)(p-center)^T dA)`，单位 m⁴，3×3 行数组。它可以投影到接触切平面用于弯曲近似；不要把它当作质量惯性张量。
- `state`：初始为 `rigid`。运行时损伤不写回共享资产。

每个 Chunk 对之间可能有多个共面子面，它们的接触面积、中心和二阶矩经过合并。材料强度由两端材料表读取；具体异材质界面规则和运行时断裂准则由结构求解层决定，本格式不把几何接触等同于已标定的工程焊缝。

## Anchor

`chunk` 指碎块 ID，`kind=fixed`，`position` 为资产空间的地面参考点。当前由碎块接触 Blender Z=0（运行时 Y=0）的底面规则生成。消费者应将其作为世界支撑边界，而不是给该 Chunk 添加额外可见网格。

## 发布顺序

先完整生成并校验临时 GLB，按完整内容 SHA 命名并发布，再原子替换 JSON。旧 JSON 因而始终引用旧的有效 GLB。消费者只以 JSON 作为发布入口，不扫描目录猜测最新 GLB；历史未被引用的网格清理由使用者决定。
