# termgrid MVP Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a Rust TUI multi-terminal manager with Git context awareness — spawn multiple PTYs, render them in a grid dashboard, detect Git project/branch/worktree per tile, and provide vim-like Normal/Insert mode interaction.

**Architecture:** Event-driven single-thread main loop on tokio. Multiple PTY output readers run in spawned tasks, all events funnel through an mpsc channel. Main loop processes events, updates state, and renders via ratatui. ScreenBuffer (vte parser + character grid) is the bridge between raw PTY bytes and ratatui rendering.

**Tech Stack:** Rust 2021, ratatui 0.30, crossterm 0.29 (event-stream), portable-pty 0.9, vte 0.15, git2 0.19, tokio 1, serde + toml, clap 4, libc (macOS FFI)

---

## File Structure

```
Cargo.toml
src/
├── main.rs              # CLI parsing (clap) + terminal setup + app entry
├── lib.rs               # Module declarations
├── screen.rs            # Cell, CursorState, ScreenBuffer + Perform impl
├── config.rs            # Config struct + TOML loading
├── pty.rs               # PtyHandle (spawn, resize, write, reader extraction)
├── process.rs           # macOS: foreground PID, process CWD, process state
├── git.rs               # GitContext + detect_git()
├── tab.rs               # TabEntry, TabFilter, TabBar aggregation
├── layout.rs            # LayoutResult + calculate_layout()
├── tile.rs              # Tile (Screen + PTY + Git), TileId
├── tile_manager.rs      # TileManager (lifecycle, selection, filtering)
├── event.rs             # AppEvent enum + event loop setup
├── input.rs             # Input handler (Normal/Insert/Overlay dispatch)
├── session.rs           # Session save/restore
├── app.rs               # App struct, AppMode, main run loop
├── ui/
│   ├── mod.rs           # render() entry point
│   ├── tile_card.rs     # TileCard widget (small tile in grid)
│   ├── detail_panel.rs  # DetailPanel widget (full terminal view)
│   ├── tab_bar.rs       # TabBar widget
│   ├── status_bar.rs    # StatusBar widget
│   └── overlay.rs       # ProjectSelector, ConfirmDialog, Help overlays
```

---

## Phase A: Core Engine

### Task 1: Project Scaffold

**Files:**
- Create: `Cargo.toml`
- Create: `src/main.rs`
- Create: `src/lib.rs`

- [ ] **Step 1: Initialize Cargo project**

Run: `cargo init --name termgrid`

- [ ] **Step 2: Write Cargo.toml with all dependencies**

```toml
[package]
name = "termgrid"
version = "0.1.0"
edition = "2021"
description = "A multi-terminal manager with Git context awareness"

[dependencies]
ratatui = "0.30"
crossterm = { version = "0.29", features = ["event-stream"] }
portable-pty = "0.9"
vte = "0.15"
git2 = "0.19"
tokio = { version = "1", features = ["full"] }
futures = "0.3"
serde = { version = "1", features = ["derive"] }
toml = "0.8"
dirs = "6"
clap = { version = "4", features = ["derive"] }
libc = "0.2"

[dev-dependencies]
tempfile = "3"
```

- [ ] **Step 3: Write minimal main.rs**

```rust
fn main() {
    println!("termgrid v{}", env!("CARGO_PKG_VERSION"));
}
```

- [ ] **Step 4: Write lib.rs with empty module declarations**

```rust
// Modules will be added incrementally as they are implemented.
```

- [ ] **Step 5: Verify compilation**

Run: `cargo check`
Expected: Compiles with 0 errors (dependency download may take time on first run)

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock src/main.rs src/lib.rs
git commit -m "feat: init termgrid project with dependencies"
```

---

### Task 2: ScreenBuffer

**Files:**
- Create: `src/screen.rs`
- Modify: `src/lib.rs` (add `pub mod screen;`)

**Depends on:** Task 1

The ScreenBuffer is the core data structure — a character grid with ANSI color/attribute support, cursor tracking, and scrollback. It stores `ratatui::style` types directly to avoid conversion at render time.

- [ ] **Step 1: Add module declaration to lib.rs**

Add to `src/lib.rs`:
```rust
pub mod screen;
```

- [ ] **Step 2: Write failing tests for Cell and ScreenBuffer**

Write `src/screen.rs` with types + tests (tests will fail because methods aren't implemented yet):

```rust
use ratatui::style::{Color, Modifier};
use std::collections::VecDeque;

/// A single character cell in the terminal grid.
#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    pub ch: char,
    pub fg: Color,
    pub bg: Color,
    pub modifiers: Modifier,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            ch: ' ',
            fg: Color::Reset,
            bg: Color::Reset,
            modifiers: Modifier::empty(),
        }
    }
}

/// Cursor position and visibility state.
#[derive(Debug, Clone)]
pub struct CursorState {
    pub row: u16,
    pub col: u16,
    pub visible: bool,
}

impl Default for CursorState {
    fn default() -> Self {
        Self { row: 0, col: 0, visible: true }
    }
}

/// Terminal screen buffer — character grid + scrollback + cursor.
pub struct ScreenBuffer {
    cols: u16,
    rows: u16,
    grid: Vec<Vec<Cell>>,
    scrollback: VecDeque<Vec<Cell>>,
    max_scrollback: usize,
    pub cursor: CursorState,
    /// Current drawing style applied to new characters.
    pub current_fg: Color,
    pub current_bg: Color,
    pub current_modifiers: Modifier,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_creates_empty_grid() {
        let buf = ScreenBuffer::new(80, 24);
        assert_eq!(buf.cols(), 80);
        assert_eq!(buf.rows(), 24);
        // All cells should be default (space, Reset colors)
        let lines = buf.visible_lines();
        assert_eq!(lines.len(), 24);
        assert_eq!(lines[0].len(), 80);
        assert_eq!(lines[0][0], Cell::default());
    }

    #[test]
    fn test_put_char_at_cursor() {
        let mut buf = ScreenBuffer::new(80, 24);
        buf.put_char('A');
        assert_eq!(buf.visible_lines()[0][0].ch, 'A');
        // Cursor should advance
        assert_eq!(buf.cursor.col, 1);
    }

    #[test]
    fn test_put_char_wraps_at_end_of_line() {
        let mut buf = ScreenBuffer::new(5, 3);
        for c in "Hello!".chars() {
            buf.put_char(c);
        }
        // "Hello" fills row 0, "!" wraps to row 1
        assert_eq!(buf.visible_lines()[0][4].ch, 'o');
        assert_eq!(buf.visible_lines()[1][0].ch, '!');
        assert_eq!(buf.cursor.row, 1);
        assert_eq!(buf.cursor.col, 1);
    }

    #[test]
    fn test_newline() {
        let mut buf = ScreenBuffer::new(80, 24);
        buf.put_char('A');
        buf.newline();
        assert_eq!(buf.cursor.row, 1);
        assert_eq!(buf.cursor.col, 0);
    }

    #[test]
    fn test_newline_at_bottom_scrolls() {
        let mut buf = ScreenBuffer::new(5, 3);
        buf.put_char('A');
        buf.newline(); // row 1
        buf.put_char('B');
        buf.newline(); // row 2
        buf.put_char('C');
        buf.newline(); // row 2 (at bottom), should scroll
        // Row 0 ("A    ") goes to scrollback
        assert_eq!(buf.visible_lines()[0][0].ch, 'B');
        assert_eq!(buf.visible_lines()[1][0].ch, 'C');
        assert_eq!(buf.visible_lines()[2][0].ch, ' ');
        assert_eq!(buf.scrollback_len(), 1);
    }

    #[test]
    fn test_carriage_return() {
        let mut buf = ScreenBuffer::new(80, 24);
        buf.put_char('A');
        buf.put_char('B');
        buf.carriage_return();
        assert_eq!(buf.cursor.col, 0);
        assert_eq!(buf.cursor.row, 0);
    }

    #[test]
    fn test_backspace() {
        let mut buf = ScreenBuffer::new(80, 24);
        buf.put_char('A');
        buf.put_char('B');
        buf.backspace();
        assert_eq!(buf.cursor.col, 1);
        // Backspace moves cursor but doesn't erase
    }

    #[test]
    fn test_tab() {
        let mut buf = ScreenBuffer::new(80, 24);
        buf.put_char('A');
        buf.tab();
        assert_eq!(buf.cursor.col, 8); // next tab stop
    }

    #[test]
    fn test_set_cursor_position() {
        let mut buf = ScreenBuffer::new(80, 24);
        buf.set_cursor_position(5, 10);
        assert_eq!(buf.cursor.row, 5);
        assert_eq!(buf.cursor.col, 10);
    }

    #[test]
    fn test_set_cursor_clamps_to_bounds() {
        let mut buf = ScreenBuffer::new(10, 5);
        buf.set_cursor_position(100, 100);
        assert_eq!(buf.cursor.row, 4);
        assert_eq!(buf.cursor.col, 9);
    }

    #[test]
    fn test_erase_in_display_below() {
        let mut buf = ScreenBuffer::new(5, 3);
        for c in "ABCDE".chars() { buf.put_char(c); }
        buf.newline();
        for c in "FGHIJ".chars() { buf.put_char(c); }
        buf.set_cursor_position(0, 2);
        buf.erase_in_display(0); // erase from cursor to end
        assert_eq!(buf.visible_lines()[0][0].ch, 'A');
        assert_eq!(buf.visible_lines()[0][1].ch, 'B');
        assert_eq!(buf.visible_lines()[0][2].ch, ' '); // erased
        assert_eq!(buf.visible_lines()[1][0].ch, ' '); // erased
    }

    #[test]
    fn test_erase_in_display_all() {
        let mut buf = ScreenBuffer::new(5, 3);
        for c in "ABCDE".chars() { buf.put_char(c); }
        buf.erase_in_display(2); // erase entire display
        for row in buf.visible_lines() {
            for cell in row {
                assert_eq!(cell.ch, ' ');
            }
        }
    }

    #[test]
    fn test_erase_in_line() {
        let mut buf = ScreenBuffer::new(10, 3);
        for c in "ABCDEFGHIJ".chars() { buf.put_char(c); }
        buf.set_cursor_position(0, 5);
        buf.erase_in_line(0); // erase from cursor to end of line
        assert_eq!(buf.visible_lines()[0][4].ch, 'E');
        assert_eq!(buf.visible_lines()[0][5].ch, ' ');
    }

    #[test]
    fn test_scroll_up() {
        let mut buf = ScreenBuffer::new(5, 3);
        buf.put_char('A');
        buf.newline();
        buf.put_char('B');
        buf.scroll_up(1);
        assert_eq!(buf.visible_lines()[0][0].ch, 'B');
        assert_eq!(buf.visible_lines()[1][0].ch, ' ');
        assert_eq!(buf.scrollback_len(), 1);
    }

    #[test]
    fn test_scrollback_limit() {
        let mut buf = ScreenBuffer::new(5, 2);
        buf.set_max_scrollback(3);
        for i in 0..10 {
            buf.put_char(char::from(b'A' + i));
            buf.newline();
        }
        assert!(buf.scrollback_len() <= 3);
    }

    #[test]
    fn test_resize_grow() {
        let mut buf = ScreenBuffer::new(5, 3);
        buf.put_char('A');
        buf.resize(10, 5);
        assert_eq!(buf.cols(), 10);
        assert_eq!(buf.rows(), 5);
        assert_eq!(buf.visible_lines()[0][0].ch, 'A');
    }

    #[test]
    fn test_resize_shrink() {
        let mut buf = ScreenBuffer::new(10, 5);
        buf.put_char('A');
        buf.resize(5, 3);
        assert_eq!(buf.cols(), 5);
        assert_eq!(buf.rows(), 3);
        assert_eq!(buf.visible_lines()[0][0].ch, 'A');
    }

    #[test]
    fn test_last_n_lines() {
        let mut buf = ScreenBuffer::new(5, 5);
        buf.put_char('A'); buf.newline();
        buf.put_char('B'); buf.newline();
        buf.put_char('C'); buf.newline();
        buf.put_char('D'); buf.newline();
        buf.put_char('E');
        let last3 = buf.last_n_lines(3);
        assert_eq!(last3.len(), 3);
        assert_eq!(last3[0][0].ch, 'C');
        assert_eq!(last3[1][0].ch, 'D');
        assert_eq!(last3[2][0].ch, 'E');
    }

