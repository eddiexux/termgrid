# termgrid

[![Crates.io](https://img.shields.io/crates/v/termgrid.svg)](https://crates.io/crates/termgrid)
[![CI](https://github.com/eddiexux/termgrid/actions/workflows/ci.yml/badge.svg)](https://github.com/eddiexux/termgrid/actions)
[![License](https://img.shields.io/crates/l/termgrid.svg)](LICENSE-MIT)

A terminal multiplexer with Git context awareness, built in Rust.

Manage multiple terminal sessions in a single dashboard. Each tile automatically detects Git project, branch, and worktree — letting you see at a glance what's running where.

[中文文档](README_CN.md)

![termgrid screenshot](docs/ScreenShot_2026-03-26_144613_171.png)

## Features

- **Multi-terminal grid** — 1/2/3 column layout with tile cards showing live terminal preview
- **Git context awareness** — auto-detect project name, branch, worktree per tile
- **Project grouping** — tab bar groups tiles by Git project, click to filter
- **Detail panel** — select a tile to see full terminal output with colors
- **Vim-like modes** — Normal (navigate), Insert (type), Overlay (dialogs)
- **Session persistence** — auto-save/restore tile layout and scrollback history on restart
- **Mouse support** — click to select tiles, drag to select text (auto-copy to clipboard)
- **Full terminal emulation** — powered by [vt100](https://crates.io/crates/vt100), supports complex TUI apps
- **CJK support** — correct wide character rendering
- **Logging** — debug logs to `~/.local/share/termgrid/termgrid.log`

## Installation

### From crates.io

```bash
cargo install termgrid
```

### Update to latest version

```bash
cargo install termgrid --force
```

### From source (latest dev)

```bash
cargo install --git https://github.com/eddiexux/termgrid.git
```

### Build locally

```bash
git clone https://github.com/eddiexux/termgrid.git
cd termgrid
cargo build --release
# Binary at target/release/termgrid
```

## Usage

```bash
termgrid                # Start with saved session (or empty dashboard)
termgrid ~/projects     # Open a tile in the given directory
termgrid --fresh        # Ignore saved session, start empty
```

### Keyboard Shortcuts

| Action | Normal Mode | Insert Mode |
|--------|------------|-------------|
| Navigate tiles | `hjkl` / Arrow keys | - |
| Enter terminal | `i` / `Enter` | - |
| Exit terminal | - | `Esc` |
| New tile | `n` | - |
| Close tile | `x` | - |
| Switch columns | `1` / `2` / `3` | - |
| Switch project tab | `Tab` / `Shift+Tab` | - |
| Help | `?` | - |
| Quit | `q` | - |

### Mouse

- **Click** tile card to select
- **Double-click** to enter Insert mode
- **Drag** in detail panel to select text (auto-copies to clipboard on release)
- **Scroll** to navigate grid

## Configuration

Optional config at `~/.config/termgrid/config.toml`:

```toml
[layout]
default_columns = 2          # 1, 2, or 3
detail_panel_width = 45      # percentage

[scan]
root_dirs = ["~/workplace"]  # project scanner roots
scan_depth = 2

[terminal]
shell = "/bin/zsh"
cwd_poll_interval = 2        # seconds

[keys]
exit_insert = "ctrl-]"       # alternative Insert exit key
```

## Platform Support

- **macOS** — full support (CWD tracking via `proc_pidinfo`)
- **Linux** — planned (CWD tracking via `/proc`)
- **Windows** — not planned

## Architecture

```
termgrid
├── App          — state machine (Normal/Insert/Overlay modes)
├── EventLoop    — tokio-driven, multiplexes PTY output + input + timers
├── TileManager  — tile lifecycle, selection, grid navigation
│   └── Tile     — PTY process + vt100 terminal emulator + Git context
├── GitDetector  — CWD change → git2 repo detection (with debounce)
├── TabBar       — dynamic project grouping from tile Git contexts
├── Layout       — multi-column grid + detail panel calculation
└── UI           — ratatui widgets (tile card, detail panel, tab bar, overlays)
```

## Tech Stack

| Component | Crate |
|-----------|-------|
| TUI framework | [ratatui](https://ratatui.rs/) + [crossterm](https://crates.io/crates/crossterm) |
| Terminal emulation | [vt100](https://crates.io/crates/vt100) |
| PTY management | [portable-pty](https://crates.io/crates/portable-pty) |
| Git detection | [git2](https://crates.io/crates/git2) |
| Async runtime | [tokio](https://tokio.rs/) |

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT License ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.
