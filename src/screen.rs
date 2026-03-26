use ratatui::style::{Color, Modifier};
use std::collections::VecDeque;

/// A single terminal character cell with styling.
#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    pub ch: char,
    pub fg: Color,
    pub bg: Color,
    pub modifiers: Modifier,
}

impl Default for Cell {
    fn default() -> Self {
        Cell {
            ch: ' ',
            fg: Color::Reset,
            bg: Color::Reset,
            modifiers: Modifier::empty(),
        }
    }
}

/// Cursor position and visibility state.
#[derive(Debug, Clone, PartialEq)]
pub struct CursorState {
    pub row: usize,
    pub col: usize,
    pub visible: bool,
}

impl Default for CursorState {
    fn default() -> Self {
        CursorState {
            row: 0,
            col: 0,
            visible: true,
        }
    }
}

/// The core terminal screen buffer: a character grid with ANSI color/attribute
/// support, cursor tracking, and scrollback history.
pub struct ScreenBuffer {
    cols: usize,
    rows: usize,
    grid: Vec<Vec<Cell>>,
    scrollback: VecDeque<Vec<Cell>>,
    max_scrollback: usize,
    pub cursor: CursorState,
    pub current_fg: Color,
    pub current_bg: Color,
    pub current_modifiers: Modifier,
}

