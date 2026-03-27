use crate::tab::TabFilter;
use crate::tile::{Tile, TileId};

pub struct TileManager {
    tiles: Vec<Tile>,
    selected: Option<TileId>,
    next_id: u64,
}

#[derive(Debug, Clone, Copy)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

impl TileManager {
    pub fn new() -> Self {
        TileManager {
            tiles: Vec::new(),
            selected: None,
            next_id: 1,
        }
    }

    /// Allocate the next tile id, incrementing the internal counter.
    pub fn next_tile_id(&mut self) -> TileId {
        let id = TileId(self.next_id);
        self.next_id += 1;
        id
    }

    /// Add a tile to the manager. If no tile is currently selected, select this one.
    pub fn add(&mut self, tile: Tile) {
        if self.selected.is_none() {
            self.selected = Some(tile.id);
        }
        self.tiles.push(tile);
    }

    /// Remove a tile by id. Adjusts the selection to an adjacent tile if the removed
    /// tile was selected.
    pub fn remove(&mut self, id: TileId) -> Option<Tile> {
        if let Some(pos) = self.tiles.iter().position(|t| t.id == id) {
            let was_selected = self.selected == Some(id);
            let tile = self.tiles.remove(pos);

            if was_selected {
                // Try to select the tile that was at the same position,
                // or the one before it.
                if self.tiles.is_empty() {
                    self.selected = None;
                } else {
                    let new_pos = pos.min(self.tiles.len() - 1);
                    self.selected = Some(self.tiles[new_pos].id);
                }
            }

            Some(tile)
        } else {
            None
        }
    }

    pub fn get(&self, id: TileId) -> Option<&Tile> {
        self.tiles.iter().find(|t| t.id == id)
    }

    pub fn get_mut(&mut self, id: TileId) -> Option<&mut Tile> {
        self.tiles.iter_mut().find(|t| t.id == id)
    }

    pub fn selected_id(&self) -> Option<TileId> {
        self.selected
    }

    pub fn selected(&self) -> Option<&Tile> {
        self.selected.and_then(|id| self.get(id))
    }

    pub fn selected_mut(&mut self) -> Option<&mut Tile> {
        let id = self.selected?;
        self.get_mut(id)
    }

    pub fn select(&mut self, id: TileId) {
        self.selected = Some(id);
        // Clear unread flag and reset burst counter when tile becomes selected
        if let Some(tile) = self.tiles.iter_mut().find(|t| t.id == id) {
            if tile.has_unread {
                tracing::info!(
                    "Tile {} unread cleared by select, burst_bytes was {}",
                    tile.id.0,
                    tile.burst_bytes,
                );
            }
            tile.has_unread = false;
            tile.burst_bytes = 0;
        }
    }

    pub fn deselect(&mut self) {
        self.selected = None;
    }

    /// Return tiles matching the given filter, grouped by project name.
    /// Within a project group, tiles are ordered by creation order (tile ID).
    /// Non-git tiles are placed after all project tiles.
    pub fn filtered_tiles(&self, filter: &TabFilter) -> Vec<&Tile> {
        let mut filtered: Vec<&Tile> = self
            .tiles
            .iter()
            .filter(|t| filter.matches(&t.git_context))
            .collect();
        filtered.sort_by(|a, b| {
            let key_a = Self::group_key(a);
            let key_b = Self::group_key(b);
            key_a.cmp(&key_b).then(a.id.0.cmp(&b.id.0))
        });
        filtered
    }

    fn group_key(tile: &Tile) -> String {
        tile.git_context
            .as_ref()
            .map(|g| format!("0:{}", g.project_name))
            .unwrap_or_else(|| format!("1:{}", tile.cwd.display()))
    }

    pub fn tiles(&self) -> &[Tile] {
        &self.tiles
    }

    pub fn tiles_mut(&mut self) -> &mut Vec<Tile> {
        &mut self.tiles
    }

    pub fn tile_count(&self) -> usize {
        self.tiles.len()
    }

