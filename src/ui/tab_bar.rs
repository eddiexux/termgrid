use crate::tab::{TabEntry, TabFilter};
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
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
    let para = Paragraph::new(line);
    frame.render_widget(para, area);
}