impl ScreenBuffer {
    /// Create a new empty screen buffer with the given dimensions.
    pub fn new(cols: usize, rows: usize) -> Self {
        let grid = vec![vec![Cell::default(); cols]; rows];
        ScreenBuffer {
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

    /// Number of columns.
    pub fn cols(&self) -> usize {
        self.cols
    }

    /// Number of rows.
    pub fn rows(&self) -> usize {
        self.rows
    }

    /// Number of lines in scrollback.
    pub fn scrollback_len(&self) -> usize {
        self.scrollback.len()
    }

    /// The visible grid lines.
    pub fn visible_lines(&self) -> &[Vec<Cell>] {
        &self.grid
    }

    /// Set the maximum number of scrollback lines.
    pub fn set_max_scrollback(&mut self, max: usize) {
        self.max_scrollback = max;
        while self.scrollback.len() > self.max_scrollback {
            self.scrollback.pop_front();
        }
    }

    /// Return the last N lines from the visible grid (for mini tile rendering).
    pub fn last_n_lines(&self, n: usize) -> Vec<&[Cell]> {
        let start = self.rows.saturating_sub(n);
        self.grid[start..].iter().map(|row| row.as_slice()).collect()
    }

    /// Put a character at the current cursor position with the current style,
    /// then advance the cursor. Wraps at the end of the line.
    pub fn put_char(&mut self, ch: char) {
        if self.cols == 0 || self.rows == 0 {
            return;
        }

        let row = self.cursor.row.min(self.rows - 1);
        let col = self.cursor.col.min(self.cols - 1);

        self.grid[row][col] = Cell {
            ch,
            fg: self.current_fg,
            bg: self.current_bg,
            modifiers: self.current_modifiers,
        };

        if self.cursor.col + 1 >= self.cols {
            // Wrap: move to start of next line
            self.cursor.col = 0;
            self.advance_row();
        } else {
            self.cursor.col += 1;
        }
    }

    /// Move cursor to the start of the next line, scrolling if at the bottom.
    pub fn newline(&mut self) {
        self.cursor.col = 0;
        self.advance_row();
    }

    /// Move cursor to column 0 (carriage return).
    pub fn carriage_return(&mut self) {
        self.cursor.col = 0;
    }

    /// Move cursor left one column (backspace, don't erase).
    pub fn backspace(&mut self) {
        if self.cursor.col > 0 {
            self.cursor.col -= 1;
        }
    }

    /// Move cursor to the next tab stop (multiples of 8).
    pub fn tab(&mut self) {
        let next = (self.cursor.col / 8 + 1) * 8;
        self.cursor.col = next.min(self.cols.saturating_sub(1));
    }

    /// Set cursor to the given (row, col), clamped to buffer bounds.
    pub fn set_cursor_position(&mut self, row: usize, col: usize) {
        self.cursor.row = row.min(self.rows.saturating_sub(1));
        self.cursor.col = col.min(self.cols.saturating_sub(1));
    }

    /// Scroll up by n lines: move top rows into scrollback, add blank rows at bottom.
    pub fn scroll_up(&mut self, n: usize) {
        for _ in 0..n {
            if self.grid.is_empty() {
                break;
            }
            let old_top = self.grid.remove(0);
            self.scrollback.push_back(old_top);
            while self.scrollback.len() > self.max_scrollback {
                self.scrollback.pop_front();
            }
            self.grid.push(vec![Cell::default(); self.cols]);
        }
    }

    /// Erase in display. mode: 0=below cursor, 1=above cursor, 2=entire display, 3=entire display+scrollback.
    pub fn erase_in_display(&mut self, mode: u8) {
        match mode {
            0 => {
                // Erase from cursor to end of display
                let row = self.cursor.row;
                let col = self.cursor.col;
                if row < self.rows {
                    for c in col..self.cols {
                        self.grid[row][c] = Cell::default();
                    }
                    for r in (row + 1)..self.rows {
                        self.grid[r] = vec![Cell::default(); self.cols];
                    }
                }
            }
            1 => {
                // Erase from start of display to cursor (inclusive)
                let row = self.cursor.row;
                let col = self.cursor.col;
                for r in 0..row {
                    self.grid[r] = vec![Cell::default(); self.cols];
                }
                if row < self.rows {
                    for c in 0..=col.min(self.cols.saturating_sub(1)) {
                        self.grid[row][c] = Cell::default();
                    }
                }
            }
            2 => {
                // Erase entire display
                for r in 0..self.rows {
                    self.grid[r] = vec![Cell::default(); self.cols];
                }
            }
            3 => {
                // Erase entire display and scrollback
                for r in 0..self.rows {
                    self.grid[r] = vec![Cell::default(); self.cols];
                }
                self.scrollback.clear();
            }
            _ => {}
        }
    }

    /// Erase in line. mode: 0=to right, 1=to left, 2=entire line.
    pub fn erase_in_line(&mut self, mode: u8) {
        let row = self.cursor.row;
        if row >= self.rows {
            return;
        }
        let col = self.cursor.col;
        match mode {
            0 => {
                // Erase from cursor to end of line
                for c in col..self.cols {
                    self.grid[row][c] = Cell::default();
                }
            }
            1 => {
                // Erase from start of line to cursor (inclusive)
                for c in 0..=col.min(self.cols.saturating_sub(1)) {
                    self.grid[row][c] = Cell::default();
                }
            }
            2 => {
                // Erase entire line
                self.grid[row] = vec![Cell::default(); self.cols];
            }
            _ => {}
        }
    }

    /// Resize the buffer to new dimensions, preserving content where possible.
    pub fn resize(&mut self, new_cols: usize, new_rows: usize) {
        // Adjust columns for each existing row
        for row in &mut self.grid {
            if row.len() < new_cols {
                row.resize(new_cols, Cell::default());
            } else {
                row.truncate(new_cols);
            }
        }

        // Adjust number of rows
        if self.grid.len() < new_rows {
            let to_add = new_rows - self.grid.len();
            for _ in 0..to_add {
                self.grid.push(vec![Cell::default(); new_cols]);
            }
        } else {
            self.grid.truncate(new_rows);
        }

        self.cols = new_cols;
        self.rows = new_rows;

        // Clamp cursor
        self.cursor.row = self.cursor.row.min(new_rows.saturating_sub(1));
        self.cursor.col = self.cursor.col.min(new_cols.saturating_sub(1));

        // Adjust scrollback rows
        for row in &mut self.scrollback {
            if row.len() < new_cols {
                row.resize(new_cols, Cell::default());
            } else {
                row.truncate(new_cols);
            }
        }
    }

    /// Move cursor up by n rows (clamped).
    pub fn cursor_up(&mut self, n: usize) {
        self.cursor.row = self.cursor.row.saturating_sub(n);
    }

    /// Move cursor down by n rows (clamped).
    pub fn cursor_down(&mut self, n: usize) {
        self.cursor.row = (self.cursor.row + n).min(self.rows.saturating_sub(1));
    }

    /// Move cursor forward (right) by n columns (clamped).
    pub fn cursor_forward(&mut self, n: usize) {
        self.cursor.col = (self.cursor.col + n).min(self.cols.saturating_sub(1));
    }

    /// Move cursor back (left) by n columns (clamped).
    pub fn cursor_back(&mut self, n: usize) {
        self.cursor.col = self.cursor.col.saturating_sub(n);
    }

    /// Reset current style to defaults.
    pub fn reset_style(&mut self) {
        self.current_fg = Color::Reset;
        self.current_bg = Color::Reset;
        self.current_modifiers = Modifier::empty();
    }

    /// Insert n blank lines at the cursor row, pushing existing lines down.
    /// Lines that fall off the bottom are discarded.
    pub fn insert_lines(&mut self, n: usize) {
        let row = self.cursor.row.min(self.rows.saturating_sub(1));
        for _ in 0..n {
            self.grid.insert(row, vec![Cell::default(); self.cols]);
            if self.grid.len() > self.rows {
                self.grid.pop();
            }
        }
    }

    /// Delete n lines at the cursor row, pulling subsequent lines up.
    /// Blank lines are added at the bottom.
    pub fn delete_lines(&mut self, n: usize) {
        let row = self.cursor.row.min(self.rows.saturating_sub(1));
        for _ in 0..n {
            if row < self.grid.len() {
                self.grid.remove(row);
                self.grid.push(vec![Cell::default(); self.cols]);
            }
        }
    }

    // --- Private helpers ---

    /// Move cursor down one row, scrolling the display up if at the bottom.
    fn advance_row(&mut self) {
        if self.rows == 0 {
            return;
        }
        if self.cursor.row + 1 >= self.rows {
            self.scroll_up(1);
            // cursor stays at the last row
            self.cursor.row = self.rows - 1;
        } else {
            self.cursor.row += 1;
        }
    }

    /// Handle SGR (Select Graphic Rendition) parameters to update current style.
    fn handle_sgr(&mut self, params: &vte::Params) {
        let mut iter = params.iter();
        while let Some(subparams) = iter.next() {
            let p0 = subparams.first().copied().unwrap_or(0) as u32;
            match p0 {
                0 => self.reset_style(),
                1 => self.current_modifiers |= Modifier::BOLD,
                2 => self.current_modifiers |= Modifier::DIM,
                3 => self.current_modifiers |= Modifier::ITALIC,
                4 => self.current_modifiers |= Modifier::UNDERLINED,
                7 => self.current_modifiers |= Modifier::REVERSED,
                8 => self.current_modifiers |= Modifier::HIDDEN,
                9 => self.current_modifiers |= Modifier::CROSSED_OUT,
                22 => self.current_modifiers &= !(Modifier::BOLD | Modifier::DIM),
                23 => self.current_modifiers &= !Modifier::ITALIC,
                24 => self.current_modifiers &= !Modifier::UNDERLINED,
                27 => self.current_modifiers &= !Modifier::REVERSED,
                28 => self.current_modifiers &= !Modifier::HIDDEN,
                29 => self.current_modifiers &= !Modifier::CROSSED_OUT,
                30 => self.current_fg = Color::Black,
                31 => self.current_fg = Color::Red,
                32 => self.current_fg = Color::Green,
                33 => self.current_fg = Color::Yellow,
                34 => self.current_fg = Color::Blue,
                35 => self.current_fg = Color::Magenta,
                36 => self.current_fg = Color::Cyan,
                37 => self.current_fg = Color::Gray,
                38 => {
                    // Extended fg color — subparams or read next params
                    if let Some(color) = Self::parse_extended_color(subparams, &mut iter) {
                        self.current_fg = color;
                    }
                }
                39 => self.current_fg = Color::Reset,
                40 => self.current_bg = Color::Black,
                41 => self.current_bg = Color::Red,
                42 => self.current_bg = Color::Green,
                43 => self.current_bg = Color::Yellow,
                44 => self.current_bg = Color::Blue,
                45 => self.current_bg = Color::Magenta,
                46 => self.current_bg = Color::Cyan,
                47 => self.current_bg = Color::Gray,
                48 => {
                    // Extended bg color
                    if let Some(color) = Self::parse_extended_color(subparams, &mut iter) {
                        self.current_bg = color;
                    }
                }
                49 => self.current_bg = Color::Reset,
                90 => self.current_fg = Color::DarkGray,
                91 => self.current_fg = Color::LightRed,
                92 => self.current_fg = Color::LightGreen,
                93 => self.current_fg = Color::LightYellow,
                94 => self.current_fg = Color::LightBlue,
                95 => self.current_fg = Color::LightMagenta,
                96 => self.current_fg = Color::LightCyan,
                97 => self.current_fg = Color::White,
                100 => self.current_bg = Color::DarkGray,
                101 => self.current_bg = Color::LightRed,
                102 => self.current_bg = Color::LightGreen,
                103 => self.current_bg = Color::LightYellow,
                104 => self.current_bg = Color::LightBlue,
                105 => self.current_bg = Color::LightMagenta,
                106 => self.current_bg = Color::LightCyan,
                107 => self.current_bg = Color::White,
                _ => {}
            }
        }
    }

    /// Parse an extended color from SGR params.
    /// subparams: the current subparam slice (e.g. [38, 5, N] or [38, 2, R, G, B]).
    /// If subparams only has the leading code (38/48), consume additional params from iter.
    fn parse_extended_color(
        subparams: &[u16],
        iter: &mut vte::ParamsIter<'_>,
    ) -> Option<Color> {
        // subparams may be [38] or [38, 5, N] or [38, 2, R, G, B] depending on encoding
        // The mode is the second element
        let mode = if subparams.len() >= 2 {
            subparams[1]
        } else {
            // Read next sub-param group as mode
            iter.next()?.first().copied()?
        };

        match mode {
            5 => {
                // 256 color: ;5;N
                let n = if subparams.len() >= 3 {
                    subparams[2]
                } else {
                    iter.next()?.first().copied()?
                };
                Some(Color::Indexed(n as u8))
            }
            2 => {
                // True color: ;2;R;G;B
                let (r, g, b) = if subparams.len() >= 5 {
                    (subparams[2], subparams[3], subparams[4])
                } else if subparams.len() == 4 {
                    // Some encodings embed R inline
                    let r = subparams[2];
                    let g = subparams[3];
                    let b = iter.next()?.first().copied()?;
                    (r, g, b)
                } else {
                    let r = iter.next()?.first().copied()?;
                    let g = iter.next()?.first().copied()?;
                    let b = iter.next()?.first().copied()?;
                    (r, g, b)
                };
                Some(Color::Rgb(r as u8, g as u8, b as u8))
            }
            _ => None,
        }
    }
}

/// VTE state: wraps a parser and ScreenBuffer to process raw terminal bytes.
pub struct VteState {
    parser: vte::Parser,
    pub screen: ScreenBuffer,
}

impl VteState {
    pub fn new(cols: usize, rows: usize) -> Self {
        Self {
            parser: vte::Parser::new(),
            screen: ScreenBuffer::new(cols, rows),
        }
    }

    pub fn process(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.screen, bytes);
    }
}

impl vte::Perform for ScreenBuffer {
    fn print(&mut self, c: char) {
        self.put_char(c);
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            0x08 => self.backspace(),
            0x09 => self.tab(),
            0x0A..=0x0C => self.advance_row(),
            0x0D => self.carriage_return(),
            _ => {}
        }
    }

