use std::io::Read;
use std::path::Path;

use crate::config::Config;
use crate::event::AppEvent;
use crate::input;
use crate::layout;
use crate::tab::{self, TabEntry, TabFilter};
use crate::tile::TileId;
use crate::tile_manager::TileManager;
use crate::ui;

/// How text selection expands during drag.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SelectionMode {
    /// Character-level (normal drag).
    Char,
    /// Word-level (double-click + drag).
    Word,
    /// Line-level (triple-click + drag).
    Line,
}

/// Text selection state for drag-to-copy.
pub struct TextSelection {
    /// Screen coordinates where drag started.
    pub start: (u16, u16),
    /// Screen coordinates where drag currently is.
    pub end: (u16, u16),
    /// Selection granularity.
    pub mode: SelectionMode,
    /// For Word/Line mode: the anchor word/line range (start_col, end_col) on the anchor row.
    /// Used to keep the original word/line selected even when dragging away.
    pub anchor_start: (u16, u16),
    pub anchor_end: (u16, u16),
}

use crossterm::event::{Event as CEvent, EventStream, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use futures::StreamExt;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::time::Duration;
use tokio::sync::mpsc;

/// Minimum accumulated bytes to consider a tile's output as a meaningful burst.
/// Must exceed Claude Code's periodic status updates (~1.2KB) to avoid false positives.
pub const UNREAD_BURST_THRESHOLD: usize = 4000;

/// How long a tile must be silent after a burst before marking as unread.
pub const UNREAD_SILENCE_DURATION: Duration = Duration::from_secs(5);

/// Tick interval for polling tile states (ms).
pub const TICK_INTERVAL_MS: u64 = 500;

/// Maximum time between clicks to count as double/triple click (ms).
pub const MULTI_CLICK_INTERVAL_MS: u64 = 400;

#[derive(Debug, Clone, PartialEq)]
pub enum AppMode {
    Normal,
    Overlay(OverlayKind),
}

#[derive(Debug, Clone, PartialEq)]
pub enum OverlayKind {
    Help,
    ConfirmClose(TileId),
    ProjectSelector {
        query: String,
        items: Vec<String>,
        selected: usize,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PtyBackendKind {
    Native,
    Tmux,
}

pub fn detect_backend() -> PtyBackendKind {
    if crate::tmux::is_tmux_available() {
        PtyBackendKind::Tmux
    } else {
        PtyBackendKind::Native
    }
}

pub struct App {
    config: Config,
    backend: PtyBackendKind,
    tile_manager: TileManager,
    mode: AppMode,
    active_tab: TabFilter,
    columns: u8,
    scroll_offset: usize,
    event_tx: mpsc::Sender<AppEvent>,
    event_rx: mpsc::Receiver<AppEvent>,
    should_quit: bool,
    /// Last computed layout for mouse hit testing.
    last_layout: Option<layout::LayoutResult>,
    /// Tile IDs in the order they appear in the grid (for mouse click mapping).
    last_filtered_ids: Vec<crate::tile::TileId>,
    /// Active text selection (drag in progress or completed).
    selection: Option<TextSelection>,
    /// Screen position where mouse was pressed (to distinguish click from drag).
    drag_origin: Option<(u16, u16)>,
    /// Multi-click tracking: timestamp of last MouseDown.
    last_click_time: Option<std::time::Instant>,
    /// Multi-click tracking: position of last MouseDown.
    last_click_pos: Option<(u16, u16)>,
    /// Multi-click tracking: consecutive click count (1=single, 2=double, 3=triple).
    click_count: u8,
    /// How many rows the detail panel is scrolled back into history (0 = follow cursor).
    detail_scroll_offset: usize,
    /// Path to the config file, used for hot reload detection.
    config_path: std::path::PathBuf,
    /// Last known modification time of the config file.
    config_last_modified: Option<std::time::SystemTime>,
    /// Spawned background tasks (readers, tick timer) to abort on shutdown.
    spawned_tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl App {
    pub fn new(config: Config) -> Self {
        let (event_tx, event_rx) = mpsc::channel(256);
        let columns = config.layout.default_columns.clamp(1, 3);
        let config_path = Config::config_path();
        let config_last_modified = std::fs::metadata(&config_path)
            .and_then(|m| m.modified())
            .ok();
        let backend = detect_backend();
        tracing::info!("PTY 后端: {:?}", backend);
        App {
            config,
            backend,
            tile_manager: TileManager::new(),
            mode: AppMode::Normal,
            active_tab: TabFilter::All,
            columns,
            scroll_offset: 0,
            event_tx,
            event_rx,
            should_quit: false,
            spawned_tasks: Vec::new(),
            last_layout: None,
            last_filtered_ids: Vec::new(),
            selection: None,
            drag_origin: None,
            last_click_time: None,
            last_click_pos: None,
            click_count: 0,
            detail_scroll_offset: 0,
            config_path,
            config_last_modified,
        }
    }

    pub fn spawn_tile(&mut self, cwd: &Path) -> anyhow::Result<TileId> {
        let id = self.tile_manager.next_tile_id();
        tracing::info!("Spawning tile {} in {:?} (backend={:?})", id.0, cwd, self.backend);
        // Use actual terminal dimensions for PTY size
        let (term_cols, term_rows) = crossterm::terminal::size().unwrap_or((80, 24));
        let (pty_cols, pty_rows) =
            Self::estimate_pty_size(term_cols, term_rows, self.config.layout.detail_panel_width);

        match self.backend {
            PtyBackendKind::Tmux => {
                let (tile, reader, _name) =
                    crate::tile::Tile::spawn_tmux(id, cwd, pty_cols, pty_rows)?;
                self.tile_manager.add(tile);
                let tx = self.event_tx.clone();
                let handle = tokio::spawn(tmux_reader_task(id, reader, tx));
                self.spawned_tasks.push(handle);
            }
            PtyBackendKind::Native => {
                let (tile, reader) = crate::tile::Tile::spawn(
                    id, &self.config.terminal.shell, cwd, pty_cols, pty_rows,
                )?;
                self.tile_manager.add(tile);
                let tx = self.event_tx.clone();
                let handle = tokio::spawn(pty_reader_task(id, reader, tx));
                self.spawned_tasks.push(handle);
            }
        }

        Ok(id)
    }

    pub async fn run(&mut self) -> anyhow::Result<()> {
        tracing::info!("App started");
        // Set up terminal
        enable_raw_mode()?;
        let mut stdout = std::io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        // Enable click + drag mouse tracking with SGR encoding.
        // Mode 1000: click events, 1002: drag (button motion), 1006: SGR extended coords.
        {
            use std::io::Write;
            stdout.write_all(b"\x1b[?1000h\x1b[?1002h\x1b[?1006h")?;
            stdout.flush()?;
        }
        let backend = CrosstermBackend::new(std::io::stdout());
        let mut terminal = Terminal::new(backend)?;

        // Spawn crossterm event reader
        let tx = self.event_tx.clone();
        self.spawned_tasks.push(tokio::spawn(crossterm_event_reader(tx.clone())));

        // Spawn tick timer
        let tx_tick = self.event_tx.clone();
        self.spawned_tasks.push(tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(TICK_INTERVAL_MS));
            loop {
                interval.tick().await;
                if tx_tick.send(AppEvent::Tick).await.is_err() {
                    break;
                }
            }
        }));

        loop {
            // Collect values before the draw borrow
            let tab_entries = self.compute_tab_entries();
            let columns = self.columns;
            let scroll_offset = self.scroll_offset;
            let detail_width = self.config.layout.detail_panel_width;
            let filtered_count = self.tile_manager.filtered_tiles(&self.active_tab).len();

            let filtered_ids: Vec<TileId> = self
                .tile_manager
                .filtered_tiles(&self.active_tab)
                .iter()
                .map(|t| t.id)
                .collect();

            let layout_result = layout::calculate_layout(
                ratatui::layout::Rect::new(0, 0, 1, 1), // placeholder, updated below
                columns,
                filtered_count,
                detail_width,
                scroll_offset,
            );

            // Build scrollback rows from output_history replay when scrolled.
            let scrollback_rows: Option<Vec<Vec<crate::screen::Cell>>> =
                if self.detail_scroll_offset > 0 {
                    if let Some(tile) = self.tile_manager.selected_mut() {
                        Some(tile.scrollback_lines().to_vec())
                    } else {
                        None
                    }
                } else {
                    None
                };

            // Draw and capture real layout + actual terminal area size
            let mut captured_layout = layout_result;
            let mut actual_terminal_size: Option<(u16, u16)> = None;
            terminal.draw(|frame| {
                let total = frame.area();
                captured_layout = layout::calculate_layout(
                    total,
                    columns,
                    filtered_count,
                    detail_width,
                    scroll_offset,
                );
                let render_result = ui::render(
                    frame,
                    &captured_layout,
                    &self.tile_manager,
                    &tab_entries,
                    &self.active_tab,
                    &self.mode,
                    columns,
                    &self.selection,
                    self.detail_scroll_offset,
                    self.tile_manager.selected_id().is_some(),
                    scrollback_rows.as_deref(),
                );
                actual_terminal_size = render_result.detail_terminal_size;
            })?;

            self.last_layout = Some(captured_layout.clone());
            self.last_filtered_ids = filtered_ids;

            // Sync PTY size with the ACTUAL terminal area reported by the renderer.
            // This is the precise size, not an estimate.
            if let Some((pty_cols, pty_rows)) = actual_terminal_size {
                self.sync_pty_sizes(pty_cols, pty_rows);
            }

            // Wait for at least one event, then drain all pending events before re-rendering.
            // This reduces render lag when multiple events arrive quickly (e.g. PTY output bursts).
            match self.event_rx.recv().await {
                Some(event) => self.handle_event(event),
                None => break,
            }
            // Drain remaining queued events
            while let Ok(event) = self.event_rx.try_recv() {
                self.handle_event(event);
                if self.should_quit {
                    break;
                }
            }

            if self.should_quit {
                break;
            }
        }

        // Restore terminal
        disable_raw_mode()?;
        {
            use std::io::Write;
            let mut stdout = std::io::stdout();
            stdout.write_all(b"\x1b[?1000l\x1b[?1002l\x1b[?1006l")?;
            stdout.flush()?;
        }
        execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
        terminal.show_cursor()?;
        tracing::info!("App stopped");

        Ok(())
    }

    fn handle_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::Crossterm(CEvent::Key(key)) => {
                // Clear any active selection on key press
                self.selection = None;

                // Only handle Press events (ignore Repeat/Release on some platforms)
                if key.kind != KeyEventKind::Press {
                    return;
                }

                // Overlay mode: delegate to input handler for confirmations/dismissals
                if matches!(self.mode, AppMode::Overlay(_)) {
                    input::handle_overlay_key(key, &mut self.mode, &mut self.tile_manager);
                    return;
                }

                // Normal mode: forward all keys to the selected PTY
                let bytes = input::key_event_to_bytes(&key);
                if !bytes.is_empty() {
                    if let Some(tile) = self.tile_manager.selected_mut() {
                        tile.has_unread = false;
                        tile.burst_bytes = 0;
                        let _ = tile.write_input(&bytes);
                    }
                }
            }
            AppEvent::Crossterm(CEvent::Mouse(mouse)) => {
                self.handle_mouse(mouse);
            }
            AppEvent::Crossterm(CEvent::Resize(_cols, _rows)) => {
                // PTY resize is handled by sync_pty_sizes after the next render,
                // using the actual terminal area dimensions from the renderer.
                // No need to estimate here.
            }
            AppEvent::Crossterm(_) => {}
            AppEvent::PtyOutput(tile_id, data) => {
                if let Some(tile) = self.tile_manager.get_mut(tile_id) {
                    tile.process_output(&data);
                    tile.burst_bytes += data.len();
                } else {
                    tracing::debug!("PtyOutput for unknown tile {:?}", tile_id);
                }
                // When user has scrolled back, keep their position (scroll lock).
                // Only auto-follow when already at bottom (offset == 0).
                // Switching tiles resets scroll separately in handle_click.
            }
            AppEvent::PtyExited(tile_id) => {
                tracing::info!("PTY exited for tile {}", tile_id.0);
                self.tile_manager.remove(tile_id);
            }
            AppEvent::CwdChanged(tile_id, new_cwd) => {
                if let Some(tile) = self.tile_manager.get_mut(tile_id) {
                    tile.update_cwd(new_cwd);
                }
            }
            AppEvent::Tick => {
                self.check_config_reload();
                self.poll_tile_states();
                // Auto-remove any tiles whose PTY has exited (fallback for cases not caught by PtyExited)
                let exited: Vec<TileId> = self
                    .tile_manager
                    .tiles()
                    .iter()
                    .filter(|t| matches!(t.status, crate::tile::TileStatus::Exited))
                    .map(|t| t.id)
                    .collect();
                for id in exited {
                    tracing::info!("Auto-removing exited tile {}", id.0);
                    self.tile_manager.remove(id);
                }
            }
        }
    }

    fn handle_mouse(&mut self, mouse: crossterm::event::MouseEvent) {
        use crossterm::event::{MouseButton, MouseEventKind};

        let x = mouse.column;
        let y = mouse.row;

        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let now = std::time::Instant::now();
                let is_multi = self.last_click_time.map_or(false, |t| {
                    now.duration_since(t) < Duration::from_millis(MULTI_CLICK_INTERVAL_MS)
                }) && self.last_click_pos.map_or(false, |(px, py)| {
                    x.abs_diff(px) <= 2 && y.abs_diff(py) <= 2
                });

                if is_multi {
                    self.click_count = (self.click_count + 1).min(3);
                } else {
                    self.click_count = 1;
                }
                self.last_click_time = Some(now);
                self.last_click_pos = Some((x, y));

                match self.click_count {
                    2 => {
                        // Double-click: select word
                        if let Some(sel) = self.select_word_at(x, y) {
                            self.selection = Some(sel);
                        }
                        self.drag_origin = Some((x, y));
                    }
                    3 => {
                        // Triple-click: select line
                        if let Some(sel) = self.select_line_at(x, y) {
                            self.selection = Some(sel);
                        }
                        self.drag_origin = Some((x, y));
                    }
                    _ => {
                        // Single click: record drag origin, clear selection
                        self.drag_origin = Some((x, y));
                        self.selection = None;
                    }
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if let Some((sx, sy)) = self.drag_origin {
                    match self.click_count {
                        2 => {
                            // Word-mode drag: extend from anchor word to current word
                            if let Some(sel) = self.extend_word_selection(sx, sy, x, y) {
                                self.selection = Some(sel);
                            }
                        }
                        3 => {
                            // Line-mode drag: extend from anchor line to current line
                            if let Some(sel) = self.extend_line_selection(sx, sy, x, y) {
                                self.selection = Some(sel);
                            }
                        }
                        _ => {
                            // Char-mode drag
                            self.selection = Some(TextSelection {
                                start: (sx, sy),
                                end: (x, y),
                                mode: SelectionMode::Char,
                                anchor_start: (sx, sy),
                                anchor_end: (sx, sy),
                            });
                        }
                    }
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                if let Some(sel) = self.selection.take() {
                    // Had a selection (drag or multi-click) — copy to clipboard
                    self.copy_selection_to_clipboard(&sel);
                    self.drag_origin = None;
                } else if let Some((ox, oy)) = self.drag_origin.take() {
                    // Single click with no drag — handle tile selection
                    if self.click_count <= 1 {
                        self.handle_click(ox, oy);
                    }
                }
            }
            MouseEventKind::ScrollUp => {
                if let Some(ref layout) = self.last_layout {
                    let grid = layout.grid_area;
                    // Detail panel area → scroll history back
                    if let Some(detail) = layout.detail_panel {
                        if x >= detail.x
                            && x < detail.x + detail.width
                            && y >= detail.y
                            && y < detail.y + detail.height
                        {
                            // Clamp to total scrollback lines from output_history
                            let max_scroll = self
                                .tile_manager
                                .selected_mut()
                                .map(|t| {
                                    let screen_rows = t.vte.rows() as usize;
                                    t.scrollback_line_count().saturating_sub(screen_rows)
                                })
                                .unwrap_or(0);
                            self.detail_scroll_offset =
                                (self.detail_scroll_offset + 3).min(max_scroll);
                            return;
                        }
                    }
                    // Grid area → scroll grid up (only if scrolled down)
                    if x >= grid.x
                        && x < grid.x + grid.width
                        && y >= grid.y
                        && y < grid.y + grid.height
                        && self.scroll_offset > 0
                    {
                        self.scroll_offset -= 1;
                    }
                }
            }
            MouseEventKind::ScrollDown => {
                if let Some(ref layout) = self.last_layout {
                    let grid = layout.grid_area;
                    // Detail panel area → scroll history forward
                    if let Some(detail) = layout.detail_panel {
                        if x >= detail.x
                            && x < detail.x + detail.width
                            && y >= detail.y
                            && y < detail.y + detail.height
                        {
                            self.detail_scroll_offset =
                                self.detail_scroll_offset.saturating_sub(3);
                            return;
                        }
                    }
                    tracing::debug!(
                        "ScrollDown: mouse=({},{}) grid=({},{} {}x{}) scroll={}/{} tiles={}",
                        x, y, grid.x, grid.y, grid.width, grid.height,
                        self.scroll_offset, layout.max_scroll_offset,
                        layout.tile_rects.len(),
                    );
                    // Grid area → scroll grid down (bounded by max)
                    if x >= grid.x
                        && x < grid.x + grid.width
                        && y >= grid.y
                        && y < grid.y + grid.height
                        && self.scroll_offset < layout.max_scroll_offset
                    {
                        self.scroll_offset += 1;
                    }
                }
            }
            _ => {}
        }
    }

    fn handle_click(&mut self, x: u16, y: u16) {
        let layout = match &self.last_layout {
            Some(l) => l.clone(),
            None => return,
        };

        // Tab bar buttons (right side): " [+] [X] "
        // [X] is rightmost (5 chars), [+] is left of [X] (5 chars)
        if y >= layout.tab_bar.y && y < layout.tab_bar.y + 1 {
            let bar_right = layout.tab_bar.x + layout.tab_bar.width;
            // [X] quit button
            let x_btn_start = bar_right.saturating_sub(5);
            if x >= x_btn_start && x < bar_right {
                self.should_quit = true;
                return;
            }
            // [+] new tile button
            let plus_btn_start = x_btn_start.saturating_sub(5);
            if x >= plus_btn_start && x < x_btn_start {
                let cwd = self
                    .tile_manager
                    .selected()
                    .map(|t| t.cwd.clone())
                    .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
                if let Ok(id) = self.spawn_tile(&cwd) {
                    self.tile_manager.select(id);
                }
                return;
            }
        }

        // Click on tab bar (but not the buttons)?
        if y >= layout.tab_bar.y && y < layout.tab_bar.y + layout.tab_bar.height {
            let tab_entries = self.compute_tab_entries();
            self.active_tab = tab::next_tab(&self.active_tab, &tab_entries);
            return;
        }

        // Status bar buttons (right side): " [?] [Ncol] "
        // From right: [Ncol]=7, [?]=5
        if y >= layout.status_bar.y
            && y < layout.status_bar.y + layout.status_bar.height
        {
            let bar_right = layout.status_bar.x + layout.status_bar.width;
            // [Ncol] column toggle
            let col_btn_start = bar_right.saturating_sub(7);
            if x >= col_btn_start && x < bar_right {
                self.columns = match self.columns {
                    1 => 2,
                    2 => 3,
                    _ => 1,
                };
                return;
            }
            // [?] help
            let help_btn_start = col_btn_start.saturating_sub(5);
            if x >= help_btn_start && x < col_btn_start {
                self.mode = AppMode::Overlay(OverlayKind::Help);
                return;
            }
        }

        // Click on a tile card?
        for (i, rect) in layout.tile_rects.iter().enumerate() {
            if x >= rect.x && x < rect.x + rect.width && y >= rect.y && y < rect.y + rect.height {
                // Map visible tile index to absolute filtered index
                let absolute_idx = layout.first_visible_tile + i;
                if let Some(&tile_id) = self.last_filtered_ids.get(absolute_idx) {
                    // Check close button on tile cards
                    // Close button is at top-right of inner area: x = rect.x + rect.width - 2 (border), y = rect.y + 1 (border)
                    let close_x = rect.x + rect.width - 2;
                    let close_y = rect.y + 1;
                    if x == close_x && y == close_y {
                        let needs_confirm = self
                            .tile_manager
                            .get(tile_id)
                            .map(|t| t.status == crate::tile::TileStatus::Running)
                            .unwrap_or(false);
                        if needs_confirm {
                            self.mode = AppMode::Overlay(OverlayKind::ConfirmClose(tile_id));
                        } else {
                            if let Some(sname) = self.tile_manager.get(tile_id).and_then(|t| t.session_name.clone()) {
                                crate::tmux::kill_session(&sname);
                            }
                            self.tile_manager.remove(tile_id);
                        }
                        return;
                    }

                    let prev_selected = self.tile_manager.selected_id();
                    self.tile_manager.select(tile_id);
                    // Reset detail scroll when switching to a different tile
                    if prev_selected != Some(tile_id) {
                        self.detail_scroll_offset = 0;
                    }
                }
                return;
            }
        }

        // Click on detail panel — clear unread for the selected tile (user is looking at it)
        if let Some(d) = layout.detail_panel {
            if x >= d.x && x < d.x + d.width && y >= d.y && y < d.y + d.height {
                if let Some(tile) = self.tile_manager.selected_mut() {
                    tile.has_unread = false;
                    tile.burst_bytes = 0;
                }
                return;
            }
        }

        // Click elsewhere → keep current selection (layout stays stable)
    }

    /// Get detail panel terminal area geometry, or None if not available.
    fn detail_term_area(&self) -> Option<(u16, u16, u16, u16)> {
        let layout = self.last_layout.as_ref()?;
        let detail = layout.detail_panel?;
        let term_x = detail.x + crate::layout::DETAIL_BORDER_WIDTH;
        let term_y = detail.y + crate::layout::DETAIL_HEADER_HEIGHT;
        let term_w = detail
            .width
            .saturating_sub(crate::layout::DETAIL_BORDER_WIDTH);
        let term_h = detail
            .height
            .saturating_sub(crate::layout::DETAIL_HEADER_HEIGHT);
        if term_w == 0 || term_h == 0 {
            return None;
        }
        Some((term_x, term_y, term_w, term_h))
    }

    /// Convert screen (x, y) to terminal buffer (row, col).
    /// Returns None if outside the detail panel terminal area.
    fn screen_to_buffer(&self, x: u16, y: u16) -> Option<(u16, u16)> {
        let (term_x, term_y, term_w, term_h) = self.detail_term_area()?;
        if x < term_x || y < term_y {
            return None;
        }
        let col = x.saturating_sub(term_x);
        let row = y.saturating_sub(term_y);
        if col >= term_w || row >= term_h {
            return None;
        }

        let tile = self.tile_manager.selected()?;
        let vte = &tile.vte;
        let (cursor_row, _) = vte.cursor_position();
        let visible_rows = term_h as usize;
        let total_rows = vte.rows() as usize;

        let view_start = if total_rows <= visible_rows || (cursor_row as usize) < visible_rows {
            0
        } else {
            (cursor_row as usize + 1).saturating_sub(visible_rows)
        };

        let buf_row = view_start as u16 + row;
        if buf_row >= vte.rows() {
            return None;
        }
        Some((buf_row, col))
    }

    /// Select the word at screen position (x, y). Returns None if outside terminal area.
    fn select_word_at(&self, x: u16, y: u16) -> Option<TextSelection> {
        let (term_x, term_y, _, _) = self.detail_term_area()?;
        let (buf_row, buf_col) = self.screen_to_buffer(x, y)?;
        let tile = self.tile_manager.selected()?;
        let (word_start, word_end) = find_word_bounds(&tile.vte, buf_row, buf_col);

        let screen_start = (term_x + word_start, term_y + (y - term_y));
        let screen_end = (term_x + word_end, term_y + (y - term_y));

        Some(TextSelection {
            start: screen_start,
            end: screen_end,
            mode: SelectionMode::Word,
            anchor_start: screen_start,
            anchor_end: screen_end,
        })
    }

    /// Select the entire line at screen position (x, y).
    fn select_line_at(&self, _x: u16, y: u16) -> Option<TextSelection> {
        let (term_x, _term_y, term_w, _) = self.detail_term_area()?;
        // Verify we're in the terminal area
        let _ = self.screen_to_buffer(term_x, y)?;

        let screen_start = (term_x, y);
        let screen_end = (term_x + term_w.saturating_sub(1), y);

        Some(TextSelection {
            start: screen_start,
            end: screen_end,
            mode: SelectionMode::Line,
            anchor_start: screen_start,
            anchor_end: screen_end,
        })
    }

    /// Extend a word-mode selection from the anchor click to the current drag position.
    fn extend_word_selection(
        &self,
        _sx: u16,
        _sy: u16,
        x: u16,
        y: u16,
    ) -> Option<TextSelection> {
        // Get the anchor from the existing selection
        let existing = self.selection.as_ref()?;
        let (term_x, term_y, _, _) = self.detail_term_area()?;

        // Find the word at the current drag position
        let (buf_row, buf_col) = self.screen_to_buffer(x, y)?;
        let tile = self.tile_manager.selected()?;
        let (word_start, word_end) = find_word_bounds(&tile.vte, buf_row, buf_col);

        let cur_start = (term_x + word_start, term_y + (y - term_y));
        let cur_end = (term_x + word_end, term_y + (y - term_y));

        // Union of anchor word and current word
        let (sel_start, sel_end) = union_ranges(
            existing.anchor_start,
            existing.anchor_end,
            cur_start,
            cur_end,
        );

        Some(TextSelection {
            start: sel_start,
            end: sel_end,
            mode: SelectionMode::Word,
            anchor_start: existing.anchor_start,
            anchor_end: existing.anchor_end,
        })
    }

    /// Extend a line-mode selection from the anchor click to the current drag position.
    fn extend_line_selection(
        &self,
        _sx: u16,
        _sy: u16,
        _x: u16,
        y: u16,
    ) -> Option<TextSelection> {
        let existing = self.selection.as_ref()?;
        let (term_x, term_y, term_w, term_h) = self.detail_term_area()?;

        // Clamp y to terminal area
        let clamped_y = y.clamp(term_y, term_y + term_h.saturating_sub(1));

        let cur_start = (term_x, clamped_y);
        let cur_end = (term_x + term_w.saturating_sub(1), clamped_y);

        let (sel_start, sel_end) = union_ranges(
            existing.anchor_start,
            existing.anchor_end,
            cur_start,
            cur_end,
        );

        Some(TextSelection {
            start: sel_start,
            end: sel_end,
            mode: SelectionMode::Line,
            anchor_start: existing.anchor_start,
            anchor_end: existing.anchor_end,
        })
    }

    fn copy_selection_to_clipboard(&self, sel: &TextSelection) {
        let layout = match &self.last_layout {
            Some(l) => l,
            None => return,
        };

        let detail = match layout.detail_panel {
            Some(d) => d,
            None => return, // No detail panel → nothing to copy from
        };

        let tile = match self.tile_manager.selected() {
            Some(t) => t,
            None => return,
        };

        // Compute the terminal area within the detail panel
        // (detail panel minus left border and header)
        let term_x = detail.x + crate::layout::DETAIL_BORDER_WIDTH;
        let term_y = detail.y + crate::layout::DETAIL_HEADER_HEIGHT;
        let term_w = detail
            .width
            .saturating_sub(crate::layout::DETAIL_BORDER_WIDTH);
        let term_h = detail
            .height
            .saturating_sub(crate::layout::DETAIL_HEADER_HEIGHT);

        if term_w == 0 || term_h == 0 {
            return;
        }

        // Normalize selection coordinates (start <= end) in terminal-area-relative coords
        let (start_row, start_col, end_row, end_col) =
            normalize_selection(sel.start, sel.end, term_x, term_y, term_w, term_h);

        // Compute view_start (same logic as detail_panel render's visible_rows_with_cursor)
        let vte = &tile.vte;
        let (cursor_row, _) = vte.cursor_position();
        let visible_rows = term_h as usize;
        let total_rows = vte.rows() as usize;

        let view_start = if total_rows <= visible_rows || (cursor_row as usize) < visible_rows {
            0
        } else {
            (cursor_row as usize + 1).saturating_sub(visible_rows)
        };

        let mut text = String::new();
        for screen_y in start_row..=end_row {
            let buf_row = view_start + screen_y as usize;
            if buf_row >= total_rows {
                break;
            }

            let col_start = if screen_y == start_row { start_col } else { 0 };
            let col_end = if screen_y == end_row {
                end_col
            } else {
                term_w.saturating_sub(1)
            };

            for col in col_start..=col_end {
                if col >= vte.cols() {
                    break;
                }
                let cell = vte.cell_at(buf_row as u16, col);
                if !cell.is_wide_continuation {
                    text.push(cell.ch);
                }
            }

            // Add newline between rows (but not after last row), trimming trailing spaces
            if screen_y < end_row {
                while text.ends_with(' ') {
                    text.pop();
                }
                text.push('\n');
            }
        }

        let text = text.trim_end().to_string();
        if text.is_empty() {
            return;
        }

        // Copy to clipboard via pbcopy (macOS)
        tracing::info!("Copying {} chars to clipboard", text.len());
        if let Ok(mut child) = std::process::Command::new("pbcopy")
            .stdin(std::process::Stdio::piped())
            .spawn()
        {
            if let Some(stdin) = child.stdin.as_mut() {
                use std::io::Write;
                let _ = stdin.write_all(text.as_bytes());
            }
            let _ = child.wait();
        }
    }

    pub fn compute_tab_entries(&self) -> Vec<TabEntry> {
        let contexts: Vec<_> = self
            .tile_manager
            .tiles()
            .iter()
            .map(|t| t.git_context.clone())
            .collect();
        tab::aggregate_tabs(&contexts)
    }

    fn check_config_reload(&mut self) {
        let current_mtime = std::fs::metadata(&self.config_path)
            .and_then(|m| m.modified())
            .ok();

        if current_mtime != self.config_last_modified && current_mtime.is_some() {
            self.config_last_modified = current_mtime;
            let new_config = Config::load(&self.config_path);
            // Apply hot-reloadable settings.
            // columns and detail_panel_width can change live;
            // shell/cwd_poll_interval require a restart to take effect.
            tracing::info!("Config reloaded from {:?}", self.config_path);
            self.config = new_config;
        }
    }

    fn poll_tile_states(&mut self) {
        #[cfg(target_os = "macos")]
        {
            use crate::process::get_process_cwd;
            let tile_ids: Vec<TileId> = self.tile_manager.tiles().iter().map(|t| t.id).collect();

            for id in tile_ids {
                if let Some(tile) = self.tile_manager.get(id) {
                    let pid = tile.pty.pid();
                    let current_cwd = tile.cwd.clone();
                    if let Some(pid) = pid {
                        if let Some(new_cwd) = get_process_cwd(pid as i32) {
                            if new_cwd != current_cwd {
                                let tx = self.event_tx.clone();
                                let _ = tx.try_send(AppEvent::CwdChanged(id, new_cwd));
                            }
                        }
                    }
                }
            }
        }

        // Update tile statuses
        let tile_ids: Vec<TileId> = self.tile_manager.tiles().iter().map(|t| t.id).collect();

        for id in tile_ids {
            #[cfg(unix)]
            let (is_fg_shell, fg_name) = {
                if let Some(tile) = self.tile_manager.get(id) {
                    let master_fd = tile.pty.master_fd();
                    let pty_pid = tile.pty.pid();
                    if let (Some(fd), Some(pid)) = (master_fd, pty_pid) {
                        // Native PTY: use tcgetpgrp to get foreground PID
                        if let Some(fg_pid) = crate::process::get_foreground_pid(fd) {
                            let name = crate::process::get_process_name(fg_pid);
                            (fg_pid == pid as i32, name)
                        } else {
                            (true, None)
                        }
                    } else if let Some(session_name) = &tile.session_name {
                        // Tmux backend: use tmux queries for foreground process
                        let fg_cmd = crate::tmux::pane_foreground_command(session_name);
                        let is_shell = fg_cmd
                            .as_ref()
                            .map(|c| matches!(c.as_str(), "zsh" | "bash" | "fish" | "sh" | "dash"))
                            .unwrap_or(true);

                        // For Claude detection, resolve via proc_pidpath if fg is not shell
                        let fg_name = if is_shell {
                            fg_cmd
                        } else {
                            // Try to get the actual process name via proc_pidpath (resolves symlinks)
                            crate::tmux::pane_foreground_pid(session_name)
                                .and_then(|pid| crate::process::get_process_name(pid))
                                .or(fg_cmd)
                        };
                        (is_shell, fg_name)
                    } else {
                        (true, None)
                    }
                } else {
                    (true, None)
                }
            };

            #[cfg(not(unix))]
            let (is_fg_shell, fg_name): (bool, Option<String>) = (true, None);

            if let Some(tile) = self.tile_manager.get_mut(id) {
                tile.update_status(is_fg_shell);
                tile.fg_process_name = fg_name;
            }
        }

        // Detect "burst + silence" pattern for Claude Code unread notification.
        // Only Claude Code tiles get the yellow border + system notification.
        for tile in self.tile_manager.tiles_mut() {
            if !tile.is_claude_code() {
                continue;
            }
            if tile.has_unread {
                continue; // already marked, wait for user to click
            }
            if tile.burst_bytes >= UNREAD_BURST_THRESHOLD
                && tile.last_active.elapsed() >= UNREAD_SILENCE_DURATION
            {
                tracing::info!(
                    "Tile {} unread triggered: burst_bytes={}, last_active_ago={:?}",
                    tile.id.0,
                    tile.burst_bytes,
                    tile.last_active.elapsed(),
                );
                tile.has_unread = true;
                tile.burst_bytes = 0;

                // System notification
                {
                    #[cfg(target_os = "macos")]
                    {
                        let context = tile
                            .git_context
                            .as_ref()
                            .map(|g| {
                                if g.is_worktree {
                                    g.worktree_name
                                        .as_deref()
                                        .unwrap_or(&g.project_name)
                                        .to_string()
                                } else {
                                    g.project_name.clone()
                                }
                            })
                            .unwrap_or_else(|| {
                                tile.cwd
                                    .file_name()
                                    .and_then(|n| n.to_str())
                                    .unwrap_or("unknown")
                                    .to_string()
                            });
                        let script = format!(
                            "display notification \"任务完成，等待输入\" with title \"Claude Code — {}\"",
                            context.replace('"', "\\\"")
                        );
                        let _ = std::process::Command::new("osascript")
                            .args(["-e", &script])
                            .spawn();
                    }
                }
            }
        }
    }

    pub fn tile_manager_ref(&self) -> &TileManager {
        &self.tile_manager
    }

    pub fn tile_manager_mut(&mut self) -> &mut TileManager {
        &mut self.tile_manager
    }

    /// Try to receive a pending event without blocking.
    pub fn try_recv_event(&mut self) -> Result<crate::event::AppEvent, tokio::sync::mpsc::error::TryRecvError> {
        self.event_rx.try_recv()
    }

    /// Reconnect to an existing tmux session.
    /// `vte_cols`/`vte_rows` should match the tmux pane's current dimensions
    /// so that captured content maps correctly to VTE coordinates.
    pub fn reconnect_tile(
        &mut self,
        session_name: &str,
        cwd: &Path,
        vte_cols: u16,
        vte_rows: u16,
    ) -> anyhow::Result<TileId> {
        let id = self.tile_manager.next_tile_id();
        let (tile, reader) =
            crate::tile::Tile::reconnect_tmux(id, session_name, cwd, vte_cols, vte_rows)?;
        self.tile_manager.add(tile);
        let tx = self.event_tx.clone();
        let handle = tokio::spawn(tmux_reader_task(id, reader, tx));
        self.spawned_tasks.push(handle);
        tracing::info!("Reconnected tile {} to tmux session {}", id.0, session_name);
        Ok(id)
    }

    /// Restore scrollback content into a tile's VTE (for native backend session restore).
    pub fn restore_tile_scrollback(&mut self, tile_id: TileId, data: &[u8]) {
        if let Some(tile) = self.tile_manager.get_mut(tile_id) {
            tile.vte.process(data);
            tile.vte.process(b"\x1b[?25h"); // show cursor
            tracing::debug!(
                "Restored {} bytes scrollback for tile {}",
                data.len(),
                tile_id.0,
            );
        }
    }

    /// Abort all spawned background tasks (readers, tick timer).
    pub fn shutdown_tasks(&mut self) {
        for handle in self.spawned_tasks.drain(..) {
            handle.abort();
        }
    }

    pub fn columns(&self) -> u8 {
        self.columns
    }

    pub fn detail_panel_width_pct(&self) -> u16 {
        self.config.layout.detail_panel_width
    }

    pub fn backend(&self) -> PtyBackendKind {
        self.backend
    }

    pub fn set_columns(&mut self, columns: u8) {
        self.columns = columns.clamp(1, 3);
    }

    /// Estimate PTY dimensions before first render (when detail panel rect is unknown).
    pub fn estimate_pty_size(term_cols: u16, term_rows: u16, detail_width_pct: u16) -> (u16, u16) {
        let detail_width = ((term_cols as u32 * detail_width_pct as u32) / 100) as u16;
        let cols = detail_width
            .saturating_sub(crate::layout::DETAIL_BORDER_WIDTH)
            .max(10);
        let rows = term_rows
            .saturating_sub(
                crate::layout::TAB_BAR_HEIGHT
                    + crate::layout::STATUS_BAR_HEIGHT
                    + crate::layout::DETAIL_HEADER_HEIGHT,
            )
            .max(5);
        (cols, rows)
    }

    /// Resize all tiles' PTY + screen buffer to match the actual detail panel terminal area.
    /// Only resizes tiles whose current screen dimensions differ.
    fn sync_pty_sizes(&mut self, cols: u16, rows: u16) {
        for tile in self.tile_manager.tiles_mut() {
            if tile.vte.cols() != cols || tile.vte.rows() != rows {
                tracing::debug!("Resizing tile {} PTY to {}x{}", tile.id.0, cols, rows);
                let _ = tile.resize(cols, rows);
            }
        }
    }
}

