use crate::git::{detect_git, GitContext};
use crate::pty::{PtyBackend, PtyHandle, PtyReader};
use crate::screen::{Cell, VteState};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TileId(pub u64);

#[derive(Debug, Clone, PartialEq)]
pub enum TileStatus {
    Running,
    Waiting,
    Idle(std::time::Duration),
    Exited,
    Error(String),
}

pub struct Tile {
    pub id: TileId,
    pub vte: VteState,
    pub pty: Box<dyn PtyBackend>,
    pub git_context: Option<GitContext>,
    pub cwd: PathBuf,
    pub status: TileStatus,
    pub last_active: Instant,
    pub waiting_since: Option<Instant>,
    /// True when a task has completed and is waiting for user attention.
    pub has_unread: bool,
    /// Accumulated output bytes since last quiet detection or selection.
    /// Used to detect "burst of output followed by silence" pattern.
    pub burst_bytes: usize,
    /// Name of the current foreground process (e.g. "claude", "node", "cargo").
    pub fg_process_name: Option<String>,
    /// tmux session name (e.g. "tg0"). None for native PTY backend.
    pub session_name: Option<String>,
    /// Ring buffer of raw PTY output for full history persistence.
    output_history: VecDeque<u8>,
    /// Maximum bytes to keep in output history (10 MB per tile).
    max_history_bytes: usize,
    /// Cached scrollback lines from output_history replay.
    scrollback_cache: Option<ScrollbackCache>,
}

/// Cached result of replaying output_history through a temporary parser.
struct ScrollbackCache {
    /// All lines from the replayed output.
    lines: Vec<Vec<Cell>>,
    /// output_history length when the cache was built.
    history_len: usize,
}

impl Tile {
    /// Create a new tile: spawn PTY, initialize VteState, detect git context.
    pub fn spawn(
        id: TileId,
        shell: &str,
        cwd: &Path,
        cols: u16,
        rows: u16,
    ) -> anyhow::Result<(Self, PtyReader)> {
        let (pty, reader) = PtyHandle::spawn(shell, cwd, cols, rows)?;
        tracing::info!("Tile {} spawned, PID {:?}", id.0, pty.pid());
        let vte = VteState::new(cols, rows);
        let git_context = detect_git(cwd);

        let tile = Tile {
            id,
            vte,
            pty: Box::new(pty),
            git_context,
            cwd: cwd.to_path_buf(),
            status: TileStatus::Running,
            last_active: Instant::now(),
            waiting_since: None,
            has_unread: false,
            burst_bytes: 0,
            fg_process_name: None,
            session_name: None,
            output_history: VecDeque::new(),
            max_history_bytes: 10 * 1024 * 1024,
            scrollback_cache: None,
        };

        Ok((tile, reader))
    }

    /// Create a tile backed by a tmux session.
    pub fn spawn_tmux(
        id: TileId,
        cwd: &Path,
        cols: u16,
        rows: u16,
    ) -> anyhow::Result<(Self, crate::tmux::TmuxReader, String)> {
        let session_id = crate::tmux::next_session_id();
        let session_name = format!("{}{}", crate::tmux::SESSION_PREFIX, session_id);
        let (backend, reader) =
            crate::tmux::TmuxPtyBackend::spawn(&session_name, cwd, cols, rows)?;
        tracing::info!(
            "Tile {} spawned tmux session '{}'",
            id.0,
            session_name
        );
        let vte = VteState::new(cols, rows);
        let git_context = detect_git(cwd);

        let tile = Tile {
            id,
            vte,
            pty: Box::new(backend),
            git_context,
            cwd: cwd.to_path_buf(),
            status: TileStatus::Running,
            last_active: Instant::now(),
            waiting_since: None,
            has_unread: false,
            burst_bytes: 0,
            fg_process_name: None,
            session_name: Some(session_name.clone()),
            output_history: VecDeque::new(),
            max_history_bytes: 10 * 1024 * 1024,
            scrollback_cache: None,
        };

        Ok((tile, reader, session_name))
    }