    fn csi_dispatch(
        &mut self,
        params: &vte::Params,
        _intermediates: &[u8],
        _ignore: bool,
        action: char,
    ) {
        // Helper: get nth param as usize, defaulting to `default` if absent or zero
        let param_list: Vec<u16> = params.iter().map(|s| s.first().copied().unwrap_or(0)).collect();
        let p = |idx: usize, default: usize| -> usize {
            let v = param_list.get(idx).copied().unwrap_or(0) as usize;
            if v == 0 { default } else { v }
        };
        let p0_raw = |default: usize| -> usize {
            let v = param_list.first().copied().unwrap_or(0) as usize;
            if v == 0 { default } else { v }
        };

        match action {
            'A' => self.cursor_up(p(0, 1)),
            'B' => self.cursor_down(p(0, 1)),
            'C' => self.cursor_forward(p(0, 1)),
            'D' => self.cursor_back(p(0, 1)),
            'H' | 'f' => {
                let row = p(0, 1).saturating_sub(1);
                let col = p(1, 1).saturating_sub(1);
                self.set_cursor_position(row, col);
            }
            'J' => {
                let mode = p0_raw(0) as u8;
                self.erase_in_display(mode);
            }
            'K' => {
                let mode = p0_raw(0) as u8;
                self.erase_in_line(mode);
            }
            'L' => self.insert_lines(p(0, 1)),
            'M' => self.delete_lines(p(0, 1)),
            'd' => {
                // VPA: set cursor row (1-indexed)
                let row = p(0, 1).saturating_sub(1);
                self.cursor.row = row.min(self.rows.saturating_sub(1));
            }
            'G' | '`' => {
                // CHA: set cursor col (1-indexed)
                let col = p(0, 1).saturating_sub(1);
                self.cursor.col = col.min(self.cols.saturating_sub(1));
            }
            'm' => self.handle_sgr(params),
            _ => {}
        }
    }

