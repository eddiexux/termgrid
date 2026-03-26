# 项目进度

## 当前任务
termgrid MVP 开发，分支 main

## 待办

### P0 — 收尾/阻塞解除
- [ ] 手动测试：`cargo run` 启动 → 按 n 创建 tile → 输入命令 → Ctrl+] 退出
- [ ] 修复手动测试中发现的问题

### P1 — 高价值
- [ ] ProjectSelector 完整模糊搜索实现（当前是 placeholder）
- [ ] 鼠标支持完善（单击选中、双击进入 Insert、点击 Tab）
- [ ] 网格滚动键绑定（PageUp/PageDown）

### P2 — 重要但大
- [ ] 会话恢复完善（--restore 路径验证、错误处理）
- [ ] 性能优化（渲染节流、只渲染可见 tile）

## 阻塞项
无

## 上下文备忘
- MVP 代码实现完成：14 个 Task 全部通过，105 个测试，21 个源文件，~8300 行
- ScreenBuffer 使用 usize（非 plan 中的 u16），避免 vector 索引时的类型转换
- PTY reader 使用 tokio::spawn_blocking 模式避免阻塞异步运行时
- 退出 Insert 模式快捷键：Ctrl+]
