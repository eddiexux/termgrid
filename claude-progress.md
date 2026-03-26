# 项目进度

## 当前任务
termgrid MVP 开发，分支 main

## 待办

### P0 — 收尾/阻塞解除
- [ ] 执行实施规划（invoke writing-plans skill），基于设计文档生成分阶段实施 plan
- [ ] cargo init 初始化 Rust 项目

### P1 — 高价值
- [ ] 核心模块实现：App 状态机、TileManager、Tile（PTY）、GitDetector、TabBar、Layout
- [ ] 键盘 + 鼠标交互
- [ ] 会话持久化与 --restore

### P2 — 重要但大
- [ ] 项目选择器（模糊搜索）
- [ ] 配置文件支持（~/.config/termgrid/config.toml）

## 阻塞项
无

## 上下文备忘
- 设计文档：docs/specs/2026-03-26-termgrid-design.md，已通过自审和用户审阅
- 用户要求严格 TDD：所有功能先写测试再实现，单测覆盖面广 + 集成测试
- 用户要求项目结构成熟、代码可复用、架构清晰
- 技术栈确认：ratatui + crossterm + portable-pty + git2-rs
- 小格与大格是同一个 PTY 的两个视图，实时同步，小格可输入
- 网格列数 1/2/3 可配，用户运行时切换
- 下一步：新会话中 cd 到项目目录，执行 writing-plans skill 生成实施计划
