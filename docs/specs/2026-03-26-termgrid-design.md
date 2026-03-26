# termgrid — 带 Git 上下文感知的多终端管理器

## 概述

termgrid 是一个 Rust TUI 应用，在一个界面内管理多个终端会话，自动从工作目录推断 Git 上下文并组织展示。不绑定任何特定工具——里面跑 claude、codex、vim、cargo 都行。

**核心价值**：一个全局仪表盘，能看到所有活跃终端的状态，知道每个终端在哪个项目/分支/worktree 下工作，并能快速切换交互。

## 技术栈

- **语言**：Rust
- **TUI 框架**：ratatui + crossterm
- **终端模拟**：portable-pty（PTY 进程管理）+ vte crate（ANSI 转义序列解析）+ 自建 ScreenBuffer（字符网格 + 样式属性）
- **Git 检测**：git2-rs（libgit2 绑定）
- **异步运行时**：tokio（多 PTY 输出流 + 输入 + 定时器多路复用）
- **平台**：macOS 优先（CWD 跟踪使用 `proc_pidinfo`）

## 架构

```
termgrid
├── App（状态机：Normal / Insert / Overlay 模式）
├── EventLoop（tokio 驱动，多路复用 PTY 输出 + 用户输入 + 定时器）
├── TileManager（管理所有终端 tile 的生命周期）
│   └── Tile（PTY 进程 + ScreenBuffer + Git 上下文）
│       └── ScreenBuffer（vte 解析器 + 字符网格 + ANSI 样式/颜色属性）
├── GitDetector（CWD 变化时重新检测 git 信息，带防抖）
├── TabBar（从所有 tile 的 git 上下文动态聚合项目分组）
└── Layout（网格列数配置 + 详情面板展开 + 外层 resize 响应）
```

### 终端模拟层

portable-pty 只提供原始 PTY I/O（字节流），不含 ANSI 解析。完整渲染链路：

```
PTY 字节流 → vte crate（解析 ANSI/VT100 转义序列）→ ScreenBuffer（维护字符网格 + 样式属性）→ ratatui 渲染
```

每个 Tile 拥有独立的 ScreenBuffer，存储：
- 字符网格（cols × rows + scrollback 行）
- 每个单元格的前景色/背景色/属性（粗体、下划线等），支持 true color（24-bit RGB）
- 光标位置和状态

小格和大格从同一个 ScreenBuffer 读取不同区域进行渲染。

### 事件循环

基于 tokio 的事件驱动架构，统一处理：
- N 个 PTY 输出流（异步读取，写入对应 ScreenBuffer）
- 用户键盘/鼠标输入（crossterm event stream）
- CWD 轮询定时器（tokio::time::interval）
- 外层终端 resize 事件（crossterm::event::Event::Resize）

## UI 布局

```
┌─────────────────────────────────────────────────┐
│ [ALL(6)] [fortune_v2(2)] [ft-alpha(2)] [...]    │  ← Tab 栏
├──────────────────────┬──────────────────────────┤
│ ┌────┐ ┌────┐       │                          │
│ │tile│ │tile│       │                          │
│ └────┘ └────┘       │     Detail Panel         │
│ ┌────┐ ┌────┐       │   （选中 tile 的完整终端）  │
│ │tile│ │tile│       │                          │
│ └────┘ └────┘       │                          │
├──────────────────────┴──────────────────────────┤
│ [Normal] termgrid | 6 sessions | 2 cols | ?help │  ← 状态栏
└─────────────────────────────────────────────────┘
```

### Tab 栏

- 从所有 tile 的 git 上下文动态生成
- "ALL" 固定在最左，显示全部会话
- 后接各项目名 + 会话数量，按会话数降序排列
- 点击或 Tab/Shift+Tab 切换过滤
- CWD 变化导致 tile 换项目时实时更新计数，空 Tab 自动移除

### 网格区

- 列数可配：1 / 2 / 3 列，快捷键 `1` `2` `3` 切换
- 卡片等高自适应排列，溢出可滚动
- 未选中 tile 时网格占满全宽；选中时网格左侧约 55%，详情面板右侧约 45%

### Tile 卡片

单行紧凑标题 + 迷你终端预览：

```
┌──────────────────────────────────────────────┐
│ [wait] ft-alpha [⑂ feature/weight] [⑃ wt] ~/...│  ← 单行标题
├──────────────────────────────────────────────┤
│ ✓ Implementation complete                     │
│ Shall I create a PR?                          │  ← 迷你终端（最近几行输出）
│ ❯ _                                           │
└──────────────────────────────────────────────┘
```

