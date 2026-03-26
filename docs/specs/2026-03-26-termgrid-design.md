# termgrid — 带 Git 上下文感知的多终端管理器

## 概述

termgrid 是一个 Rust TUI 应用，在一个界面内管理多个终端会话，自动从工作目录推断 Git 上下文并组织展示。不绑定任何特定工具——里面跑 claude、codex、vim、cargo 都行。

**核心价值**：一个全局仪表盘，能看到所有活跃终端的状态，知道每个终端在哪个项目/分支/worktree 下工作，并能快速切换交互。

## 技术栈

- **语言**：Rust
- **TUI 框架**：ratatui + crossterm
- **终端模拟**：portable-pty（每个 tile 内嵌一个 PTY）
- **Git 检测**：git2-rs（libgit2 绑定）
- **平台**：macOS 优先（CWD 跟踪使用 `proc_pidinfo`）

## 架构

```
termgrid
├── App（状态机：Normal / Insert 模式）
├── TileManager（管理所有终端 tile 的生命周期）
│   └── Tile（PTY 进程 + 输出 buffer + Git 上下文）
├── GitDetector（CWD 变化时重新检测 git 信息）
├── TabBar（从所有 tile 的 git 上下文动态聚合项目分组）
└── Layout（网格列数配置 + 详情面板展开）
```

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

### 状态栏

显示当前模式（Normal/Insert）、会话总数、列数、帮助提示。

## 交互模型

### 模式切换（类 vim）

- **Normal 模式**：控制 Dashboard（导航、展开、管理 tile）
- **Insert 模式**：所有按键透传给选中 tile 的 PTY

### 快捷键

| 操作 | Normal 模式 | Insert 模式 |
|------|------------|------------|
| 导航卡片 | ↑↓←→ / hjkl / 鼠标点击 | — |
| 进入终端 | `i` / Enter / 鼠标双击 | — |
| 退出终端 | — | `Ctrl+\` |
| 展开详情 | 选中自动展开 | 保持展开 |
| 关闭详情 | `Esc` | — |
| 新建终端 | `n` | — |
| 关闭终端 | `x`（有运行进程时带确认） | — |
| 切列数 | `1` / `2` / `3` | — |
| 切项目 Tab | `Tab` / `Shift+Tab` / 鼠标点击 | — |
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

macOS 上通过 `proc_pidinfo` 获取 PTY 前台进程的 CWD，轮询间隔 1-2 秒。不需要 hook shell，对用户透明。

### Tab 聚合规则

- 项目名 = Git 仓库的根目录名（worktree 取主仓库名）
- 多个 tile 在同一项目的不同 worktree 下 → 归入同一个 Tab
- 非 git 目录归入 "Other" 分组

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
exit_insert = "ctrl-\\"      # 退出终端模式的快捷键
```

不做的事：
- 不做主题/颜色配置（跟随终端自身配色）
- 不做 tile 大小自定义（列数控制密度就够了）
- 不做插件系统（MVP 不需要）

## MVP 范围

第一版聚焦核心价值，以下功能明确排除：

- ❌ 与 Claude Code / Codex 的深度集成
- ❌ 终端历史持久化
- ❌ 插件系统
- ❌ 远程终端 / SSH
- ❌ 主题定制
- ❌ Linux 支持（MVP 只做 macOS，CWD 跟踪实现不同）
