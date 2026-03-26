use ratatui::style::{Color, Modifier};

/// A single terminal character cell with styling.
#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    pub ch: char,
    pub fg: Color,
    pub bg: Color,
    pub modifiers: Modifier,
    /// True if this cell is a wide character continuation (should be skipped during rendering).
    pub is_wide_continuation: bool,
}

impl Default for Cell {
    fn default() -> Self {
        Cell {
            ch: ' ',
            fg: Color::Reset,
            bg: Color::Reset,
            modifiers: Modifier::empty(),
            is_wide_continuation: false,
        }
    }
}

/// Terminal screen state backed by the vt100 crate.
/// Provides a complete VT100/xterm terminal emulator.
pub struct VteState {
    parser: vt100::Parser,
}

impl VteState {
    pub fn new(cols: u16, rows: u16) -> Self {
        Self {
            parser: vt100::Parser::new(rows, cols, 1000),
        }
    }

    /// Feed raw bytes from PTY into the terminal emulator.
    pub fn process(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
    }

    /// Get the current screen state.
    pub fn screen(&self) -> &vt100::Screen {
        self.parser.screen()
    }

    /// Resize the terminal.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.parser.set_size(rows, cols);
    }

    /// Set scrollback viewing offset. When > 0, `cell()` returns content
    /// from the scrollback buffer (older history).
    /// Clamped to screen row count (vt100 replaces screen rows 1:1 with scrollback rows).
    pub fn set_scrollback(&mut self, rows: usize) {
        let max = self.parser.screen().size().0 as usize;
        self.parser.set_scrollback(rows.min(max));
    }

    /// Get terminal dimensions.
    pub fn cols(&self) -> u16 {
        self.parser.screen().size().1
    }

    pub fn rows(&self) -> u16 {
        self.parser.screen().size().0
    }

    /// Get cursor position (row, col).
    pub fn cursor_position(&self) -> (u16, u16) {
        self.parser.screen().cursor_position()
    }

    /// Export screen content with ANSI formatting for session persistence.
    /// Can be fed back into process() to restore visual state.
    pub fn contents_formatted(&self) -> Vec<u8> {
        self.parser.screen().contents_formatted()
    }

    /// Whether cursor is visible.
    pub fn cursor_visible(&self) -> bool {
        !self.parser.screen().hide_cursor()
    }

    /// Get a cell at (row, col) as our Cell type.
    pub fn cell_at(&self, row: u16, col: u16) -> Cell {
        match self.parser.screen().cell(row, col) {
            Some(vt_cell) => convert_cell(vt_cell),
            None => Cell::default(),
        }
    }

    /// Get a row of cells for rendering.
    pub fn row_cells(&self, row: u16, max_cols: u16) -> Vec<Cell> {
        let screen = self.parser.screen();
        let cols = screen.size().1;
        (0..max_cols.min(cols))
            .map(|col| match screen.cell(row, col) {
                Some(vt_cell) => convert_cell(vt_cell),
                None => Cell::default(),
            })
            .collect()
    }

    /// Get visible rows around the cursor for the small tile preview.
    /// Returns rows ending at cursor row (up to max_rows rows).
    pub fn visible_rows_around_cursor(&self, max_rows: usize, max_cols: u16) -> Vec<Vec<Cell>> {
        let screen = self.parser.screen();
        let (total_rows, _) = screen.size();
        let (cursor_row, _) = screen.cursor_position();

        let end_row = (cursor_row as usize + 1).min(total_rows as usize);
        let start_row = end_row.saturating_sub(max_rows);

        (start_row..end_row)
            .map(|r| self.row_cells(r as u16, max_cols))
            .collect()
    }

    /// Get visible rows with scroll offset for detail panel.
    /// scroll_back = 0 means current view (following cursor).
    /// scroll_back > 0 means scrolled N rows into history.
    /// Get visible rows with scrollback support.
    ///
    /// The caller must call `set_scrollback(N)` before this method if N > 0,
    /// and reset it to 0 after rendering. When scrollback > 0, vt100's
    /// `cell()` automatically returns content from the scrollback buffer,
    /// so we just read all visible rows from top to bottom.
    pub fn visible_rows_with_scroll(
        &self,
        max_rows: usize,
        max_cols: u16,
        scroll_back: usize,
    ) -> (usize, Vec<Vec<Cell>>) {
        if scroll_back == 0 {
            return self.visible_rows_with_cursor(max_rows, max_cols);
        }

        // Scrollback is pre-set by caller via set_scrollback().
        // vt100's visible_rows() already includes scrollback content.
        // Just read all rows from top.
        let total = self.parser.screen().size().0 as usize;
        let rows_to_show = max_rows.min(total);

        let rows = (0..rows_to_show)
            .map(|r| self.row_cells(r as u16, max_cols))
            .collect();

        (0, rows)
    }

    /// Get all visible rows for the detail panel, ensuring cursor is visible.
    /// Returns (start_row, rows).
    pub fn visible_rows_with_cursor(
        &self,
        max_rows: usize,
        max_cols: u16,
    ) -> (usize, Vec<Vec<Cell>>) {
        let screen = self.parser.screen();
        let (total_rows, _) = screen.size();
        let (cursor_row, _) = screen.cursor_position();
        let total = total_rows as usize;

        let start_row = if total <= max_rows || (cursor_row as usize) < max_rows {
            0
        } else {
            (cursor_row as usize + 1).saturating_sub(max_rows)
        };
        let end_row = (start_row + max_rows).min(total);

        let rows = (start_row..end_row)
            .map(|r| self.row_cells(r as u16, max_cols))
            .collect();

        (start_row, rows)
    }
}

