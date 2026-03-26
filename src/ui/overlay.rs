use crate::app::OverlayKind;
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
    Frame,
};

pub fn render(frame: &mut Frame, area: Rect, overlay: &OverlayKind) {
    match overlay {
        OverlayKind::Help => render_help(frame, area),
        OverlayKind::ConfirmClose(tile_id) => render_confirm_close(frame, area, *tile_id),
        OverlayKind::ProjectSelector {
            query,
            items,
            selected,
        } => render_project_selector(frame, area, query, items, *selected),
    }
}

fn render_help(frame: &mut Frame, area: Rect) {
    let width = 50u16.min(area.width.saturating_sub(4));
    let height = 20u16.min(area.height.saturating_sub(4));
    let popup = centered_rect(width, height, area);

    frame.render_widget(Clear, popup);

    let block = Block::default()
        .title(" Help ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .style(Style::default().bg(Color::Black));

    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let help_text = vec![
        Line::from(vec![Span::styled(
            "  Navigation",
            Style::default().add_modifier(Modifier::BOLD),
        )]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  h/j/k/l  ", Style::default().fg(Color::Cyan)),
            Span::raw("Move selection"),
        ]),
        Line::from(vec![
            Span::styled("  Tab      ", Style::default().fg(Color::Cyan)),
            Span::raw("Next tab"),
        ]),
        Line::from(vec![
            Span::styled("  Shift+Tab", Style::default().fg(Color::Cyan)),
            Span::raw("Previous tab"),
        ]),
        Line::from(""),
        Line::from(vec![Span::styled(
            "  Actions",
            Style::default().add_modifier(Modifier::BOLD),
        )]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Enter    ", Style::default().fg(Color::Cyan)),
            Span::raw("Focus selected tile"),
        ]),
        Line::from(vec![
            Span::styled("  n        ", Style::default().fg(Color::Cyan)),
            Span::raw("New tile"),
        ]),
        Line::from(vec![
            Span::styled("  x        ", Style::default().fg(Color::Cyan)),
            Span::raw("Close tile"),
        ]),
        Line::from(vec![
            Span::styled("  +/-      ", Style::default().fg(Color::Cyan)),
            Span::raw("Increase/decrease columns"),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  ?        ", Style::default().fg(Color::Cyan)),
            Span::raw("Toggle this help"),
        ]),
        Line::from(vec![
            Span::styled("  q / Esc  ", Style::default().fg(Color::Cyan)),
            Span::raw("Quit"),
        ]),
    ];

    let para = Paragraph::new(Text::from(help_text));
    frame.render_widget(para, inner);
}

fn render_confirm_close(frame: &mut Frame, area: Rect, _tile_id: crate::tile::TileId) {
    let width = 40u16.min(area.width.saturating_sub(4));
    let height = 5u16.min(area.height.saturating_sub(4));
    let popup = centered_rect(width, height, area);

    frame.render_widget(Clear, popup);

    let block = Block::default()
        .title(" Confirm Close ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow))
        .style(Style::default().bg(Color::Black));

    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let text = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw("  Process running. Close? "),
            Span::styled("(y/n)", Style::default().fg(Color::Yellow)),
        ]),
    ];

    let para = Paragraph::new(Text::from(text));
    frame.render_widget(para, inner);
}

fn render_project_selector(
    frame: &mut Frame,
    area: Rect,
    query: &str,
    items: &[String],
    selected: usize,
) {
    let width = 60u16.min(area.width.saturating_sub(4));
    let height = 20u16.min(area.height.saturating_sub(4));
    let popup = centered_rect(width, height, area);

    frame.render_widget(Clear, popup);

    let title = format!(" Project Selector: {} ", query);
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .style(Style::default().bg(Color::Black));

    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    if inner.height == 0 {
        return;
    }

    let list_items: Vec<ListItem> = items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            if i == selected {
                ListItem::new(Line::from(vec![Span::styled(
                    format!("> {}", item),
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )]))
            } else {
                ListItem::new(Line::from(vec![Span::styled(
                    format!("  {}", item),
                    Style::default().fg(Color::White),
                )]))
            }
        })
        .collect();

    let list = List::new(list_items);
    frame.render_widget(list, inner);
}

/// Compute a centered rectangle of given width and height within the parent area.
pub fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect {
        x,
        y,
        width: width.min(area.width),
        height: height.min(area.height),
    }
}
