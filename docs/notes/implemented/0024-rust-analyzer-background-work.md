# ADR 0024：限制 Rust Analyzer 的后台检查开销

## 状态

2026-11-28 采用并实现。

## 背景

VS Code 中保存 Rust 文件会触发工作区 Cargo 检查，并行编译会占用额外内存。应用入口通过 `#[path]` 使用仓库根目录的共享 `src/`，而根目录下也有编译、验收和调试产物。[应用装配](0019-workspace-app-entrypoints.md)

## 决策与实现

Rust Analyzer 向保存检查传入 `--jobs 2`，保留完整的检查范围和默认开启的启动缓存预热。VFS 保留工作区根目录，扫描和 VS Code 文件监听跳过 `tmp/`、`target/` 和 `output/`。[工作区配置](../../../.vscode/settings.json)

## 备选方案与取舍

关闭启动预热会将首次导航和补全的部分成本移到编辑时。限制 Cargo 并发降低的是保存检查的内存峰值，冷缓存检查可能耗时更久。保留根目录有利于共享源码导航和锁文件变更监听。

## 验证要求

通过独立语言服务器确认启动预热完成、共享模块可跳转、产物文件被排除，且 Cargo 保存检查使用 `--workspace --all-targets --jobs 2`。

## 未决问题

本项目的内存收益尚未进行前后对照测量。

## 来源

[Rust Analyzer 检查参数](https://rust-analyzer.github.io/book/configuration.html#check.extraArgs) · [VFS 排除项](https://rust-analyzer.github.io/book/configuration.html#files.exclude)
