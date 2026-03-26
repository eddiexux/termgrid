pub mod detail_panel;
pub mod overlay;
pub mod status_bar;
pub mod tab_bar;
pub mod tile_card;

use crate::app::AppMode;
use crate::layout::LayoutResult;
use crate::tab::{TabEntry, TabFilter};
use crate::tile_manager::TileManager;
use ratatui::Frame;

pub fn render(
    frame: &mut Frame,
    layout: &LayoutResult,
    tile_manager: &TileManager,
    tab_entries: &[TabEntry],
    active_tab: &TabFilter,
    mode: &AppMode,
    columns: u8,
) {
    tab_bar::render(
        frame,
        layout.tab_bar,
        tab_entries,
        active_tab,
        tile_manager.tile_count(),
    );

    let filtered = tile_manager.filtered_tiles(active_tab);
    let selected_id = tile_manager.selected_id();
    for (i, rect) in layout.tile_rects.iter().enumerate() {
        if let Some(tile) = filtered.get(i) {
            let is_selected = selected_id == Some(tile.id);
            tile_card::render(frame, *rect, tile, is_selected);
        }
    }

    if let (Some(detail_area), Some(tile)) = (layout.detail_panel, tile_manager.selected()) {
        detail_panel::render(frame, detail_area, tile);
    }

    status_bar::render(
        frame,
        layout.status_bar,
        mode,
        tile_manager.tile_count(),
        columns,
    );

    if let AppMode::Overlay(ref kind) = mode {
        let total_area = frame.area();
        overlay::render(frame, total_area, kind);
    }
}
