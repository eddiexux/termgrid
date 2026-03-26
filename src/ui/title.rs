use crate::tile::{Tile, TileStatus};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// Build the standard title line used by both tile card and detail panel.
pub fn build_title_line(tile: &Tile, index_label: Option<&str>) -> Line<'static> {
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
