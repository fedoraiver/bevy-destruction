# 项目协作规则

以下规则适用于本仓库内工作的所有 agent。

## 1. 禁止使用 computer-use

- 不得使用 computer-use 技能或工具，也不得通过其他工具间接调用 computer-use。
- 通过命令行、项目代码、测试或非图形界面的 API 完成必要操作。

## 2. 记录设计决策与设定

- 设计决策、架构取舍和重要设定必须写入 `docs/adr/xxx.md`，不能只保留在聊天中。
- 文件采用连续编号和描述性名称，例如 `docs/adr/0001-prefractured-graph-and-clusters.md`。
- 每份 ADR 记录状态、日期、背景、决策或提议、备选方案与取舍、验证要求和未决问题；涉及外部资料时注明来源。
- 明确区分提议、已采用的决策、已经实现的功能和已经完成的验证。方案变更时同步更新相关 ADR；替代既有决策时保留历史并建立引用。
- 相关代码、参数和 ADR 应保持一致，并随对应逻辑变更一起提交。

## 3. 使用 Conventional Commits

Git 提交遵循 [Conventional Commits 1.0.0](https://www.conventionalcommits.org/zh-hans/v1.0.0/)。

提交标题格式：

```text
<type>[optional scope][!]: <description>
```

- 新增功能使用 `feat`，修复 bug 使用 `fix`，仅修改文档使用 `docs`；其他变更选择对应的 `refactor`、`test`、`perf`、`build`、`ci` 或 `chore` 等类型。
- 破坏性变更使用 `!` 或 `BREAKING CHANGE:` 脚注明确标记。
- 一次提交表达一个完整的逻辑变更，描述应简洁、准确。

## 4. Agent 主动提交

- 每完成一个 bug 修复或小功能，agent 必须完成适当验证并主动执行 Git 提交，不能只留下未提交的修改或仅建议提交。
- 提交前检查实际差异和暂存内容，只提交本次任务负责的文件，不混入用户或其他任务的无关修改。
- 完成独立的文档或设计记录变更后，同样按逻辑变更提交。
- 不绕过提交钩子。验证或提交失败时先处理失败；无法完成时如实报告原因，不得声称已提交。
