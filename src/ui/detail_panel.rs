use crate::tile::Tile;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    symbols,
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph},
    Frame,
};
/// Render result: cursor position + actual terminal area dimensions for PTY sync.
pub struct DetailRenderResult {
    pub cursor_pos: Option<(u16, u16)>,
    /// Actual terminal area dimensions (cols, rows) for PTY size synchronization.
    pub terminal_size: (u16, u16),
}

/// Render the detail panel. Returns cursor position and actual terminal area size.
pub fn render(frame: &mut Frame, area: Rect, tile: &Tile, index_label: Option<&str>) -> DetailRenderResult {
    // Render the outer block with left border as vertical separator
    let block = Block::default()
        .borders(Borders::LEFT)
        .border_set(symbols::border::PLAIN)
        .border_style(Style::default().fg(Color::DarkGray));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 || inner.width == 0 {
        return DetailRenderResult { cursor_pos: None, terminal_size: (0, 0) };
    }

    // Split: header lines + separator + terminal area
    let header_height = crate::layout::DETAIL_HEADER_HEIGHT;
    if inner.height <= header_height {
        return DetailRenderResult { cursor_pos: None, terminal_size: (0, 0) };
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(header_height),
            Constraint::Min(0),
        ])
        .split(inner);

    let header_area = chunks[0];
    let terminal_area = chunks[1];

    // Build header
    let mut header_lines: Vec<Line> = Vec::new();

    // Line 1: unified title (same format as tile card)
    header_lines.push(super::title::build_title_line(tile, index_label));

    // Line 2: keyboard hints
    header_lines.push(Line::from(vec![Span::styled(
        "Esc close │ ↑↓ switch │ i insert",
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
        let cols = terminal_area.width as usize;
        let screen = &tile.vte.screen;
        let visible = screen.visible_lines();
        let total_visible = visible.len();
        let cursor_row = screen.cursor.row.min(total_visible.saturating_sub(1));

        // Ensure cursor is within the visible window
        let start_row = if total_visible <= rows {
            0 // Everything fits
        } else if cursor_row < rows {
            0 // Cursor near top — show from top
        } else {
            // Cursor below the first screenful — scroll to keep cursor visible
            (cursor_row + 1).saturating_sub(rows)
        };
        let end_row = (start_row + rows).min(total_visible);

        let slice: Vec<&[crate::screen::Cell]> = visible[start_row..end_row]
            .iter()
            .map(|row| row.as_slice())
            .collect();
        let text_lines = super::screen_rows_to_lines(&slice, cols);

        let terminal_para = Paragraph::new(Text::from(text_lines));
        frame.render_widget(terminal_para, terminal_area);

        // Compute cursor screen position
        let cursor = &screen.cursor;
        if cursor.visible && cursor.row >= start_row {
            let screen_row = (cursor.row - start_row) as u16;
            let screen_col = cursor.col as u16;
            if screen_row < terminal_area.height && screen_col < terminal_area.width {
                cursor_pos = Some((
                    terminal_area.x + screen_col,
                    terminal_area.y + screen_row,
                ));
            }
        }
    }
    DetailRenderResult {
        cursor_pos,
        terminal_size: (terminal_area.width, terminal_area.height),
    }
}
