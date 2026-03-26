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

use crossterm::event::{Event as CEvent, EventStream, KeyEventKind};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::{event::DisableMouseCapture, event::EnableMouseCapture, execute};
use futures::StreamExt;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::time::Duration;
use tokio::sync::mpsc;

#[derive(Debug, Clone, PartialEq)]
pub enum AppMode {
    Normal,
    Insert,
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
    /// For double-click detection: (time, column, row).
    last_click: Option<(std::time::Instant, u16, u16)>,
    /// Whether mouse capture is enabled (can be toggled for text selection).
    mouse_captured: bool,
}

impl App {
    pub fn new(config: Config) -> Self {
        let (event_tx, event_rx) = mpsc::channel(256);
        let columns = config.layout.default_columns.clamp(1, 3);
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
            last_click: None,
            mouse_captured: false,
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
            let has_selection = self.tile_manager.selected_id().is_some();
            let columns = self.columns;
            let scroll_offset = self.scroll_offset;
            let detail_width = self.config.layout.detail_panel_width;
            let filtered_count = self.tile_manager.filtered_tiles(&self.active_tab).len();
            let mouse_captured = self.mouse_captured;

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
                has_selection,
                detail_width,
                scroll_offset,
            );

            // Draw and capture real layout + actual terminal area size
            let mut captured_layout = layout_result;
            let mut actual_terminal_size: Option<(u16, u16)> = None;
            terminal.draw(|frame| {
                let total = frame.area();
                captured_layout = layout::calculate_layout(
                    total,
                    columns,
                    filtered_count,
                    has_selection,
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
                    mouse_captured,
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
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )?;
        terminal.show_cursor()?;
        tracing::info!("App stopped");

        Ok(())
    }

    fn handle_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::Crossterm(CEvent::Key(key)) => {
                // Only handle Press events (ignore Repeat/Release on some platforms)
                if key.kind != KeyEventKind::Press {
                    return;
                }

                // Handle keys that need App-level access in Normal mode
                if self.mode == AppMode::Normal {
                    use crossterm::event::KeyCode;
                    match key.code {
                        KeyCode::Char('1') => {
                            self.columns = 1;
                            return;
                        }
                        KeyCode::Char('2') => {
                            self.columns = 2;
                            return;
                        }
                        KeyCode::Char('3') => {
                            self.columns = 3;
                            return;
                        }
                        KeyCode::Char('n') => {
                            // Create tile in current directory (skip project selector for MVP)
                            let cwd = std::env::current_dir().unwrap_or_default();
                            if let Ok(id) = self.spawn_tile(&cwd) {
                                self.tile_manager.select(id);
                            }
                            return;
                        }
                        KeyCode::Char('m') => {
                            // Toggle mouse capture for text selection
                            self.mouse_captured = !self.mouse_captured;
                            let mut stdout = std::io::stdout();
                            if self.mouse_captured {
                                let _ = execute!(stdout, EnableMouseCapture);
                                tracing::info!("Mouse capture enabled");
                            } else {
                                let _ = execute!(stdout, DisableMouseCapture);
                                tracing::info!("Mouse capture disabled — use terminal native selection");
                            }
                            return;
                        }
                        _ => {}
                    }
                }

                let tab_entries = self.compute_tab_entries();
                let result = input::handle_key(
                    key,
                    &mut self.mode,
                    &mut self.tile_manager,
                    &mut self.active_tab,
                    &tab_entries,
                    self.columns,
                );
                if matches!(result, input::InputResult::Quit) {
                    self.should_quit = true;
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
                } else {
                    tracing::debug!("PtyOutput for unknown tile {:?}", tile_id);
                }
            }
            AppEvent::PtyExited(tile_id) => {
                tracing::info!("PTY exited for tile {}", tile_id.0);
                self.tile_manager.remove(tile_id);
                if self.mode == AppMode::Insert && self.tile_manager.selected_id().is_none() {
                    self.mode = AppMode::Normal;
                }
            }
            AppEvent::CwdChanged(tile_id, new_cwd) => {
                if let Some(tile) = self.tile_manager.get_mut(tile_id) {
                    tile.update_cwd(new_cwd);
                }
            }
            AppEvent::Tick => {
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
                if self.mode == AppMode::Insert && self.tile_manager.selected_id().is_none() {
                    self.mode = AppMode::Normal;
                }
            }
        }
    }

    fn handle_mouse(&mut self, mouse: crossterm::event::MouseEvent) {
        use crossterm::event::MouseEventKind;

        let layout = match &self.last_layout {
            Some(l) => l,
            None => return,
        };

        let x = mouse.column;
        let y = mouse.row;

        match mouse.kind {
            MouseEventKind::Down(crossterm::event::MouseButton::Left) => {
                let now = std::time::Instant::now();

                // Detect double-click: same position within 400ms
                let is_double_click = self
                    .last_click
                    .map(|(t, lx, ly)| {
                        now.duration_since(t).as_millis() < 400 && lx == x && ly == y
                    })
                    .unwrap_or(false);

                self.last_click = Some((now, x, y));

                // Click on tab bar?
                if y >= layout.tab_bar.y && y < layout.tab_bar.y + layout.tab_bar.height {
                    let tab_entries = self.compute_tab_entries();
                    self.active_tab = tab::next_tab(&self.active_tab, &tab_entries);
                    return;
                }

                // Click on a tile card?
                for (i, rect) in layout.tile_rects.iter().enumerate() {
                    if x >= rect.x
                        && x < rect.x + rect.width
                        && y >= rect.y
                        && y < rect.y + rect.height
                    {
                        if let Some(&tile_id) = self.last_filtered_ids.get(i) {
                            self.tile_manager.select(tile_id);
                            if is_double_click {
                                self.mode = AppMode::Insert; // double-click → Insert
                            } else {
                                self.mode = AppMode::Normal;
                            }
                        }
                        return;
                    }
                }

                // Click on detail panel while in Normal → enter Insert
                if let Some(d) = layout.detail_panel {
                    if x >= d.x && x < d.x + d.width && y >= d.y && y < d.y + d.height {
                        if is_double_click && self.tile_manager.selected_id().is_some() {
                            self.mode = AppMode::Insert;
                        }
                        return;
                    }
                }

                // Click elsewhere → deselect
                self.tile_manager.deselect();
            }
            MouseEventKind::ScrollUp => {
                if self.scroll_offset > 0 {
                    self.scroll_offset -= 1;
                }
            }
            MouseEventKind::ScrollDown => {
                self.scroll_offset += 1;
            }
            _ => {}
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
