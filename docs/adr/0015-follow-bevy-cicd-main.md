# ADR 0015：跟踪 bevy-cicd 主分支

## 状态

2026-11-22 采用。替代 [ADR 0013](0013-shared-bevy-cicd.md) 的共享工作流固定 SHA 策略。

## 背景

项目通过 bevy-cicd 运行 Rust 检查、按需运行 Bevy 编译器规则，并构建和发布 [ADR 0014](0014-generated-ecs-architecture.md) 的架构站点。上游通过 `main` 提供这些工作流和调度导出代码。

## 决策与实现

- 四个可复用工作流统一引用 `bevy-cicd@main`。GitHub Actions 解析上游引用，调用方继续按项目的 `master` 分支触发检查和发布。
- `bevy-schedule-export` 从 bevy-cicd 的默认分支 `main` 解析。`Cargo.lock` 记录具体提交，通过 `cargo update -p bevy-schedule-export` 更新。

## 备选方案与取舍

固定工作流 SHA 可以控制上游改动进入项目的时机，每次升级都需要修改调用配置。跟踪 `main` 使新运行直接采用上游改动，同一项目提交的不同运行可能使用不同的工作流版本。Cargo 继续通过锁文件复现构建依赖。

## 验证要求

核对四个工作流引用和锁文件中的上游提交，运行带 `debugdump` 的锁定编译检查。远端验收以 Rust CI、架构构建和 Pages 发布通过为准。

## 未决问题

待验证 GitHub 上使用 `@main` 的首次运行。

## 来源

[bevy-cicd 使用说明](https://github.com/fedoraiver/bevy-cicd/blob/main/README.md)。
