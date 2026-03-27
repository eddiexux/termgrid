use crate::app::TextSelection;
use crate::tile::Tile;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    symbols,
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph},
    Frame,
};
/// Check if a screen coordinate falls within a text selection.
pub fn selection_contains(
    selection: Option<&TextSelection>,
    screen_x: u16,
    screen_y: u16,
) -> bool {
    let Some(sel) = selection else {
        return false;
    };
    let (sx, sy) = sel.start;
    let (ex, ey) = sel.end;
    let (min_y, max_y) = if sy <= ey { (sy, ey) } else { (ey, sy) };
    let (min_x_first, max_x_last) = if sy <= ey { (sx, ex) } else { (ex, sx) };
    if screen_y < min_y || screen_y > max_y {
        return false;
    }
    if screen_y == min_y && screen_y == max_y {
        let min_x = min_x_first.min(max_x_last);
        let max_x = min_x_first.max(max_x_last);
        return screen_x >= min_x && screen_x <= max_x;
    }
    if screen_y == min_y {
        return screen_x >= min_x_first;
    }
    if screen_y == max_y {
        return screen_x <= max_x_last;
    }
    true // middle rows fully selected
}

/// Render result: cursor position + actual terminal area dimensions for PTY sync.
pub struct DetailRenderResult {
    pub cursor_pos: Option<(u16, u16)>,
    /// Actual terminal area dimensions (cols, rows) for PTY size synchronization.
    pub terminal_size: (u16, u16),
}