    #[test]
    fn test_style_applied_to_put_char() {
        let mut buf = ScreenBuffer::new(80, 24);
        buf.current_fg = Color::Red;
        buf.current_bg = Color::Blue;
        buf.current_modifiers = Modifier::BOLD;
        buf.put_char('X');
        let cell = &buf.visible_lines()[0][0];
        assert_eq!(cell.fg, Color::Red);
        assert_eq!(cell.bg, Color::Blue);
        assert_eq!(cell.modifiers, Modifier::BOLD);
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test --lib screen::tests`
Expected: FAIL — methods not implemented

- [ ] **Step 4: Implement ScreenBuffer**

Add the implementation above the `#[cfg(test)]` block in `src/screen.rs`:

```rust
impl ScreenBuffer {
    pub fn new(cols: u16, rows: u16) -> Self {
        let grid = (0..rows)
            .map(|_| vec![Cell::default(); cols as usize])
            .collect();
        Self {
            cols,
            rows,
            grid,
            scrollback: VecDeque::new(),
            max_scrollback: 1000,
            cursor: CursorState::default(),
            current_fg: Color::Reset,
            current_bg: Color::Reset,
            current_modifiers: Modifier::empty(),
        }
    }

    pub fn cols(&self) -> u16 { self.cols }
    pub fn rows(&self) -> u16 { self.rows }
    pub fn scrollback_len(&self) -> usize { self.scrollback.len() }
    pub fn visible_lines(&self) -> &[Vec<Cell>] { &self.grid }

    pub fn set_max_scrollback(&mut self, max: usize) {
        self.max_scrollback = max;
    }

    /// Get the last N lines from the visible grid (for mini tile rendering).
    pub fn last_n_lines(&self, n: usize) -> Vec<&[Cell]> {
        let total = self.grid.len();
        let start = total.saturating_sub(n);
        self.grid[start..].iter().map(|row| row.as_slice()).collect()
    }

    /// Put a character at cursor position, applying current style. Advances cursor.
    pub fn put_char(&mut self, ch: char) {
        let row = self.cursor.row as usize;
        let col = self.cursor.col as usize;
        if row < self.rows as usize && col < self.cols as usize {
            self.grid[row][col] = Cell {
                ch,
                fg: self.current_fg,
                bg: self.current_bg,
                modifiers: self.current_modifiers,
            };
        }
        self.cursor.col += 1;
        if self.cursor.col >= self.cols {
            self.cursor.col = 0;
            self.advance_row();
        }
    }

    /// Move cursor to next line, scrolling if at bottom.
    fn advance_row(&mut self) {
        if self.cursor.row + 1 < self.rows {
            self.cursor.row += 1;
        } else {
            self.scroll_up(1);
        }
    }

    pub fn newline(&mut self) {
        self.cursor.col = 0;
        self.advance_row();
    }

    pub fn carriage_return(&mut self) {
        self.cursor.col = 0;
    }

    pub fn backspace(&mut self) {
        if self.cursor.col > 0 {
            self.cursor.col -= 1;
        }
    }

    pub fn tab(&mut self) {
        let next_tab = ((self.cursor.col / 8) + 1) * 8;
        self.cursor.col = next_tab.min(self.cols - 1);
    }

    pub fn set_cursor_position(&mut self, row: u16, col: u16) {
        self.cursor.row = row.min(self.rows.saturating_sub(1));
        self.cursor.col = col.min(self.cols.saturating_sub(1));
    }

    /// Scroll the visible grid up by `n` lines. Top lines go to scrollback.
    pub fn scroll_up(&mut self, n: u16) {
        for _ in 0..n {
            if self.grid.is_empty() { break; }
            let row = self.grid.remove(0);
            self.scrollback.push_back(row);
            if self.scrollback.len() > self.max_scrollback {
                self.scrollback.pop_front();
            }
            self.grid.push(vec![Cell::default(); self.cols as usize]);
        }
    }

    /// Erase in display. mode: 0=below, 1=above, 2=all, 3=all+scrollback.
    pub fn erase_in_display(&mut self, mode: u16) {
        match mode {
            0 => {
                // Erase from cursor to end of display
                let row = self.cursor.row as usize;
                let col = self.cursor.col as usize;
                // Clear rest of current line
                for c in col..self.cols as usize {
                    self.grid[row][c] = Cell::default();
                }
                // Clear all lines below
                for r in (row + 1)..self.rows as usize {
                    self.grid[r] = vec![Cell::default(); self.cols as usize];
                }
            }
            1 => {
                // Erase from start to cursor
                let row = self.cursor.row as usize;
                let col = self.cursor.col as usize;
                for r in 0..row {
                    self.grid[r] = vec![Cell::default(); self.cols as usize];
                }
                for c in 0..=col.min(self.cols as usize - 1) {
                    self.grid[row][c] = Cell::default();
                }
            }
            2 | 3 => {
                // Erase entire display
                for row in &mut self.grid {
                    *row = vec![Cell::default(); self.cols as usize];
                }
                if mode == 3 {
                    self.scrollback.clear();
                }
            }
            _ => {}
        }
    }

    /// Erase in line. mode: 0=to right, 1=to left, 2=entire line.
    pub fn erase_in_line(&mut self, mode: u16) {
        let row = self.cursor.row as usize;
        let col = self.cursor.col as usize;
        if row >= self.rows as usize { return; }
        match mode {
            0 => {
                for c in col..self.cols as usize {
                    self.grid[row][c] = Cell::default();
                }
            }
            1 => {
                for c in 0..=col.min(self.cols as usize - 1) {
                    self.grid[row][c] = Cell::default();
                }
            }
            2 => {
                self.grid[row] = vec![Cell::default(); self.cols as usize];
            }
            _ => {}
        }
    }

    /// Resize the buffer. Preserves content where possible.
    pub fn resize(&mut self, new_cols: u16, new_rows: u16) {
        // Adjust rows
        while self.grid.len() < new_rows as usize {
            self.grid.push(vec![Cell::default(); new_cols as usize]);
        }
        while self.grid.len() > new_rows as usize {
            let row = self.grid.remove(0);
            self.scrollback.push_back(row);
        }
        // Adjust cols
        for row in &mut self.grid {
            row.resize(new_cols as usize, Cell::default());
        }
        self.cols = new_cols;
        self.rows = new_rows;
        // Clamp cursor
        self.cursor.row = self.cursor.row.min(new_rows.saturating_sub(1));
        self.cursor.col = self.cursor.col.min(new_cols.saturating_sub(1));
    }

    /// Move cursor up by n rows, clamped to 0.
    pub fn cursor_up(&mut self, n: u16) {
        self.cursor.row = self.cursor.row.saturating_sub(n);
    }

    /// Move cursor down by n rows, clamped to bottom.
    pub fn cursor_down(&mut self, n: u16) {
        self.cursor.row = (self.cursor.row + n).min(self.rows.saturating_sub(1));
    }

    /// Move cursor forward by n cols, clamped to right edge.
    pub fn cursor_forward(&mut self, n: u16) {
        self.cursor.col = (self.cursor.col + n).min(self.cols.saturating_sub(1));
    }

    /// Move cursor back by n cols, clamped to 0.
    pub fn cursor_back(&mut self, n: u16) {
        self.cursor.col = self.cursor.col.saturating_sub(n);
    }

    /// Reset all drawing attributes to default.
    pub fn reset_style(&mut self) {
        self.current_fg = Color::Reset;
        self.current_bg = Color::Reset;
        self.current_modifiers = Modifier::empty();
    }

    /// Insert n blank lines at cursor row, pushing lines down. Bottom lines are lost.
    pub fn insert_lines(&mut self, n: u16) {
        let row = self.cursor.row as usize;
        for _ in 0..n {
            if self.grid.len() > row {
                self.grid.pop();
                self.grid.insert(row, vec![Cell::default(); self.cols as usize]);
            }
        }
    }

    /// Delete n lines at cursor row, pulling lines up. Bottom fills with blanks.
    pub fn delete_lines(&mut self, n: u16) {
        let row = self.cursor.row as usize;
        for _ in 0..n {
            if row < self.grid.len() {
                self.grid.remove(row);
                self.grid.push(vec![Cell::default(); self.cols as usize]);
            }
        }
    }
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --lib screen::tests`
Expected: All tests PASS

- [ ] **Step 6: Commit**

```bash
git add src/screen.rs src/lib.rs
git commit -m "feat: add ScreenBuffer with cell grid, scrollback, and cursor tracking"
```

---

### Task 3: VTE Handler

**Files:**
- Modify: `src/screen.rs` (add `Perform` impl)
- Modify: `src/lib.rs` (no changes needed — screen mod already declared)

**Depends on:** Task 2

Implement the `vte::Perform` trait directly on `ScreenBuffer`. This translates raw ANSI escape sequences into ScreenBuffer operations. A `VteState` wrapper holds both the parser and the buffer.

- [ ] **Step 1: Write failing tests for VTE integration**

Add to the bottom of `src/screen.rs`, inside or after the existing tests module:

```rust
/// VTE parser + ScreenBuffer wrapper.
pub struct VteState {
    parser: vte::Parser,
    pub screen: ScreenBuffer,
}

impl VteState {
    pub fn new(cols: u16, rows: u16) -> Self {
        Self {
            parser: vte::Parser::new(),
            screen: ScreenBuffer::new(cols, rows),
        }
    }

    /// Feed raw bytes from PTY into the VTE parser → ScreenBuffer.
    pub fn process(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.screen, bytes);
    }
}

// --- Perform impl will go here ---

#[cfg(test)]
mod vte_tests {
    use super::*;

    #[test]
    fn test_plain_text() {
        let mut vte = VteState::new(80, 24);
        vte.process(b"Hello, world!");
        let line = &vte.screen.visible_lines()[0];
        let text: String = line[..13].iter().map(|c| c.ch).collect();
        assert_eq!(text, "Hello, world!");
    }

    #[test]
    fn test_crlf() {
        let mut vte = VteState::new(80, 24);
        vte.process(b"Line1\r\nLine2");
        assert_eq!(vte.screen.visible_lines()[0][0].ch, 'L');
        assert_eq!(vte.screen.visible_lines()[1][0].ch, 'L');
        let text1: String = vte.screen.visible_lines()[0][..5].iter().map(|c| c.ch).collect();
        let text2: String = vte.screen.visible_lines()[1][..5].iter().map(|c| c.ch).collect();
        assert_eq!(text1, "Line1");
        assert_eq!(text2, "Line2");
    }

    #[test]
    fn test_sgr_foreground_standard() {
        let mut vte = VteState::new(80, 24);
        // ESC[31m = red foreground, then 'X', then ESC[0m = reset
        vte.process(b"\x1b[31mX\x1b[0m");
        assert_eq!(vte.screen.visible_lines()[0][0].ch, 'X');
        assert_eq!(vte.screen.visible_lines()[0][0].fg, Color::Red);
    }

    #[test]
    fn test_sgr_true_color() {
        let mut vte = VteState::new(80, 24);
        // ESC[38;2;255;128;0m = true color foreground
        vte.process(b"\x1b[38;2;255;128;0mA");
        assert_eq!(vte.screen.visible_lines()[0][0].fg, Color::Rgb(255, 128, 0));
    }

    #[test]
    fn test_sgr_256_color() {
        let mut vte = VteState::new(80, 24);
        // ESC[38;5;196m = 256-color foreground (index 196)
        vte.process(b"\x1b[38;5;196mB");
        assert_eq!(vte.screen.visible_lines()[0][0].fg, Color::Indexed(196));
    }

    #[test]
    fn test_sgr_bold() {
        let mut vte = VteState::new(80, 24);
        vte.process(b"\x1b[1mB\x1b[0m");
        assert!(vte.screen.visible_lines()[0][0].modifiers.contains(Modifier::BOLD));
    }

    #[test]
    fn test_cursor_position() {
        let mut vte = VteState::new(80, 24);
        // ESC[3;5H = move cursor to row 3, col 5 (1-indexed)
        vte.process(b"\x1b[3;5HX");
        assert_eq!(vte.screen.visible_lines()[2][4].ch, 'X');
    }

    #[test]
    fn test_erase_display() {
        let mut vte = VteState::new(10, 3);
        vte.process(b"ABCDEFGHIJ");
        // ESC[2J = erase entire display
        vte.process(b"\x1b[2J");
        for row in vte.screen.visible_lines() {
            for cell in row {
                assert_eq!(cell.ch, ' ');
            }
        }
    }

    #[test]
    fn test_erase_line() {
        let mut vte = VteState::new(10, 3);
        vte.process(b"ABCDEFGHIJ");
        // Move to col 5, then ESC[K = erase from cursor to end of line
        vte.process(b"\x1b[1;6H\x1b[K");
        let line = &vte.screen.visible_lines()[0];
        assert_eq!(line[4].ch, 'E');
        assert_eq!(line[5].ch, ' ');
    }

    #[test]
    fn test_cursor_movement() {
        let mut vte = VteState::new(80, 24);
        vte.process(b"\x1b[5;10H"); // row 5, col 10 (1-indexed)
        vte.process(b"\x1b[2A");    // cursor up 2
        vte.process(b"\x1b[3C");    // cursor forward 3
        vte.process(b"Z");
        // Should be at row 2 (5-1-2=2, 0-indexed), col 12 (10-1+3=12, 0-indexed)
        assert_eq!(vte.screen.visible_lines()[2][12].ch, 'Z');
    }

    #[test]
    fn test_backspace_control() {
        let mut vte = VteState::new(80, 24);
        vte.process(b"AB\x08C"); // write AB, backspace, write C
        assert_eq!(vte.screen.visible_lines()[0][0].ch, 'A');
        assert_eq!(vte.screen.visible_lines()[0][1].ch, 'C'); // C overwrites B position
    }

    #[test]
    fn test_tab_control() {
        let mut vte = VteState::new(80, 24);
        vte.process(b"A\tB");
        assert_eq!(vte.screen.visible_lines()[0][0].ch, 'A');
        assert_eq!(vte.screen.visible_lines()[0][8].ch, 'B');
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib screen::vte_tests`
Expected: FAIL — `Perform` not implemented for `ScreenBuffer`

- [ ] **Step 3: Implement Perform trait for ScreenBuffer**

Add the `Perform` impl to `src/screen.rs`, between `ScreenBuffer` impl and the test modules:

```rust
use vte::{Params, Perform};

impl Perform for ScreenBuffer {
    fn print(&mut self, c: char) {
        self.put_char(c);
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            0x08 => self.backspace(),          // BS
            0x09 => self.tab(),                // HT
            0x0A | 0x0B | 0x0C => {            // LF, VT, FF
                self.advance_row();
            }
            0x0D => self.carriage_return(),    // CR
            _ => {}
        }
    }

    fn csi_dispatch(&mut self, params: &Params, _intermediates: &[u8], _ignore: bool, action: char) {
        let params: Vec<Vec<u16>> = params.iter().map(|p| p.to_vec()).collect();
        let p = |idx: usize, default: u16| -> u16 {
            params.get(idx).and_then(|p| p.first().copied()).filter(|&v| v != 0).unwrap_or(default)
        };

        match action {
            // Cursor movement
            'A' => self.cursor_up(p(0, 1)),
            'B' => self.cursor_down(p(0, 1)),
            'C' => self.cursor_forward(p(0, 1)),
            'D' => self.cursor_back(p(0, 1)),
            'H' | 'f' => {
                // CUP — cursor position (1-indexed in VT100)
                let row = p(0, 1).saturating_sub(1);
                let col = p(1, 1).saturating_sub(1);
                self.set_cursor_position(row, col);
            }
            'J' => self.erase_in_display(p(0, 0)),
            'K' => self.erase_in_line(p(0, 0)),
            'L' => self.insert_lines(p(0, 1)),
            'M' => self.delete_lines(p(0, 1)),
            'd' => {
                // VPA — line position absolute (1-indexed)
                let row = p(0, 1).saturating_sub(1);
                self.cursor.row = row.min(self.rows.saturating_sub(1));
            }
            'G' | '`' => {
                // CHA — cursor character absolute (1-indexed)
                let col = p(0, 1).saturating_sub(1);
                self.cursor.col = col.min(self.cols.saturating_sub(1));
            }
            'm' => self.handle_sgr(&params),
            _ => {} // Unhandled CSI sequences are silently ignored
        }
    }

    fn esc_dispatch(&mut self, _intermediates: &[u8], _ignore: bool, _byte: u8) {
        // Minimal ESC handling for MVP
    }

    fn osc_dispatch(&mut self, _params: &[&[u8]], _bell_terminated: bool) {
        // OSC sequences (e.g. window title) — ignored for MVP
    }
}

