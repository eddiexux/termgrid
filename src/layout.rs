use ratatui::layout::Rect;

/// Tab bar height including bottom border.
pub const TAB_BAR_HEIGHT: u16 = 2;
/// Status bar height.
pub const STATUS_BAR_HEIGHT: u16 = 1;
/// Minimum tile card height in the grid.
pub const MIN_TILE_HEIGHT: u16 = 5;
/// Maximum tile card height in the grid.
pub const MAX_TILE_HEIGHT: u16 = 14;
/// Detail panel header height (title + path + separator).
pub const DETAIL_HEADER_HEIGHT: u16 = 3;
/// Detail panel left border width.
pub const DETAIL_BORDER_WIDTH: u16 = 1;

/// Result of layout calculation containing all UI component rectangles
#[derive(Debug, Clone)]
pub struct LayoutResult {
    pub tab_bar: Rect,
    pub grid_area: Rect,
    pub detail_panel: Option<Rect>,
    pub status_bar: Rect,
    pub tile_rects: Vec<Rect>,
    /// Maximum valid scroll offset (in rows). 0 means no scrolling possible.
    pub max_scroll_offset: usize,
    /// Index of the first visible tile in the filtered tile list.
    pub first_visible_tile: usize,
}

/// Calculate layout for the terminal UI
///
/// # Arguments
/// * `total` - Full terminal area
/// * `columns` - Number of columns (1-3)
/// * `tile_count` - Number of tiles to lay out
/// * `detail_width_pct` - Detail panel width as percentage (0-100)
/// * `scroll_offset` - Grid scroll offset in rows (0 = no scroll)
///
/// # Returns
/// LayoutResult with all component rectangles and tile positions
pub fn calculate_layout(
    total: Rect,
    columns: u8,
    tile_count: usize,
    detail_width_pct: u16,
    scroll_offset: usize,
) -> LayoutResult {
    // Clamp columns to valid range
    let columns = columns.clamp(1, 3);

    // Tab bar at top (2 lines: content + bottom border)
    let tab_bar = Rect {
        x: total.x,
        y: total.y,
        width: total.width,
        height: TAB_BAR_HEIGHT,
    };

    // Status bar at bottom
    let status_bar = Rect {
        x: total.x,
        y: total.y + total.height.saturating_sub(1),
        width: total.width,
        height: STATUS_BAR_HEIGHT,
    };

    // Middle area between tab bar and status bar
    let middle_height = total
        .height
        .saturating_sub(TAB_BAR_HEIGHT + STATUS_BAR_HEIGHT)
        .max(1);
    let middle_y = total.y + TAB_BAR_HEIGHT;

    // Determine grid and detail panel layout — detail panel always visible when space allows
    let (grid_area, detail_panel) =
        if total.width > 40 && detail_width_pct > 0 && detail_width_pct < 100 {
            let detail_width = ((total.width * detail_width_pct) / 100).max(1);
            let grid_width = total.width.saturating_sub(detail_width);

            let grid = Rect {
                x: total.x,
                y: middle_y,
                width: grid_width,
                height: middle_height,
            };

            let detail = Rect {
                x: total.x + grid_width,
                y: middle_y,
                width: detail_width,
                height: middle_height,
            };

            (grid, Some(detail))
        } else {
            let grid = Rect {
                x: total.x,
                y: middle_y,
                width: total.width,
                height: middle_height,
            };
            (grid, None)
        };

    // Calculate tile rects with row-based scrolling
    let (tile_rects, max_scroll_offset, first_visible_tile) =
        calculate_tile_rects(grid_area, columns as usize, tile_count, scroll_offset);

    LayoutResult {
        tab_bar,
        grid_area,
        detail_panel,
        status_bar,
        tile_rects,
        max_scroll_offset,
        first_visible_tile,
    }
}

