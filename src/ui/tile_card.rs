use crate::tile::{Tile, TileStatus};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

pub fn render(frame: &mut Frame, area: Rect, tile: &Tile, is_selected: bool) {
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
        return;
    }

    // Split inner area: first line for title, rest for preview
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(inner);

    let title_area = chunks[0];
    let preview_area = chunks[1];

    // Build title line
    let title_line = build_title_line(tile);
    let title_para = Paragraph::new(title_line);
    frame.render_widget(title_para, title_area);

    // Render screen buffer preview
    if preview_area.height > 0 {
        let preview_height = preview_area.height as usize;
        let lines = tile.vte.screen.last_n_lines(preview_height);
        let preview_width = preview_area.width as usize;

        let text_lines: Vec<Line> = lines
            .iter()
            .map(|row| {
                let spans: Vec<Span> = row
                    .iter()
                    .take(preview_width)
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

        let preview_para = Paragraph::new(Text::from(text_lines));
        frame.render_widget(preview_para, preview_area);
    }
}

fn build_title_line(tile: &Tile) -> Line<'static> {
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
