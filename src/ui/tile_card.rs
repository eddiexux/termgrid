use crate::tile::{Tile, TileStatus};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

/// Render a tile card. Returns cursor screen position if the tile has a visible cursor.
/// `index_label` is shown when multiple tiles share the same project/directory (e.g. "[2]").
pub fn render(frame: &mut Frame, area: Rect, tile: &Tile, is_selected: bool, index_label: Option<&str>) -> Option<(u16, u16)> {
    let border_color = if is_selected {
        Color::Cyan
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
    let title_line = build_title_line(tile, index_label);
    let title_para = Paragraph::new(title_line);
    frame.render_widget(title_para, title_area);

    // Render screen buffer preview — show lines around the cursor, not the buffer bottom.
    // The PTY is sized to the detail panel (e.g. 30 rows), but the small tile only has ~8 rows.
    // If we take "last 8 lines" we'd get blank rows because the shell prompt is near the top.
    // Instead, show lines ending at the cursor row so the active content is always visible.
    let mut cursor_pos = None;
    if preview_area.height > 0 {
        let preview_height = preview_area.height as usize;
        let preview_width = preview_area.width as usize;
        let screen = &tile.vte.screen;
        let visible = screen.visible_lines();
        let total_visible = visible.len();
        let cursor_row = screen.cursor.row.min(total_visible.saturating_sub(1));
        let end_row = (cursor_row + 1).min(total_visible);
        let start_row = end_row.saturating_sub(preview_height);

        let cursor = &screen.cursor;
        // Only show visual cursor on the selected tile
        let cursor_visible = is_selected && cursor.visible;
        let cursor_grid_row = cursor.row;
        let cursor_grid_col = cursor.col;

        let text_lines: Vec<Line> = visible[start_row..end_row]
            .iter()
            .enumerate()
            .map(|(line_idx, row)| {
                let grid_row = start_row + line_idx;
                let spans: Vec<Span> = row
                    .iter()
                    .take(preview_width)
                    .enumerate()
                    .map(|(col_idx, cell)| {
                        let is_cursor = cursor_visible
                            && grid_row == cursor_grid_row
                            && col_idx == cursor_grid_col;
                        let style = if is_cursor {
                            // Visual cursor: reversed colors
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
                        Span::styled(cell.ch.to_string(), style)
                    })
                    .collect();
                Line::from(spans)
            })
            .collect();

        let preview_para = Paragraph::new(Text::from(text_lines));
        frame.render_widget(preview_para, preview_area);

        // Compute cursor position in the preview area
        let cursor = &screen.cursor;
        if cursor.visible && cursor.row >= start_row && cursor.col < preview_width {
            let screen_row = (cursor.row - start_row) as u16;
            if screen_row < preview_area.height {
                cursor_pos = Some((
                    preview_area.x + cursor.col as u16,
                    preview_area.y + screen_row,
                ));
            }
        }
    }
    cursor_pos
}

fn build_title_line(tile: &Tile, index_label: Option<&str>) -> Line<'static> {
    let mut spans = Vec::new();

    // Status tag
    let (status_label, status_color) = match &tile.status {
        TileStatus::Running => ("▶ RUN", Color::Green),
        TileStatus::Waiting => ("◉ WAIT", Color::Yellow),
        TileStatus::Idle(_) => ("◌ IDLE", Color::DarkGray),
        TileStatus::Exited => ("✕ EXIT", Color::Red),
        TileStatus::Error(_) => ("! ERR", Color::Red),
    };
    spans.push(Span::styled(
        format!("[{}] ", status_label),
        Style::default().fg(status_color),
    ));

    // Index label for disambiguation (e.g. "[2]")
    if let Some(label) = index_label {
        spans.push(Span::styled(
            format!("{} ", label),
            Style::default().fg(Color::DarkGray),
        ));
    }

    // Project name
    if let Some(ref git_ctx) = tile.git_context {
        spans.push(Span::styled(
            git_ctx.project_name.clone(),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ));

        // Branch tag
        if let Some(ref branch) = git_ctx.branch {
            spans.push(Span::styled(
                format!(" ⑂{}", branch),
                Style::default().fg(Color::Blue),
            ));
        }

        // Worktree tag
        if git_ctx.is_worktree {
            if let Some(ref wt_name) = git_ctx.worktree_name {
                spans.push(Span::styled(
                    format!(" ⑃{}", wt_name),
                    Style::default().fg(Color::Magenta),
                ));
            }
        }

        spans.push(Span::raw(" "));
    }

    // Path (gray)
    let path_str = tile.cwd.display().to_string();
    spans.push(Span::styled(path_str, Style::default().fg(Color::Gray)));

    Line::from(spans)
}
