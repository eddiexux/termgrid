use crate::tile::Tile;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols,
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

/// Render the detail panel. Returns cursor screen position if cursor should be shown.
pub fn render(frame: &mut Frame, area: Rect, tile: &Tile) -> Option<(u16, u16)> {
    // Render the outer block with left border as vertical separator
    let block = Block::default()
        .borders(Borders::LEFT)
        .border_set(symbols::border::PLAIN)
        .border_style(Style::default().fg(Color::DarkGray));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 || inner.width == 0 {
        return None;
    }

    // Split: header lines + separator + terminal area
    let header_height = 3u16; // project line + path/branch line + separator
    if inner.height <= header_height {
        return None;
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

    // Line 1: project name + hints
    let mut project_spans = Vec::new();
    if let Some(ref git_ctx) = tile.git_context {
        project_spans.push(Span::styled(
            git_ctx.project_name.clone(),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ));
        if let Some(ref branch) = git_ctx.branch {
            project_spans.push(Span::styled(
                format!("  ⑂ {}", branch),
                Style::default().fg(Color::Blue),
            ));
        }
        if git_ctx.is_worktree {
            if let Some(ref wt_name) = git_ctx.worktree_name {
                project_spans.push(Span::styled(
                    format!("  ⑃ {}", wt_name),
                    Style::default().fg(Color::Magenta),
                ));
            }
        }
    } else {
        project_spans.push(Span::styled(
            "(no project)",
            Style::default().fg(Color::DarkGray),
        ));
    }
    header_lines.push(Line::from(project_spans));

    // Line 2: path
    let path_str = tile.cwd.display().to_string();
    header_lines.push(Line::from(vec![Span::styled(
        path_str,
        Style::default().fg(Color::Gray),
    )]));

    // Line 3: separator
    let sep = "─".repeat(inner.width as usize);
    header_lines.push(Line::from(vec![Span::styled(
        sep,
        Style::default().fg(Color::DarkGray),
    )]));

    let header_para = Paragraph::new(Text::from(header_lines));
    frame.render_widget(header_para, header_area);

    // Render full terminal area and compute cursor position
    let mut cursor_pos = None;
    if terminal_area.height > 0 {
        let rows = terminal_area.height as usize;
        let cols = terminal_area.width as usize;
        let screen = &tile.vte.screen;
        let visible = screen.visible_lines();
        let total_visible = visible.len();
        let start_row = total_visible.saturating_sub(rows);

        let text_lines: Vec<Line> = visible[start_row..]
            .iter()
            .map(|row| {
                let spans: Vec<Span> = row
                    .iter()
                    .take(cols)
                    .map(|cell| {
                        let style = Style::default()
                            .fg(cell.fg)
                            .bg(cell.bg)
                            .add_modifier(cell.modifiers);
                        Span::styled(cell.ch.to_string(), style)
                    })
                    .collect();
                Line::from(spans)
            })
            .collect();

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
    cursor_pos
}
