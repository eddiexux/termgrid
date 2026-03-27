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
    let border_color = if tile.has_unread {
        Color::Yellow
    } else if tile.is_claude_code() {
        Color::Magenta
    } else {
        Color::DarkGray
    };

    let border_type = if is_selected {
        ratatui::widgets::BorderType::Double
    } else {
        ratatui::widgets::BorderType::Plain
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(border_type)
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

    // Render close button at right end of title area
    if title_area.width >= 3 {
        let close_area = Rect {
            x: title_area.x + title_area.width - 1,
            y: title_area.y,
            width: 1,
            height: 1,
        };
        let close_btn = Paragraph::new(Span::styled(
            "\u{00d7}", // × symbol
            Style::default().fg(Color::Red),
        ));
        frame.render_widget(close_btn, close_area);
    }

    // Render screen buffer preview using the same logic as the detail panel.
    // The VTE is sized to the detail panel dimensions; the tile card just shows
    // the bottom portion of that same view (a cropped preview).
    let mut cursor_pos = None;
    if preview_area.height > 0 {
        let preview_height = preview_area.height as usize;
        let preview_width = preview_area.width;
        let screen = &tile.vte;
        let (cursor_row, cursor_col) = screen.cursor_position();

        // Use the same visible_rows_with_cursor as the detail panel
        let (start_row, row_cells) =
            screen.visible_rows_with_cursor(preview_height, preview_width);

        let cursor_visible = is_selected;

        let text_lines: Vec<Line> = row_cells
            .iter()
            .enumerate()
            .map(|(display_row, row)| {
                let grid_row = start_row + display_row;
                let spans: Vec<Span> = row
                    .iter()
                    .enumerate()
                    .filter_map(|(col_idx, cell)| {
                        if cell.is_wide_continuation {
                            return None;
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
