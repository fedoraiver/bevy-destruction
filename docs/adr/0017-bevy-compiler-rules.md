# ADR 0017：在 Rust CI 中自动运行 Bevy 编译器规则

## 状态

2026-11-23 采用。替代 [ADR 0013](0013-shared-bevy-cicd.md) 的编译器规则手动触发策略。

## 背景

Rust CI 已能调用 bevy-cicd 的编译器规则。原先需手动勾选运行选项，常规 PR 检查无法发现新增的实体命名和具名字段违规。

## 决策与实现

- 面向 `master` 的 PR、对 `master` 的推送和手动运行均执行 `compiler-rules`，工作流继续按 [ADR 0015](0015-follow-bevy-cicd-main.md) 引用 `bevy-cicd@main`。
- 共享工作流对仓库根目录的 Cargo 包运行 `entity-names` 和 `named-fields`。生产代码创建实体时提供可辨认的 `Name`，单字段元组结构体改用描述用途的具名字段。

## 备选方案与取舍

保留手动选项可减少 Nightly 工具链的编译开销。自动运行可以在合并前发现违规，并沿用上游的规则实现和工具链版本。

## 验证要求

用上游固定的工具链和规则分析器检查默认 Cargo 配置，运行项目 Rust 回归并校验工作流语法。远端验收需确认 Rust CI 中的两个矩阵任务都执行并通过。

本地在 Windows 上用 bevy-cicd `421c7afb9ed4` 和 `nightly-2026-10-31` 完成检查。`entity-names` 检查了 131 处实体声明，无违规和无法解析的名称。`named-fields` 检查了 4 个编译目标，无违规。147 项 Rust 测试、严格 Clippy、格式和工作流语法检查均通过。

## 未决问题

首次 GitHub Actions 运行结果待验证。

## 来源

[bevy-cicd 编译器规则工作流](https://github.com/fedoraiver/bevy-cicd/blob/main/.github/workflows/bevy-rules.yml)。
