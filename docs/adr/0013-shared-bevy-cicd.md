# ADR 0013：复用 Bevy 持续集成与架构发布

## 状态

2026-11-21 采用。静态架构索引的发布方案由 [ADR 0014](0014-generated-ecs-architecture.md) 替代。

## 背景

项目已有无窗口 Rust 回归和三张随架构决策维护的 SVG。共用的 Rust 检查和 GitHub Pages 发布逻辑由 bevy-cicd 维护，项目决定检查触发条件和发布内容。

## 决策与实现

- PR 和 master 推送运行共享 Rust 工作流。实体命名和具名字段检查由 Actions 中的 Rust CI 手动触发，使用已有代码报告规则结果。
- 架构站点打包三张现有 SVG 与静态首页，按同一次运行上传站点和发布证明。PR 仅打包，master 推送和默认分支手动运行可发布。Pages 发布与 Rust 测试分开运行。
- 调用配置固定到共享工作流的提交 SHA。基于提交链接可追溯每次运行所使用的检查与发布代码。

## 备选方案与取舍

把完整工作流复制到本项目便于独立修改，但通用规则会分散维护。项目仍负责准备并校验自身的站点产物，以便沿用已维护的架构图和 ADR。

## 验证要求

- PR 检查执行全部 Rust 测试，包含 [ADR 0003](0003-collision-driven-fracture.md) 记录的高楼落地回归。
- PR 打包首页、三张架构图和发布证明。默认分支的 Pages 运行应公开这三张图，且线上首页哈希与运行产物匹配。

## 未决问题

手动规则报告用于评估今后能否在常规 CI 中启用实体命名和具名字段检查。

## 来源

[Bevy CI/CD 工作流](https://github.com/fedoraiver/bevy-cicd)和 [GitHub Pages 自定义工作流](https://docs.github.com/en/pages/getting-started-with-github-pages/using-custom-workflows-with-github-pages)。
