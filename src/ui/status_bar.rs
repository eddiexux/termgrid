use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

pub fn render(frame: &mut Frame, area: Rect, session_count: usize, columns: u8) {
    let info = format!(" termgrid | {} sessions ", session_count);
    let spans = vec![Span::styled(info, Style::default().fg(Color::Gray))];

    let line = Line::from(spans);
    let para = Paragraph::new(line);
    frame.render_widget(para, area);

    // Render buttons at right side: " [?] [Ncol] "
    let col_label = format!(" [{}col] ", columns);
    let buttons: Vec<(&str, Color)> = vec![
        (" [?] ", Color::Yellow),
    ];

    let total_btn_width: u16 =
        buttons.iter().map(|(s, _)| s.len() as u16).sum::<u16>() + col_label.len() as u16;

    if area.width >= total_btn_width {
        let mut btn_x = area.x + area.width - total_btn_width;
        for (label, color) in &buttons {
            let btn_width = label.len() as u16;
            let btn_area = Rect {
                x: btn_x,
                y: area.y,
                width: btn_width,
                height: 1,
            };
            let btn = Paragraph::new(Span::styled(
                *label,
                Style::default()
                    .fg(*color)
                    .add_modifier(Modifier::BOLD),
            ));
            frame.render_widget(btn, btn_area);
            btn_x += btn_width;
        }
        // [Ncol] button
        let col_width = col_label.len() as u16;
        let col_area = Rect {
            x: btn_x,
            y: area.y,
            width: col_width,
            height: 1,
        };
        let col_btn = Paragraph::new(Span::styled(
            col_label,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));
        frame.render_widget(col_btn, col_area);
    }
}