/// Normalize selection to (start_row, start_col, end_row, end_col) in terminal-area-relative coords.
fn normalize_selection(
    start: (u16, u16),
    end: (u16, u16),
    term_x: u16,
    term_y: u16,
    term_w: u16,
    term_h: u16,
) -> (u16, u16, u16, u16) {
    let clamp_x = |x: u16| x.saturating_sub(term_x).min(term_w.saturating_sub(1));
    let clamp_y = |y: u16| y.saturating_sub(term_y).min(term_h.saturating_sub(1));

    let (sy, sx) = (clamp_y(start.1), clamp_x(start.0));
    let (ey, ex) = (clamp_y(end.1), clamp_x(end.0));

    // Ensure start <= end (row-major order)
    if sy < ey || (sy == ey && sx <= ex) {
        (sy, sx, ey, ex)
    } else {
        (ey, ex, sy, sx)
    }
}

/// Check if a character is a word separator.
fn is_word_separator(ch: char) -> bool {
    ch.is_whitespace() || matches!(ch, '/' | '\\' | ':' | '.' | ',' | ';' | '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>' | '"' | '\'' | '`' | '|' | '&' | '=' | '+' | '-' | '*' | '!' | '?' | '@' | '#' | '$' | '%' | '^' | '~')
}

/// Find word boundaries around a given column in a buffer row.
/// Returns (start_col, end_col) inclusive.
fn find_word_bounds(vte: &crate::screen::VteState, row: u16, col: u16) -> (u16, u16) {
    let cols = vte.cols();
    let center_cell = vte.cell_at(row, col);

    // If clicked on a separator, select just that character
    if is_word_separator(center_cell.ch) {
        return (col, col);
    }

    // Expand left
    let mut start = col;
    while start > 0 {
        let cell = vte.cell_at(row, start - 1);
        if is_word_separator(cell.ch) {
            break;
        }
        start -= 1;
    }

    // Expand right
    let mut end = col;
    while end + 1 < cols {
        let cell = vte.cell_at(row, end + 1);
        if is_word_separator(cell.ch) {
            break;
        }
        end += 1;
    }

    (start, end)
}

