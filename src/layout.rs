use ratatui::layout::Rect;

/// Result of layout calculation containing all UI component rectangles
#[derive(Debug, Clone)]
pub struct LayoutResult {
    pub tab_bar: Rect,
    pub grid_area: Rect,
    pub detail_panel: Option<Rect>,
    pub status_bar: Rect,
    pub tile_rects: Vec<Rect>,
}

/// Calculate layout for the terminal UI
///
/// # Arguments
/// * `total` - Full terminal area
/// * `columns` - Number of columns (1-3)
/// * `tile_count` - Number of tiles to lay out
/// * `has_selection` - Whether detail panel should be shown
/// * `detail_width_pct` - Detail panel width as percentage (0-100)
/// * `scroll_offset` - Grid scroll offset (number of tiles to skip from top)
///
/// # Returns
/// LayoutResult with all component rectangles and tile positions
pub fn calculate_layout(
    total: Rect,
    columns: u8,
    tile_count: usize,
    has_selection: bool,
    detail_width_pct: u16,
    scroll_offset: usize,
) -> LayoutResult {
    // Clamp columns to valid range
    let columns = columns.clamp(1, 3);

    // Tab bar at top
    let tab_bar = Rect {
        x: total.x,
        y: total.y,
        width: total.width,
        height: 1,
    };

    // Status bar at bottom
    let status_bar = Rect {
        x: total.x,
        y: total.y + total.height.saturating_sub(1),
        width: total.width,
        height: 1,
    };

    // Middle area between tab bar and status bar
    let middle_height = total.height.saturating_sub(2).max(1);
    let middle_y = total.y + 1;

    // Determine grid and detail panel layout
    let (grid_area, detail_panel) = if has_selection
        && total.width > 40
        && detail_width_pct > 0
        && detail_width_pct < 100
    {
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

    // Calculate tile rects
    let tile_rects = calculate_tile_rects(
        grid_area,
        columns as usize,
        tile_count,
        scroll_offset,
    );

    LayoutResult {
        tab_bar,
        grid_area,
        detail_panel,
        status_bar,
        tile_rects,
    }
}

/// Calculate rectangles for tiles within the grid area
fn calculate_tile_rects(
    grid: Rect,
    columns: usize,
    tile_count: usize,
    scroll_offset: usize,
) -> Vec<Rect> {
    if tile_count == 0 || grid.width == 0 || grid.height == 0 {
        return vec![];
    }

    let cols = columns.max(1);
    let col_width = grid.width / cols as u16;
    let min_tile_height: u16 = 5;

    let total_rows = total_grid_rows(tile_count, cols as u8);
    let tile_height = (grid.height / total_rows as u16).max(min_tile_height);

    let mut tile_rects = vec![];

    for tile_idx in 0..tile_count {
        // Skip tiles above scroll offset
        if tile_idx < scroll_offset {
            continue;
        }

        let row = tile_idx / cols;
        let col = tile_idx % cols;

        let tile_y = grid.y + (row as u16 * tile_height);

        // Skip tiles below visible area
        if tile_y >= grid.y + grid.height {
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

    tile_rects
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
        let result = calculate_layout(total, 2, 4, false, 45, 0);

        // Tab bar should be 1 line at top
        assert_eq!(result.tab_bar.y, 0);
        assert_eq!(result.tab_bar.height, 1);
        assert_eq!(result.tab_bar.width, 120);

        // Status bar should be 1 line at bottom
        assert_eq!(result.status_bar.y, 39);
        assert_eq!(result.status_bar.height, 1);
        assert_eq!(result.status_bar.width, 120);

        // Grid area should be between tab and status bars
        assert_eq!(result.grid_area.y, 1);
        assert_eq!(result.grid_area.height, 38);

        // No detail panel without selection
        assert!(result.detail_panel.is_none());
    }

    #[test]
    fn test_detail_panel_shown_when_selected() {
        let total = Rect::new(0, 0, 120, 40);
        let result = calculate_layout(total, 2, 4, true, 45, 0);

        // Detail panel should be shown
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
        let result = calculate_layout(total, 2, 4, true, 45, 0);

        // Terminal too narrow for detail panel
        assert!(result.detail_panel.is_none());

        // Grid takes full width
        assert_eq!(result.grid_area.width, 30);
    }

    #[test]
    fn test_tile_rects_2_columns() {
        let total = Rect::new(0, 0, 100, 40);
        let result = calculate_layout(total, 2, 4, false, 45, 0);

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
        let result = calculate_layout(total, 3, 9, false, 45, 0);

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
        let result = calculate_layout(total, 1, 4, false, 45, 0);

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
        let result = calculate_layout(total, 2, 0, false, 45, 0);

        assert!(result.tile_rects.is_empty());
    }

    #[test]
    fn test_scroll_offset() {
        let total = Rect::new(0, 0, 100, 40);
        let result_no_scroll = calculate_layout(total, 2, 8, false, 45, 0);
        let result_with_scroll = calculate_layout(total, 2, 8, false, 45, 2);

        // With scroll offset, fewer tiles should be visible
        assert!(result_with_scroll.tile_rects.len() <= result_no_scroll.tile_rects.len());

        // Scrolled tiles should start lower on screen
        if !result_no_scroll.tile_rects.is_empty() && !result_with_scroll.tile_rects.is_empty() {
            let first_no_scroll = result_no_scroll.tile_rects[0].y;
            let first_with_scroll = result_with_scroll.tile_rects[0].y;
            assert!(first_with_scroll >= first_no_scroll);
        }
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
        let result_0 = calculate_layout(total, 0, 4, false, 45, 0);
        assert_eq!(result_0.tile_rects.len(), 4);

        // Column 4 should be clamped to 3
        let result_4 = calculate_layout(total, 4, 9, false, 45, 0);
        // Should layout with 3 columns
        assert!(!result_4.tile_rects.is_empty());
    }

    #[test]
    fn test_detail_panel_percentage_calculation() {
        let total = Rect::new(0, 0, 100, 40);

        // Test different percentages
        let result_30 = calculate_layout(total, 2, 4, true, 30, 0);
        let result_50 = calculate_layout(total, 2, 4, true, 50, 0);

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
        let result = calculate_layout(total, 2, 4, true, 0, 0);

        // 0% detail width should result in no detail panel
        assert!(result.detail_panel.is_none());
    }

    #[test]
    fn test_tile_minimum_height() {
        let total = Rect::new(0, 0, 100, 15); // Very small height
        let result = calculate_layout(total, 2, 4, false, 45, 0);

        for tile in &result.tile_rects {
            assert!(tile.height >= 5, "Tile height {} should be at least 5", tile.height);
        }
    }
}