    /// Reconnect to an existing tmux session.
    pub fn reconnect_tmux(
        id: TileId,
        session_name: &str,
        cwd: &Path,
        cols: u16,
        rows: u16,
    ) -> anyhow::Result<(Self, crate::tmux::TmuxReader)> {
        let (backend, reader) = crate::tmux::TmuxPtyBackend::reconnect(session_name)?;
        tracing::info!(
            "Tile {} reconnected to tmux session '{}'",
            id.0,
            session_name
        );
        let vte = VteState::new(cols, rows);
        let git_context = detect_git(cwd);

        let tile = Tile {
            id,
            vte,
            pty: Box::new(backend),
            git_context,
            cwd: cwd.to_path_buf(),
            status: TileStatus::Running,
            last_active: Instant::now(),
            waiting_since: None,
            has_unread: false,
            burst_bytes: 0,
            fg_process_name: None,
            session_name: Some(session_name.to_string()),
            output_history: VecDeque::new(),
            max_history_bytes: 10 * 1024 * 1024,
            scrollback_cache: None,
        };

        Ok((tile, reader))
    }

    /// Feed bytes into the VTE parser and update last_active.
    ///
    /// When the program exits alternate screen (e.g. Claude Code after Ctrl+C),
    /// the alternate screen content is captured and injected into output_history
    /// so it remains accessible via scrollback. Without this, alternate screen
    /// content would be lost on exit (standard terminal behavior).
    pub fn process_output(&mut self, bytes: &[u8]) {
        tracing::trace!("Tile {} received {} bytes", self.id.0, bytes.len());

        // Capture alternate screen content BEFORE processing (it will be lost after).
        let alt_capture = if self.vte.alternate_screen() {
            Some(self.vte.capture_screen())
        } else {
            None
        };

        self.vte.process(bytes);
        self.last_active = Instant::now();

        // If we were on alt screen and now we're not, inject captured content.
        if let Some(captured) = alt_capture {
            if !self.vte.alternate_screen() && !captured.is_empty() {
                tracing::debug!(
                    "Tile {}: capturing {} bytes from alternate screen exit",
                    self.id.0,
                    captured.len()
                );

                // Inject captured alt screen content into output_history
                // BEFORE the current bytes (which contain the exit sequence).
                // Format: separator + captured ANSI content + separator + newline
                let mut injected = Vec::new();
                injected.extend_from_slice(b"\r\n");
                injected.extend_from_slice(&captured);
                injected.extend_from_slice(b"\r\n");

                // Append injected content, then the current bytes
                for &b in &injected {
                    if self.output_history.len() >= self.max_history_bytes {
                        self.output_history.pop_front();
                    }
                    self.output_history.push_back(b);
                }
                for &b in bytes {
                    if self.output_history.len() >= self.max_history_bytes {
                        self.output_history.pop_front();
                    }
                    self.output_history.push_back(b);
                }

                // Invalidate scrollback cache
                self.scrollback_cache = None;
                return; // already stored bytes, skip the normal path
            }
        }

        // Store raw output in ring buffer for full history persistence.
        for &b in bytes {
            if self.output_history.len() >= self.max_history_bytes {
                self.output_history.pop_front();
            }
            self.output_history.push_back(b);
        }
    }

    /// Return all buffered raw PTY output as a contiguous Vec.
    ///
    /// On session restore, replaying these bytes through the VTE emulator
    /// reconstructs the full scrollback (up to 10 MB per tile).
    pub fn output_history(&self) -> Vec<u8> {
        self.output_history.iter().copied().collect()
    }

    /// Get scrollback lines by replaying output_history.
    /// Returns cached result if output hasn't changed since last call.
    /// The returned lines cover the full terminal history, newest at the end.
    pub fn scrollback_lines(&mut self) -> &[Vec<Cell>] {
        let current_len = self.output_history.len();
        let cache_valid = self
            .scrollback_cache
            .as_ref()
            .map_or(false, |c| c.history_len == current_len);

        if !cache_valid {
            let history: Vec<u8> = self.output_history.iter().copied().collect();
            let cols = self.vte.cols();
            let lines = VteState::replay_history(&history, cols);
            self.scrollback_cache = Some(ScrollbackCache {
                lines,
                history_len: current_len,
            });
        }

        &self.scrollback_cache.as_ref().unwrap().lines
    }

    /// Total number of scrollback lines available.
    pub fn scrollback_line_count(&mut self) -> usize {
        self.scrollback_lines().len()
    }

    /// Update cwd; if it changed, re-detect git context.
    pub fn update_cwd(&mut self, new_cwd: PathBuf) {
        if new_cwd != self.cwd {
            tracing::debug!("Tile {} CWD changed to {:?}", self.id.0, new_cwd);
            self.cwd = new_cwd;
            self.git_context = detect_git(&self.cwd);
        }
    }