    fn esc_dispatch(&mut self, _intermediates: &[u8], _ignore: bool, _byte: u8) {}

    fn osc_dispatch(&mut self, _params: &[&[u8]], _bell_terminated: bool) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_buf(cols: usize, rows: usize) -> ScreenBuffer {
        ScreenBuffer::new(cols, rows)
    }

    #[test]
    fn test_new_creates_empty_grid() {
        let buf = make_buf(80, 24);
        assert_eq!(buf.cols(), 80);
        assert_eq!(buf.rows(), 24);
        assert_eq!(buf.visible_lines().len(), 24);
        for row in buf.visible_lines() {
            assert_eq!(row.len(), 80);
            for cell in row {
                assert_eq!(*cell, Cell::default());
            }
        }
        assert_eq!(buf.scrollback_len(), 0);
        assert_eq!(buf.cursor, CursorState::default());
    }

    #[test]
    fn test_put_char_at_cursor() {
        let mut buf = make_buf(80, 24);
        buf.put_char('A');
        assert_eq!(buf.visible_lines()[0][0].ch, 'A');
        assert_eq!(buf.cursor.col, 1);
        assert_eq!(buf.cursor.row, 0);
    }

    #[test]
    fn test_put_char_wraps_at_end_of_line() {
        let mut buf = make_buf(5, 3);
        // Fill entire first line
        for _ in 0..5 {
            buf.put_char('X');
        }
        // After writing 5 chars in a 5-col buffer, cursor wraps to next line
        assert_eq!(buf.cursor.col, 0);
        assert_eq!(buf.cursor.row, 1);
        // All 5 chars in first row should be 'X'
        for c in 0..5 {
            assert_eq!(buf.visible_lines()[0][c].ch, 'X');
        }
    }

