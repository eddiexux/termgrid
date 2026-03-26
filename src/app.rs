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

/// Text selection state for drag-to-copy.
pub struct TextSelection {
    /// Screen coordinates where drag started.
    pub start: (u16, u16),
    /// Screen coordinates where drag currently is.
    pub end: (u16, u16),
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

pub struct App {
    config: Config,
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
    /// How many rows the detail panel is scrolled back into history (0 = follow cursor).
    detail_scroll_offset: usize,
    /// Path to the config file, used for hot reload detection.
    config_path: std::path::PathBuf,
    /// Last known modification time of the config file.
    config_last_modified: Option<std::time::SystemTime>,
}

impl App {
    pub fn new(config: Config) -> Self {
        let (event_tx, event_rx) = mpsc::channel(256);
        let columns = config.layout.default_columns.clamp(1, 3);
        let config_path = Config::config_path();
        let config_last_modified = std::fs::metadata(&config_path)
            .and_then(|m| m.modified())
            .ok();
        App {
            config,
            tile_manager: TileManager::new(),
            mode: AppMode::Normal,
            active_tab: TabFilter::All,
            columns,
            scroll_offset: 0,
            event_tx,
            event_rx,
            should_quit: false,
            last_layout: None,
            last_filtered_ids: Vec::new(),
            selection: None,
            drag_origin: None,
            detail_scroll_offset: 0,
            config_path,
            config_last_modified,
        }
    }

    pub fn spawn_tile(&mut self, cwd: &Path) -> anyhow::Result<TileId> {
        let id = self.tile_manager.next_tile_id();
        tracing::info!("Spawning tile {} in {:?}", id.0, cwd);
        // Use actual terminal dimensions for PTY size
        let (term_cols, term_rows) = crossterm::terminal::size().unwrap_or((80, 24));
        let (pty_cols, pty_rows) =
            Self::estimate_pty_size(term_cols, term_rows, self.config.layout.detail_panel_width);
        let (tile, reader) =
            crate::tile::Tile::spawn(id, &self.config.terminal.shell, cwd, pty_cols, pty_rows)?;
        self.tile_manager.add(tile);

        // Spawn async reader task
        let tx = self.event_tx.clone();
        tokio::spawn(pty_reader_task(id, reader, tx));

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
        tokio::spawn(crossterm_event_reader(tx.clone()));

        // Spawn tick timer
        let tx_tick = self.event_tx.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(500));
            loop {
                interval.tick().await;
                if tx_tick.send(AppEvent::Tick).await.is_err() {
                    break;
                }
            }
        });

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

            // Set scrollback on selected tile BEFORE draw (needs &mut, can't do inside draw closure)
            if self.detail_scroll_offset > 0 {
                if let Some(tile) = self.tile_manager.selected_mut() {
                    tile.vte.set_scrollback(self.detail_scroll_offset);
                }
            }

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
                );
                actual_terminal_size = render_result.detail_terminal_size;
            })?;

            // Reset scrollback after draw
            if self.detail_scroll_offset > 0 {
                if let Some(tile) = self.tile_manager.selected_mut() {
                    tile.vte.set_scrollback(0);
                }
            }

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
                let selected_id = self.tile_manager.selected_id();
                if let Some(tile) = self.tile_manager.get_mut(tile_id) {
                    tile.process_output(&data);
                    // Mark as unread if not currently selected
                    if selected_id != Some(tile_id) {
                        tile.has_unread = true;
                    }
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
                // Record drag origin, clear any existing selection
                self.drag_origin = Some((x, y));
                self.selection = None;
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                // Dragging — update selection
                if let Some((sx, sy)) = self.drag_origin {
                    self.selection = Some(TextSelection {
                        start: (sx, sy),
                        end: (x, y),
                    });
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                if let Some(sel) = self.selection.take() {
                    // Was dragging — copy selected text to clipboard
                    self.copy_selection_to_clipboard(&sel);
                    self.drag_origin = None;
                } else if let Some((ox, oy)) = self.drag_origin.take() {
                    // Was a click (no drag) — handle tile selection
                    self.handle_click(ox, oy);
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
                            // Clamp to max scrollback (screen rows) to prevent ghost offset
                            let max_scroll = self
                                .tile_manager
                                .selected()
                                .map(|t| t.vte.rows() as usize)
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
                let cwd = std::env::current_dir().unwrap_or_default();
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

        // Status bar buttons (right side): " [?] [×] [Ncol] "
        // From right: [Ncol]=7, [×]=5, [?]=5
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
            // [×] close selected tile
            let close_btn_start = col_btn_start.saturating_sub(5);
            if x >= close_btn_start && x < col_btn_start {
                if let Some(id) = self.tile_manager.selected_id() {
                    let needs_confirm = self
                        .tile_manager
                        .get(id)
                        .map(|t| t.status == crate::tile::TileStatus::Running)
                        .unwrap_or(false);
                    if needs_confirm {
                        self.mode = AppMode::Overlay(OverlayKind::ConfirmClose(id));
                    } else {
                        self.tile_manager.remove(id);
                    }
                }
                return;
            }
            // [?] help
            let help_btn_start = close_btn_start.saturating_sub(5);
            if x >= help_btn_start && x < close_btn_start {
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

        // Click on detail panel — no special action needed (keyboard already goes to selected PTY)
        if let Some(d) = layout.detail_panel {
            if x >= d.x && x < d.x + d.width && y >= d.y && y < d.y + d.height {
                return;
            }
        }

        // Click elsewhere → keep current selection (layout stays stable)
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
            let is_fg_shell = {
                if let Some(tile) = self.tile_manager.get(id) {
                    let master_fd = tile.pty.master_fd();
                    let pty_pid = tile.pty.pid();
                    if let (Some(fd), Some(pid)) = (master_fd, pty_pid) {
                        if let Some(fg_pid) = crate::process::get_foreground_pid(fd) {
                            fg_pid == pid as i32
                        } else {
                            true
                        }
                    } else {
                        true
                    }
                } else {
                    true
                }
            };

            #[cfg(not(unix))]
            let is_fg_shell = true;

            if let Some(tile) = self.tile_manager.get_mut(id) {
                tile.update_status(is_fg_shell);
            }
        }
    }

    pub fn tile_manager_ref(&self) -> &TileManager {
        &self.tile_manager
    }

    /// Restore scrollback content into a tile's VTE (for session restore).
    pub fn restore_tile_scrollback(&mut self, tile_id: TileId, data: &[u8]) {
        if let Some(tile) = self.tile_manager.get_mut(tile_id) {
            tile.vte.process(data);
            // Ensure cursor is visible and at a sane position after restore.
            // The restored content may have left cursor hidden or at an old position.
            tile.vte.process(b"\x1b[?25h"); // show cursor
            tracing::debug!(
                "Restored {} bytes scrollback for tile {}",
                data.len(),
                tile_id.0
            );
        }
    }

    pub fn columns(&self) -> u8 {
        self.columns
    }

    pub fn set_columns(&mut self, columns: u8) {
        self.columns = columns.clamp(1, 3);
    }

    /// Estimate PTY dimensions before first render (when detail panel rect is unknown).
    fn estimate_pty_size(term_cols: u16, term_rows: u16, detail_width_pct: u16) -> (u16, u16) {
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

async fn crossterm_event_reader(tx: mpsc::Sender<AppEvent>) {
    let mut stream = EventStream::new();
    while let Some(Ok(event)) = stream.next().await {
        if tx.send(AppEvent::Crossterm(event)).await.is_err() {
            break;
        }
    }
}