    /// Update status based on whether the foreground process is the shell.
    ///
    /// - If pty is not alive → Exited
    /// - If fg_shell: track waiting_since; if elapsed >= 60s → Idle(elapsed), else → Waiting
    /// - If not fg_shell → Running, reset waiting_since
    pub fn update_status(&mut self, is_fg_shell: bool) {
        if !self.pty.is_alive() {
            self.status = TileStatus::Exited;
            self.waiting_since = None;
            return;
        }

        if is_fg_shell {
            let since = self.waiting_since.get_or_insert_with(Instant::now);
            let elapsed = since.elapsed();
            if elapsed >= std::time::Duration::from_secs(60) {
                self.status = TileStatus::Idle(elapsed);
            } else {
                self.status = TileStatus::Waiting;
            }
        } else {
            self.waiting_since = None;
            self.status = TileStatus::Running;
        }
    }

    /// Write data to the PTY.
    pub fn write_input(&mut self, data: &[u8]) -> anyhow::Result<()> {
        self.pty.write_input(data)
    }

    /// Resize the PTY and the screen buffer.
    pub fn resize(&mut self, cols: u16, rows: u16) -> anyhow::Result<()> {
        self.pty.resize(cols, rows)?;
        self.vte.resize(cols, rows);
        Ok(())
    }