/// Convert a vt100::Cell to our Cell type.
fn convert_cell(vt_cell: &vt100::Cell) -> Cell {
    let contents = vt_cell.contents();
    // Wide character continuation cell: contents is empty string.
    // Mark it so rendering can skip it.
    if contents.is_empty() {
        return Cell {
            ch: ' ',
            fg: convert_color(vt_cell.fgcolor()),
            bg: convert_color(vt_cell.bgcolor()),
            modifiers: Modifier::empty(),
            is_wide_continuation: true,
        };
    }
    let ch = contents.chars().next().unwrap_or(' ');
    let fg = convert_color(vt_cell.fgcolor());
    let bg = convert_color(vt_cell.bgcolor());
    let mut modifiers = Modifier::empty();
    if vt_cell.bold() {
        modifiers |= Modifier::BOLD;
    }
    if vt_cell.italic() {
        modifiers |= Modifier::ITALIC;
    }
    if vt_cell.underline() {
        modifiers |= Modifier::UNDERLINED;
    }
    if vt_cell.inverse() {
        modifiers |= Modifier::REVERSED;
    }
    Cell {
        ch,
        fg,
        bg,
        modifiers,
        is_wide_continuation: false,
    }
}

/// Convert vt100::Color to ratatui::style::Color.
fn convert_color(color: vt100::Color) -> Color {
    match color {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(i) => match i {
            0 => Color::Black,
            1 => Color::Red,
            2 => Color::Green,
            3 => Color::Yellow,
            4 => Color::Blue,
            5 => Color::Magenta,
            6 => Color::Cyan,
            7 => Color::Gray,
            8 => Color::DarkGray,
            9 => Color::LightRed,
            10 => Color::LightGreen,
            11 => Color::LightYellow,
            12 => Color::LightBlue,
            13 => Color::LightMagenta,
            14 => Color::LightCyan,
            15 => Color::White,
            _ => Color::Indexed(i),
        },
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_creates_correct_dimensions() {
        let vte = VteState::new(80, 24);
        assert_eq!(vte.cols(), 80);
        assert_eq!(vte.rows(), 24);
    }

    #[test]
    fn test_basic_text() {
        let mut vte = VteState::new(80, 24);
        vte.process(b"Hello");
        assert_eq!(vte.cell_at(0, 0).ch, 'H');
        assert_eq!(vte.cell_at(0, 1).ch, 'e');
        assert_eq!(vte.cell_at(0, 4).ch, 'o');
    }

    #[test]
    fn test_cursor_movement() {
        let mut vte = VteState::new(80, 24);
        vte.process(b"Hello");
        let (row, col) = vte.cursor_position();
        assert_eq!(row, 0);
        assert_eq!(col, 5);
    }

    #[test]
    fn test_colors() {
        let mut vte = VteState::new(80, 24);
        vte.process(b"\x1b[31mR\x1b[0m");
        assert_eq!(vte.cell_at(0, 0).fg, Color::Red);
    }

    #[test]
    fn test_resize() {
        let mut vte = VteState::new(80, 24);
        vte.resize(120, 40);
        assert_eq!(vte.cols(), 120);
        assert_eq!(vte.rows(), 40);
    }

    #[test]
    fn test_cursor_visible_by_default() {
        let vte = VteState::new(80, 24);
        assert!(vte.cursor_visible());
    }

    #[test]
    fn test_hide_cursor() {
        let mut vte = VteState::new(80, 24);
        vte.process(b"\x1b[?25l"); // hide cursor
        assert!(!vte.cursor_visible());
        vte.process(b"\x1b[?25h"); // show cursor
        assert!(vte.cursor_visible());
    }

    #[test]
    fn test_row_cells() {
        let mut vte = VteState::new(80, 24);
        vte.process(b"ABC");
        let row = vte.row_cells(0, 5);
        assert_eq!(row.len(), 5);
        assert_eq!(row[0].ch, 'A');
        assert_eq!(row[1].ch, 'B');
        assert_eq!(row[2].ch, 'C');
    }

    #[test]
    fn test_visible_rows_around_cursor() {
        let mut vte = VteState::new(80, 24);
        vte.process(b"line1\r\nline2\r\nline3");
        let rows = vte.visible_rows_around_cursor(2, 10);
        // Should return 2 rows ending at cursor row
        assert!(rows.len() <= 2);
    }

    #[test]
    fn test_bold_modifier() {
        let mut vte = VteState::new(80, 24);
        vte.process(b"\x1b[1mB\x1b[0m");
        let cell = vte.cell_at(0, 0);
        assert!(cell.modifiers.contains(Modifier::BOLD));
    }

    #[test]
    fn test_256_color() {
        let mut vte = VteState::new(80, 24);
        vte.process(b"\x1b[38;5;200mX\x1b[0m");
        let cell = vte.cell_at(0, 0);
        assert_eq!(cell.fg, Color::Indexed(200));
    }

    #[test]
    fn test_truecolor() {
        let mut vte = VteState::new(80, 24);
        vte.process(b"\x1b[38;2;100;150;200mX\x1b[0m");
        let cell = vte.cell_at(0, 0);
        assert_eq!(cell.fg, Color::Rgb(100, 150, 200));
    }

    #[test]
    fn test_cursor_position_after_move() {
        let mut vte = VteState::new(80, 24);
        vte.process(b"\x1b[5;10H"); // move to row 5, col 10 (1-indexed)
        let (row, col) = vte.cursor_position();
        assert_eq!(row, 4); // 0-indexed
        assert_eq!(col, 9); // 0-indexed
    }

    #[test]
    fn test_wide_char_chinese() {
        let mut vte = VteState::new(80, 24);
        vte.process("你好".as_bytes());
        // '你' occupies cols 0-1, '好' occupies cols 2-3
        let cell0 = vte.cell_at(0, 0);
        assert_eq!(cell0.ch, '你');
        assert!(!cell0.is_wide_continuation);
        // Col 1 is wide continuation
        let cell1 = vte.cell_at(0, 1);
        assert!(cell1.is_wide_continuation);
        // '好' at col 2
        let cell2 = vte.cell_at(0, 2);
        assert_eq!(cell2.ch, '好');
        assert!(!cell2.is_wide_continuation);
        // Col 3 is continuation
        let cell3 = vte.cell_at(0, 3);
        assert!(cell3.is_wide_continuation);
        // Cursor should be at col 4
        let (_, col) = vte.cursor_position();
        assert_eq!(col, 4);
    }

    #[test]
    fn test_wide_char_row_cells_skip() {
        let mut vte = VteState::new(80, 24);
        vte.process("A你B".as_bytes());
        let row = vte.row_cells(0, 10);
        // row[0]='A', row[1]='你', row[2]=continuation, row[3]='B'
        let non_cont: Vec<&Cell> = row.iter().filter(|c| !c.is_wide_continuation).collect();
        assert_eq!(non_cont[0].ch, 'A');
        assert_eq!(non_cont[1].ch, '你');
        assert_eq!(non_cont[2].ch, 'B');
    }

    #[test]
    fn test_alt_screen() {
        let mut vte = VteState::new(80, 24);
        vte.process(b"main screen");
        vte.process(b"\x1b[?1049h"); // enter alt screen
        vte.process(b"alt screen");
        vte.process(b"\x1b[?1049l"); // leave alt screen
                                     // After leaving alt screen, content should be back
        let cell = vte.cell_at(0, 0);
        assert_eq!(cell.ch, 'm'); // 'm' from "main screen"
    }

    #[test]
    fn test_scroll_back_changes_visible_rows() {
        let mut vte = VteState::new(20, 24);
        // Fill terminal with enough content to create scrollback history.
        // 50 lines in a 24-row terminal → 26+ lines go to scrollback buffer.
        for i in 0..50 {
            vte.process(format!("line{:02}\r\n", i).as_bytes());
        }

        // Without scroll: shows current screen (near cursor)
        let (_, rows_no_scroll) = vte.visible_rows_with_scroll(24, 20, 0);

        // With scroll_back=10: set scrollback, read, reset
        vte.set_scrollback(10);
        let (_, rows_scrolled) = vte.visible_rows_with_scroll(24, 20, 10);
        vte.set_scrollback(0);

        // Both should return full screens
        assert_eq!(rows_no_scroll.len(), 24);
        assert_eq!(rows_scrolled.len(), 24);

        // Content should differ — scrolled view shows older history
        let first_row_no_scroll: String = rows_no_scroll[0].iter().map(|c| c.ch).collect();
        let first_row_scrolled: String = rows_scrolled[0].iter().map(|c| c.ch).collect();
        assert_ne!(
            first_row_no_scroll, first_row_scrolled,
            "scrolled view should show different (older) content"
        );
    }

    #[test]
    fn test_scroll_back_at_maximum() {
        let mut vte = VteState::new(20, 24);
        // Fill with lots of content (100 lines → 76+ in scrollback)
        for i in 0..100 {
            vte.process(format!("L{:02}\r\n", i).as_bytes());
        }

        // Scroll back maximum (clamped to screen height = 24)
        // This replaces all 24 screen rows with the 24 most recent scrollback rows
        vte.set_scrollback(10000); // clamped to 24 internally
        let (start, rows) = vte.visible_rows_with_scroll(24, 20, 24);
        vte.set_scrollback(0);

        assert_eq!(start, 0);
        assert_eq!(rows.len(), 24, "should return full screen of scrollback rows");

        // Content should be from history, not current screen
        let first_content: String = rows[0].iter().map(|c| c.ch).collect();
        let no_scroll_rows = vte.visible_rows_with_scroll(24, 20, 0).1;
        let first_no_scroll: String = no_scroll_rows[0].iter().map(|c| c.ch).collect();
        assert_ne!(
            first_content, first_no_scroll,
            "max scrollback should show different content than current screen"
        );
    }
}