标题元素从左到右：
1. **状态标签**：`run`（绿）/ `wait`（黄）/ `idle Xm`（灰）
2. **项目名**：加粗白色
3. **Git 分支 tag**：蓝色，带 ⑂ 图标（仅 git 项目）
4. **Worktree tag**：紫色，带 ⑃ 图标（仅 worktree）
5. **完整路径**：灰色，靠右对齐，溢出省略

三种目录类型：
- **Git + Worktree**：项目名 + Git tag + Worktree tag + 路径
- **Git 项目**：项目名 + Git tag + 路径
- **普通目录**：📁 + 目录名 + 路径

### 详情面板

选中 tile 时右侧展开：
- 顶部：项目名、worktree 名、快捷键提示（ESC 关闭 / ↑↓ 切换）
- 信息栏：完整路径、分支名、活跃时间
- 终端区：完整终端历史，可滚动，可输入

**关键设计：小格与大格是同一个 PTY 的两个视图。** 小格显示 ScreenBuffer 的最后几行（裁剪版），大格显示完整终端。两者实时同步——在大格里输入，小格同步更新；在小格里输入，大格同样可见。小格不是只读预览，是可交互的迷你终端。

**PTY 尺寸策略**：PTY 始终以详情面板尺寸（约 45% 终端宽度 × 面板高度）设置 cols/rows。小格作为"裁剪视口"，从 ScreenBuffer 读取最后 N 行并按小格宽度截断渲染——宽度不匹配可能导致长行被截断而非回流，这是可接受的视觉折中。当详情面板关闭时，PTY 保持上一次的尺寸不变（避免频繁 resize 导致程序输出抖动）。

**输入路由**：Insert 模式下，键盘输入始终发送给当前选中 tile 的 PTY，无论详情面板是否打开。详情面板打开时用户在大格中看到完整交互；关闭时在小格中看到压缩交互——功能等价，仅视觉密度不同。

### 状态栏

显示当前模式（Normal/Insert）、会话总数、列数、帮助提示。

## 交互模型

### 模式切换（类 vim）

- **Normal 模式**：控制 Dashboard（导航、展开、管理 tile）
- **Insert 模式**：所有按键透传给选中 tile 的 PTY
- **Overlay 模式**：模态浮窗（ProjectSelector、ConfirmDialog、Help），按 Esc 返回前一模式

### 快捷键

| 操作 | Normal 模式 | Insert 模式 |
|------|------------|------------|
| 导航卡片 | ↑↓←→ / hjkl / 鼠标点击 | — |
| 进入终端 | `i` / Enter / 鼠标双击 | — |
| 退出终端 | — | `Ctrl+]`（避免与 SIGQUIT 的 `Ctrl+\` 冲突） |
| 展开详情 | 选中自动展开 | 保持展开 |
| 关闭详情 | `Esc` | — |
| 新建终端 | `n` | — |
| 关闭终端 | `x`（有运行进程时带确认） | — |
| 切列数 | `1` / `2` / `3` | — |
| 切项目 Tab | `Tab` / `Shift+Tab` / 鼠标点击 | — |
| 帮助 | `?` | — |
| 退出程序 | `q` | — |
| 滚动 | 滚轮 / PageUp / PageDown | 滚轮滚终端历史 |

### 鼠标支持

- 单击卡片：选中
- 双击卡片：进入终端（Insert 模式）
- 点击 Tab 栏：切换项目过滤
- 点击详情面板关闭区域：关闭展开
- 滚轮：网格区滚卡片列表，终端区滚历史

## Git 上下文感知

### 检测逻辑

每个 tile 持续跟踪 PTY 的 CWD，变化时触发 git 检测：

```
CWD 变化 → git2 打开 repo
  ├── 失败 → 普通目录（📁 + 目录名）
  └── 成功 → 读取 branch
        ├── .git 是文件（非目录）→ Worktree，额外读取主仓库名
        └── .git 是目录 → 普通 Git 项目