/// Calculate rectangles for tiles within the grid area.
/// Returns (tile_rects, max_scroll_offset, first_visible_tile_index).
/// `scroll_offset` is in rows (not individual tiles).
fn calculate_tile_rects(
    grid: Rect,
    columns: usize,
    tile_count: usize,
    scroll_offset: usize,
) -> (Vec<Rect>, usize, usize) {
    if tile_count == 0 || grid.width == 0 || grid.height == 0 {
        return (vec![], 0, 0);
    }

    let cols = columns.max(1);
    let col_width = grid.width / cols as u16;

    let total_rows = total_grid_rows(tile_count, cols as u8);
    let tile_height = (grid.height / total_rows as u16).clamp(MIN_TILE_HEIGHT, MAX_TILE_HEIGHT);
    let visible_rows = (grid.height / tile_height) as usize;
    let max_scroll = total_rows.saturating_sub(visible_rows);

    // Clamp scroll to valid range
    let scroll = scroll_offset.min(max_scroll);
    let first_visible_tile = scroll * cols;

    let mut tile_rects = vec![];

    for tile_idx in first_visible_tile..tile_count {
        let visible_idx = tile_idx - first_visible_tile;
        let row = visible_idx / cols;
        let col = visible_idx % cols;

        let tile_y = grid.y + (row as u16 * tile_height);

        // Stop at tiles that won't fully fit in visible area
        if tile_y + tile_height > grid.y + grid.height {
            break;
        }

        let tile_x = grid.x + (col as u16 * col_width);
        let tile_w = if col == cols - 1 {
            // Last column takes remaining width
            grid.width.saturating_sub(col as u16 * col_width)
        } else {
            col_width
        };

        tile_rects.push(Rect {
            x: tile_x,
            y: tile_y,
            width: tile_w,
            height: tile_height,
        });
    }

    (tile_rects, max_scroll, first_visible_tile)
}

