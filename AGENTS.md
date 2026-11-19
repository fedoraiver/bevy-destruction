# 项目协作规则

以下规则适用于本仓库内工作的所有 agent。

## 1. 禁止使用 computer-use

- 不得使用 computer-use 技能或工具，也不得通过其他工具间接调用 computer-use。

## 2. 记录设计决策与设定

- 设计决策、架构取舍和重要设定写入 `docs/adr/xxx.md`。
- 文件采用连续编号和描述性名称，例如 `docs/adr/0001-rigid-cluster-runtime.md`。
- 每份 ADR 记录状态、日期、背景、决策或提议、备选方案与取舍、验证要求和未决问题；涉及外部资料时注明来源。
- 明确区分提议、已采用的决策、已经实现的功能和已经完成的验证。方案变更时同步更新相关 ADR；替代既有决策时保留历史并建立引用。
- 相关代码、参数和 ADR 应保持一致，并随对应逻辑变更一起提交。

## 3. 使用 Conventional Commits

Git 提交遵循 Conventional Commits 1.0.0，具体规则如下，无需查阅外部文档。

### 提交结构

```text
<type>[(<scope>)][!]: <description>

[正文]

[脚注]
```

- 尖括号表示需要填写的字段，方括号表示可选部分，不写入实际提交信息。
- 标题必须包含类型、英文冒号、一个空格及非空描述。范围可选，填写时用圆括号包围；破坏性变更的 `!` 紧接类型或范围，位于冒号前。
- 类型统一使用小写；范围使用被修改的模块或子系统名称，例如 `physics`、`graph`、`adr`。
- 描述准确说明本次变更，使用简洁的动作表述，建议标题不超过 72 个字符。
- 正文可选，与标题空一行，说明变更原因、行为和必要的取舍。
- 脚注可选，与前文空一行。普通脚注采用 `Token: value` 或 `Token #value` 格式；名称中的空格用连字符替代，例如 `Reviewed-by`。脚注值可以换行，直到下一条脚注开始。

### 类型选择

本项目采用以下类型；规范允许扩展其他类型。上文的小写类型、范围命名和标题长度要求属于项目约定。

| 类型 | 用途 |
| --- | --- |
| `feat` | 新增功能 |
| `fix` | 修复 bug |
| `docs` | 仅修改文档 |
| `refactor` | 调整代码结构，不新增功能或修复 bug |
| `perf` | 改善性能 |
| `test` | 添加或修改测试 |
| `style` | 调整代码格式，不改变行为 |
| `build` | 修改构建系统或依赖 |
| `ci` | 修改持续集成配置 |
| `chore` | 不属于上述类型的维护工作 |
| `revert` | 撤销已有提交，正文注明原提交及撤销原因 |

新增功能必须使用 `feat`，修复 bug 必须使用 `fix`。一次提交表达一个完整的逻辑变更；包含多个独立目的时拆分提交。

### 破坏性变更与版本语义

- 任何类型的提交都可能包含破坏性变更，必须在标题冒号前添加 `!`，或添加 `BREAKING CHANGE: 说明` 脚注，也可以同时使用两者。
- 使用 `!` 且省略破坏性变更脚注时，标题描述必须说明破坏性变更；脚注中应解释不兼容行为和必要的迁移方式。
- `BREAKING CHANGE` 必须大写，允许包含这个空格；`BREAKING-CHANGE` 是等价写法。
- 按语义化版本解释，`fix` 对应补丁版本 PATCH，`feat` 对应次版本 MINOR，破坏性变更对应主版本 MAJOR。其他类型本身不隐含版本升级。

### 示例

```text
feat(physics): add collision-driven fracture
```

```text
fix(graph): preserve velocity when splitting clusters

Include the angular contribution when initializing each child cluster.
```

```text
feat(asset)!: require explicit bond identifiers

BREAKING CHANGE: assets must include stable bond identifiers; regenerate existing assets before loading them.
```

## 4. Agent 主动提交

- 每完成一个 bug 修复或小功能，agent 必须完成适当验证并主动执行 Git 提交。
- 提交前检查实际差异和暂存内容，只提交本次任务负责的文件，不混入用户或其他任务的无关修改。
- 完成独立的文档或设计记录变更后，同样按逻辑变更提交。