    /// Select the next tile in the filtered list, cycling around.
    pub fn select_next(&mut self, filter: &TabFilter) {
        let filtered: Vec<TileId> = self.filtered_tiles(filter).iter().map(|t| t.id).collect();

        if filtered.is_empty() {
            return;
        }

        let next_id = if let Some(current) = self.selected {
            if let Some(pos) = filtered.iter().position(|&id| id == current) {
                filtered[(pos + 1) % filtered.len()]
            } else {
                filtered[0]
            }
        } else {
            filtered[0]
        };

        self.select(next_id);
    }

    /// Select the previous tile in the filtered list, cycling around.
    pub fn select_prev(&mut self, filter: &TabFilter) {
        let filtered: Vec<TileId> = self.filtered_tiles(filter).iter().map(|t| t.id).collect();

        if filtered.is_empty() {
            return;
        }

        let prev_id = if let Some(current) = self.selected {
            if let Some(pos) = filtered.iter().position(|&id| id == current) {
                filtered[(pos + filtered.len() - 1) % filtered.len()]
            } else {
                filtered[filtered.len() - 1]
            }
        } else {
            filtered[filtered.len() - 1]
        };

        self.select(prev_id);
    }

    /// Navigate in a grid layout with the given number of columns.
    ///
    /// Tiles are laid out row by row based on their position in the filtered list.
    /// Left/Right move within a row, Up/Down move between rows.
    pub fn select_direction(&mut self, filter: &TabFilter, columns: usize, direction: Direction) {
        if columns == 0 {
            return;
        }

        let filtered: Vec<TileId> = self.filtered_tiles(filter).iter().map(|t| t.id).collect();

        if filtered.is_empty() {
            return;
        }

        let current_pos = if let Some(current) = self.selected {
            filtered.iter().position(|&id| id == current)
        } else {
            None
        };

        let current_idx = match current_pos {
            Some(i) => i,
            None => {
                self.selected = Some(filtered[0]);
                return;
            }
        };

        let len = filtered.len();
        let row = current_idx / columns;
        let col = current_idx % columns;

        let new_idx = match direction {
            Direction::Left => {
                if col > 0 {
                    current_idx - 1
                } else {
                    // Wrap to end of previous row or stay
                    current_idx
                }
            }
            Direction::Right => {
                if current_idx + 1 < len && col + 1 < columns {
                    current_idx + 1
                } else {
                    current_idx
                }
            }
            Direction::Up => {
                if row > 0 {
                    let target = (row - 1) * columns + col;
                    target.min(len - 1)
                } else {
                    current_idx
                }
            }
            Direction::Down => {
                let target = (row + 1) * columns + col;
                if target < len {
                    target
                } else {
                    current_idx
                }
            }
        };

        self.select(filtered[new_idx]);
    }
}

