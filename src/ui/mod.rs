pub mod detail_panel;
pub mod overlay;
pub mod status_bar;
pub mod tab_bar;
pub mod tile_card;
pub mod title;

use crate::app::{AppMode, TextSelection};
use crate::layout::LayoutResult;
use crate::tab::{TabEntry, TabFilter};
use crate::tile_manager::TileManager;
use ratatui::Frame;
use std::collections::HashMap;

/// Render result with actual terminal area size for PTY synchronization.
pub struct RenderResult {
    /// Actual detail panel terminal area dimensions (cols, rows), if panel is visible.
    pub detail_terminal_size: Option<(u16, u16)>,
}

#[allow(clippy::too_many_arguments)]
pub fn render(
    frame: &mut Frame,
    layout: &LayoutResult,
    tile_manager: &TileManager,
    tab_entries: &[TabEntry],
    active_tab: &TabFilter,
    mode: &AppMode,
    columns: u8,
    selection: &Option<TextSelection>,
    detail_scroll_back: usize,
    has_selected_tile: bool,
    scrollback_rows: Option<&[Vec<crate::screen::Cell>]>,
) -> RenderResult {
    tab_bar::render(
        frame,
        layout.tab_bar,
        tab_entries,
        active_tab,
        tile_manager.tile_count(),
    );

    let filtered = tile_manager.filtered_tiles(active_tab);
    let selected_id = tile_manager.selected_id();

    // Compute index labels for tiles sharing the same project name.
    // Only add labels when there are duplicates (e.g. "[1]", "[2]").
    let mut project_counts: HashMap<String, usize> = HashMap::new();
    for tile in &filtered {
        let key = tile
            .git_context
            .as_ref()
            .map(|g| g.project_name.clone())
            .unwrap_or_else(|| tile.cwd.display().to_string());
        *project_counts.entry(key).or_default() += 1;
    }
    let mut project_indices: HashMap<String, usize> = HashMap::new();
    let index_labels: Vec<Option<String>> = filtered
        .iter()
        .map(|tile| {
            let key = tile
                .git_context
                .as_ref()
                .map(|g| g.project_name.clone())
                .unwrap_or_else(|| tile.cwd.display().to_string());
            let count = project_counts.get(&key).copied().unwrap_or(1);
            if count > 1 {
                let idx = project_indices.entry(key).or_insert(0);
                *idx += 1;
                Some(format!("[{}]", *idx))
            } else {
                None
            }
        })
        .collect();

    // Render tile cards, collect cursor from selected tile's card
    let mut tile_card_cursor = None;
    for (i, rect) in layout.tile_rects.iter().enumerate() {
        let absolute_idx = layout.first_visible_tile + i;
        if let Some(tile) = filtered.get(absolute_idx) {
            let is_selected = selected_id == Some(tile.id);
            let label = index_labels.get(absolute_idx).and_then(|l| l.as_deref());
            let card_cursor = tile_card::render(frame, *rect, tile, is_selected, label);
            if is_selected {
                tile_card_cursor = card_cursor;
            }
        }
    }

    // Render detail panel — always render when layout provides it
    let mut cursor_pos = None;
    let mut detail_terminal_size = None;
    if let Some(detail_area) = layout.detail_panel {
        if let Some(tile) = tile_manager.selected() {
            let selected_label = selected_id.and_then(|sid| {
                filtered
                    .iter()
                    .position(|t| t.id == sid)
                    .and_then(|i| index_labels.get(i))
                    .and_then(|l| l.as_deref())
            });
            let result = detail_panel::render(
                frame,
                detail_area,
                tile,
                selected_label,
                selection.as_ref(),
                detail_scroll_back,
                scrollback_rows,
            );
            cursor_pos = result.cursor_pos;
            if result.terminal_size.0 > 0 && result.terminal_size.1 > 0 {
                detail_terminal_size = Some(result.terminal_size);
            }
        } else {
            // No tile selected — render empty detail panel with border
            let block = ratatui::widgets::Block::default()
                .borders(ratatui::widgets::Borders::LEFT)
                .border_set(ratatui::symbols::border::PLAIN)
                .border_style(ratatui::style::Style::default().fg(ratatui::style::Color::DarkGray));
            frame.render_widget(block, detail_area);
        }
    }

    // If no detail panel, use tile card cursor
    if cursor_pos.is_none() {
        cursor_pos = tile_card_cursor;
    }

    status_bar::render(
        frame,
        layout.status_bar,
        tile_manager.tile_count(),
        columns,
    );

    if let AppMode::Overlay(ref kind) = mode {
        let total_area = frame.area();
        overlay::render(frame, total_area, kind);
    }

    // Show blinking hardware cursor when a tile is selected (keyboard goes to PTY).
    if has_selected_tile {
        if let Some((cx, cy)) = cursor_pos {
            frame.set_cursor_position(ratatui::layout::Position::new(cx, cy));
        }
    }

    RenderResult {
        detail_terminal_size,
    }
}