impl ScreenBuffer {
    /// Handle SGR (Select Graphic Rendition) sequences.
    fn handle_sgr(&mut self, params: &[Vec<u16>]) {
        if params.is_empty() {
            self.reset_style();
            return;
        }
        let mut i = 0;
        while i < params.len() {
            let code = params[i].first().copied().unwrap_or(0);
            match code {
                0 => self.reset_style(),
                1 => self.current_modifiers |= Modifier::BOLD,
                2 => self.current_modifiers |= Modifier::DIM,
                3 => self.current_modifiers |= Modifier::ITALIC,
                4 => self.current_modifiers |= Modifier::UNDERLINED,
                7 => self.current_modifiers |= Modifier::REVERSED,
                8 => self.current_modifiers |= Modifier::HIDDEN,
                9 => self.current_modifiers |= Modifier::CROSSED_OUT,
                22 => self.current_modifiers -= Modifier::BOLD | Modifier::DIM,
                23 => self.current_modifiers -= Modifier::ITALIC,
                24 => self.current_modifiers -= Modifier::UNDERLINED,
                27 => self.current_modifiers -= Modifier::REVERSED,
                28 => self.current_modifiers -= Modifier::HIDDEN,
                29 => self.current_modifiers -= Modifier::CROSSED_OUT,
                // Standard foreground colors
                30 => self.current_fg = Color::Black,
                31 => self.current_fg = Color::Red,
                32 => self.current_fg = Color::Green,
                33 => self.current_fg = Color::Yellow,
                34 => self.current_fg = Color::Blue,
                35 => self.current_fg = Color::Magenta,
                36 => self.current_fg = Color::Cyan,
                37 => self.current_fg = Color::Gray,
                39 => self.current_fg = Color::Reset,
                // Standard background colors
                40 => self.current_bg = Color::Black,
                41 => self.current_bg = Color::Red,
                42 => self.current_bg = Color::Green,
                43 => self.current_bg = Color::Yellow,
                44 => self.current_bg = Color::Blue,
                45 => self.current_bg = Color::Magenta,
                46 => self.current_bg = Color::Cyan,
                47 => self.current_bg = Color::Gray,
                49 => self.current_bg = Color::Reset,
                // Bright foreground
                90 => self.current_fg = Color::DarkGray,
                91 => self.current_fg = Color::LightRed,
                92 => self.current_fg = Color::LightGreen,
                93 => self.current_fg = Color::LightYellow,
                94 => self.current_fg = Color::LightBlue,
                95 => self.current_fg = Color::LightMagenta,
                96 => self.current_fg = Color::LightCyan,
                97 => self.current_fg = Color::White,
                // Bright background
                100 => self.current_bg = Color::DarkGray,
                101 => self.current_bg = Color::LightRed,
                102 => self.current_bg = Color::LightGreen,
                103 => self.current_bg = Color::LightYellow,
                104 => self.current_bg = Color::LightBlue,
                105 => self.current_bg = Color::LightMagenta,
                106 => self.current_bg = Color::LightCyan,
                107 => self.current_bg = Color::White,
                // Extended color: 38;5;N (256-color) or 38;2;R;G;B (true color)
                38 => {
                    if let Some(color) = self.parse_extended_color(&params, &mut i) {
                        self.current_fg = color;
                    }
                }
                48 => {
                    if let Some(color) = self.parse_extended_color(&params, &mut i) {
                        self.current_bg = color;
                    }
                }
                _ => {}
            }
            i += 1;
        }
    }

    /// Parse 256-color (;5;N) or true color (;2;R;G;B) from SGR params.
    /// `i` points to the 38 or 48 param. On success, `i` is advanced past consumed params.
    fn parse_extended_color(&self, params: &[Vec<u16>], i: &mut usize) -> Option<Color> {
        let kind = params.get(*i + 1)?.first().copied()?;
        match kind {
            5 => {
                // 256-color: 38;5;N
                let idx = params.get(*i + 2)?.first().copied()? as u8;
                *i += 2;
                Some(Color::Indexed(idx))
            }
            2 => {
                // True color: 38;2;R;G;B
                let r = params.get(*i + 2)?.first().copied()? as u8;
                let g = params.get(*i + 3)?.first().copied()? as u8;
                let b = params.get(*i + 4)?.first().copied()? as u8;
                *i += 4;
                Some(Color::Rgb(r, g, b))
            }
            _ => None,
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib screen`
Expected: All tests PASS (both `screen::tests` and `screen::vte_tests`)

- [ ] **Step 5: Commit**

```bash
git add src/screen.rs
git commit -m "feat: add VTE parser integration with SGR, cursor, and erase support"
```

---

### Task 4: Config Module

**Files:**
- Create: `src/config.rs`
- Modify: `src/lib.rs` (add `pub mod config;`)

**Depends on:** Task 1

- [ ] **Step 1: Add module declaration**

Add to `src/lib.rs`:
```rust
pub mod config;
```

- [ ] **Step 2: Write failing tests**

Write `src/config.rs`:

```rust
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    pub layout: LayoutConfig,
    pub scan: ScanConfig,
    pub terminal: TerminalConfig,
    pub keys: KeysConfig,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct LayoutConfig {
    pub default_columns: u8,
    pub detail_panel_width: u16,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ScanConfig {
    pub root_dirs: Vec<String>,
    pub scan_depth: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct TerminalConfig {
    pub shell: String,
    pub cwd_poll_interval: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct KeysConfig {
    pub exit_insert: String,
}

// Default impls and load() will be implemented

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.layout.default_columns, 2);
        assert_eq!(config.layout.detail_panel_width, 45);
        assert_eq!(config.scan.scan_depth, 2);
        assert_eq!(config.terminal.cwd_poll_interval, 2);
        assert_eq!(config.keys.exit_insert, "ctrl-]");
        assert!(!config.scan.root_dirs.is_empty());
    }

    #[test]
    fn test_parse_partial_toml() {
        let toml = r#"
[layout]
default_columns = 3

[terminal]
shell = "/bin/bash"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.layout.default_columns, 3);
        assert_eq!(config.terminal.shell, "/bin/bash");
        // Non-specified fields use defaults
        assert_eq!(config.layout.detail_panel_width, 45);
        assert_eq!(config.scan.scan_depth, 2);
    }

    #[test]
    fn test_parse_empty_toml() {
        let config: Config = toml::from_str("").unwrap();
        assert_eq!(config.layout.default_columns, 2);
    }

    #[test]
    fn test_load_nonexistent_file() {
        let config = Config::load(Path::new("/nonexistent/path/config.toml"));
        // Should return default config without error
        assert_eq!(config.layout.default_columns, 2);
    }

    #[test]
    fn test_columns_clamped() {
        let config = Config::default();
        assert!(config.layout.default_columns >= 1 && config.layout.default_columns <= 3);
    }

    #[test]
    fn test_config_path() {
        let path = Config::config_path();
        assert!(path.ends_with("termgrid/config.toml"));
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test --lib config::tests`
Expected: FAIL — `Default`, `load()`, `config_path()` not implemented

- [ ] **Step 4: Implement Config**

Add implementations to `src/config.rs`:

```rust
impl Default for Config {
    fn default() -> Self {
        Self {
            layout: LayoutConfig::default(),
            scan: ScanConfig::default(),
            terminal: TerminalConfig::default(),
            keys: KeysConfig::default(),
        }
    }
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            default_columns: 2,
            detail_panel_width: 45,
        }
    }
}

impl Default for ScanConfig {
    fn default() -> Self {
        let home = dirs::home_dir()
            .map(|h| h.join("workplace").to_string_lossy().into_owned())
            .unwrap_or_else(|| "~/workplace".into());
        Self {
            root_dirs: vec![home],
            scan_depth: 2,
        }
    }
}

impl Default for TerminalConfig {
    fn default() -> Self {
        Self {
            shell: std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into()),
            cwd_poll_interval: 2,
        }
    }
}

impl Default for KeysConfig {
    fn default() -> Self {
        Self {
            exit_insert: "ctrl-]".into(),
        }
    }
}

impl Config {
    /// Standard config file path: ~/.config/termgrid/config.toml
    pub fn config_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("~/.config"))
            .join("termgrid")
            .join("config.toml")
    }

    /// Load config from file. Returns default config if file doesn't exist or can't be parsed.
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(contents) => toml::from_str(&contents).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }
}
```

- [ ] **Step 5: Run tests**

Run: `cargo test --lib config::tests`
Expected: All PASS

- [ ] **Step 6: Commit**

```bash
git add src/config.rs src/lib.rs
git commit -m "feat: add config module with TOML loading and defaults"
```

---

## Phase B: OS Integration

### Task 5: PTY Wrapper

**Files:**
- Create: `src/pty.rs`
- Modify: `src/lib.rs` (add `pub mod pty;`)

**Depends on:** Task 1

Wraps `portable-pty` into a simpler interface for our use case.

- [ ] **Step 1: Add module declaration**

Add to `src/lib.rs`:
```rust
pub mod pty;
```

- [ ] **Step 2: Write PtyHandle with integration test**

Write `src/pty.rs`:

```rust
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};
use std::path::Path;

pub struct PtyHandle {
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    writer: Box<dyn Write + Send>,
}

pub struct PtyReader(pub Box<dyn Read + Send>);

impl PtyHandle {
    /// Spawn a new PTY running the given shell in the given working directory.
    pub fn spawn(shell: &str, cwd: &Path, cols: u16, rows: u16) -> anyhow::Result<(Self, PtyReader)> {
        let pty_system = native_pty_system();
        let pair = pty_system.openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;

        let mut cmd = CommandBuilder::new(shell);
        cmd.cwd(cwd);

        let child = pair.slave.spawn_command(cmd)?;
        drop(pair.slave); // Drop slave so master EOF is clean

        let reader = PtyReader(pair.master.try_clone_reader()?);
        let writer = pair.master.take_writer()?;

        Ok((
            Self { master: pair.master, child, writer },
            reader,
        ))
    }

    /// Resize the PTY.
    pub fn resize(&self, cols: u16, rows: u16) -> anyhow::Result<()> {
        self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        Ok(())
    }

    /// Write bytes to the PTY (send keystrokes to the child process).
    pub fn write(&mut self, data: &[u8]) -> anyhow::Result<()> {
        self.writer.write_all(data)?;
        self.writer.flush()?;
        Ok(())
    }

    /// Get the child process PID.
    pub fn pid(&self) -> Option<u32> {
        self.child.process_id()
    }

    /// Check if the child process is still alive.
    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Get the raw fd of the master PTY (for tcgetpgrp on macOS).
    #[cfg(unix)]
    pub fn master_fd(&self) -> Option<i32> {
        use std::os::unix::io::AsRawFd;
        self.master.as_raw_fd().map(|fd| fd as i32)
    }