```

### CWD 跟踪

macOS 上通过 `proc_pidinfo(PROC_PIDVNODEPATHINFO)` 获取进程 CWD，轮询间隔 2 秒。不需要 hook shell，对用户透明。

**前台进程识别**：PTY spawn 的直接子进程是 shell，但需要跟踪的是 shell 的前台子进程（如 `vim`、`cargo build`）的 CWD。通过 `tcgetpgrp` 获取 PTY 的前台进程组 ID，再用 `proc_pidinfo` 获取该进程的 CWD。

**防抖**：CWD 变化后延迟 500ms 再触发 git 检测，避免频繁 chdir 的构建工具（如 cargo、make）导致大量重复检测。

### Tab 聚合规则

- 项目名 = Git 仓库的根目录名（worktree 取主仓库名）
- 多个 tile 在同一项目的不同 worktree 下 → 归入同一个 Tab
- 非 git 目录归入 "Other" 分组

**边界情况处理**：
- **bare repo**：worktree 的主仓库是 bare repo 时，取 bare repo 目录名（去掉 `.git` 后缀）作为项目名
- **submodule**：归入父项目的 Tab（取顶层仓库名）
- **嵌套 repo**：以最近的 `.git` 为准，不穿透嵌套
- **损坏的 `.git`**：git2 打开失败时降级为普通目录处理，不 panic

### Tile 状态检测

通过进程检测（不依赖特定工具）：
- **running**：PTY 前台有子进程在执行（shell 本身不算）
- **waiting**：前台是 shell 本身，等待用户输入
- **idle Xm**：waiting 状态持续超过一定时间

## 终端生命周期

### 启动

```bash
termgrid                    # 在当前目录启动，空白 dashboard
termgrid ~/workplace        # 扫描目录，列出可打开的项目
termgrid --restore          # 恢复上次退出时的会话布局
```

- 无参数启动：空白 dashboard，按 `n` 手动创建终端（使用配置文件中的 `root_dirs` 作为选择器来源）
- 带路径启动：扫描该目录下的 git repo 和 worktree，生成项目选择器，用户选择后创建 tile，不自动批量开
- `--restore`：从持久化文件恢复上次布局

### 创建

Normal 模式按 `n` → 弹出项目选择器（根目录下的 git 项目列表 + 手动输入路径），支持模糊搜索。选择后在对应目录下 spawn shell，创建新 tile。

### 关闭

Normal 模式按 `x`：
- PTY 内有运行中子进程 → 弹确认提示
- 空闲 shell → 直接关闭

关闭的是 tile 和 PTY 进程，不影响 git 仓库。

### 会话持久化

退出 termgrid 时保存布局到 `~/.config/termgrid/sessions.json`：
- 每个 tile 的工作目录
- 网格列数
- 当前选中的 Tab

`termgrid --restore` 恢复布局，重新 spawn shell。不保存终端历史内容——终端历史靠各工具自身的 session 机制（如 `claude --continue`）。

## 配置

`~/.config/termgrid/config.toml`，全部有合理默认值，零配置可用：

```toml
[layout]
default_columns = 2          # 默认列数 1-3
detail_panel_width = 45      # 详情面板宽度百分比

[scan]
root_dirs = ["~/workplace"]  # 项目选择器扫描的根目录，支持多个
scan_depth = 2               # 扫描深度

[terminal]
shell = "/bin/zsh"           # 默认 shell
cwd_poll_interval = 2        # CWD 检测轮询间隔（秒）

[keys]
exit_insert = "ctrl-]"       # 退出终端模式的快捷键
```

不做的事：
- 不做主题/颜色配置（跟随终端自身配色）
- 不做 tile 大小自定义（列数控制密度就够了）
- 不做插件系统（MVP 不需要）

## Resize 处理

外层终端窗口大小变化时：
1. crossterm 发出 `Event::Resize(cols, rows)` 事件
2. Layout 重新计算网格布局和详情面板尺寸
3. 选中 tile 的 PTY 按新的详情面板尺寸发送 `SIGWINCH`（通过 `set_size`）
4. 非选中 tile 的 PTY 不 resize（延迟到被选中时再调整）
5. 所有可见 tile 的小格按新尺寸重新从 ScreenBuffer 裁剪渲染

## 性能策略

- **只渲染可见 tile**：被 Tab 过滤隐藏的 tile 不执行渲染，但 PTY 输出仍写入 ScreenBuffer
- **ScreenBuffer 限制 scrollback**：默认保留 1000 行 scrollback，防止内存无限增长
- **渲染节流**：PTY 输出密集时（如 `cat` 大文件），合并多次写入后再触发一次渲染，避免帧率压力

## 错误处理

- **PTY spawn 失败**：在 tile 位置显示错误信息（如 shell 路径无效），允许用户关闭重试
- **git2 打开失败**：降级为普通目录，不影响 tile 正常使用
- **PTY 意外退出**：tile 显示 "[exited]" 状态，保留最后输出，按 `x` 清理

## MVP 范围

第一版聚焦核心价值，以下功能明确排除：

- ❌ 与 Claude Code / Codex 的深度集成
- ❌ 终端历史持久化
- ❌ 插件系统
- ❌ 远程终端 / SSH
- ❌ 主题定制
- ❌ Linux 支持（MVP 只做 macOS，CWD 跟踪实现不同）
