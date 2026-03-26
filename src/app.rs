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
}

impl App {
    pub fn new(config: Config) -> Self {
        let (event_tx, event_rx) = mpsc::channel(256);
        let columns = config.layout.default_columns.max(1).min(3);
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
        }
    }

    pub fn spawn_tile(&mut self, cwd: &Path) -> anyhow::Result<TileId> {
        let id = self.tile_manager.next_tile_id();
        let (tile, reader) = crate::tile::Tile::spawn(
            id,
            &self.config.terminal.shell,
            cwd,
            80,
            24,
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

            terminal.draw(|frame| {
                let total = frame.area();
                let layout_result = layout::calculate_layout(
                    total,
                    columns,
                    filtered_count,
                    has_selection,
                    detail_width,
                    scroll_offset,
                );
                ui::render(
                    frame,
                    &layout_result,
                    &self.tile_manager,
                    &tab_entries,
                    &self.active_tab,
                    &self.mode,
                    columns,
                );
            })?;

            // Wait for next event
            match self.event_rx.recv().await {
                Some(event) => self.handle_event(event),
                None => break,
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

                // Handle column keys in Normal mode before generic dispatch
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
                input::handle_mouse(
                    mouse,
                    &mut self.mode,
                    &mut self.tile_manager,
                    &mut self.active_tab,
                );
            }
            AppEvent::Crossterm(CEvent::Resize(_cols, _rows)) => {
                // Terminal will re-render on next loop iteration
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