/// Union two screen-coordinate ranges into a single range covering both.
/// Each range is (start_x, start_y) to (end_x, end_y) in screen coords.
fn union_ranges(
    a_start: (u16, u16),
    a_end: (u16, u16),
    b_start: (u16, u16),
    b_end: (u16, u16),
) -> ((u16, u16), (u16, u16)) {
    // Compare in row-major order: (y, x)
    let min_start = if (a_start.1, a_start.0) <= (b_start.1, b_start.0) {
        a_start
    } else {
        b_start
    };
    let max_end = if (a_end.1, a_end.0) >= (b_end.1, b_end.0) {
        a_end
    } else {
        b_end
    };
    (min_start, max_end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_word_separator() {
        assert!(is_word_separator(' '));
        assert!(is_word_separator('/'));
        assert!(is_word_separator(':'));
        assert!(is_word_separator('.'));
        assert!(!is_word_separator('a'));
        assert!(!is_word_separator('Z'));
        assert!(!is_word_separator('0'));
        assert!(!is_word_separator('_')); // underscore is part of identifiers
    }

    #[test]
    fn test_find_word_bounds_middle() {
        let mut vte = crate::screen::VteState::new(20, 5);
        vte.process(b"hello world foo");
        // "hello" at cols 0-4, "world" at 6-10, "foo" at 12-14
        assert_eq!(find_word_bounds(&vte, 0, 2), (0, 4)); // click on 'l' in "hello"
        assert_eq!(find_word_bounds(&vte, 0, 7), (6, 10)); // click on 'o' in "world"
        assert_eq!(find_word_bounds(&vte, 0, 12), (12, 14)); // click on 'f' in "foo"
    }

    #[test]
    fn test_find_word_bounds_separator() {
        let mut vte = crate::screen::VteState::new(20, 5);
        vte.process(b"hello world");
        // Click on space between words
        assert_eq!(find_word_bounds(&vte, 0, 5), (5, 5));
    }

    #[test]
    fn test_find_word_bounds_path() {
        let mut vte = crate::screen::VteState::new(30, 5);
        vte.process(b"/usr/local/bin");
        // '/' is a separator, so each path component is a word
        assert_eq!(find_word_bounds(&vte, 0, 1), (1, 3)); // "usr"
        assert_eq!(find_word_bounds(&vte, 0, 5), (5, 9)); // "local"
        assert_eq!(find_word_bounds(&vte, 0, 0), (0, 0)); // "/"
    }

    #[test]
    fn test_find_word_bounds_edges() {
        let mut vte = crate::screen::VteState::new(10, 5);
        vte.process(b"abcdefghij");
        // Entire row is one word (no separators)
        assert_eq!(find_word_bounds(&vte, 0, 0), (0, 9));
        assert_eq!(find_word_bounds(&vte, 0, 9), (0, 9));
    }

    #[test]
    fn test_union_ranges_same_line() {
        let (start, end) = union_ranges((2, 0), (5, 0), (8, 0), (12, 0));
        assert_eq!(start, (2, 0));
        assert_eq!(end, (12, 0));
    }

    #[test]
    fn test_union_ranges_reversed() {
        // Second range before first
        let (start, end) = union_ranges((8, 1), (12, 1), (2, 0), (5, 0));
        assert_eq!(start, (2, 0));
        assert_eq!(end, (12, 1));
    }

    #[test]
    fn test_union_ranges_multi_line() {
        let (start, end) = union_ranges((0, 2), (10, 2), (0, 5), (10, 5));
        assert_eq!(start, (0, 2));
        assert_eq!(end, (10, 5));
    }

    #[test]
    fn test_estimate_pty_size_normal() {
        let (cols, rows) = App::estimate_pty_size(200, 50, 60);
        // detail_width = 200 * 60 / 100 = 120, minus border
        assert!(cols > 100);
        assert!(rows > 40);
    }

    #[test]
    fn test_estimate_pty_size_small_terminal() {
        let (cols, rows) = App::estimate_pty_size(20, 10, 60);
        // Should clamp to minimums (10 cols, 5 rows)
        assert!(cols >= 10);
        assert!(rows >= 5);
    }

    #[test]
    fn test_estimate_pty_size_zero_detail_width() {
        let (cols, rows) = App::estimate_pty_size(100, 50, 0);
        // 0% detail width → cols should be minimum (10)
        assert_eq!(cols, 10);
        assert!(rows > 5);
    }

    #[test]
    fn test_normalize_selection_forward() {
        let (sy, sx, ey, ex) = normalize_selection((15, 5), (25, 10), 10, 3, 40, 20);
        // Relative to terminal area: start=(5,2), end=(15,7)
        assert_eq!(sy, 2);
        assert_eq!(sx, 5);
        assert_eq!(ey, 7);
        assert_eq!(ex, 15);
    }

    #[test]
    fn test_normalize_selection_reversed() {
        // End before start → should normalize to forward order
        let (sy, sx, ey, ex) = normalize_selection((25, 10), (15, 5), 10, 3, 40, 20);
        assert_eq!(sy, 2);
        assert_eq!(sx, 5);
        assert_eq!(ey, 7);
        assert_eq!(ex, 15);
    }

    #[test]
    fn test_normalize_selection_clamped() {
        // Coordinates outside terminal area should be clamped
        let (sy, sx, ey, ex) = normalize_selection((0, 0), (200, 200), 10, 10, 40, 20);
        assert_eq!(sx, 0);
        assert_eq!(sy, 0);
        assert_eq!(ex, 39);
        assert_eq!(ey, 19);
    }

    #[test]
    fn test_normalize_selection_same_line() {
        let (sy, sx, ey, ex) = normalize_selection((12, 5), (18, 5), 10, 3, 40, 20);
        assert_eq!(sy, 2);
        assert_eq!(sx, 2);
        assert_eq!(ey, 2);
        assert_eq!(ex, 8);
    }
}

async fn pty_reader_task(
    tile_id: TileId,
    mut reader: crate::pty::PtyReader,
    tx: mpsc::Sender<AppEvent>,
) {
    loop {
        let read_result = tokio::task::spawn_blocking(move || {
            let mut buf = vec![0u8; 4096];
            let n = reader.0.read(&mut buf);
            (reader, buf, n)
        })
        .await;

        match read_result {
            Ok((r, _buf, Ok(0))) => {
                // EOF
                tracing::debug!("PTY reader EOF for tile {}", tile_id.0);
                let _ = r; // drop reader
                let _ = tx.send(AppEvent::PtyExited(tile_id)).await;
                break;
            }
            Ok((r, buf, Ok(n))) => {
                reader = r;
                let data = buf[..n].to_vec();
                if tx.send(AppEvent::PtyOutput(tile_id, data)).await.is_err() {
                    break;
                }
            }
            _ => {
                tracing::debug!("PTY reader error for tile {}", tile_id.0);
                break;
            }
        }
    }
}

async fn tmux_reader_task(
    tile_id: TileId,
    reader: crate::tmux::TmuxReader,
    tx: mpsc::Sender<AppEvent>,
) {
    use tokio::io::AsyncReadExt;
    let file = match tokio::fs::File::open(&reader.pipe_path).await {
        Ok(f) => f,
        Err(e) => {
            tracing::error!("FIFO 打开失败 {}: {}", reader.pipe_path.display(), e);
            let _ = tx.send(AppEvent::PtyExited(tile_id)).await;
            return;
        }
    };
    let mut buf_reader = tokio::io::BufReader::new(file);
    let mut buf = vec![0u8; 4096];
    loop {
        match buf_reader.read(&mut buf).await {
            Ok(0) => {
                let _ = tx.send(AppEvent::PtyExited(tile_id)).await;
                break;
            }
            Ok(n) => {
                let _ = tx
                    .send(AppEvent::PtyOutput(tile_id, buf[..n].to_vec()))
                    .await;
            }
            Err(e) => {
                tracing::error!("FIFO 读取错误 {:?}: {}", tile_id, e);
                let _ = tx.send(AppEvent::PtyExited(tile_id)).await;
                break;
            }
        }
    }
}

async fn crossterm_event_reader(tx: mpsc::Sender<AppEvent>) {
    let mut stream = EventStream::new();
    while let Some(Ok(event)) = stream.next().await {
        if tx.send(AppEvent::Crossterm(event)).await.is_err() {
            break;
        }
    }
}