/// Calculate total number of rows needed to layout tiles
pub fn total_grid_rows(tile_count: usize, columns: u8) -> usize {
    let cols = columns.max(1) as usize;
    tile_count.div_ceil(cols)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_layout_structure() {
        let total = Rect::new(0, 0, 120, 40);
        let result = calculate_layout(total, 2, 4, 45, 0);

        // Tab bar at top (2 lines: content + border)
        assert_eq!(result.tab_bar.y, 0);
        assert_eq!(result.tab_bar.height, 2);
        assert_eq!(result.tab_bar.width, 120);

        // Status bar should be 1 line at bottom
        assert_eq!(result.status_bar.y, 39);
        assert_eq!(result.status_bar.height, 1);
        assert_eq!(result.status_bar.width, 120);

        // Grid area should be between tab bar and status bar
        assert_eq!(result.grid_area.y, 2);
        assert_eq!(result.grid_area.height, 37);

        // Detail panel always shown when space allows
        assert!(result.detail_panel.is_some());
    }

    #[test]
    fn test_detail_panel_always_visible() {
        let total = Rect::new(0, 0, 120, 40);
        let result = calculate_layout(total, 2, 4, 45, 0);

        // Detail panel always shown regardless of selection state
        assert!(result.detail_panel.is_some());

        let detail = result.detail_panel.unwrap();
        let expected_detail_width = (120 * 45) / 100;

        // Detail panel width should be approximately 45% of total
        assert!(detail.width >= expected_detail_width as u16 - 1);
        assert!(detail.width <= expected_detail_width as u16 + 1);

        // Grid area should be reduced
        assert!(result.grid_area.width < 120);

        // Detail panel should start where grid area ends
        assert_eq!(result.grid_area.x + result.grid_area.width, detail.x);
    }

    #[test]
    fn test_no_detail_panel_on_narrow_terminal() {
        let total = Rect::new(0, 0, 30, 20);
        let result = calculate_layout(total, 2, 4, 45, 0);

        // Terminal too narrow for detail panel
        assert!(result.detail_panel.is_none());

        // Grid takes full width
        assert_eq!(result.grid_area.width, 30);
    }

    #[test]
    fn test_tile_rects_2_columns() {
        let total = Rect::new(0, 0, 100, 40);
        // Use pct=0 to get full-width grid for simpler assertions
        let result = calculate_layout(total, 2, 4, 0, 0);

        assert_eq!(result.tile_rects.len(), 4);

        // First row: tiles 0 and 1
        let tile_0 = result.tile_rects[0];
        let tile_1 = result.tile_rects[1];

        assert_eq!(tile_0.y, tile_1.y);
        assert!(tile_1.x > tile_0.x);

        // Second row: tiles 2 and 3
        let tile_2 = result.tile_rects[2];
        let tile_3 = result.tile_rects[3];

        assert_eq!(tile_2.y, tile_3.y);
        assert!(tile_2.y > tile_0.y);
        assert!(tile_3.x > tile_2.x);
    }

    #[test]
    fn test_tile_rects_3_columns() {
        let total = Rect::new(0, 0, 120, 40);
        let result = calculate_layout(total, 3, 9, 0, 0);

        assert_eq!(result.tile_rects.len(), 9);

        // First row: tiles 0, 1, 2
        assert_eq!(result.tile_rects[0].y, result.tile_rects[1].y);
        assert_eq!(result.tile_rects[1].y, result.tile_rects[2].y);

        // Second row: tiles 3, 4, 5
        assert_eq!(result.tile_rects[3].y, result.tile_rects[4].y);
        assert!(result.tile_rects[3].y > result.tile_rects[0].y);

        // Verify column arrangement
        assert!(result.tile_rects[1].x > result.tile_rects[0].x);
        assert!(result.tile_rects[2].x > result.tile_rects[1].x);
    }

    #[test]
    fn test_tile_rects_1_column() {
        let total = Rect::new(0, 0, 50, 40);
        // Use pct=0 to get full-width grid
        let result = calculate_layout(total, 1, 4, 0, 0);

        assert_eq!(result.tile_rects.len(), 4);

        // All tiles should have same x (full width)
        for rect in &result.tile_rects {
            assert_eq!(rect.x, 0);
            assert_eq!(rect.width, 50);
        }

        // Verify vertical stacking
        for i in 1..result.tile_rects.len() {
            assert!(result.tile_rects[i].y > result.tile_rects[i - 1].y);
        }
    }

    #[test]
    fn test_empty_tiles() {
        let total = Rect::new(0, 0, 100, 40);
        let result = calculate_layout(total, 2, 0, 0, 0);

        assert!(result.tile_rects.is_empty());
        assert_eq!(result.max_scroll_offset, 0);
        assert_eq!(result.first_visible_tile, 0);
    }

    #[test]
    fn test_scroll_row_based() {
        // Small height so tiles overflow: grid_height=19, tile_height=5(min), visible_rows=3
        // 10 tiles, 2 columns = 5 rows, max_scroll = 5-3 = 2
        let total = Rect::new(0, 0, 100, 22);
        let result_no_scroll = calculate_layout(total, 2, 10, 0, 0);
        let result_with_scroll = calculate_layout(total, 2, 10, 0, 1);

        // With row-based scroll, scrolled tiles start at the SAME y (grid top)
        assert_eq!(result_no_scroll.tile_rects[0].y, result_with_scroll.tile_rects[0].y);

        // First visible tile should skip one row (2 tiles for 2 columns)
        assert_eq!(result_with_scroll.first_visible_tile, 2);
        assert_eq!(result_no_scroll.first_visible_tile, 0);

        // Same number of visible tiles (both show 3 full rows), but different tiles
        assert_eq!(result_no_scroll.tile_rects.len(), result_with_scroll.tile_rects.len());
        assert!(result_with_scroll.max_scroll_offset > 0);
    }

    #[test]
    fn test_max_scroll_single_tile() {
        let total = Rect::new(0, 0, 100, 40);
        let result = calculate_layout(total, 1, 1, 0, 0);

        // Single tile always fits, no scrolling
        assert_eq!(result.max_scroll_offset, 0);
        assert_eq!(result.first_visible_tile, 0);
        assert_eq!(result.tile_rects.len(), 1);
    }

    #[test]
    fn test_max_scroll_all_tiles_fit() {
        let total = Rect::new(0, 0, 100, 40);
        // 4 tiles, 2 columns = 2 rows, grid height 37, should fit
        let result = calculate_layout(total, 2, 4, 0, 0);
        assert_eq!(result.max_scroll_offset, 0);
    }

    #[test]
    fn test_max_scroll_tiles_overflow() {
        let total = Rect::new(0, 0, 100, 22);
        // Grid height = 22 - 2(tab) - 1(status) = 19
        // 10 tiles, 1 column = 10 rows
        // tile_height = (19/10)=1 → clamped to MIN_TILE_HEIGHT(5)
        // visible_rows = 19/5 = 3
        // max_scroll = 10 - 3 = 7
        let result = calculate_layout(total, 1, 10, 0, 0);
        assert_eq!(result.max_scroll_offset, 7);
    }

    #[test]
    fn test_scroll_offset_clamped_to_max() {
        let total = Rect::new(0, 0, 100, 40);
        // 2 tiles in 1 column, all fit → max_scroll = 0
        // Pass scroll_offset = 5 (way past max)
        let result = calculate_layout(total, 1, 2, 0, 5);

        // Should still show all tiles (scroll clamped to 0)
        assert_eq!(result.tile_rects.len(), 2);
        assert_eq!(result.max_scroll_offset, 0);
        assert_eq!(result.first_visible_tile, 0);
    }

    #[test]
    fn test_scroll_tiles_start_at_grid_top() {
        let total = Rect::new(0, 0, 100, 22);
        // With scroll_offset=2, first visible tile should start at grid.y (no gap)
        let result = calculate_layout(total, 1, 10, 0, 2);
        assert!(!result.tile_rects.is_empty());
        assert_eq!(result.tile_rects[0].y, result.grid_area.y);
    }

    #[test]
    fn test_first_visible_tile_multicolumn() {
        let total = Rect::new(0, 0, 100, 22);
        // 2 columns, scroll_offset=2 rows → first visible tile = 4
        let result = calculate_layout(total, 2, 10, 0, 2);
        assert_eq!(result.first_visible_tile, 4);
    }

    #[test]
    fn test_total_grid_rows() {
        // Single column
        assert_eq!(total_grid_rows(4, 1), 4);

        // Multiple columns
        assert_eq!(total_grid_rows(4, 2), 2);
        assert_eq!(total_grid_rows(5, 2), 3);
        assert_eq!(total_grid_rows(9, 3), 3);
        assert_eq!(total_grid_rows(10, 3), 4);

        // Empty
        assert_eq!(total_grid_rows(0, 2), 0);

        // Single tile
        assert_eq!(total_grid_rows(1, 2), 1);
    }

    #[test]
    fn test_columns_clamped_to_valid_range() {
        let total = Rect::new(0, 0, 100, 40);

        // Column 0 should be clamped to 1
        let result_0 = calculate_layout(total, 0, 4, 0, 0);
        assert_eq!(result_0.tile_rects.len(), 4);

        // Column 4 should be clamped to 3
        let result_4 = calculate_layout(total, 4, 9, 0, 0);
        // Should layout with 3 columns
        assert!(!result_4.tile_rects.is_empty());
    }

    #[test]
    fn test_detail_panel_percentage_calculation() {
        let total = Rect::new(0, 0, 100, 40);

        // Test different percentages
        let result_30 = calculate_layout(total, 2, 4, 30, 0);
        let result_50 = calculate_layout(total, 2, 4, 50, 0);

        assert!(result_30.detail_panel.is_some());
        assert!(result_50.detail_panel.is_some());

        let detail_30 = result_30.detail_panel.unwrap();
        let detail_50 = result_50.detail_panel.unwrap();

        // 50% should be wider than 30%
        assert!(detail_50.width > detail_30.width);

        // Grids should be appropriately sized
        assert!(result_30.grid_area.width > result_50.grid_area.width);
    }

    #[test]
    fn test_no_detail_panel_with_zero_percentage() {
        let total = Rect::new(0, 0, 100, 40);
        let result = calculate_layout(total, 2, 4, 0, 0);

        // 0% detail width should result in no detail panel
        assert!(result.detail_panel.is_none());
    }

    #[test]
    fn test_tile_minimum_height() {
        let total = Rect::new(0, 0, 100, 15); // Very small height
        // Use pct=0 to get full-width grid
        let result = calculate_layout(total, 2, 4, 0, 0);

        for tile in &result.tile_rects {
            assert!(
                tile.height >= 5,
                "Tile height {} should be at least 5",
                tile.height
            );
        }
    }
}