    #[test]
    fn test_newline() {
        let mut buf = make_buf(80, 24);
        buf.cursor.col = 40;
        buf.newline();
        assert_eq!(buf.cursor.col, 0);
        assert_eq!(buf.cursor.row, 1);
    }

    #[test]
    fn test_newline_at_bottom_scrolls() {
        let mut buf = make_buf(5, 3);
        // Write something in first row so we can track it
        buf.put_char('A');
        buf.cursor.row = 2;
        buf.cursor.col = 0;
        buf.newline();
        // Should still be at row 2 (last row), but grid scrolled up
        assert_eq!(buf.cursor.row, 2);
        assert_eq!(buf.scrollback_len(), 1);
    }

    #[test]
    fn test_carriage_return() {
        let mut buf = make_buf(80, 24);
        buf.cursor.col = 50;
        buf.cursor.row = 5;
        buf.carriage_return();
        assert_eq!(buf.cursor.col, 0);
        assert_eq!(buf.cursor.row, 5); // row unchanged
    }

    #[test]
    fn test_backspace() {
        let mut buf = make_buf(80, 24);
        buf.cursor.col = 10;
        buf.backspace();
        assert_eq!(buf.cursor.col, 9);
        // At column 0, backspace should not go negative
        buf.cursor.col = 0;
        buf.backspace();
        assert_eq!(buf.cursor.col, 0);
    }