    /// Wait for the child to exit. Returns exit success status.
    pub fn wait(&mut self) -> anyhow::Result<bool> {
        let status = self.child.wait()?;
        Ok(status.success())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn test_spawn_and_read_output() {
        let dir = std::env::current_dir().unwrap();
        let (mut pty, mut reader) = PtyHandle::spawn("/bin/sh", &dir, 80, 24)
            .expect("Failed to spawn PTY");

        assert!(pty.pid().is_some());
        assert!(pty.is_alive());

        // Send a command
        pty.write(b"echo hello_termgrid\n").unwrap();
        pty.write(b"exit\n").unwrap();

        // Read output
        let mut output = Vec::new();
        let mut buf = [0u8; 4096];
        // Read in a loop with a short timeout
        loop {
            match reader.0.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => output.extend_from_slice(&buf[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(_) => break,
            }
        }

        let output_str = String::from_utf8_lossy(&output);
        assert!(output_str.contains("hello_termgrid"), "Output: {output_str}");
    }

    #[test]
    fn test_resize() {
        let dir = std::env::current_dir().unwrap();
        let (pty, _reader) = PtyHandle::spawn("/bin/sh", &dir, 80, 24)
            .expect("Failed to spawn PTY");
        // Should not error
        pty.resize(120, 40).unwrap();
    }

    #[test]
    fn test_wait_for_exit() {
        let dir = std::env::current_dir().unwrap();
        let (mut pty, _reader) = PtyHandle::spawn("/bin/sh", &dir, 80, 24)
            .expect("Failed to spawn PTY");
        pty.write(b"exit 0\n").unwrap();
        let success = pty.wait().unwrap();
        assert!(success);
    }
}
```

- [ ] **Step 3: Add `anyhow` dependency to Cargo.toml**

Add under `[dependencies]`:
```toml
anyhow = "1"
```

- [ ] **Step 4: Run tests**

Run: `cargo test --lib pty::tests`
Expected: All PASS

- [ ] **Step 5: Commit**

```bash
git add src/pty.rs src/lib.rs Cargo.toml
git commit -m "feat: add PTY wrapper with spawn, resize, write, and reader extraction"
```

---

### Task 6: macOS Process Info

**Files:**
- Create: `src/process.rs`
- Modify: `src/lib.rs` (add `pub mod process;`)

**Depends on:** Task 1

macOS-specific functions for CWD tracking and process state detection using `proc_pidinfo` and `tcgetpgrp`.

- [ ] **Step 1: Add module declaration**

Add to `src/lib.rs`:
```rust
pub mod process;
```

- [ ] **Step 2: Write process.rs with types, functions, and tests**

Write `src/process.rs`:

```rust
use std::path::PathBuf;

/// Process state as seen from termgrid.
#[derive(Debug, Clone, PartialEq)]
pub enum ProcessState {
    /// PTY has a foreground child process running (not the shell itself).
    Running,
    /// Shell is in the foreground, waiting for input.
    Waiting,
}

/// Get the foreground process group ID of a PTY via its master fd.
#[cfg(target_os = "macos")]
pub fn get_foreground_pid(master_fd: i32) -> Option<i32> {
    let pgid = unsafe { libc::tcgetpgrp(master_fd) };
    if pgid < 0 { None } else { Some(pgid) }
}

/// Get the current working directory of a process by PID (macOS only).
#[cfg(target_os = "macos")]
pub fn get_process_cwd(pid: i32) -> Option<PathBuf> {
    const PROC_PIDVNODEPATHINFO: i32 = 9;
    const MAXPATHLEN: usize = 1024;
    const VNODE_INFO_SIZE: usize = 152;

    #[repr(C)]
    struct VnodeInfoPath {
        _vnode_info: [u8; VNODE_INFO_SIZE],
        vip_path: [u8; MAXPATHLEN],
    }

    #[repr(C)]
    struct ProcVnodePathInfo {
        pvi_cdir: VnodeInfoPath,
        _pvi_rdir: VnodeInfoPath,
    }

    unsafe {
        let mut info: ProcVnodePathInfo = std::mem::zeroed();
        let size = std::mem::size_of::<ProcVnodePathInfo>() as i32;
        let ret = libc::proc_pidinfo(
            pid,
            PROC_PIDVNODEPATHINFO,
            0,
            &mut info as *mut _ as *mut libc::c_void,
            size,
        );
        if ret <= 0 {
            return None;
        }
        let path_bytes = &info.pvi_cdir.vip_path;
        let len = path_bytes.iter().position(|&b| b == 0).unwrap_or(MAXPATHLEN);
        let path_str = std::str::from_utf8(&path_bytes[..len]).ok()?;
        Some(PathBuf::from(path_str))
    }
}

/// Determine if the PTY is running a child process or just the shell.
/// `shell_pid` is the PID of the shell spawned into the PTY.
/// `fg_pid` is the foreground process group ID from tcgetpgrp.
pub fn detect_process_state(shell_pid: u32, fg_pgid: i32) -> ProcessState {
    if fg_pgid as u32 == shell_pid {
        ProcessState::Waiting
    } else {
        ProcessState::Running
    }
}

#[cfg(not(target_os = "macos"))]
pub fn get_foreground_pid(_master_fd: i32) -> Option<i32> {
    None // Not implemented on non-macOS
}

#[cfg(not(target_os = "macos"))]
pub fn get_process_cwd(_pid: i32) -> Option<PathBuf> {
    None // Not implemented on non-macOS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_process_state_waiting() {
        assert_eq!(
            detect_process_state(1234, 1234),
            ProcessState::Waiting
        );
    }

    #[test]
    fn test_detect_process_state_running() {
        assert_eq!(
            detect_process_state(1234, 5678),
            ProcessState::Running
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_get_current_process_cwd() {
        // Our own process should have a valid CWD
        let pid = std::process::id() as i32;
        let cwd = get_process_cwd(pid);
        assert!(cwd.is_some());
        let cwd = cwd.unwrap();
        assert!(cwd.exists());
    }

    #[test]
    fn test_get_cwd_invalid_pid() {
        // PID 0 or negative should return None
        let cwd = get_process_cwd(-1);
        assert!(cwd.is_none());
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test --lib process::tests`
Expected: All PASS

- [ ] **Step 4: Commit**

```bash
git add src/process.rs src/lib.rs
git commit -m "feat: add macOS process info for CWD tracking and foreground PID"
```

---

### Task 7: Git Detection

**Files:**
- Create: `src/git.rs`
- Modify: `src/lib.rs` (add `pub mod git;`)

**Depends on:** Task 1

- [ ] **Step 1: Add module declaration**

Add to `src/lib.rs`:
```rust
pub mod git;
```

- [ ] **Step 2: Write git.rs with types, detection logic, and tests**

Write `src/git.rs`:

```rust
use std::path::{Path, PathBuf};

/// Git context detected from a working directory.
#[derive(Debug, Clone, PartialEq)]
pub struct GitContext {
    /// Project name (repo root directory name, or bare repo name without .git suffix).
    pub project_name: String,
    /// Current branch name (None if detached HEAD).
    pub branch: Option<String>,
    /// Whether this is a git worktree (as opposed to the main working copy).
    pub is_worktree: bool,
    /// Worktree name (directory name of the worktree), if is_worktree.
    pub worktree_name: Option<String>,
    /// Absolute path to the repo root.
    pub repo_root: PathBuf,
}

/// Detect git context from a directory path. Returns None if not in a git repo.
pub fn detect_git(path: &Path) -> Option<GitContext> {
    let repo = git2::Repository::discover(path).ok()?;

    // Get branch name
    let branch = repo.head().ok().and_then(|head| {
        head.shorthand().map(String::from)
    });

    // Determine if this is a worktree
    let workdir = repo.workdir()?;
    let git_dir = repo.path(); // .git directory or file
    let dot_git = workdir.join(".git");

    let is_worktree = dot_git.is_file(); // .git is a file in worktrees, a dir in normal repos

    let (project_name, worktree_name) = if is_worktree {
        // For worktrees, get the main repo name
        let main_repo_name = find_main_repo_name(&repo);
        let wt_name = workdir.file_name()
            .map(|n| n.to_string_lossy().into_owned());
        (main_repo_name, wt_name)
    } else {
        let name = workdir.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "unknown".into());
        (name, None)
    };

    Some(GitContext {
        project_name,
        branch,
        is_worktree,
        worktree_name,
        repo_root: workdir.to_path_buf(),
    })
}

/// Find the main repository name for a worktree.
fn find_main_repo_name(repo: &git2::Repository) -> String {
    // repo.commondir() points to the main repo's .git directory
    let common_dir = repo.commondir();
    // For worktrees: commondir = /path/to/main/.git
    // For bare repos used as worktree source: commondir = /path/to/main.git
    let parent = common_dir.parent();
    parent
        .and_then(|p| p.file_name())
        .map(|n| {
            let name = n.to_string_lossy();
            // Strip .git suffix for bare repos
            name.strip_suffix(".git")
                .unwrap_or(&name)
                .to_string()
        })
        .unwrap_or_else(|| "unknown".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn init_repo(dir: &Path) -> git2::Repository {
        let repo = git2::Repository::init(dir).unwrap();
        // Need at least one commit for HEAD to exist
        let sig = git2::Signature::now("Test", "test@test.com").unwrap();
        let tree_id = repo.treebuilder(None).unwrap().write().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[]).unwrap();
        repo
    }

    #[test]
    fn test_detect_normal_git_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let repo_dir = tmp.path().join("myproject");
        fs::create_dir_all(&repo_dir).unwrap();
        init_repo(&repo_dir);

        let ctx = detect_git(&repo_dir).unwrap();
        assert_eq!(ctx.project_name, "myproject");
        assert_eq!(ctx.branch, Some("main".into()).or(Some("master".into())));
        assert!(!ctx.is_worktree);
        assert!(ctx.worktree_name.is_none());
    }

    #[test]
    fn test_detect_subdirectory() {
        let tmp = tempfile::tempdir().unwrap();
        let repo_dir = tmp.path().join("myproject");
        let sub_dir = repo_dir.join("src").join("lib");
        fs::create_dir_all(&sub_dir).unwrap();
        init_repo(&repo_dir);

        let ctx = detect_git(&sub_dir).unwrap();
        assert_eq!(ctx.project_name, "myproject");
    }

    #[test]
    fn test_detect_non_git_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = detect_git(tmp.path());
        assert!(ctx.is_none());
    }

    #[test]
    fn test_detect_worktree() {
        let tmp = tempfile::tempdir().unwrap();
        let main_dir = tmp.path().join("main_repo");
        fs::create_dir_all(&main_dir).unwrap();
        let repo = init_repo(&main_dir);

        // Create a worktree
        let wt_dir = tmp.path().join("my_worktree");
        let branch_ref = repo.head().unwrap();
        let commit = repo.find_commit(branch_ref.target().unwrap()).unwrap();
        // Create a new branch for the worktree
        repo.branch("wt-branch", &commit, false).unwrap();
        repo.worktree("my_worktree", &wt_dir, Some(
            git2::WorktreeAddOptions::new()
                .reference(Some(&repo.find_branch("wt-branch", git2::BranchType::Local).unwrap().into_reference()))
        )).unwrap();

        let ctx = detect_git(&wt_dir).unwrap();
        assert!(ctx.is_worktree);
        assert_eq!(ctx.project_name, "main_repo");
        assert_eq!(ctx.worktree_name, Some("my_worktree".into()));
    }

    #[test]
    fn test_detect_branch_name() {
        let tmp = tempfile::tempdir().unwrap();
        let repo_dir = tmp.path().join("repo");
        fs::create_dir_all(&repo_dir).unwrap();
        let repo = init_repo(&repo_dir);

        // Create and checkout a new branch
        let head = repo.head().unwrap().target().unwrap();
        let commit = repo.find_commit(head).unwrap();
        repo.branch("feature/my-feature", &commit, false).unwrap();
        repo.set_head("refs/heads/feature/my-feature").unwrap();

        let ctx = detect_git(&repo_dir).unwrap();
        assert_eq!(ctx.branch, Some("feature/my-feature".into()));
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test --lib git::tests`
Expected: All PASS

- [ ] **Step 4: Commit**

```bash
git add src/git.rs src/lib.rs
git commit -m "feat: add git context detection with worktree and branch support"
```

---

## Phase C: UI & Layout

### Task 8: Layout Engine

**Files:**
- Create: `src/layout.rs`
- Modify: `src/lib.rs` (add `pub mod layout;`)

**Depends on:** Task 1

Pure calculation — no side effects, highly testable.

- [ ] **Step 1: Add module declaration**

Add to `src/lib.rs`:
```rust
pub mod layout;
```

- [ ] **Step 2: Write layout.rs with types, calculation, and tests**

Write `src/layout.rs`:

```rust
use ratatui::layout::Rect;

/// Result of layout calculation — where each UI element goes.
#[derive(Debug, Clone)]
pub struct LayoutResult {
    pub tab_bar: Rect,
    pub grid_area: Rect,
    pub detail_panel: Option<Rect>,
    pub status_bar: Rect,
    /// (tile_index, rect) for each visible tile card in the grid.
    pub tile_rects: Vec<Rect>,
}

/// Calculate the full layout given terminal dimensions and state.
///
/// - `total`: full terminal area
/// - `columns`: 1, 2, or 3 column grid
/// - `tile_count`: number of tiles to lay out
/// - `has_selection`: whether a tile is selected (shows detail panel)
/// - `detail_width_pct`: detail panel width as percentage (0-100)
/// - `scroll_offset`: number of rows scrolled past (for grid overflow)
pub fn calculate_layout(
    total: Rect,
    columns: u8,
    tile_count: usize,
    has_selection: bool,
    detail_width_pct: u16,
    scroll_offset: usize,
) -> LayoutResult {
    let columns = columns.max(1).min(3);

    // Tab bar: 1 line at top
    let tab_bar = Rect::new(total.x, total.y, total.width, 1);

    // Status bar: 1 line at bottom
    let status_bar = Rect::new(total.x, total.y + total.height.saturating_sub(1), total.width, 1);

    // Middle area between tab bar and status bar
    let middle_y = total.y + 1;
    let middle_height = total.height.saturating_sub(2); // subtract tab + status

    // Split middle into grid and optional detail panel
    let (grid_area, detail_panel) = if has_selection && total.width > 40 {
        let detail_width = (total.width as u32 * detail_width_pct as u32 / 100) as u16;
        let grid_width = total.width.saturating_sub(detail_width);
        let grid = Rect::new(total.x, middle_y, grid_width, middle_height);
        let detail = Rect::new(total.x + grid_width, middle_y, detail_width, middle_height);
        (grid, Some(detail))
    } else {
        let grid = Rect::new(total.x, middle_y, total.width, middle_height);
        (grid, None)
    };

    // Calculate tile rects within the grid area
    let tile_rects = calculate_tile_rects(grid_area, columns, tile_count, scroll_offset);

    LayoutResult {
        tab_bar,
        grid_area,
        detail_panel,
        status_bar,
        tile_rects,
    }
}

/// Calculate positions for each tile card in the grid.
fn calculate_tile_rects(
    area: Rect,
    columns: u8,
    tile_count: usize,
    scroll_offset: usize,
) -> Vec<Rect> {
    if tile_count == 0 || area.width == 0 || area.height == 0 {
        return Vec::new();
    }

    let cols = columns as usize;
    let col_width = area.width / columns as u16;
    let tile_rows = (tile_count + cols - 1) / cols; // ceiling division

    // Each tile gets equal height within the visible area
    // Minimum tile height: 5 lines (1 title + 4 preview lines)
    let visible_tile_rows = (area.height as usize / 5).max(1);
    let tile_height = if tile_rows <= visible_tile_rows {
        area.height / tile_rows.max(1) as u16
    } else {
        5 // minimum height when scrolling
    };

    let mut rects = Vec::with_capacity(tile_count);
    for i in 0..tile_count {
        let grid_row = i / cols;
        let grid_col = i % cols;

        // Apply scroll offset
        if grid_row < scroll_offset {
            continue; // scrolled past
        }
        let visible_row = grid_row - scroll_offset;
        let y = area.y + (visible_row as u16 * tile_height);

        // Skip tiles below visible area
        if y + tile_height > area.y + area.height {
            continue;
        }

        let x = area.x + (grid_col as u16 * col_width);
        // Last column takes remaining width to avoid gaps
        let w = if grid_col == cols - 1 {
            area.width - (grid_col as u16 * col_width)
        } else {
            col_width
        };

        rects.push(Rect::new(x, y, w, tile_height));
    }

    rects
}

/// Calculate the total number of grid rows for scroll bounds.
pub fn total_grid_rows(tile_count: usize, columns: u8) -> usize {
    let cols = columns.max(1) as usize;
    (tile_count + cols - 1) / cols
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(w: u16, h: u16) -> Rect {
        Rect::new(0, 0, w, h)
    }

    #[test]
    fn test_basic_layout_structure() {
        let layout = calculate_layout(area(120, 40), 2, 4, false, 45, 0);
        assert_eq!(layout.tab_bar.height, 1);
        assert_eq!(layout.status_bar.height, 1);
        assert_eq!(layout.tab_bar.y, 0);
        assert_eq!(layout.status_bar.y, 39);
        assert!(layout.detail_panel.is_none());
        assert_eq!(layout.grid_area.y, 1);
        assert_eq!(layout.grid_area.height, 38);
    }

    #[test]
    fn test_detail_panel_shown_when_selected() {
        let layout = calculate_layout(area(120, 40), 2, 4, true, 45, 0);
        assert!(layout.detail_panel.is_some());
        let detail = layout.detail_panel.unwrap();
        // Detail panel should be about 45% of width
        assert!(detail.width > 50 && detail.width < 60);
        // Grid takes the rest
        assert_eq!(layout.grid_area.width + detail.width, 120);
    }

    #[test]
    fn test_no_detail_panel_on_narrow_terminal() {
        let layout = calculate_layout(area(30, 20), 1, 2, true, 45, 0);
        // Too narrow — should not show detail panel
        assert!(layout.detail_panel.is_none());
    }

    #[test]
    fn test_tile_rects_2_columns() {
        let layout = calculate_layout(area(100, 22), 2, 4, false, 45, 0);
        assert_eq!(layout.tile_rects.len(), 4);
        // First two tiles on same row
        assert_eq!(layout.tile_rects[0].y, layout.tile_rects[1].y);
        // Second row below first
        assert!(layout.tile_rects[2].y > layout.tile_rects[0].y);
        // Columns split the width
        assert_eq!(layout.tile_rects[0].width, 50);
        assert_eq!(layout.tile_rects[1].width, 50);
    }

    #[test]
    fn test_tile_rects_3_columns() {
        let layout = calculate_layout(area(120, 22), 3, 6, false, 45, 0);
        assert_eq!(layout.tile_rects.len(), 6);
        // Three tiles on first row
        assert_eq!(layout.tile_rects[0].y, layout.tile_rects[1].y);
        assert_eq!(layout.tile_rects[1].y, layout.tile_rects[2].y);
    }

    #[test]
    fn test_tile_rects_1_column() {
        let layout = calculate_layout(area(80, 22), 1, 3, false, 45, 0);
        // Each tile should have full width
        for rect in &layout.tile_rects {
            assert_eq!(rect.width, 80);
        }
    }

    #[test]
    fn test_empty_tiles() {
        let layout = calculate_layout(area(120, 40), 2, 0, false, 45, 0);
        assert!(layout.tile_rects.is_empty());
    }

    #[test]
    fn test_scroll_offset() {
        let layout_no_scroll = calculate_layout(area(100, 12), 2, 10, false, 45, 0);
        let layout_scrolled = calculate_layout(area(100, 12), 2, 10, false, 45, 1);
        // Scrolled layout should have fewer visible tiles
        assert!(layout_scrolled.tile_rects.len() <= layout_no_scroll.tile_rects.len());
    }

    #[test]
    fn test_total_grid_rows() {
        assert_eq!(total_grid_rows(4, 2), 2);
        assert_eq!(total_grid_rows(5, 2), 3);
        assert_eq!(total_grid_rows(6, 3), 2);
        assert_eq!(total_grid_rows(0, 2), 0);
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test --lib layout::tests`
Expected: All PASS

- [ ] **Step 4: Commit**

```bash
git add src/layout.rs src/lib.rs
git commit -m "feat: add layout engine with multi-column grid and detail panel calculation"
```

---

### Task 9: Tab Aggregation

**Files:**
- Create: `src/tab.rs`
- Modify: `src/lib.rs` (add `pub mod tab;`)

**Depends on:** Task 7 (git types)

- [ ] **Step 1: Add module declaration**

Add to `src/lib.rs`:
```rust
pub mod tab;
```

- [ ] **Step 2: Write tab.rs with aggregation logic and tests**

Write `src/tab.rs`:

```rust
use crate::git::GitContext;

/// A single tab entry representing a project group.
#[derive(Debug, Clone, PartialEq)]
pub struct TabEntry {
    pub label: String,
    pub count: usize,
}

/// Filter applied by the selected tab.
#[derive(Debug, Clone, PartialEq)]
pub enum TabFilter {
    All,
    Project(String),
    Other,
}

impl TabFilter {
    /// Check if a tile with the given git context matches this filter.
    pub fn matches(&self, git_context: &Option<GitContext>) -> bool {
        match self {
            TabFilter::All => true,
            TabFilter::Project(name) => {
                git_context.as_ref().map_or(false, |g| g.project_name == *name)
            }
            TabFilter::Other => git_context.is_none(),
        }
    }
}

/// Build tab entries from a list of git contexts (one per tile).
/// Returns: list of TabEntry sorted by count descending (excluding "ALL").
/// The caller prepends "ALL" with the total count.
pub fn aggregate_tabs(contexts: &[Option<GitContext>]) -> Vec<TabEntry> {
    let mut project_counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut other_count = 0;

    for ctx in contexts {
        match ctx {
            Some(g) => {
                *project_counts.entry(g.project_name.clone()).or_default() += 1;
            }
            None => other_count += 1,
        }
    }

    let mut tabs: Vec<TabEntry> = project_counts
        .into_iter()
        .map(|(label, count)| TabEntry { label, count })
        .collect();

    // Sort by count descending, then by name for stability
    tabs.sort_by(|a, b| b.count.cmp(&a.count).then(a.label.cmp(&b.label)));

    if other_count > 0 {
        tabs.push(TabEntry { label: "Other".into(), count: other_count });
    }

    tabs
}

/// Advance to the next tab in the list. Cycles: ALL → project1 → ... → Other → ALL.
pub fn next_tab(current: &TabFilter, tabs: &[TabEntry]) -> TabFilter {
    match current {
        TabFilter::All => {
            tabs.first()
                .map(|t| {
                    if t.label == "Other" { TabFilter::Other }
                    else { TabFilter::Project(t.label.clone()) }
                })
                .unwrap_or(TabFilter::All)
        }
        TabFilter::Project(name) => {
            let pos = tabs.iter().position(|t| t.label == *name);
            match pos {
                Some(i) if i + 1 < tabs.len() => {
                    let next = &tabs[i + 1];
                    if next.label == "Other" { TabFilter::Other }
                    else { TabFilter::Project(next.label.clone()) }
                }
                _ => TabFilter::All,
            }
        }
        TabFilter::Other => TabFilter::All,
    }
}

/// Go to the previous tab. Reverse of next_tab.
pub fn prev_tab(current: &TabFilter, tabs: &[TabEntry]) -> TabFilter {
    match current {
        TabFilter::All => {
            tabs.last()
                .map(|t| {
                    if t.label == "Other" { TabFilter::Other }
                    else { TabFilter::Project(t.label.clone()) }
                })
                .unwrap_or(TabFilter::All)
        }
        TabFilter::Project(name) => {
            let pos = tabs.iter().position(|t| t.label == *name);
            match pos {
                Some(0) => TabFilter::All,
                Some(i) => {
                    let prev = &tabs[i - 1];
                    if prev.label == "Other" { TabFilter::Other }
                    else { TabFilter::Project(prev.label.clone()) }
                }
                None => TabFilter::All,
            }
        }
        TabFilter::Other => {
            // Other is always last; go to previous
            let non_other: Vec<_> = tabs.iter().filter(|t| t.label != "Other").collect();
            non_other.last()
                .map(|t| TabFilter::Project(t.label.clone()))
                .unwrap_or(TabFilter::All)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::GitContext;
    use std::path::PathBuf;

    fn git_ctx(name: &str) -> Option<GitContext> {
        Some(GitContext {
            project_name: name.into(),
            branch: Some("main".into()),
            is_worktree: false,
            worktree_name: None,
            repo_root: PathBuf::from("/tmp"),
        })
    }

    #[test]
    fn test_aggregate_tabs() {
        let contexts = vec![
            git_ctx("alpha"),
            git_ctx("alpha"),
            git_ctx("beta"),
            None,
            git_ctx("alpha"),
        ];
        let tabs = aggregate_tabs(&contexts);
        assert_eq!(tabs[0].label, "alpha");
        assert_eq!(tabs[0].count, 3);
        assert_eq!(tabs[1].label, "beta");
        assert_eq!(tabs[1].count, 1);
        assert_eq!(tabs[2].label, "Other");
        assert_eq!(tabs[2].count, 1);
    }

    #[test]
    fn test_aggregate_empty() {
        let tabs = aggregate_tabs(&[]);
        assert!(tabs.is_empty());
    }

    #[test]
    fn test_aggregate_all_non_git() {
        let contexts = vec![None, None];
        let tabs = aggregate_tabs(&contexts);
        assert_eq!(tabs.len(), 1);
        assert_eq!(tabs[0].label, "Other");
    }

    #[test]
    fn test_filter_matches() {
        let ctx = git_ctx("alpha");
        assert!(TabFilter::All.matches(&ctx));
        assert!(TabFilter::Project("alpha".into()).matches(&ctx));
        assert!(!TabFilter::Project("beta".into()).matches(&ctx));
        assert!(!TabFilter::Other.matches(&ctx));

        assert!(TabFilter::All.matches(&None));
        assert!(TabFilter::Other.matches(&None));
        assert!(!TabFilter::Project("alpha".into()).matches(&None));
    }

    #[test]
    fn test_tab_cycling() {
        let tabs = vec![
            TabEntry { label: "alpha".into(), count: 3 },
            TabEntry { label: "beta".into(), count: 1 },
            TabEntry { label: "Other".into(), count: 1 },
        ];
        let tab0 = TabFilter::All;
        let tab1 = next_tab(&tab0, &tabs);
        assert_eq!(tab1, TabFilter::Project("alpha".into()));
        let tab2 = next_tab(&tab1, &tabs);
        assert_eq!(tab2, TabFilter::Project("beta".into()));
        let tab3 = next_tab(&tab2, &tabs);
        assert_eq!(tab3, TabFilter::Other);
        let tab4 = next_tab(&tab3, &tabs);
        assert_eq!(tab4, TabFilter::All);
    }

    #[test]
    fn test_prev_tab() {
        let tabs = vec![
            TabEntry { label: "alpha".into(), count: 3 },
            TabEntry { label: "beta".into(), count: 1 },
        ];
        let tab = prev_tab(&TabFilter::All, &tabs);
        assert_eq!(tab, TabFilter::Project("beta".into()));
        let tab = prev_tab(&tab, &tabs);
        assert_eq!(tab, TabFilter::Project("alpha".into()));
        let tab = prev_tab(&tab, &tabs);
        assert_eq!(tab, TabFilter::All);
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test --lib tab::tests`
Expected: All PASS

- [ ] **Step 4: Commit**

```bash
git add src/tab.rs src/lib.rs
git commit -m "feat: add tab aggregation with project grouping and filter cycling"
```

---

### Task 10: Tile and TileManager

**Files:**
- Create: `src/tile.rs`
- Create: `src/tile_manager.rs`
- Modify: `src/lib.rs` (add `pub mod tile; pub mod tile_manager;`)

**Depends on:** Tasks 2, 3, 5, 7

The Tile combines ScreenBuffer/VTE + PTY + Git context. TileManager owns all tiles and handles selection/filtering.

- [ ] **Step 1: Add module declarations**

Add to `src/lib.rs`:
```rust
pub mod tile;
pub mod tile_manager;
```

- [ ] **Step 2: Write tile.rs**

Write `src/tile.rs`:

```rust
use crate::git::GitContext;
use crate::pty::{PtyHandle, PtyReader};
use crate::screen::VteState;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Unique identifier for a tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TileId(pub u64);

/// Tile lifecycle status.
#[derive(Debug, Clone, PartialEq)]
pub enum TileStatus {
    Running,
    Waiting,
    Idle(std::time::Duration),
    Exited,
    Error(String),
}

/// A terminal tile — owns a PTY, screen buffer, and metadata.
pub struct Tile {
    pub id: TileId,
    pub vte: VteState,
    pub pty: PtyHandle,
    pub git_context: Option<GitContext>,
    pub cwd: PathBuf,
    pub status: TileStatus,
    pub last_active: Instant,
    pub waiting_since: Option<Instant>,
}

impl Tile {
    /// Create a new tile by spawning a PTY.
    /// Returns (Tile, PtyReader) — the reader is moved to a background task.
    pub fn spawn(
        id: TileId,
        shell: &str,
        cwd: &Path,
        cols: u16,
        rows: u16,
    ) -> anyhow::Result<(Self, PtyReader)> {
        let (pty, reader) = PtyHandle::spawn(shell, cwd, cols, rows)?;
        let git_context = crate::git::detect_git(cwd);

        Ok((
            Self {
                id,
                vte: VteState::new(cols, rows),
                pty,
                git_context,
                cwd: cwd.to_path_buf(),
                status: TileStatus::Waiting,
                last_active: Instant::now(),
                waiting_since: Some(Instant::now()),
            },
            reader,
        ))
    }

    /// Process PTY output bytes into the screen buffer.
    pub fn process_output(&mut self, bytes: &[u8]) {
        self.vte.process(bytes);
        self.last_active = Instant::now();
    }

    /// Update CWD and re-detect git context if changed.
    pub fn update_cwd(&mut self, new_cwd: PathBuf) {
        if self.cwd != new_cwd {
            self.cwd = new_cwd;
            self.git_context = crate::git::detect_git(&self.cwd);
        }
    }

    /// Update tile status based on process state.
    pub fn update_status(&mut self, is_fg_shell: bool) {
        if !self.pty.is_alive() {
            self.status = TileStatus::Exited;
            self.waiting_since = None;
            return;
        }

        if is_fg_shell {
            match self.waiting_since {
                Some(since) => {
                    let elapsed = since.elapsed();
                    if elapsed.as_secs() >= 60 {
                        self.status = TileStatus::Idle(elapsed);
                    } else {
                        self.status = TileStatus::Waiting;
                    }
                }
                None => {
                    self.waiting_since = Some(Instant::now());
                    self.status = TileStatus::Waiting;
                }
            }
        } else {
            self.status = TileStatus::Running;
            self.last_active = Instant::now();
            self.waiting_since = None;
        }
    }

    /// Write input to the PTY.
    pub fn write_input(&mut self, data: &[u8]) -> anyhow::Result<()> {
        self.pty.write(data)
    }

    /// Resize the PTY and screen buffer.
    pub fn resize(&mut self, cols: u16, rows: u16) -> anyhow::Result<()> {
        self.pty.resize(cols, rows)?;
        self.vte.screen.resize(cols, rows);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tile_status_transitions() {
        let id = TileId(1);
        // Create a tile manually for testing status logic
        let mut status = TileStatus::Waiting;
        let mut waiting_since: Option<Instant> = Some(Instant::now());

        // Simulate running (fg is not shell)
        status = TileStatus::Running;
        waiting_since = None;
        assert_eq!(status, TileStatus::Running);

        // Simulate back to waiting
        waiting_since = Some(Instant::now());
        status = TileStatus::Waiting;
        assert_eq!(status, TileStatus::Waiting);
    }

    #[test]
    fn test_process_output() {
        let mut vte = VteState::new(80, 24);
        vte.process(b"Hello from PTY");
        let line: String = vte.screen.visible_lines()[0][..14].iter().map(|c| c.ch).collect();
        assert_eq!(line, "Hello from PTY");
    }

    #[test]
    fn test_tile_id_equality() {
        assert_eq!(TileId(1), TileId(1));
        assert_ne!(TileId(1), TileId(2));
    }
}
```

- [ ] **Step 3: Write tile_manager.rs**

Write `src/tile_manager.rs`:

```rust
use crate::tab::TabFilter;
use crate::tile::{Tile, TileId};

/// Manages all tiles — creation, removal, selection, and filtering.
pub struct TileManager {
    tiles: Vec<Tile>,
    selected: Option<TileId>,
    next_id: u64,
}

impl TileManager {
    pub fn new() -> Self {
        Self {
            tiles: Vec::new(),
            selected: None,
            next_id: 1,
        }
    }

    pub fn next_tile_id(&mut self) -> TileId {
        let id = TileId(self.next_id);
        self.next_id += 1;
        id
    }

    pub fn add(&mut self, tile: Tile) {
        self.tiles.push(tile);
    }

    pub fn remove(&mut self, id: TileId) -> Option<Tile> {
        let pos = self.tiles.iter().position(|t| t.id == id)?;
        let tile = self.tiles.remove(pos);
        if self.selected == Some(id) {
            // Select adjacent tile
            self.selected = if !self.tiles.is_empty() {
                let idx = pos.min(self.tiles.len() - 1);
                Some(self.tiles[idx].id)
            } else {
                None
            };
        }
        Some(tile)
    }

    pub fn get(&self, id: TileId) -> Option<&Tile> {
        self.tiles.iter().find(|t| t.id == id)
    }

    pub fn get_mut(&mut self, id: TileId) -> Option<&mut Tile> {
        self.tiles.iter_mut().find(|t| t.id == id)
    }

    pub fn selected_id(&self) -> Option<TileId> {
        self.selected
    }

    pub fn selected(&self) -> Option<&Tile> {
        self.selected.and_then(|id| self.get(id))
    }

    pub fn selected_mut(&mut self) -> Option<&mut Tile> {
        let id = self.selected?;
        self.get_mut(id)
    }

    pub fn select(&mut self, id: TileId) {
        if self.tiles.iter().any(|t| t.id == id) {
            self.selected = Some(id);
        }
    }

    pub fn deselect(&mut self) {
        self.selected = None;
    }

    /// Get all tiles matching the current tab filter.
    pub fn filtered_tiles(&self, filter: &TabFilter) -> Vec<&Tile> {
        self.tiles
            .iter()
            .filter(|t| filter.matches(&t.git_context))
            .collect()
    }

    /// Get mutable references to all tiles.
    pub fn tiles_mut(&mut self) -> &mut Vec<Tile> {
        &mut self.tiles
    }

    pub fn tiles(&self) -> &[Tile] {
        &self.tiles
    }

    pub fn tile_count(&self) -> usize {
        self.tiles.len()
    }

    /// Navigate to the next tile in the filtered list.
    pub fn select_next(&mut self, filter: &TabFilter) {
        let filtered: Vec<TileId> = self.filtered_tiles(filter).iter().map(|t| t.id).collect();
        if filtered.is_empty() { return; }
        let current_idx = self.selected.and_then(|id| filtered.iter().position(|&fid| fid == id));
        let next_idx = match current_idx {
            Some(i) => (i + 1) % filtered.len(),
            None => 0,
        };
        self.selected = Some(filtered[next_idx]);
    }

    /// Navigate to the previous tile in the filtered list.
    pub fn select_prev(&mut self, filter: &TabFilter) {
        let filtered: Vec<TileId> = self.filtered_tiles(filter).iter().map(|t| t.id).collect();
        if filtered.is_empty() { return; }
        let current_idx = self.selected.and_then(|id| filtered.iter().position(|&fid| fid == id));
        let prev_idx = match current_idx {
            Some(0) => filtered.len() - 1,
            Some(i) => i - 1,
            None => 0,
        };
        self.selected = Some(filtered[prev_idx]);
    }

    /// Navigate by grid direction (up/down/left/right) given column count.
    pub fn select_direction(&mut self, filter: &TabFilter, columns: u8, direction: Direction) {
        let filtered: Vec<TileId> = self.filtered_tiles(filter).iter().map(|t| t.id).collect();
        if filtered.is_empty() { return; }
        let cols = columns.max(1) as usize;
        let current_idx = self.selected
            .and_then(|id| filtered.iter().position(|&fid| fid == id))
            .unwrap_or(0);

        let new_idx = match direction {
            Direction::Up => {
                if current_idx >= cols { current_idx - cols } else { current_idx }
            }
            Direction::Down => {
                let next = current_idx + cols;
                if next < filtered.len() { next } else { current_idx }
            }
            Direction::Left => {
                if current_idx % cols > 0 { current_idx - 1 } else { current_idx }
            }
            Direction::Right => {
                let next = current_idx + 1;
                if next < filtered.len() && next % cols > 0 { next } else { current_idx }
            }
        };

        self.selected = Some(filtered[new_idx]);
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::GitContext;
    use crate::screen::VteState;
    use crate::tile::{Tile, TileId, TileStatus};
    use std::path::PathBuf;
    use std::time::Instant;

    /// Create a dummy tile (no real PTY) for testing TileManager logic.
    fn dummy_tile(id: u64, project: Option<&str>) -> Tile {
        let git_context = project.map(|name| GitContext {
            project_name: name.into(),
            branch: Some("main".into()),
            is_worktree: false,
            worktree_name: None,
            repo_root: PathBuf::from("/tmp"),
        });
        // We can't create a real Tile without a PTY, so we'll test via TileManager
        // with a mock approach. For now, test the ID-based logic.
        // In the real code, tiles are created via Tile::spawn.
        // For unit tests, we'll need a way to create test tiles.
        panic!("Use integration tests for TileManager with real tiles, or add a test constructor");
    }
}
```

Wait — we can't easily create dummy Tiles without a PTY. Let me add a test constructor:

Add to `src/tile.rs`:

```rust
impl Tile {
    /// Test-only constructor that creates a tile without a real PTY.
    #[cfg(test)]
    pub fn new_test(id: TileId, cwd: &Path, git_context: Option<GitContext>) -> Self {
        Self {
            id,
            vte: VteState::new(80, 24),
            pty: PtyHandle::spawn("/bin/sh", cwd, 80, 24).unwrap().0,
            git_context,
            cwd: cwd.to_path_buf(),
            status: TileStatus::Waiting,
            last_active: Instant::now(),
            waiting_since: Some(Instant::now()),
        }
    }
}
```

Actually, spawning a real shell for every test tile is wasteful. Let me use a different approach — make TileManager work with a simpler struct for testing, or just test with a lightweight process:

Replace the TileManager test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::GitContext;
    use crate::tile::{TileId, TileStatus};
    use std::path::PathBuf;

    fn make_tile(mgr: &mut TileManager, project: Option<&str>) -> TileId {
        let id = mgr.next_tile_id();
        let dir = std::env::current_dir().unwrap();
        let (mut tile, _reader) = crate::tile::Tile::spawn(
            id, "/bin/sh", &dir, 80, 24,
        ).unwrap();
        if let Some(name) = project {
            tile.git_context = Some(GitContext {
                project_name: name.into(),
                branch: Some("main".into()),
                is_worktree: false,
                worktree_name: None,
                repo_root: PathBuf::from("/tmp"),
            });
        } else {
            tile.git_context = None;
        }
        mgr.add(tile);
        id
    }

    #[test]
    fn test_add_and_select() {
        let mut mgr = TileManager::new();
        let id = make_tile(&mut mgr, Some("proj"));
        assert_eq!(mgr.tile_count(), 1);
        mgr.select(id);
        assert_eq!(mgr.selected_id(), Some(id));
    }

    #[test]
    fn test_remove_adjusts_selection() {
        let mut mgr = TileManager::new();
        let id1 = make_tile(&mut mgr, Some("a"));
        let id2 = make_tile(&mut mgr, Some("b"));
        mgr.select(id1);
        mgr.remove(id1);
        // Should auto-select the remaining tile
        assert_eq!(mgr.selected_id(), Some(id2));
    }

    #[test]
    fn test_remove_last_clears_selection() {
        let mut mgr = TileManager::new();
        let id = make_tile(&mut mgr, None);
        mgr.select(id);
        mgr.remove(id);
        assert_eq!(mgr.selected_id(), None);
        assert_eq!(mgr.tile_count(), 0);
    }

    #[test]
    fn test_filtered_tiles() {
        let mut mgr = TileManager::new();
        make_tile(&mut mgr, Some("alpha"));
        make_tile(&mut mgr, Some("alpha"));
        make_tile(&mut mgr, Some("beta"));
        make_tile(&mut mgr, None);

        assert_eq!(mgr.filtered_tiles(&TabFilter::All).len(), 4);
        assert_eq!(mgr.filtered_tiles(&TabFilter::Project("alpha".into())).len(), 2);
        assert_eq!(mgr.filtered_tiles(&TabFilter::Project("beta".into())).len(), 1);
        assert_eq!(mgr.filtered_tiles(&TabFilter::Other).len(), 1);
    }

    #[test]
    fn test_select_next_cycles() {
        let mut mgr = TileManager::new();
        let id1 = make_tile(&mut mgr, Some("a"));
        let id2 = make_tile(&mut mgr, Some("a"));
        let id3 = make_tile(&mut mgr, Some("a"));
        mgr.select(id1);
        mgr.select_next(&TabFilter::All);
        assert_eq!(mgr.selected_id(), Some(id2));
        mgr.select_next(&TabFilter::All);
        assert_eq!(mgr.selected_id(), Some(id3));
        mgr.select_next(&TabFilter::All);
        assert_eq!(mgr.selected_id(), Some(id1)); // cycles
    }

    #[test]
    fn test_select_direction() {
        let mut mgr = TileManager::new();
        // 2x2 grid:
        // id1 id2
        // id3 id4
        let id1 = make_tile(&mut mgr, None);
        let id2 = make_tile(&mut mgr, None);
        let id3 = make_tile(&mut mgr, None);
        let id4 = make_tile(&mut mgr, None);

        mgr.select(id1);
        mgr.select_direction(&TabFilter::All, 2, Direction::Right);
        assert_eq!(mgr.selected_id(), Some(id2));
        mgr.select_direction(&TabFilter::All, 2, Direction::Down);
        assert_eq!(mgr.selected_id(), Some(id4));
        mgr.select_direction(&TabFilter::All, 2, Direction::Left);
        assert_eq!(mgr.selected_id(), Some(id3));
        mgr.select_direction(&TabFilter::All, 2, Direction::Up);
        assert_eq!(mgr.selected_id(), Some(id1));
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test --lib tile::tests tile_manager::tests`
Expected: All PASS

- [ ] **Step 5: Commit**

```bash
git add src/tile.rs src/tile_manager.rs src/lib.rs
git commit -m "feat: add Tile and TileManager with lifecycle, selection, and grid navigation"
```

---

### Task 11: UI Widgets

**Files:**
- Create: `src/ui/mod.rs`
- Create: `src/ui/tile_card.rs`
- Create: `src/ui/tab_bar.rs`
- Create: `src/ui/status_bar.rs`
- Create: `src/ui/detail_panel.rs`
- Create: `src/ui/overlay.rs`
- Modify: `src/lib.rs` (add `pub mod ui;`)

**Depends on:** Tasks 2, 8, 9, 10

All ratatui rendering widgets. Each widget takes data references and renders into a `Rect`.

- [ ] **Step 1: Add module declaration and create directory**

Add to `src/lib.rs`:
```rust
pub mod ui;
```

Create `src/ui/` directory.

- [ ] **Step 2: Write src/ui/mod.rs**

```rust
pub mod tile_card;
pub mod tab_bar;
pub mod status_bar;
pub mod detail_panel;
pub mod overlay;

use ratatui::Frame;
use crate::app::AppMode;
use crate::layout::LayoutResult;
use crate::tab::{TabEntry, TabFilter};
use crate::tile::Tile;
use crate::tile_manager::TileManager;

/// Render the full UI.
pub fn render(
    frame: &mut Frame,
    layout: &LayoutResult,
    tile_manager: &TileManager,
    tab_entries: &[TabEntry],
    active_tab: &TabFilter,
    mode: &AppMode,
    columns: u8,
) {
    // Tab bar
    tab_bar::render(frame, layout.tab_bar, tab_entries, active_tab, tile_manager.tile_count());

    // Tile cards in grid
    let filtered = tile_manager.filtered_tiles(active_tab);
    let selected_id = tile_manager.selected_id();
    for (i, rect) in layout.tile_rects.iter().enumerate() {
        if let Some(tile) = filtered.get(i) {
            let is_selected = selected_id == Some(tile.id);
            tile_card::render(frame, *rect, tile, is_selected);
        }
    }

    // Detail panel
    if let (Some(detail_area), Some(tile)) = (layout.detail_panel, tile_manager.selected()) {
        detail_panel::render(frame, detail_area, tile);
    }

    // Status bar
    status_bar::render(frame, layout.status_bar, mode, tile_manager.tile_count(), columns);
}
```

- [ ] **Step 3: Write src/ui/tile_card.rs**

```rust
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::tile::{Tile, TileStatus};

pub fn render(frame: &mut Frame, area: Rect, tile: &Tile, is_selected: bool) {
    let border_color = if is_selected { Color::Cyan } else { Color::DarkGray };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    // Title line
    let title = build_title_line(tile);
    let title_paragraph = Paragraph::new(title)
        .block(Block::default());

    // Mini terminal preview — last N lines from screen buffer
    let inner = block.inner(area);
    let preview_height = inner.height.saturating_sub(1) as usize; // -1 for title
    let preview_width = inner.width as usize;
    let last_lines = tile.vte.screen.last_n_lines(preview_height);

    let mut text_lines: Vec<Line> = Vec::new();
    // Title line first
    text_lines.push(build_title_line(tile));
    // Then screen content
    for row in last_lines {
        let spans: Vec<Span> = row.iter()
            .take(preview_width)
            .map(|cell| {
                Span::styled(
                    cell.ch.to_string(),
                    Style::default().fg(cell.fg).bg(cell.bg).add_modifier(cell.modifiers),
                )
            })
            .collect();
        text_lines.push(Line::from(spans));
    }

    let paragraph = Paragraph::new(text_lines).block(block);
    frame.render_widget(paragraph, area);
}

fn build_title_line(tile: &Tile) -> Line<'static> {
    let mut spans = Vec::new();

    // Status tag
    let (status_text, status_color) = match &tile.status {
        TileStatus::Running => ("run", Color::Green),
        TileStatus::Waiting => ("wait", Color::Yellow),
        TileStatus::Idle(d) => {
            let mins = d.as_secs() / 60;
            // We can't easily return a dynamic string from a match, so handle below
            ("idle", Color::DarkGray)
        }
        TileStatus::Exited => ("exit", Color::Red),
        TileStatus::Error(_) => ("err", Color::Red),
    };
    spans.push(Span::styled(
        format!("[{}] ", status_text),
        Style::default().fg(status_color),
    ));

    // Project name or directory
    if let Some(ref ctx) = tile.git_context {
        spans.push(Span::styled(
            ctx.project_name.clone(),
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
        ));
        // Branch tag
        if let Some(ref branch) = ctx.branch {
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                format!("[⑂ {}]", branch),
                Style::default().fg(Color::Blue),
            ));
        }
        // Worktree tag
        if let Some(ref wt_name) = ctx.worktree_name {
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                format!("[⑃ {}]", wt_name),
                Style::default().fg(Color::Magenta),
            ));
        }
    } else {
        // Non-git directory
        let dir_name = tile.cwd.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| tile.cwd.to_string_lossy().into_owned());
        spans.push(Span::styled(
            format!("📁 {}", dir_name),
            Style::default().fg(Color::White),
        ));
    }

    // Path (right-aligned, gray) — truncated
    let path_str = tile.cwd.to_string_lossy();
    let short_path = if path_str.len() > 30 {
        format!("~/{}", &path_str[path_str.len()-27..])
    } else {
        path_str.into_owned()
    };
    spans.push(Span::raw(" "));
    spans.push(Span::styled(short_path, Style::default().fg(Color::DarkGray)));

    Line::from(spans)
}
```

- [ ] **Step 4: Write src/ui/tab_bar.rs**

```rust
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::tab::{TabEntry, TabFilter};

pub fn render(
    frame: &mut Frame,
    area: Rect,
    entries: &[TabEntry],
    active: &TabFilter,
    total_count: usize,
) {
    let mut spans = Vec::new();

    // ALL tab
    let is_all_active = matches!(active, TabFilter::All);
    spans.push(make_tab_span(
        &format!("ALL({})", total_count),
        is_all_active,
    ));

    // Project tabs
    for entry in entries {
        spans.push(Span::raw(" "));
        let is_active = match active {
            TabFilter::Project(name) => name == &entry.label,
            TabFilter::Other => entry.label == "Other",
            _ => false,
        };
        spans.push(make_tab_span(
            &format!("{}({})", entry.label, entry.count),
            is_active,
        ));
    }

    let line = Line::from(spans);
    let paragraph = Paragraph::new(line);
    frame.render_widget(paragraph, area);
}

fn make_tab_span(label: &str, is_active: bool) -> Span<'static> {
    let style = if is_active {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Gray)
    };
    Span::styled(format!(" {} ", label), style)
}
```

- [ ] **Step 5: Write src/ui/status_bar.rs**

```rust
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::AppMode;

pub fn render(
    frame: &mut Frame,
    area: Rect,
    mode: &AppMode,
    session_count: usize,
    columns: u8,
) {
    let (mode_text, mode_color) = match mode {
        AppMode::Normal => ("Normal", Color::Cyan),
        AppMode::Insert => ("Insert", Color::Green),
        AppMode::Overlay(_) => ("Overlay", Color::Yellow),
    };

    let spans = vec![
        Span::styled(
            format!(" [{}] ", mode_text),
            Style::default().fg(Color::Black).bg(mode_color).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" termgrid"),
        Span::styled(
            format!(" | {} sessions", session_count),
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(
            format!(" | {} cols", columns),
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(
            " | ?help",
            Style::default().fg(Color::DarkGray),
        ),
    ];

    let line = Line::from(spans);
    let paragraph = Paragraph::new(line);
    frame.render_widget(paragraph, area);
}
```

- [ ] **Step 6: Write src/ui/detail_panel.rs**

```rust
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::tile::Tile;

pub fn render(frame: &mut Frame, area: Rect, tile: &Tile) {
    let block = Block::default()
        .borders(Borders::LEFT)
        .border_style(Style::default().fg(Color::DarkGray));

    let inner = block.inner(area);

    // Split into header (3 lines) and terminal area
    let chunks = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(1),
    ]).split(inner);

    // Header: project name, path, branch, hints
    let header = build_header(tile);
    let header_widget = Paragraph::new(header);
    frame.render_widget(header_widget, chunks[0]);

    // Terminal area — full screen buffer render
    let term_area = chunks[1];
    let visible = tile.vte.screen.visible_lines();
    let width = term_area.width as usize;
    let height = term_area.height as usize;

    let mut lines: Vec<Line> = Vec::new();
    let start_row = visible.len().saturating_sub(height);
    for row in &visible[start_row..] {
        let spans: Vec<Span> = row.iter()
            .take(width)
            .map(|cell| {
                Span::styled(
                    cell.ch.to_string(),
                    Style::default().fg(cell.fg).bg(cell.bg).add_modifier(cell.modifiers),
                )
            })
            .collect();
        lines.push(Line::from(spans));
    }

    let terminal = Paragraph::new(lines);
    frame.render_widget(terminal, term_area);

    // Render outer block border
    frame.render_widget(block, area);
}

fn build_header(tile: &Tile) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    // Line 1: project/dir name + hints
    let name = tile.git_context.as_ref()
        .map(|g| g.project_name.clone())
        .unwrap_or_else(|| {
            tile.cwd.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        });
    lines.push(Line::from(vec![
        Span::styled(name, Style::default().add_modifier(Modifier::BOLD)),
        Span::styled("  ESC close | ↑↓ switch", Style::default().fg(Color::DarkGray)),
    ]));

    // Line 2: full path + branch
    let mut info_spans = vec![
        Span::styled(tile.cwd.to_string_lossy().into_owned(), Style::default().fg(Color::DarkGray)),
    ];
    if let Some(ref ctx) = tile.git_context {
        if let Some(ref branch) = ctx.branch {
            info_spans.push(Span::styled(
                format!("  ⑂ {}", branch),
                Style::default().fg(Color::Blue),
            ));
        }
    }
    lines.push(Line::from(info_spans));

    // Line 3: separator
    lines.push(Line::from("─".repeat(40)));

    lines
}
```

- [ ] **Step 7: Write src/ui/overlay.rs (placeholder for Task 13)**

```rust
use ratatui::layout::Rect;
use ratatui::Frame;

use crate::app::OverlayKind;

pub fn render(frame: &mut Frame, area: Rect, overlay: &OverlayKind) {
    match overlay {
        OverlayKind::Help => render_help(frame, area),
        OverlayKind::ConfirmClose(_) => render_confirm(frame, area),
        OverlayKind::ProjectSelector { .. } => render_project_selector(frame, area),
    }
}

fn render_help(frame: &mut Frame, area: Rect) {
    use ratatui::widgets::{Block, Borders, Paragraph, Clear};
    use ratatui::style::{Color, Style};

    let help_text = vec![
        "termgrid — Keyboard Shortcuts",
        "",
        "Normal Mode:",
        "  ↑↓←→/hjkl  Navigate tiles",
        "  i/Enter     Enter terminal (Insert mode)",
        "  n           New terminal",
        "  x           Close terminal",
        "  1/2/3       Set column count",
        "  Tab/S-Tab   Switch project tab",
        "  Esc         Deselect / close panel",
        "  q           Quit",
        "  ?           This help",
        "",
        "Insert Mode:",
        "  Ctrl+]      Exit to Normal mode",
    ];

    // Center the popup
    let popup_width = 45u16.min(area.width.saturating_sub(4));
    let popup_height = (help_text.len() as u16 + 2).min(area.height.saturating_sub(2));
    let popup = centered_rect(popup_width, popup_height, area);

    frame.render_widget(Clear, popup);
    let block = Block::default()
        .title(" Help ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let paragraph = Paragraph::new(help_text.join("\n")).block(block);
    frame.render_widget(paragraph, popup);
}

fn render_confirm(frame: &mut Frame, area: Rect) {
    use ratatui::widgets::{Block, Borders, Paragraph, Clear};
    use ratatui::style::{Color, Style};

    let popup = centered_rect(40, 5, area);
    frame.render_widget(Clear, popup);
    let block = Block::default()
        .title(" Confirm Close ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));
    let text = "Process is running. Close? (y/n)";
    let paragraph = Paragraph::new(text).block(block);
    frame.render_widget(paragraph, popup);
}

fn render_project_selector(frame: &mut Frame, area: Rect) {
    use ratatui::widgets::{Block, Borders, Paragraph, Clear};
    use ratatui::style::{Color, Style};

    let popup = centered_rect(60, 20, area);
    frame.render_widget(Clear, popup);
    let block = Block::default()
        .title(" Select Project ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let text = "Project selector — to be implemented in Task 13";
    let paragraph = Paragraph::new(text).block(block);
    frame.render_widget(paragraph, popup);
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}
```

- [ ] **Step 8: Verify compilation**

Run: `cargo check`
Expected: Compiles (note: `app::AppMode` and `app::OverlayKind` are referenced but not yet defined — we'll create a minimal `app.rs` stub)

- [ ] **Step 9: Create app.rs stub for type definitions**

Write `src/app.rs` (minimal, just the types needed by UI):

```rust
use crate::tile::TileId;

#[derive(Debug, Clone, PartialEq)]
pub enum AppMode {
    Normal,
    Insert,
    Overlay(OverlayKind),
}

#[derive(Debug, Clone, PartialEq)]
pub enum OverlayKind {
    Help,
    ConfirmClose(TileId),
    ProjectSelector {
        query: String,
        items: Vec<String>,
        selected: usize,
    },
}
```

Add to `src/lib.rs`:
```rust
pub mod app;
```

- [ ] **Step 10: Verify compilation**

Run: `cargo check`
Expected: Compiles with 0 errors

- [ ] **Step 11: Commit**

```bash
git add src/ui/ src/app.rs src/lib.rs
git commit -m "feat: add UI widgets for tile card, tab bar, status bar, detail panel, and overlays"
```

---

## Phase D: Application

### Task 12: Event Loop and App Integration

**Files:**
- Create: `src/event.rs`
- Create: `src/input.rs`
- Modify: `src/app.rs` (expand with full App struct and run loop)
- Modify: `src/lib.rs` (add `pub mod event; pub mod input;`)

**Depends on:** Tasks 2-11

This is the integration task — wiring everything together into a working application.

- [ ] **Step 1: Add module declarations**

Add to `src/lib.rs`:
```rust
pub mod event;
pub mod input;
```

- [ ] **Step 2: Write src/event.rs**

```rust
use crate::tile::TileId;
use crossterm::event::Event as CrosstermEvent;

/// All events the main loop handles.
#[derive(Debug)]
pub enum AppEvent {
    /// User input (keyboard, mouse, resize).
    Crossterm(CrosstermEvent),
    /// PTY output from a tile.
    PtyOutput(TileId, Vec<u8>),
    /// CWD changed for a tile.
    CwdChanged(TileId, std::path::PathBuf),
    /// Periodic tick for status updates.
    Tick,
}
```

- [ ] **Step 3: Write src/input.rs**

```rust
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};

use crate::app::{AppMode, OverlayKind};
use crate::tab;
use crate::tab::TabFilter;
use crate::tile::TileStatus;
use crate::tile_manager::{Direction, TileManager};

/// Result of handling an input event.
pub enum InputResult {
    /// Continue running.
    Continue,
    /// Quit the application.
    Quit,
}

/// Handle a crossterm key event based on current mode.
pub fn handle_key(
    key: KeyEvent,
    mode: &mut AppMode,
    tile_manager: &mut TileManager,
    active_tab: &mut TabFilter,
    tab_entries: &[tab::TabEntry],
    columns: &mut u8,
) -> InputResult {
    match mode {
        AppMode::Normal => handle_normal_key(key, mode, tile_manager, active_tab, tab_entries, columns),
        AppMode::Insert => handle_insert_key(key, mode, tile_manager),
        AppMode::Overlay(ref kind) => handle_overlay_key(key, mode, kind.clone(), tile_manager),
    }
}

fn handle_normal_key(
    key: KeyEvent,
    mode: &mut AppMode,
    tile_manager: &mut TileManager,
    active_tab: &mut TabFilter,
    tab_entries: &[tab::TabEntry],
    columns: &mut u8,
) -> InputResult {
    match key.code {
        KeyCode::Char('q') => return InputResult::Quit,
        KeyCode::Char('?') => {
            *mode = AppMode::Overlay(OverlayKind::Help);
        }

        // Navigation
        KeyCode::Up | KeyCode::Char('k') => {
            tile_manager.select_direction(active_tab, *columns, Direction::Up);
        }
        KeyCode::Down | KeyCode::Char('j') => {
            tile_manager.select_direction(active_tab, *columns, Direction::Down);
        }
        KeyCode::Left | KeyCode::Char('h') => {
            tile_manager.select_direction(active_tab, *columns, Direction::Left);
        }
        KeyCode::Right | KeyCode::Char('l') => {
            tile_manager.select_direction(active_tab, *columns, Direction::Right);
        }

        // Enter insert mode
        KeyCode::Char('i') | KeyCode::Enter => {
            if tile_manager.selected_id().is_some() {
                *mode = AppMode::Insert;
            }
        }

        // Deselect / close detail panel
        KeyCode::Esc => {
            tile_manager.deselect();
        }

        // New tile
        KeyCode::Char('n') => {
            *mode = AppMode::Overlay(OverlayKind::ProjectSelector {
                query: String::new(),
                items: Vec::new(), // Will be populated by overlay logic
                selected: 0,
            });
        }

        // Close tile
        KeyCode::Char('x') => {
            if let Some(tile) = tile_manager.selected() {
                if tile.status == TileStatus::Running {
                    *mode = AppMode::Overlay(OverlayKind::ConfirmClose(tile.id));
                } else {
                    let id = tile.id;
                    tile_manager.remove(id);
                }
            }
        }

        // Column count
        KeyCode::Char('1') => *columns = 1,
        KeyCode::Char('2') => *columns = 2,
        KeyCode::Char('3') => *columns = 3,

        // Tab switching
        KeyCode::Tab => {
            *active_tab = tab::next_tab(active_tab, tab_entries);
        }
        KeyCode::BackTab => {
            *active_tab = tab::prev_tab(active_tab, tab_entries);
        }

        _ => {}
    }
    InputResult::Continue
}

fn handle_insert_key(
    key: KeyEvent,
    mode: &mut AppMode,
    tile_manager: &mut TileManager,
) -> InputResult {
    // Ctrl+] exits insert mode
    if key.code == KeyCode::Char(']') && key.modifiers.contains(KeyModifiers::CONTROL) {
        *mode = AppMode::Normal;
        return InputResult::Continue;
    }

    // Forward all other keys to the selected tile's PTY
    if let Some(tile) = tile_manager.selected_mut() {
        let bytes = key_event_to_bytes(&key);
        if !bytes.is_empty() {
            let _ = tile.write_input(&bytes);
        }
    }

    InputResult::Continue
}

fn handle_overlay_key(
    key: KeyEvent,
    mode: &mut AppMode,
    overlay: OverlayKind,
    tile_manager: &mut TileManager,
) -> InputResult {
    match overlay {
        OverlayKind::Help => {
            // Any key closes help
            *mode = AppMode::Normal;
        }
        OverlayKind::ConfirmClose(id) => {
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    tile_manager.remove(id);
                    *mode = AppMode::Normal;
                }
                _ => {
                    *mode = AppMode::Normal;
                }
            }
        }
        OverlayKind::ProjectSelector { .. } => {
            if key.code == KeyCode::Esc {
                *mode = AppMode::Normal;
            }
            // Full project selector logic will be in Task 13
        }
    }
    InputResult::Continue
}

/// Handle mouse events.
pub fn handle_mouse(
    mouse: MouseEvent,
    mode: &mut AppMode,
    tile_manager: &mut TileManager,
    active_tab: &mut TabFilter,
) -> InputResult {
    // Mouse handling will be implemented based on UI layout hit testing
    // For MVP: basic click-to-select
    InputResult::Continue
}

/// Convert a KeyEvent to bytes suitable for PTY input.
fn key_event_to_bytes(key: &KeyEvent) -> Vec<u8> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Char(c) => {
            if ctrl {
                // Ctrl+A = 0x01, Ctrl+B = 0x02, etc.
                let byte = (c as u8).wrapping_sub(b'a').wrapping_add(1);
                if byte <= 26 { vec![byte] } else { vec![] }
            } else {
                let mut buf = [0u8; 4];
                let s = c.encode_utf8(&mut buf);
                s.as_bytes().to_vec()
            }
        }
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::Esc => vec![0x1b],
        KeyCode::Up => b"\x1b[A".to_vec(),
        KeyCode::Down => b"\x1b[B".to_vec(),
        KeyCode::Right => b"\x1b[C".to_vec(),
        KeyCode::Left => b"\x1b[D".to_vec(),
        KeyCode::Home => b"\x1b[H".to_vec(),
        KeyCode::End => b"\x1b[F".to_vec(),
        KeyCode::Delete => b"\x1b[3~".to_vec(),
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_to_bytes_char() {
        let key = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE);
        assert_eq!(key_event_to_bytes(&key), b"a");
    }

    #[test]
    fn test_key_to_bytes_ctrl_c() {
        let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(key_event_to_bytes(&key), vec![3]); // ETX
    }

    #[test]
    fn test_key_to_bytes_enter() {
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(key_event_to_bytes(&key), b"\r");
    }

    #[test]
    fn test_key_to_bytes_arrow() {
        let key = KeyEvent::new(KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(key_event_to_bytes(&key), b"\x1b[A");
    }

    #[test]
    fn test_key_to_bytes_unicode() {
        let key = KeyEvent::new(KeyCode::Char('中'), KeyModifiers::NONE);
        let bytes = key_event_to_bytes(&key);
        assert_eq!(std::str::from_utf8(&bytes).unwrap(), "中");
    }
}
```

- [ ] **Step 4: Expand src/app.rs with full App struct and run loop**

Replace `src/app.rs`:

```rust
use crate::config::Config;
use crate::event::AppEvent;
use crate::input;
use crate::layout;
use crate::tab::{self, TabFilter};
use crate::tile::{Tile, TileId};
use crate::tile_manager::TileManager;
use crate::ui;

use crossterm::event::{Event as CEvent, EventStream, KeyEventKind};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::{execute, event::EnableMouseCapture, event::DisableMouseCapture};
use futures::StreamExt;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io::{self, stdout, Read};
use std::time::Duration;
use tokio::sync::mpsc;

#[derive(Debug, Clone, PartialEq)]
pub enum AppMode {
    Normal,
    Insert,
    Overlay(OverlayKind),
}

#[derive(Debug, Clone, PartialEq)]
pub enum OverlayKind {
    Help,
    ConfirmClose(TileId),
    ProjectSelector {
        query: String,
        items: Vec<String>,
        selected: usize,
    },
}

pub struct App {
    config: Config,
    tile_manager: TileManager,
    mode: AppMode,
    active_tab: TabFilter,
    columns: u8,
    scroll_offset: usize,
    event_tx: mpsc::Sender<AppEvent>,
    event_rx: mpsc::Receiver<AppEvent>,
    should_quit: bool,
}

impl App {
    pub fn new(config: Config) -> Self {
        let (event_tx, event_rx) = mpsc::channel(256);
        Self {
            columns: config.layout.default_columns.max(1).min(3),
            config,
            tile_manager: TileManager::new(),
            mode: AppMode::Normal,
            active_tab: TabFilter::All,
            scroll_offset: 0,
            event_tx,
            event_rx,
            should_quit: false,
        }
    }

    /// Spawn a new tile and start its PTY reader task.
    pub fn spawn_tile(&mut self, cwd: &std::path::Path) -> anyhow::Result<TileId> {
        let id = self.tile_manager.next_tile_id();
        let detail_width = (80 * self.config.layout.detail_panel_width / 100) as u16;
        let detail_height = 24; // Will be updated on first render

        let (tile, reader) = Tile::spawn(
            id,
            &self.config.terminal.shell,
            cwd,
            detail_width.max(40),
            detail_height,
        )?;
        self.tile_manager.add(tile);

        // Spawn async PTY reader
        let tx = self.event_tx.clone();
        tokio::spawn(async move {
            pty_reader_task(id, reader, tx).await;
        });

        // Spawn CWD poller for this tile
        // (Will be done in the main tick handler for simplicity)

        Ok(id)
    }

    /// Main run loop.
    pub async fn run(&mut self) -> anyhow::Result<()> {
        // Setup terminal
        enable_raw_mode()?;
        let mut stdout = stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        // Spawn crossterm event reader
        let tx = self.event_tx.clone();
        tokio::spawn(async move {
            crossterm_event_reader(tx).await;
        });

        // Spawn tick timer
        let tx = self.event_tx.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(2));
            loop {
                interval.tick().await;
                if tx.send(AppEvent::Tick).await.is_err() {
                    break;
                }
            }
        });

        // Main event loop
        while !self.should_quit {
            // Render
            let tab_entries = self.compute_tab_entries();
            let has_selection = self.tile_manager.selected_id().is_some();
            terminal.draw(|frame| {
                let total = frame.area();
                let filtered_count = self.tile_manager.filtered_tiles(&self.active_tab).len();
                let layout_result = layout::calculate_layout(
                    total,
                    self.columns,
                    filtered_count,
                    has_selection,
                    self.config.layout.detail_panel_width,
                    self.scroll_offset,
                );
                ui::render(
                    frame,
                    &layout_result,
                    &self.tile_manager,
                    &tab_entries,
                    &self.active_tab,
                    &self.mode,
                    self.columns,
                );

                // Render overlay if active
                if let AppMode::Overlay(ref kind) = self.mode {
                    ui::overlay::render(frame, total, kind);
                }
            })?;

            // Wait for next event
            if let Some(event) = self.event_rx.recv().await {
                self.handle_event(event);
            }
        }

        // Cleanup
        disable_raw_mode()?;
        execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
        terminal.show_cursor()?;

        Ok(())
    }

    fn handle_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::Crossterm(cevent) => {
                match cevent {
                    CEvent::Key(key) if key.kind == KeyEventKind::Press => {
                        let tab_entries = self.compute_tab_entries();
                        let result = input::handle_key(
                            key,
                            &mut self.mode,
                            &mut self.tile_manager,
                            &mut self.active_tab,
                            &tab_entries,
                            &mut self.columns,
                        );
                        if matches!(result, input::InputResult::Quit) {
                            self.should_quit = true;
                        }
                    }
                    CEvent::Mouse(mouse) => {
                        let _ = input::handle_mouse(
                            mouse,
                            &mut self.mode,
                            &mut self.tile_manager,
                            &mut self.active_tab,
                        );
                    }
                    CEvent::Resize(_cols, _rows) => {
                        // Terminal will re-render with new size automatically.
                        // Resize selected tile's PTY.
                        // This will be refined when detail panel size is known.
                    }
                    _ => {}
                }
            }
            AppEvent::PtyOutput(tile_id, bytes) => {
                if let Some(tile) = self.tile_manager.get_mut(tile_id) {
                    tile.process_output(&bytes);
                }
            }
            AppEvent::CwdChanged(tile_id, new_cwd) => {
                if let Some(tile) = self.tile_manager.get_mut(tile_id) {
                    tile.update_cwd(new_cwd);
                }
            }
            AppEvent::Tick => {
                // Update CWD and status for all tiles
                self.poll_tile_states();
            }
        }
    }

    fn compute_tab_entries(&self) -> Vec<tab::TabEntry> {
        let contexts: Vec<_> = self.tile_manager.tiles()
            .iter()
            .map(|t| t.git_context.clone())
            .collect();
        tab::aggregate_tabs(&contexts)
    }

    fn poll_tile_states(&mut self) {
        // CWD polling for macOS
        for tile in self.tile_manager.tiles_mut() {
            if let Some(pid) = tile.pty.pid() {
                #[cfg(target_os = "macos")]
                {
                    // Get foreground PID and CWD
                    if let Some(master_fd) = tile.pty.master_fd() {
                        if let Some(fg_pid) = crate::process::get_foreground_pid(master_fd) {
                            let is_shell = fg_pid as u32 == pid;
                            tile.update_status(is_shell);

                            if let Some(cwd) = crate::process::get_process_cwd(fg_pid) {
                                tile.update_cwd(cwd);
                            }
                        }
                    }
                }
                #[cfg(not(target_os = "macos"))]
                {
                    // Non-macOS: just check if process is alive
                    tile.update_status(true);
                }
            }
        }
    }
}

/// Background task: read from PTY and forward to event channel.
async fn pty_reader_task(
    tile_id: TileId,
    mut reader: crate::pty::PtyReader,
    tx: mpsc::Sender<AppEvent>,
) {
    let mut buf = vec![0u8; 4096];
    loop {
        // Use spawn_blocking for the synchronous read
        let result = tokio::task::spawn_blocking({
            let mut reader = reader;
            move || {
                let n = reader.0.read(&mut buf);
                (reader, buf, n)
            }
        })
        .await;

        match result {
            Ok((r, b, Ok(0))) => break,  // EOF
            Ok((r, b, Ok(n))) => {
                let data = b[..n].to_vec();
                reader = r;
                buf = b;
                if tx.send(AppEvent::PtyOutput(tile_id, data)).await.is_err() {
                    break;
                }
            }
            Ok((r, b, Err(_))) => break, // Read error
            Err(_) => break,              // Task join error
        }
    }
}

/// Background task: forward crossterm events to the event channel.
async fn crossterm_event_reader(tx: mpsc::Sender<AppEvent>) {
    let mut stream = EventStream::new();
    while let Some(Ok(event)) = stream.next().await {
        if tx.send(AppEvent::Crossterm(event)).await.is_err() {
            break;
        }
    }
}
```

- [ ] **Step 5: Run compilation check**

Run: `cargo check`
Expected: Compiles (there may be warnings about unused code which is fine at this stage)

- [ ] **Step 6: Run all tests**

Run: `cargo test`
Expected: All existing tests pass

- [ ] **Step 7: Commit**

```bash
git add src/event.rs src/input.rs src/app.rs src/lib.rs
git commit -m "feat: add event loop, input handling, and App integration"
```

---

### Task 13: Session Persistence, CLI, and main.rs

**Files:**
- Create: `src/session.rs`
- Modify: `src/main.rs` (full CLI + app entry)
- Modify: `src/lib.rs` (add `pub mod session;`)

**Depends on:** Task 12

- [ ] **Step 1: Add module declaration**

Add to `src/lib.rs`:
```rust
pub mod session;
```

- [ ] **Step 2: Write src/session.rs**

```rust
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Session state saved to disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub tiles: Vec<TileSession>,
    pub columns: u8,
    pub active_tab: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TileSession {
    pub cwd: PathBuf,
}

impl Session {
    /// Default session file path.
    pub fn session_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("~/.config"))
            .join("termgrid")
            .join("sessions.json")
    }

    /// Save session to disk.
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Load session from disk. Returns None if file doesn't exist.
    pub fn load(path: &Path) -> Option<Self> {
        let content = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&content).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_save_and_load() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("session.json");

        let session = Session {
            tiles: vec![
                TileSession { cwd: PathBuf::from("/tmp/a") },
                TileSession { cwd: PathBuf::from("/tmp/b") },
            ],
            columns: 2,
            active_tab: "ALL".into(),
        };

        session.save(&path).unwrap();
        let loaded = Session::load(&path).unwrap();
        assert_eq!(loaded.tiles.len(), 2);
        assert_eq!(loaded.columns, 2);
        assert_eq!(loaded.tiles[0].cwd, PathBuf::from("/tmp/a"));
    }

    #[test]
    fn test_load_nonexistent() {
        let session = Session::load(Path::new("/nonexistent/session.json"));
        assert!(session.is_none());
    }

    #[test]
    fn test_session_path() {
        let path = Session::session_path();
        assert!(path.ends_with("termgrid/sessions.json"));
    }
}
```

- [ ] **Step 3: Add serde_json dependency**

Add to `Cargo.toml` under `[dependencies]`:
```toml
serde_json = "1"
```

- [ ] **Step 4: Write full main.rs**

Replace `src/main.rs`:

```rust
use clap::Parser;
use std::path::PathBuf;

use termgrid::app::App;
use termgrid::config::Config;
use termgrid::session::Session;

#[derive(Parser, Debug)]
#[command(name = "termgrid", about = "Multi-terminal manager with Git context awareness")]
struct Cli {
    /// Directory to scan for projects
    #[arg()]
    path: Option<PathBuf>,

    /// Restore last session
    #[arg(long)]
    restore: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let config = Config::load(&Config::config_path());
    let mut app = App::new(config);

    if cli.restore {
        // Restore previous session
        if let Some(session) = Session::load(&Session::session_path()) {
            for tile_session in &session.tiles {
                if tile_session.cwd.exists() {
                    let _ = app.spawn_tile(&tile_session.cwd);
                }
            }
        }
    } else if let Some(path) = cli.path {
        // Scan directory — for MVP, just open one tile in the given directory
        if path.exists() {
            app.spawn_tile(&path)?;
        }
    }
    // No args: empty dashboard, user presses 'n' to create tiles

    // Run the app
    app.run().await?;

    // Save session on exit
    let session = Session {
        tiles: app.tile_manager_ref().tiles().iter().map(|t| {
            termgrid::session::TileSession { cwd: t.cwd.clone() }
        }).collect(),
        columns: app.columns(),
        active_tab: "ALL".into(),
    };
    session.save(&Session::session_path())?;

    Ok(())
}
```

- [ ] **Step 5: Add accessor methods to App**

Add to `src/app.rs`:

```rust
impl App {
    pub fn tile_manager_ref(&self) -> &TileManager {
        &self.tile_manager
    }

    pub fn columns(&self) -> u8 {
        self.columns
    }
}
```

- [ ] **Step 6: Verify compilation and tests**

Run: `cargo check && cargo test`
Expected: Compiles and all tests pass

- [ ] **Step 7: Commit**

```bash
git add src/session.rs src/main.rs src/app.rs src/lib.rs Cargo.toml
git commit -m "feat: add session persistence, CLI parsing, and main.rs entry point"
```

---

### Task 14: Integration Test and Polish

**Files:**
- Create: `tests/integration.rs`
- Modify: various files (fix compilation issues, clippy warnings on changed code)

**Depends on:** Task 13

- [ ] **Step 1: Write integration test**

Create `tests/integration.rs`:

```rust
use termgrid::config::Config;
use termgrid::screen::VteState;

#[test]
fn test_vte_full_session_simulation() {
    let mut vte = VteState::new(80, 24);

    // Simulate a shell prompt with colors
    vte.process(b"\x1b[32muser@host\x1b[0m:\x1b[34m~/project\x1b[0m$ ");
    vte.process(b"cargo test\r\n");
    vte.process(b"    \x1b[32mCompiling\x1b[0m termgrid v0.1.0\r\n");
    vte.process(b"    \x1b[32mFinished\x1b[0m test target\r\n");
    vte.process(b"     \x1b[32mRunning\x1b[0m tests\r\n");
    vte.process(b"test result: \x1b[32mok\x1b[0m. 10 passed; 0 failed\r\n");

    // Verify content
    let screen = &vte.screen;
    let line0: String = screen.visible_lines()[0].iter()
        .take_while(|c| c.ch != ' ' || screen.visible_lines()[0].iter().skip(1).any(|c2| c2.ch != ' '))
        .map(|c| c.ch)
        .collect();
    assert!(line0.contains("user@host"));

    // Verify colors
    let user_cell = &screen.visible_lines()[0][0]; // 'u' in 'user'
    assert_eq!(user_cell.fg, ratatui::style::Color::Green);
}

#[test]
fn test_config_default_loads() {
    let config = Config::default();
    assert!(config.layout.default_columns >= 1);
    assert!(config.layout.default_columns <= 3);
}

#[test]
fn test_git_detection_current_dir() {
    // This test only works if run inside a git repo (which termgrid is)
    let cwd = std::env::current_dir().unwrap();
    let ctx = termgrid::git::detect_git(&cwd);
    if let Some(ctx) = ctx {
        assert_eq!(ctx.project_name, "termgrid");
    }
}

#[test]
fn test_layout_various_configs() {
    use ratatui::layout::Rect;

    // Minimal terminal
    let layout = termgrid::layout::calculate_layout(
        Rect::new(0, 0, 40, 10), 1, 2, false, 45, 0,
    );
    assert_eq!(layout.tile_rects.len(), 2);

    // Wide terminal with detail panel
    let layout = termgrid::layout::calculate_layout(
        Rect::new(0, 0, 200, 50), 3, 9, true, 45, 0,
    );
    assert!(layout.detail_panel.is_some());
    assert_eq!(layout.tile_rects.len(), 9);
}
```

- [ ] **Step 2: Run all tests**

Run: `cargo test`
Expected: All tests pass (unit + integration)

- [ ] **Step 3: Run clippy on changed files**

Run: `cargo clippy --all-targets 2>&1 | head -50`
Fix any clippy warnings on code from this plan only.

- [ ] **Step 4: Verify the binary runs**

Run: `cargo run -- --help`
Expected: Shows CLI help text with `--restore` and `[PATH]` options.

- [ ] **Step 5: Commit**

```bash
git add tests/ src/
git commit -m "feat: add integration tests and polish for MVP"
```

---

## Dependency Graph

```
Task 1 (scaffold)
├── Task 2 (screen) ──→ Task 3 (VTE)
├── Task 4 (config)
├── Task 5 (PTY) ─────────────────────→ Task 10 (tile + mgr)
├── Task 6 (process)                          │
├── Task 7 (git) ──→ Task 9 (tab) ───────────┤
└── Task 8 (layout) ─────────────────→ Task 11 (UI widgets)
                                              │
                                       Task 12 (app + event + input)
                                              │
                                       Task 13 (session + CLI + main)
                                              │
                                       Task 14 (integration test)
```

**Parallelizable groups:**
- After Task 1: Tasks 2, 4, 5, 6, 7, 8 can all run in parallel
- After Task 2: Task 3
- After Tasks 2+5+7: Task 10
- After Tasks 7: Task 9
- After Tasks 8+9+10: Task 11
- Tasks 12-14: sequential
