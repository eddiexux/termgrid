pub mod detail_panel;
pub mod overlay;
pub mod status_bar;
pub mod tab_bar;
pub mod tile_card;
pub mod title;

use crate::app::{AppMode, TextSelection};
use crate::layout::LayoutResult;
use crate::tab::{TabEntry, TabFilter};
use crate::tile::Tile;
use crate::tile_manager::TileManager;
use ratatui::Frame;
use std::collections::HashMap;

/// Compute index labels for tiles sharing the same project name.
/// Returns `None` for unique projects, `Some("[1]")`, `Some("[2]")` etc for duplicates.
pub fn compute_index_labels(tiles: &[&Tile]) -> Vec<Option<String>> {
    let mut project_counts: HashMap<String, usize> = HashMap::new();
    for tile in tiles {
        let key = tile
            .git_context
            .as_ref()
            .map(|g| g.project_name.clone())
            .unwrap_or_else(|| tile.cwd.display().to_string());
        *project_counts.entry(key).or_default() += 1;
    }
    let mut project_indices: HashMap<String, usize> = HashMap::new();
    tiles
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
        .collect()
}

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

    let index_labels = compute_index_labels(&filtered);

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::GitContext;
    use crate::tile::{Tile, TileId};
    use std::path::PathBuf;

    fn make_tile(id: u64, project_name: Option<&str>, cwd: &str) -> Tile {
        let git_context = project_name.map(|name| GitContext {
            project_name: name.to_string(),
            branch: Some("main".to_string()),
            is_worktree: false,
            worktree_name: None,
            repo_root: PathBuf::from(cwd),
        });
        Tile::new_test(TileId(id), &PathBuf::from(cwd), git_context)
    }

    #[test]
    fn test_index_labels_unique_projects() {
        let t1 = make_tile(1, Some("alpha"), "/alpha");
        let t2 = make_tile(2, Some("beta"), "/beta");
        let tiles: Vec<&Tile> = vec![&t1, &t2];
        let labels = compute_index_labels(&tiles);
        assert_eq!(labels, vec![None, None]);
    }

    #[test]
    fn test_index_labels_duplicate_projects() {
        let t1 = make_tile(1, Some("alpha"), "/a1");
        let t2 = make_tile(2, Some("alpha"), "/a2");
        let t3 = make_tile(3, Some("beta"), "/b");
        let tiles: Vec<&Tile> = vec![&t1, &t2, &t3];
        let labels = compute_index_labels(&tiles);
        assert_eq!(
            labels,
            vec![Some("[1]".to_string()), Some("[2]".to_string()), None]
        );
    }

    #[test]
    fn test_index_labels_all_same_project() {
        let t1 = make_tile(1, Some("proj"), "/p1");
        let t2 = make_tile(2, Some("proj"), "/p2");
        let t3 = make_tile(3, Some("proj"), "/p3");
        let tiles: Vec<&Tile> = vec![&t1, &t2, &t3];
        let labels = compute_index_labels(&tiles);
        assert_eq!(
            labels,
            vec![
                Some("[1]".to_string()),
                Some("[2]".to_string()),
                Some("[3]".to_string()),
            ]
        );
    }

    #[test]
    fn test_index_labels_no_git_context_uses_cwd() {
        let t1 = make_tile(1, None, "/same/path");
        let t2 = make_tile(2, None, "/same/path");
        let tiles: Vec<&Tile> = vec![&t1, &t2];
        let labels = compute_index_labels(&tiles);
        assert_eq!(
            labels,
            vec![Some("[1]".to_string()), Some("[2]".to_string())]
        );
    }

    #[test]
    fn test_index_labels_empty() {
        let tiles: Vec<&Tile> = vec![];
        let labels = compute_index_labels(&tiles);
        assert!(labels.is_empty());
    }

    #[test]
    fn test_index_labels_single_tile() {
        let t1 = make_tile(1, Some("proj"), "/p");
        let tiles: Vec<&Tile> = vec![&t1];
        let labels = compute_index_labels(&tiles);
        assert_eq!(labels, vec![None]);
    }
}