    #[test]
    fn test_tab() {
        let mut buf = make_buf(80, 24);
        buf.cursor.col = 0;
        buf.tab();
        assert_eq!(buf.cursor.col, 8);
        buf.cursor.col = 5;
        buf.tab();
        assert_eq!(buf.cursor.col, 8);
        buf.cursor.col = 8;
        buf.tab();
        assert_eq!(buf.cursor.col, 16);
        // Tab near end of line should clamp
        buf.cursor.col = 78;
        buf.tab();
        assert_eq!(buf.cursor.col, 79); // clamped to cols-1
    }

    #[test]
    fn test_set_cursor_position() {
        let mut buf = make_buf(80, 24);
        buf.set_cursor_position(10, 20);
        assert_eq!(buf.cursor.row, 10);
        assert_eq!(buf.cursor.col, 20);
    }

    #[test]
    fn test_set_cursor_clamps_to_bounds() {
        let mut buf = make_buf(80, 24);
        buf.set_cursor_position(100, 200);
        assert_eq!(buf.cursor.row, 23); // rows - 1
        assert_eq!(buf.cursor.col, 79); // cols - 1
    }

    #[test]
    fn test_erase_in_display_below() {
        let mut buf = make_buf(5, 3);
        // Fill everything with 'X'
        for r in 0..3 {
            for c in 0..5 {
                buf.grid[r][c].ch = 'X';
            }
        }
        buf.cursor.row = 1;
        buf.cursor.col = 2;
        buf.erase_in_display(0); // erase below cursor
        // Row 0 should be untouched
        for c in 0..5 {
            assert_eq!(buf.visible_lines()[0][c].ch, 'X');
        }
        // Row 1, cols 0..2 untouched; cols 2..5 erased
        assert_eq!(buf.visible_lines()[1][0].ch, 'X');
        assert_eq!(buf.visible_lines()[1][1].ch, 'X');
        assert_eq!(buf.visible_lines()[1][2].ch, ' ');
        assert_eq!(buf.visible_lines()[1][3].ch, ' ');
        assert_eq!(buf.visible_lines()[1][4].ch, ' ');
        // Row 2 fully erased
        for c in 0..5 {
            assert_eq!(buf.visible_lines()[2][c].ch, ' ');
        }
    }

    #[test]
    fn test_erase_in_display_all() {
        let mut buf = make_buf(5, 3);
        for r in 0..3 {
            for c in 0..5 {
                buf.grid[r][c].ch = 'X';
            }
        }
        buf.erase_in_display(2);
        for r in 0..3 {
            for c in 0..5 {
                assert_eq!(buf.visible_lines()[r][c].ch, ' ');
            }
        }
    }

    #[test]
    fn test_erase_in_line() {
        let mut buf = make_buf(5, 3);
        for c in 0..5 {
            buf.grid[1][c].ch = 'X';
        }
        buf.cursor.row = 1;
        buf.cursor.col = 2;
        buf.erase_in_line(0); // erase to right
        assert_eq!(buf.visible_lines()[1][0].ch, 'X');
        assert_eq!(buf.visible_lines()[1][1].ch, 'X');
        assert_eq!(buf.visible_lines()[1][2].ch, ' ');
        assert_eq!(buf.visible_lines()[1][3].ch, ' ');
        assert_eq!(buf.visible_lines()[1][4].ch, ' ');
    }

