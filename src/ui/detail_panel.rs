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
        let cols = terminal_area.width as u16;
        let screen = &tile.vte;
        let (cursor_row, cursor_col) = screen.cursor_position();

        let (start_row, row_cells) = screen.visible_rows_with_cursor(rows, cols);

        let text_lines: Vec<Line> = row_cells
            .iter()
            .map(|row| {
                let spans: Vec<Span> = row
                    .iter()
                    .filter(|cell| !cell.is_wide_continuation)
                    .map(|cell| {
                        Span::styled(
                            cell.ch.to_string(),
                            Style::default()
                                .fg(cell.fg)
                                .bg(cell.bg)
                                .add_modifier(cell.modifiers),
                        )
                    })
                    .collect();
                Line::from(spans)
            })
            .collect();

        let terminal_para = Paragraph::new(Text::from(text_lines));
        frame.render_widget(terminal_para, terminal_area);

        // Compute cursor screen position
        if screen.cursor_visible() && cursor_row as usize >= start_row {
            let screen_row = (cursor_row as usize - start_row) as u16;
            if screen_row < terminal_area.height && cursor_col < terminal_area.width {
                cursor_pos = Some((
                    terminal_area.x + cursor_col,
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
