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
        }
    }

    pub fn spawn_tile(&mut self, cwd: &Path) -> anyhow::Result<TileId> {
        let id = self.tile_manager.next_tile_id();
        // Use actual terminal dimensions for PTY size
        let (term_cols, term_rows) = crossterm::terminal::size().unwrap_or((80, 24));
        // PTY size based on detail panel: ~45% width, minus tab bar and status bar
        let pty_cols = ((term_cols as u32 * self.config.layout.detail_panel_width as u32) / 100) as u16;
        let pty_rows = term_rows.saturating_sub(4); // subtract tab bar (2) + status bar (1) + header (3) margin
        let pty_cols = pty_cols.max(40);
        let pty_rows = pty_rows.max(10);
        let (tile, reader) = crate::tile::Tile::spawn(
            id,
            &self.config.terminal.shell,
            cwd,
            pty_cols,
            pty_rows,
        )?;
        self.tile_manager.add(tile);

        // Spawn async reader task
        let tx = self.event_tx.clone();
        tokio::spawn(pty_reader_task(id, reader, tx));

        Ok(id)
    }

    pub async fn run(&mut self) -> anyhow::Result<()> {
        // Set up terminal
        enable_raw_mode()?;
        let mut stdout = std::io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(std::io::stdout());
        let mut terminal = Terminal::new(backend)?;

        // Spawn crossterm event reader
        let tx = self.event_tx.clone();
        tokio::spawn(crossterm_event_reader(tx.clone()));

        // Spawn tick timer
        let tx_tick = self.event_tx.clone();
        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(Duration::from_millis(500));
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

            let filtered_ids: Vec<TileId> = self.tile_manager
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

            // Draw and capture real layout
            let mut captured_layout = layout_result;
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
                ui::render(
                    frame,
                    &captured_layout,
                    &self.tile_manager,
                    &tab_entries,
                    &self.active_tab,
                    &self.mode,
                    columns,
                );
            })?;

            self.last_layout = Some(captured_layout);
            self.last_filtered_ids = filtered_ids;

            // Wait for at least one event, then drain all pending events before re-rendering.
            // This reduces render lag when multiple events arrive quickly (e.g. PTY output bursts).
            match self.event_rx.recv().await {
                Some(event) => self.handle_event(event),
                None => break,
            }
            // Drain remaining queued events
            while let Ok(event) = self.event_rx.try_recv() {
                self.handle_event(event);
                if self.should_quit { break; }
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
                        KeyCode::Char('1') => { self.columns = 1; return; }
                        KeyCode::Char('2') => { self.columns = 2; return; }
                        KeyCode::Char('3') => { self.columns = 3; return; }
                        KeyCode::Char('n') => {
                            // Create tile in current directory (skip project selector for MVP)
                            let cwd = std::env::current_dir().unwrap_or_default();
                            if let Ok(id) = self.spawn_tile(&cwd) {
                                self.tile_manager.select(id);
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
            AppEvent::Crossterm(CEvent::Resize(cols, rows)) => {
                // Resize selected tile's PTY to match new detail panel size
                let pty_cols = ((cols as u32 * self.config.layout.detail_panel_width as u32) / 100) as u16;
                let pty_rows = rows.saturating_sub(6);
                let pty_cols = pty_cols.max(40);
                let pty_rows = pty_rows.max(10);
                if let Some(tile) = self.tile_manager.selected_mut() {
                    let _ = tile.resize(pty_cols, pty_rows);
                }
            }
            AppEvent::Crossterm(_) => {}
            AppEvent::PtyOutput(tile_id, data) => {
                if let Some(tile) = self.tile_manager.get_mut(tile_id) {
                    tile.process_output(&data);
                }
            }
            AppEvent::CwdChanged(tile_id, new_cwd) => {
                if let Some(tile) = self.tile_manager.get_mut(tile_id) {
                    tile.update_cwd(new_cwd);
                }
            }
            AppEvent::Tick => {
                self.poll_tile_states();
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
                let is_double_click = self.last_click
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
                    if x >= rect.x && x < rect.x + rect.width
                        && y >= rect.y && y < rect.y + rect.height
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
            let tile_ids: Vec<TileId> = self
                .tile_manager
                .tiles()
                .iter()
                .map(|t| t.id)
                .collect();

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
        let tile_ids: Vec<TileId> = self
            .tile_manager
            .tiles()
            .iter()
            .map(|t| t.id)
            .collect();

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

    pub fn columns(&self) -> u8 {
        self.columns
    }

    pub fn set_columns(&mut self, columns: u8) {
        self.columns = columns.clamp(1, 3);
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
                let _ = r; // drop reader
                break;
            }
            Ok((r, buf, Ok(n))) => {
                reader = r;
                let data = buf[..n].to_vec();
                if tx.send(AppEvent::PtyOutput(tile_id, data)).await.is_err() {
                    break;
                }
            }
            _ => break,
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