    #[test]
    fn test_scroll_up() {
        let mut buf = make_buf(5, 3);
        buf.grid[0][0].ch = 'A';
        buf.grid[1][0].ch = 'B';
        buf.grid[2][0].ch = 'C';
        buf.scroll_up(1);
        assert_eq!(buf.visible_lines()[0][0].ch, 'B');
        assert_eq!(buf.visible_lines()[1][0].ch, 'C');
        assert_eq!(buf.visible_lines()[2][0].ch, ' '); // blank row added
        assert_eq!(buf.scrollback_len(), 1);
    }

    #[test]
    fn test_scrollback_limit() {
        let mut buf = make_buf(5, 3);
        buf.set_max_scrollback(2);
        // Scroll up 5 times — only 2 lines should be retained in scrollback
        for _ in 0..5 {
            buf.scroll_up(1);
        }
        assert_eq!(buf.scrollback_len(), 2);
    }

    #[test]
    fn test_resize_grow() {
        let mut buf = make_buf(5, 3);
        buf.grid[0][0].ch = 'A';
        buf.resize(10, 6);
        assert_eq!(buf.cols(), 10);
        assert_eq!(buf.rows(), 6);
        assert_eq!(buf.visible_lines().len(), 6);
        assert_eq!(buf.visible_lines()[0].len(), 10);
        // Original content preserved
        assert_eq!(buf.visible_lines()[0][0].ch, 'A');
    }

    #[test]
    fn test_resize_shrink() {
        let mut buf = make_buf(10, 6);
        buf.grid[0][0].ch = 'A';
        buf.grid[0][9].ch = 'Z';
        buf.resize(5, 3);
        assert_eq!(buf.cols(), 5);
        assert_eq!(buf.rows(), 3);
        assert_eq!(buf.visible_lines()[0][0].ch, 'A');
        // Column 9 no longer exists; row 9 is truncated
        assert_eq!(buf.visible_lines()[0].len(), 5);
    }

    #[test]
    fn test_last_n_lines() {
        let mut buf = make_buf(5, 4);
        buf.grid[2][0].ch = 'Y';
        buf.grid[3][0].ch = 'Z';
        let last2 = buf.last_n_lines(2);
        assert_eq!(last2.len(), 2);
        assert_eq!(last2[0][0].ch, 'Y');
        assert_eq!(last2[1][0].ch, 'Z');
    }

    #[test]
    fn test_style_applied_to_put_char() {
        let mut buf = make_buf(80, 24);
        buf.current_fg = Color::Red;
        buf.current_bg = Color::Blue;
        buf.current_modifiers = Modifier::BOLD;
        buf.put_char('X');
        let cell = &buf.visible_lines()[0][0];
        assert_eq!(cell.ch, 'X');
        assert_eq!(cell.fg, Color::Red);
        assert_eq!(cell.bg, Color::Blue);
        assert_eq!(cell.modifiers, Modifier::BOLD);
    }
}

#[cfg(test)]
mod vte_tests {
    use super::*;
    use ratatui::style::{Color, Modifier};

    fn make_vte(cols: usize, rows: usize) -> VteState {
        VteState::new(cols, rows)
    }

    #[test]
    fn test_plain_text() {
        let mut vte = make_vte(80, 24);
        vte.process(b"Hello, world!");
        let lines = vte.screen.visible_lines();
        let text: String = lines[0][..13].iter().map(|c| c.ch).collect();
        assert_eq!(text, "Hello, world!");
    }

