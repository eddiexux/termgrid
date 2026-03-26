use crate::tab::{TabEntry, TabFilter};
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

pub fn render(
    frame: &mut Frame,
    area: Rect,
    entries: &[TabEntry],
    active: &TabFilter,
    total_count: usize,
) {
    let mut spans = Vec::new();

    // "ALL(N)" tab
    let all_label = format!(" ALL({}) ", total_count);
    let all_is_active = matches!(active, TabFilter::All);
    if all_is_active {
        spans.push(Span::styled(
            all_label,
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));
    } else {
        spans.push(Span::styled(all_label, Style::default().fg(Color::Gray)));
    }

    // Project and Other tabs
    for entry in entries {
        let is_active = match active {
            TabFilter::Project(name) => name == &entry.label,
            TabFilter::Other => entry.label == "Other",
            TabFilter::All => false,
        };

        let label = format!(" {}({}) ", entry.label, entry.count);
        if is_active {
            spans.push(Span::styled(
                label,
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ));
        } else {
            spans.push(Span::styled(label, Style::default().fg(Color::Gray)));
        }
    }

    let line = Line::from(spans);
    let block = Block::default()
        .borders(Borders::BOTTOM)
        .border_style(Style::default().fg(Color::DarkGray));
    let para = Paragraph::new(line)
        .style(Style::default().bg(Color::Indexed(236))) // dark background
        .block(block);
    frame.render_widget(para, area);

    // Render buttons at top-right: " [+] [X] "
    let buttons = vec![
        (" [+] ", Color::Green),
        (" [X] ", Color::Red),
    ];
    let total_btn_width: u16 = buttons.iter().map(|(s, _)| s.len() as u16).sum();
    if area.width >= total_btn_width {
        let mut btn_x = area.x + area.width - total_btn_width;
        for (label, color) in buttons {
            let btn_width = label.len() as u16;
            let btn_area = Rect {
                x: btn_x,
                y: area.y,
                width: btn_width,
                height: 1,
            };
            let btn = Paragraph::new(Span::styled(
                label,
                Style::default()
                    .fg(color)
                    .bg(Color::Indexed(236))
                    .add_modifier(Modifier::BOLD),
            ));
            frame.render_widget(btn, btn_area);
            btn_x += btn_width;
        }
    }
}