/// Render the detail panel. Returns cursor position and actual terminal area size.
/// When `scrollback_rows` is provided (non-empty), it overrides the VTE scrollback
/// and renders from the replay-based full history instead.
pub fn render(
    frame: &mut Frame,
    area: Rect,
    tile: &Tile,
    index_label: Option<&str>,
    selection: Option<&TextSelection>,
    scroll_back: usize,
    scrollback_rows: Option<&[Vec<crate::screen::Cell>]>,
) -> DetailRenderResult {
    // Match border color to tile card: yellow for unread, magenta for Claude Code, gray otherwise
    let border_color = if tile.has_unread {
        Color::Yellow
    } else if tile.is_claude_code() {
        Color::Magenta
    } else {
        Color::DarkGray
    };

    // Render the outer block with left border as vertical separator
    let block = Block::default()
        .borders(Borders::LEFT)
        .border_set(symbols::border::PLAIN)
        .border_style(Style::default().fg(border_color));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 || inner.width == 0 {
        return DetailRenderResult {
            cursor_pos: None,
            terminal_size: (0, 0),
        };
    }

    // Split: header lines + separator + terminal area
    let header_height = crate::layout::DETAIL_HEADER_HEIGHT;
    if inner.height <= header_height {
        return DetailRenderResult {
            cursor_pos: None,
            terminal_size: (0, 0),
        };
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(header_height), Constraint::Min(0)])
        .split(inner);

    let header_area = chunks[0];
    let terminal_area = chunks[1];

    // Build header
    let mut header_lines: Vec<Line> = Vec::new();

    // Line 1: unified title (same format as tile card)
    header_lines.push(super::title::build_title_line(tile, index_label));

    // Line 2: keyboard hints (with scroll indicator if scrolled)
    let hint_text = if scroll_back > 0 {
        format!(
            "Esc close │ ↑↓ switch │ i insert  [SCROLL -{}]",
            scroll_back
        )
    } else {
        "Esc close │ ↑↓ switch │ i insert".to_string()
    };
    header_lines.push(Line::from(vec![Span::styled(
        hint_text,
        Style::default().fg(Color::DarkGray),
    )]));

    // Line 3: separator
    let sep = "─".repeat(inner.width as usize);
    header_lines.push(Line::from(vec![Span::styled(
        sep,
        Style::default().fg(Color::DarkGray),
    )]));

    let header_para = Paragraph::new(Text::from(header_lines));
    frame.render_widget(header_para, header_area);

    // Render full terminal area and compute cursor position.
    // Show the region that includes the cursor so it's always visible.
    let mut cursor_pos = None;
    if terminal_area.height > 0 {
        let rows = terminal_area.height as usize;
        let cols = terminal_area.width;
        let screen = &tile.vte;
        let (cursor_row, cursor_col) = screen.cursor_position();

        // When scrollback_rows is provided, use the replay-based history
        // to render a window at the given scroll offset.
        let (start_row, row_cells) = if let Some(all_lines) = scrollback_rows {
            // all_lines contains the full history. We want to show a window
            // ending at (total - scroll_back) so that scroll_back=0 shows the bottom.
            let total = all_lines.len();
            let end = total.saturating_sub(scroll_back);
            let start = end.saturating_sub(rows);
            let window: Vec<Vec<crate::screen::Cell>> = all_lines
                [start..end.min(total)]
                .to_vec();
            (start, window)
        } else {
            screen.visible_rows_with_scroll(rows, cols, scroll_back)
        };

        let is_selected = |screen_x: u16, screen_y: u16| -> bool {
            selection_contains(selection, screen_x, screen_y)
        };

        let text_lines: Vec<Line> = row_cells
            .iter()
            .enumerate()
            .map(|(display_row, row)| {
                let cell_screen_y = terminal_area.y + display_row as u16;
                let spans: Vec<Span> = row
                    .iter()
                    .enumerate()
                    .filter(|(_, cell)| !cell.is_wide_continuation)
                    .map(|(col_offset, cell)| {
                        let cell_screen_x = terminal_area.x + col_offset as u16;
                        let highlight = is_selected(cell_screen_x, cell_screen_y);
                        let style = if highlight {
                            Style::default().fg(Color::Black).bg(Color::LightBlue)
                        } else {
                            Style::default()
                                .fg(cell.fg)
                                .bg(cell.bg)
                                .add_modifier(cell.modifiers)
                        };
                        Span::styled(cell.ch.to_string(), style)
                    })
                    .collect();
                Line::from(spans)
            })
            .collect();

        let terminal_para = Paragraph::new(Text::from(text_lines));
        frame.render_widget(terminal_para, terminal_area);

        // Compute cursor screen position when not scrolled into history.
        // Don't gate on cursor_visible() — shells temporarily hide cursor during
        // prompt rendering, which would make cursor_pos None and break Insert mode.
        // The caller (ui/mod.rs) decides when to actually display the hardware cursor.
        // When scroll_back > 0, user is viewing history — don't show cursor.
        if scroll_back == 0 && cursor_row as usize >= start_row {
            let screen_row = (cursor_row as usize - start_row) as u16;
            if screen_row < terminal_area.height && cursor_col < terminal_area.width {
                cursor_pos = Some((terminal_area.x + cursor_col, terminal_area.y + screen_row));
            }
        }
    }
    DetailRenderResult {
        cursor_pos,
        terminal_size: (terminal_area.width, terminal_area.height),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::SelectionMode;

    fn make_selection(start: (u16, u16), end: (u16, u16)) -> TextSelection {
        TextSelection {
            start,
            end,
            mode: SelectionMode::Char,
            anchor_start: start,
            anchor_end: end,
        }
    }

    #[test]
    fn test_selection_contains_no_selection() {
        assert!(!selection_contains(None, 5, 5));
    }

    #[test]
    fn test_selection_contains_single_line() {
        let sel = make_selection((2, 5), (8, 5));
        assert!(!selection_contains(Some(&sel), 1, 5)); // before
        assert!(selection_contains(Some(&sel), 2, 5)); // start
        assert!(selection_contains(Some(&sel), 5, 5)); // middle
        assert!(selection_contains(Some(&sel), 8, 5)); // end
        assert!(!selection_contains(Some(&sel), 9, 5)); // after
        assert!(!selection_contains(Some(&sel), 5, 4)); // wrong row
        assert!(!selection_contains(Some(&sel), 5, 6)); // wrong row
    }

    #[test]
    fn test_selection_contains_multi_line() {
        // Selection from (3, 2) to (7, 4)
        let sel = make_selection((3, 2), (7, 4));
        // Row 2: x >= 3 is selected
        assert!(!selection_contains(Some(&sel), 2, 2));
        assert!(selection_contains(Some(&sel), 3, 2));
        assert!(selection_contains(Some(&sel), 50, 2));
        // Row 3: fully selected (middle row)
        assert!(selection_contains(Some(&sel), 0, 3));
        assert!(selection_contains(Some(&sel), 100, 3));
        // Row 4: x <= 7 is selected
        assert!(selection_contains(Some(&sel), 0, 4));
        assert!(selection_contains(Some(&sel), 7, 4));
        assert!(!selection_contains(Some(&sel), 8, 4));
        // Outside rows
        assert!(!selection_contains(Some(&sel), 5, 1));
        assert!(!selection_contains(Some(&sel), 5, 5));
    }

    #[test]
    fn test_selection_contains_reversed() {
        // Selection dragged upward: end is above start
        let sel = make_selection((7, 4), (3, 2));
        // Should work the same as forward selection
        assert!(selection_contains(Some(&sel), 3, 2));
        assert!(selection_contains(Some(&sel), 50, 2));
        assert!(selection_contains(Some(&sel), 5, 3));
        assert!(selection_contains(Some(&sel), 7, 4));
        assert!(!selection_contains(Some(&sel), 8, 4));
    }

    #[test]
    fn test_selection_contains_single_line_reversed() {
        // Same line but end < start (dragged left)
        let sel = make_selection((8, 5), (2, 5));
        assert!(selection_contains(Some(&sel), 2, 5));
        assert!(selection_contains(Some(&sel), 5, 5));
        assert!(selection_contains(Some(&sel), 8, 5));
        assert!(!selection_contains(Some(&sel), 1, 5));
        assert!(!selection_contains(Some(&sel), 9, 5));
    }
}