    #[test]
    fn test_crlf() {
        let mut vte = make_vte(80, 24);
        vte.process(b"Line1\r\nLine2");
        let lines = vte.screen.visible_lines();
        let line0: String = lines[0][..5].iter().map(|c| c.ch).collect();
        let line1: String = lines[1][..5].iter().map(|c| c.ch).collect();
        assert_eq!(line0, "Line1");
        assert_eq!(line1, "Line2");
    }

    #[test]
    fn test_sgr_foreground_standard() {
        let mut vte = make_vte(80, 24);
        // ESC[31m — red foreground
        vte.process(b"\x1b[31m");
        assert_eq!(vte.screen.current_fg, Color::Red);
    }

    #[test]
    fn test_sgr_true_color() {
        let mut vte = make_vte(80, 24);
        // ESC[38;2;255;128;0m — true color fg
        vte.process(b"\x1b[38;2;255;128;0m");
        assert_eq!(vte.screen.current_fg, Color::Rgb(255, 128, 0));
    }

    #[test]
    fn test_sgr_256_color() {
        let mut vte = make_vte(80, 24);
        // ESC[38;5;196m — 256 color fg
        vte.process(b"\x1b[38;5;196m");
        assert_eq!(vte.screen.current_fg, Color::Indexed(196));
    }

    #[test]
    fn test_sgr_bold() {
        let mut vte = make_vte(80, 24);
        vte.process(b"\x1b[1m");
        assert!(vte.screen.current_modifiers.contains(Modifier::BOLD));
    }

    #[test]
    fn test_cursor_position() {
        let mut vte = make_vte(80, 24);
        // ESC[3;5H — row 3, col 5 (1-indexed → row 2, col 4 zero-indexed)
        vte.process(b"\x1b[3;5H");
        assert_eq!(vte.screen.cursor.row, 2);
        assert_eq!(vte.screen.cursor.col, 4);
    }

    #[test]
    fn test_erase_display() {
        let mut vte = make_vte(10, 5);
        vte.process(b"Hello");
        // ESC[2J — erase entire display
        vte.process(b"\x1b[2J");
        for row in vte.screen.visible_lines() {
            for cell in row {
                assert_eq!(cell.ch, ' ');
            }
        }
    }

    #[test]
    fn test_erase_line() {
        let mut vte = make_vte(80, 24);
        // Move to row 1, col 5
        vte.process(b"\x1b[1;6H");
        // Write some chars so there's content
        vte.process(b"Hello");
        // Move back to start of that line, col 6 (where we started writing)
        vte.process(b"\x1b[1;6H");
        // ESC[K — erase from cursor to end of line (mode 0)
        vte.process(b"\x1b[K");
        let row = &vte.screen.visible_lines()[0];
        // Cols 0-4 should be spaces (never written), cols 5+ should be erased
        for (c, cell) in row.iter().enumerate().take(80).skip(5) {
            assert_eq!(cell.ch, ' ', "col {} should be space", c);
        }
    }

    #[test]
    fn test_cursor_movement() {
        let mut vte = make_vte(80, 24);
        // ESC[5;10H → row 4, col 9
        vte.process(b"\x1b[5;10H");
        // ESC[2A → cursor up 2 → row 2
        vte.process(b"\x1b[2A");
        // ESC[3C → cursor forward 3 → col 12
        vte.process(b"\x1b[3C");
        assert_eq!(vte.screen.cursor.row, 2);
        assert_eq!(vte.screen.cursor.col, 12);
    }

    #[test]
    fn test_backspace_control() {
        let mut vte = make_vte(80, 24);
        // 'A', 'B', backspace, 'C' → A at col 0, C at col 1
        vte.process(b"AB\x08C");
        let row = &vte.screen.visible_lines()[0];
        assert_eq!(row[0].ch, 'A');
        assert_eq!(row[1].ch, 'C');
    }

    #[test]
    fn test_tab_control() {
        let mut vte = make_vte(80, 24);
        // 'A', tab, 'B' → A at col 0, B at col 8
        vte.process(b"A\tB");
        let row = &vte.screen.visible_lines()[0];
        assert_eq!(row[0].ch, 'A');
        assert_eq!(row[8].ch, 'B');
    }
}