impl Default for TileManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::GitContext;
    use crate::tile::Tile;
    use std::path::PathBuf;

    fn make_tile(mgr: &mut TileManager, project: Option<&str>) -> TileId {
        let id = mgr.next_tile_id();
        let dir = std::env::current_dir().unwrap();
        let (mut tile, _reader) = Tile::spawn(id, "/bin/sh", &dir, 80, 24).unwrap();
        if let Some(name) = project {
            tile.git_context = Some(GitContext {
                project_name: name.into(),
                branch: Some("main".into()),
                is_worktree: false,
                worktree_name: None,
                repo_root: PathBuf::from("/tmp"),
            });
        } else {
            tile.git_context = None;
        }
        mgr.add(tile);
        id
    }

    #[test]
    fn test_tile_id_equality() {
        let a = TileId(42);
        let b = TileId(42);
        let c = TileId(99);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn test_process_output() {
        use crate::screen::VteState;
        let mut vte = VteState::new(80, 24);
        vte.process(b"hello");
        let (_, col) = vte.cursor_position();
        assert_eq!(col, 5);
    }

    #[test]
    fn test_add_and_select() {
        let mut mgr = TileManager::new();
        let id1 = make_tile(&mut mgr, Some("alpha"));
        let id2 = make_tile(&mut mgr, Some("beta"));

        // First tile should be auto-selected
        assert_eq!(mgr.selected_id(), Some(id1));
        assert_eq!(mgr.tile_count(), 2);

        // Explicit select
        mgr.select(id2);
        assert_eq!(mgr.selected_id(), Some(id2));
    }

    #[test]
    fn test_remove_adjusts_selection() {
        let mut mgr = TileManager::new();
        let id1 = make_tile(&mut mgr, Some("alpha"));
        let id2 = make_tile(&mut mgr, Some("beta"));
        let id3 = make_tile(&mut mgr, Some("gamma"));

        // Select the middle tile and remove it
        mgr.select(id2);
        let removed = mgr.remove(id2);
        assert!(removed.is_some());
        assert_eq!(mgr.tile_count(), 2);

        // Selection should have moved to what was at the same position (now id3)
        let selected = mgr.selected_id();
        assert!(selected == Some(id1) || selected == Some(id3));
    }

    #[test]
    fn test_remove_last_clears_selection() {
        let mut mgr = TileManager::new();
        let id = make_tile(&mut mgr, None);

        mgr.select(id);
        mgr.remove(id);

        assert_eq!(mgr.selected_id(), None);
        assert_eq!(mgr.tile_count(), 0);
    }

    #[test]
    fn test_filtered_tiles() {
        let mut mgr = TileManager::new();
        let _id_alpha1 = make_tile(&mut mgr, Some("alpha"));
        let _id_alpha2 = make_tile(&mut mgr, Some("alpha"));
        let _id_beta = make_tile(&mut mgr, Some("beta"));
        let _id_none = make_tile(&mut mgr, None);

        let all = mgr.filtered_tiles(&TabFilter::All);
        assert_eq!(all.len(), 4);

        let alpha = mgr.filtered_tiles(&TabFilter::Project("alpha".into()));
        assert_eq!(alpha.len(), 2);

        let beta = mgr.filtered_tiles(&TabFilter::Project("beta".into()));
        assert_eq!(beta.len(), 1);

        let other = mgr.filtered_tiles(&TabFilter::Other);
        assert_eq!(other.len(), 1);
    }

    #[test]
    fn test_select_next_cycles() {
        let mut mgr = TileManager::new();
        let id1 = make_tile(&mut mgr, Some("alpha"));
        let id2 = make_tile(&mut mgr, Some("alpha"));
        let id3 = make_tile(&mut mgr, Some("alpha"));

        let filter = TabFilter::All;
        mgr.select(id1);

        mgr.select_next(&filter);
        assert_eq!(mgr.selected_id(), Some(id2));

        mgr.select_next(&filter);
        assert_eq!(mgr.selected_id(), Some(id3));

        // Should cycle back to first
        mgr.select_next(&filter);
        assert_eq!(mgr.selected_id(), Some(id1));
    }

    #[test]
    fn test_select_direction() {
        // 2x2 grid: [t0, t1, t2, t3]
        // Layout with 2 columns:
        //   row 0: t0 t1
        //   row 1: t2 t3
        let mut mgr = TileManager::new();
        let id0 = make_tile(&mut mgr, Some("proj"));
        let id1 = make_tile(&mut mgr, Some("proj"));
        let id2 = make_tile(&mut mgr, Some("proj"));
        let id3 = make_tile(&mut mgr, Some("proj"));

        let filter = TabFilter::All;
        let cols = 2;

        // Start at t0 (row=0, col=0)
        mgr.select(id0);

        // Right → t1
        mgr.select_direction(&filter, cols, Direction::Right);
        assert_eq!(mgr.selected_id(), Some(id1));

        // Down → t3
        mgr.select_direction(&filter, cols, Direction::Down);
        assert_eq!(mgr.selected_id(), Some(id3));

        // Left → t2
        mgr.select_direction(&filter, cols, Direction::Left);
        assert_eq!(mgr.selected_id(), Some(id2));

        // Up → t0
        mgr.select_direction(&filter, cols, Direction::Up);
        assert_eq!(mgr.selected_id(), Some(id0));

        // Up at top row → stays at t0
        mgr.select_direction(&filter, cols, Direction::Up);
        assert_eq!(mgr.selected_id(), Some(id0));

        // Down → t2
        mgr.select_direction(&filter, cols, Direction::Down);
        assert_eq!(mgr.selected_id(), Some(id2));

        // Down from bottom row → stays at t2
        mgr.select_direction(&filter, cols, Direction::Down);
        assert_eq!(mgr.selected_id(), Some(id2));
    }

    #[test]
    fn test_get_nonexistent_returns_none() {
        let mgr = TileManager::new();
        assert!(mgr.get(TileId(999)).is_none());
    }

    #[test]
    fn test_remove_nonexistent_returns_none() {
        let mut mgr = TileManager::new();
        assert!(mgr.remove(TileId(999)).is_none());
    }

    #[test]
    fn test_select_clears_unread_and_burst() {
        let mut mgr = TileManager::new();
        let id = make_tile(&mut mgr, Some("proj"));
        // Simulate burst detection having triggered
        mgr.get_mut(id).unwrap().has_unread = true;
        mgr.get_mut(id).unwrap().burst_bytes = 1000;
        assert!(mgr.get(id).unwrap().has_unread);

        // Selecting should clear both has_unread and burst_bytes
        mgr.select(id);
        assert!(!mgr.get(id).unwrap().has_unread);
        assert_eq!(mgr.get(id).unwrap().burst_bytes, 0);
    }

    #[test]
    fn test_deselect_then_select_next() {
        let mut mgr = TileManager::new();
        let id1 = make_tile(&mut mgr, Some("alpha"));
        let _id2 = make_tile(&mut mgr, Some("alpha"));

        mgr.deselect();
        assert_eq!(mgr.selected_id(), None);

        // select_next with no selection should pick first
        mgr.select_next(&TabFilter::All);
        assert_eq!(mgr.selected_id(), Some(id1));
    }

    #[test]
    fn test_select_next_empty_filter() {
        let mut mgr = TileManager::new();
        let id = make_tile(&mut mgr, Some("proj"));
        mgr.select(id);

        // Filter for a nonexistent project — no tiles match
        mgr.select_next(&TabFilter::Project("nonexistent".into()));
        // Should keep current selection
        assert_eq!(mgr.selected_id(), Some(id));
    }

    #[test]
    fn test_select_prev_cycles() {
        let mut mgr = TileManager::new();
        let id1 = make_tile(&mut mgr, Some("alpha"));
        let id2 = make_tile(&mut mgr, Some("alpha"));
        let id3 = make_tile(&mut mgr, Some("alpha"));

        let filter = TabFilter::All;
        mgr.select(id1);

        // prev from first should wrap to last
        mgr.select_prev(&filter);
        assert_eq!(mgr.selected_id(), Some(id3));

        mgr.select_prev(&filter);
        assert_eq!(mgr.selected_id(), Some(id2));

        mgr.select_prev(&filter);
        assert_eq!(mgr.selected_id(), Some(id1));
    }

    #[test]
    fn test_select_direction_uneven_grid() {
        // 3 tiles in 2-column layout:
        //   row 0: t0 t1
        //   row 1: t2
        let mut mgr = TileManager::new();
        let id0 = make_tile(&mut mgr, Some("proj"));
        let id1 = make_tile(&mut mgr, Some("proj"));
        let id2 = make_tile(&mut mgr, Some("proj"));

        let filter = TabFilter::All;
        let cols = 2;

        // From t1 (row=0, col=1), Down should stay (no tile below)
        mgr.select(id1);
        mgr.select_direction(&filter, cols, Direction::Down);
        assert_eq!(mgr.selected_id(), Some(id1));

        // From t0 (row=0, col=0), Down → t2
        mgr.select(id0);
        mgr.select_direction(&filter, cols, Direction::Down);
        assert_eq!(mgr.selected_id(), Some(id2));

        // From t2 (row=1, col=0), Right should stay (no tile to right in last row)
        mgr.select(id2);
        mgr.select_direction(&filter, cols, Direction::Right);
        assert_eq!(mgr.selected_id(), Some(id2));
    }

    #[test]
    fn test_select_direction_zero_columns() {
        let mut mgr = TileManager::new();
        let id = make_tile(&mut mgr, Some("proj"));
        mgr.select(id);

        // 0 columns should be a no-op
        mgr.select_direction(&TabFilter::All, 0, Direction::Right);
        assert_eq!(mgr.selected_id(), Some(id));
    }

    #[test]
    fn test_select_direction_no_selection() {
        let mut mgr = TileManager::new();
        let id = make_tile(&mut mgr, Some("proj"));
        mgr.deselect();

        // With no selection, direction should select first tile
        mgr.select_direction(&TabFilter::All, 2, Direction::Right);
        assert_eq!(mgr.selected_id(), Some(id));
    }

    #[test]
    fn test_select_direction_single_column() {
        // 3 tiles in 1-column layout → pure vertical
        let mut mgr = TileManager::new();
        let id0 = make_tile(&mut mgr, Some("proj"));
        let id1 = make_tile(&mut mgr, Some("proj"));
        let id2 = make_tile(&mut mgr, Some("proj"));

        let filter = TabFilter::All;
        mgr.select(id0);

        // Right at col boundary → stays
        mgr.select_direction(&filter, 1, Direction::Right);
        assert_eq!(mgr.selected_id(), Some(id0));

        // Down → next tile
        mgr.select_direction(&filter, 1, Direction::Down);
        assert_eq!(mgr.selected_id(), Some(id1));

        mgr.select_direction(&filter, 1, Direction::Down);
        assert_eq!(mgr.selected_id(), Some(id2));
    }

    #[test]
    fn test_filtered_tiles_ordering() {
        let mut mgr = TileManager::new();
        // Git tiles should come before non-git tiles
        let _id_none = make_tile(&mut mgr, None);
        let _id_proj = make_tile(&mut mgr, Some("zeta"));
        let _id_proj2 = make_tile(&mut mgr, Some("alpha"));

        let all = mgr.filtered_tiles(&TabFilter::All);
        // Git tiles (prefixed "0:") sorted before non-git ("1:")
        assert!(all[0].git_context.is_some());
        assert!(all[1].git_context.is_some());
        assert!(all[2].git_context.is_none());
        // Git tiles sorted by project name
        assert_eq!(
            all[0].git_context.as_ref().unwrap().project_name,
            "alpha"
        );
        assert_eq!(
            all[1].git_context.as_ref().unwrap().project_name,
            "zeta"
        );
    }

    #[test]
    fn test_remove_first_tile_selects_next() {
        let mut mgr = TileManager::new();
        let id1 = make_tile(&mut mgr, Some("a"));
        let id2 = make_tile(&mut mgr, Some("b"));

        mgr.select(id1);
        mgr.remove(id1);

        // Should select id2 (the tile now at position 0)
        assert_eq!(mgr.selected_id(), Some(id2));
    }

    #[test]
    fn test_remove_last_in_list_selects_previous() {
        let mut mgr = TileManager::new();
        let id1 = make_tile(&mut mgr, Some("a"));
        let id2 = make_tile(&mut mgr, Some("b"));

        mgr.select(id2);
        mgr.remove(id2);

        // Should select id1 (clamped to len-1)
        assert_eq!(mgr.selected_id(), Some(id1));
    }

    #[test]
    fn test_remove_unselected_keeps_selection() {
        let mut mgr = TileManager::new();
        let id1 = make_tile(&mut mgr, Some("a"));
        let id2 = make_tile(&mut mgr, Some("b"));
        let id3 = make_tile(&mut mgr, Some("c"));

        mgr.select(id1);
        mgr.remove(id2);

        // id1 still selected
        assert_eq!(mgr.selected_id(), Some(id1));
        assert_eq!(mgr.tile_count(), 2);
        assert!(mgr.get(id3).is_some());
    }
}
