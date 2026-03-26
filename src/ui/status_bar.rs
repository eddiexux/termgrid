use crate::app::AppMode;
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

pub fn render(frame: &mut Frame, area: Rect, mode: &AppMode, session_count: usize, columns: u8) {
    let mut spans = Vec::new();

    // Mode tag
    let (mode_label, mode_color) = match mode {
        AppMode::Normal => (" Normal ", Color::Cyan),
        AppMode::Insert => (" Insert ", Color::Green),
        AppMode::Overlay(_) => (" Overlay ", Color::Yellow),
    };
    spans.push(Span::styled(
        mode_label,
        Style::default()
            .fg(Color::Black)
            .bg(mode_color)
            .add_modifier(Modifier::BOLD),
    ));

    let info = format!(
        " termgrid | {} sessions | {} cols | ?help",
        session_count, columns
    );
    spans.push(Span::styled(info, Style::default().fg(Color::Gray)));

    let line = Line::from(spans);
    let para = Paragraph::new(line);
    frame.render_widget(para, area);
}
