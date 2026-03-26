# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-03-26

### Added

- Multi-terminal grid dashboard with 1/2/3 column layouts
- Git context awareness: auto-detect project, branch, worktree per tile
- Tab bar with dynamic project grouping and filtering
- Detail panel showing full terminal output with ANSI colors
- Vim-like mode system: Normal, Insert, Overlay
- Session persistence with scrollback history restore
- Mouse support: click to select, double-click to enter, scroll
- Full terminal emulation via vt100 crate
- CJK wide character support
- macOS CWD tracking via proc_pidinfo
- Auto-close tiles on PTY exit (Ctrl+D)
- Tile grouping by project with index labels for duplicates
- File-based logging to ~/.local/share/termgrid/
- TOML configuration (~/.config/termgrid/config.toml)
- CLI with `--fresh` flag and optional path argument