    /// Whether the foreground process is Claude Code.
    pub fn is_claude_code(&self) -> bool {
        self.fg_process_name
            .as_ref()
            .map_or(false, |name| name == "claude")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screen::VteState;

    #[test]
    fn test_tile_id_equality() {
        let a = TileId(1);
        let b = TileId(1);
        let c = TileId(2);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn test_process_output() {
        let mut vte = VteState::new(80, 24);
        vte.process(b"hello");
        // cursor moved 5 columns right
        let (_, col) = vte.cursor_position();
        assert_eq!(col, 5);
    }

    #[test]
    fn test_tile_spawn_and_process_output() {
        let id = TileId(1);
        let dir = std::env::current_dir().unwrap();
        let (mut tile, _reader) = Tile::spawn(id, "/bin/sh", &dir, 80, 24).unwrap();

        let before = tile.last_active;
        // Small sleep to ensure Instant advances
        std::thread::sleep(std::time::Duration::from_millis(10));
        tile.process_output(b"hello");
        assert!(tile.last_active >= before);
        let (_, col) = tile.vte.cursor_position();
        assert_eq!(col, 5);
    }

    #[test]
    fn test_update_status_running() {
        let id = TileId(2);
        let dir = std::env::current_dir().unwrap();
        let (mut tile, _reader) = Tile::spawn(id, "/bin/sh", &dir, 80, 24).unwrap();

        tile.update_status(false);
        assert_eq!(tile.status, TileStatus::Running);
        assert!(tile.waiting_since.is_none());
    }

    #[test]
    fn test_update_status_waiting() {
        let id = TileId(3);
        let dir = std::env::current_dir().unwrap();
        let (mut tile, _reader) = Tile::spawn(id, "/bin/sh", &dir, 80, 24).unwrap();

        tile.update_status(true);
        assert_eq!(tile.status, TileStatus::Waiting);
        assert!(tile.waiting_since.is_some());
    }

    #[test]
    fn test_update_status_resets_waiting_when_running() {
        let id = TileId(4);
        let dir = std::env::current_dir().unwrap();
        let (mut tile, _reader) = Tile::spawn(id, "/bin/sh", &dir, 80, 24).unwrap();

        // First mark as waiting
        tile.update_status(true);
        assert!(tile.waiting_since.is_some());

        // Then switch to running (fg process is not shell)
        tile.update_status(false);
        assert_eq!(tile.status, TileStatus::Running);
        assert!(tile.waiting_since.is_none());
    }

    #[test]
    fn test_resize() {
        let id = TileId(5);
        let dir = std::env::current_dir().unwrap();
        let (mut tile, _reader) = Tile::spawn(id, "/bin/sh", &dir, 80, 24).unwrap();

        tile.resize(120, 40).unwrap();
        assert_eq!(tile.vte.cols(), 120);
        assert_eq!(tile.vte.rows(), 40);
    }

    #[test]
    fn test_output_history_accumulates() {
        let id = TileId(6);
        let dir = std::env::current_dir().unwrap();
        let (mut tile, _reader) = Tile::spawn(id, "/bin/sh", &dir, 80, 24).unwrap();

        assert!(tile.output_history().is_empty());

        tile.process_output(b"hello");
        tile.process_output(b" world");
        let history = tile.output_history();
        assert_eq!(history, b"hello world");
    }

    #[test]
    fn test_output_history_ring_buffer_eviction() {
        let id = TileId(7);
        let dir = std::env::current_dir().unwrap();
        let (mut tile, _reader) = Tile::spawn(id, "/bin/sh", &dir, 80, 24).unwrap();

        // Fill to capacity + 5 bytes
        let capacity = tile.max_history_bytes;
        let chunk = vec![0u8; capacity + 5];
        tile.process_output(&chunk);

        // History must not exceed max capacity
        assert_eq!(tile.output_history().len(), capacity);
    }

    #[test]
    fn test_update_status_idle_after_timeout() {
        let id = TileId(10);
        let dir = std::env::current_dir().unwrap();
        let (mut tile, _reader) = Tile::spawn(id, "/bin/sh", &dir, 80, 24).unwrap();

        // Simulate waiting for a long time by setting waiting_since in the past
        tile.waiting_since = Some(Instant::now() - std::time::Duration::from_secs(120));
        tile.update_status(true);

        match tile.status {
            TileStatus::Idle(elapsed) => {
                assert!(elapsed >= std::time::Duration::from_secs(60));
            }
            other => panic!("Expected Idle, got {:?}", other),
        }
    }

    #[test]
    fn test_update_cwd_same_path_no_redetect() {
        let id = TileId(11);
        let dir = std::env::current_dir().unwrap();
        let (mut tile, _reader) = Tile::spawn(id, "/bin/sh", &dir, 80, 24).unwrap();

        let git_before = tile.git_context.clone();
        // Update with the same path — should not change git_context
        tile.update_cwd(dir.clone());
        assert_eq!(tile.git_context, git_before);
        assert_eq!(tile.cwd, dir);
    }

    #[test]
    fn test_update_cwd_different_path() {
        let id = TileId(12);
        let dir = std::env::current_dir().unwrap();
        let (mut tile, _reader) = Tile::spawn(id, "/bin/sh", &dir, 80, 24).unwrap();

        let new_dir = std::path::PathBuf::from("/tmp");
        tile.update_cwd(new_dir.clone());
        assert_eq!(tile.cwd, new_dir);
        // /tmp is not a git repo
        assert!(tile.git_context.is_none());
    }

    #[test]
    fn test_output_history_preserves_order() {
        let id = TileId(13);
        let dir = std::env::current_dir().unwrap();
        let (mut tile, _reader) = Tile::spawn(id, "/bin/sh", &dir, 80, 24).unwrap();

        tile.process_output(b"AAA");
        tile.process_output(b"BBB");
        tile.process_output(b"CCC");
        assert_eq!(tile.output_history(), b"AAABBBCCC");
    }

    #[test]
    fn test_output_history_eviction_preserves_recent() {
        let id = TileId(14);
        let dir = std::env::current_dir().unwrap();
        let (mut tile, _reader) = Tile::spawn(id, "/bin/sh", &dir, 80, 24).unwrap();

        let capacity = tile.max_history_bytes;
        // Fill with 'A's to capacity
        tile.process_output(&vec![b'A'; capacity]);
        // Then add 10 'B's — should evict oldest 10 'A's
        tile.process_output(&vec![b'B'; 10]);

        let history = tile.output_history();
        assert_eq!(history.len(), capacity);
        // Last 10 bytes should be 'B'
        assert!(history[capacity - 10..].iter().all(|&b| b == b'B'));
        // First bytes should be 'A'
        assert_eq!(history[0], b'A');
    }

    #[test]
    fn test_max_history_bytes_is_10mb() {
        let id = TileId(15);
        let dir = std::env::current_dir().unwrap();
        let (tile, _reader) = Tile::spawn(id, "/bin/sh", &dir, 80, 24).unwrap();
        assert_eq!(tile.max_history_bytes, 10 * 1024 * 1024);
    }

    #[test]
    fn test_burst_bytes_not_incremented_by_process_output() {
        let id = TileId(20);
        let dir = std::env::current_dir().unwrap();
        let (mut tile, _reader) = Tile::spawn(id, "/bin/sh", &dir, 80, 24).unwrap();

        assert_eq!(tile.burst_bytes, 0);
        // process_output does NOT increment burst_bytes (app.rs does it conditionally)
        tile.process_output(b"hello world");
        assert_eq!(tile.burst_bytes, 0);
    }

    #[test]
    fn test_burst_bytes_manual_accumulation() {
        let id = TileId(21);
        let dir = std::env::current_dir().unwrap();
        let (mut tile, _reader) = Tile::spawn(id, "/bin/sh", &dir, 80, 24).unwrap();

        // Simulate what app.rs does for non-selected tiles
        tile.process_output(b"chunk1");
        tile.burst_bytes += 6;
        tile.process_output(b"chunk2");
        tile.burst_bytes += 6;

        assert_eq!(tile.burst_bytes, 12);
    }

    #[test]
    fn test_has_unread_initially_false() {
        let id = TileId(22);
        let dir = std::env::current_dir().unwrap();
        let (tile, _reader) = Tile::spawn(id, "/bin/sh", &dir, 80, 24).unwrap();
        assert!(!tile.has_unread);
        assert_eq!(tile.burst_bytes, 0);
    }

    #[test]
    fn test_alt_screen_content_captured_on_exit() {
        let id = TileId(30);
        let dir = std::env::current_dir().unwrap();
        let (mut tile, _reader) = Tile::spawn(id, "/bin/sh", &dir, 80, 24).unwrap();

        // Enter alternate screen and write content
        tile.process_output(b"\x1b[?1049h"); // enter alt screen
        assert!(tile.vte.alternate_screen());
        tile.process_output(b"important task output\r\nline two\r\n");

        let history_before = tile.output_history().len();

        // Exit alternate screen — content should be captured
        tile.process_output(b"\x1b[?1049l"); // exit alt screen
        assert!(!tile.vte.alternate_screen());

        let history_after = tile.output_history().len();
        // History should be larger than before (captured content injected)
        assert!(
            history_after > history_before,
            "history should grow after alt screen capture: {} > {}",
            history_after,
            history_before,
        );

        // The captured content should be in the history
        let history_bytes = tile.output_history();
        let history = String::from_utf8_lossy(&history_bytes);
        assert!(
            history.contains("important task output"),
            "captured content should be in history"
        );
    }

    #[test]
    fn test_alt_screen_no_capture_when_staying_in_alt() {
        let id = TileId(31);
        let dir = std::env::current_dir().unwrap();
        let (mut tile, _reader) = Tile::spawn(id, "/bin/sh", &dir, 80, 24).unwrap();

        // Enter alternate screen
        tile.process_output(b"\x1b[?1049h");
        tile.process_output(b"some content");
        let history_len = tile.output_history().len();

        // More output while still in alt screen — no capture
        tile.process_output(b"more content");
        // History grows by the new output only (no capture injection)
        let expected_growth = b"more content".len();
        assert_eq!(
            tile.output_history().len(),
            history_len + expected_growth,
        );
    }

    #[test]
    fn test_alt_screen_capture_preserves_scrollback_cache_invalidation() {
        let id = TileId(32);
        let dir = std::env::current_dir().unwrap();
        let (mut tile, _reader) = Tile::spawn(id, "/bin/sh", &dir, 80, 24).unwrap();

        // Build a cache first
        tile.process_output(b"initial output\r\n");
        let _ = tile.scrollback_lines();
        assert!(tile.scrollback_cache.is_some());

        // Enter and exit alt screen
        tile.process_output(b"\x1b[?1049h");
        tile.process_output(b"alt content");
        tile.process_output(b"\x1b[?1049l");

        // Cache should be invalidated
        assert!(tile.scrollback_cache.is_none());
    }

    #[test]
    fn test_is_claude_code_detection() {
        let id = TileId(40);
        let dir = std::env::current_dir().unwrap();
        let (mut tile, _reader) = Tile::spawn(id, "/bin/sh", &dir, 80, 24).unwrap();

        // Initially no fg process name
        assert!(!tile.is_claude_code());

        // Set to claude
        tile.fg_process_name = Some("claude".to_string());
        assert!(tile.is_claude_code());

        // Set to something else
        tile.fg_process_name = Some("cargo".to_string());
        assert!(!tile.is_claude_code());

        // None
        tile.fg_process_name = None;
        assert!(!tile.is_claude_code());
    }

    #[test]
    fn test_normal_output_no_spurious_capture() {
        let id = TileId(33);
        let dir = std::env::current_dir().unwrap();
        let (mut tile, _reader) = Tile::spawn(id, "/bin/sh", &dir, 80, 24).unwrap();

        // Normal output — no alt screen involved
        tile.process_output(b"hello world\r\n");
        tile.process_output(b"another line\r\n");

        let history = tile.output_history();
        assert_eq!(history, b"hello world\r\nanother line\r\n");
    }
}
