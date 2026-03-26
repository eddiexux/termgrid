use crate::tile::Tile;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

/// Render a tile card. Returns cursor screen position if the tile has a visible cursor.
/// `index_label` is shown when multiple tiles share the same project/directory (e.g. "[2]").
pub fn render(
    frame: &mut Frame,
    area: Rect,
    tile: &Tile,
    is_selected: bool,
    index_label: Option<&str>,
) -> Option<(u16, u16)> {
    let border_color = if is_selected {
        Color::Cyan
    } else if tile.has_unread {
        Color::Yellow
    } else {
        Color::DarkGray
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 || inner.width == 0 {
        return None;
    }

    // Split inner area: first line for title, rest for preview
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(inner);

    let title_area = chunks[0];
    let preview_area = chunks[1];

    // Build title line
    let title_line = super::title::build_title_line(tile, index_label);
    let title_para = Paragraph::new(title_line);
    frame.render_widget(title_para, title_area);

    // Render screen buffer preview — show lines around the cursor, not the buffer bottom.
    // The PTY is sized to the detail panel (e.g. 30 rows), but the small tile only has ~8 rows.
    // If we take "last 8 lines" we'd get blank rows because the shell prompt is near the top.
    // Instead, show lines ending at the cursor row so the active content is always visible.
    let mut cursor_pos = None;
    if preview_area.height > 0 {
        let preview_height = preview_area.height as usize;
        let preview_width = preview_area.width;
        let screen = &tile.vte;
        let (cursor_row, cursor_col) = screen.cursor_position();
        let total_rows = screen.rows() as usize;

        let end_row = (cursor_row as usize + 1).min(total_rows);
        let start_row = end_row.saturating_sub(preview_height);

        // Visual cursor in tile card: always show on selected tile.
        // Don't depend on terminal cursor visibility state (shell may temporarily
        // hide cursor during prompt rendering, causing flickering).
        let cursor_visible = is_selected;

        let text_lines: Vec<Line> = (start_row..end_row)
            .enumerate()
            .map(|(display_idx, grid_row)| {
                let row_cells = screen.row_cells(grid_row as u16, preview_width);
                let spans: Vec<Span> = row_cells
                    .iter()
                    .enumerate()
                    .filter_map(|(col_idx, cell)| {
                        if cell.is_wide_continuation {
                            return None; // skip, but col_idx still tracks the real column
                        }
                        let is_cursor = cursor_visible
                            && grid_row == cursor_row as usize
                            && col_idx == cursor_col as usize;
                        let style = if is_cursor {
                            Style::default()
                                .fg(cell.bg)
                                .bg(Color::White)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default()
                                .fg(cell.fg)
                                .bg(cell.bg)
                                .add_modifier(cell.modifiers)
                        };
                        Some(Span::styled(cell.ch.to_string(), style))
                    })
                    .collect();
                let _ = display_idx;
                Line::from(spans)
            })
            .collect();

        let preview_para = Paragraph::new(Text::from(text_lines));
        frame.render_widget(preview_para, preview_area);

        // Compute cursor position in the preview area (for hardware cursor fallback)
        if is_selected && cursor_row as usize >= start_row && cursor_col < preview_width {
            let screen_row = (cursor_row as usize - start_row) as u16;
            if screen_row < preview_area.height {
                cursor_pos = Some((preview_area.x + cursor_col, preview_area.y + screen_row));
            }
        }
    }
    cursor_pos
}
