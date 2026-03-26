use crate::git::{detect_git, GitContext};
use crate::pty::{PtyHandle, PtyReader};
use crate::screen::VteState;
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
    pub pty: PtyHandle,
    pub git_context: Option<GitContext>,
    pub cwd: PathBuf,
    pub status: TileStatus,
    pub last_active: Instant,
    pub waiting_since: Option<Instant>,
    /// True when new PTY output arrived while this tile was not selected.
    pub has_unread: bool,
    /// Ring buffer of raw PTY output for full history persistence.
    output_history: VecDeque<u8>,
    /// Maximum bytes to keep in output history (256 KB per tile).
    max_history_bytes: usize,
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
            pty,
            git_context,
            cwd: cwd.to_path_buf(),
            status: TileStatus::Running,
            last_active: Instant::now(),
            waiting_since: None,
            has_unread: false,
            output_history: VecDeque::new(),
            max_history_bytes: 10 * 1024 * 1024,
        };

        Ok((tile, reader))
    }

    /// Feed bytes into the VTE parser and update last_active.
    pub fn process_output(&mut self, bytes: &[u8]) {
        tracing::trace!("Tile {} received {} bytes", self.id.0, bytes.len());
        self.vte.process(bytes);
        self.last_active = Instant::now();
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
    /// reconstructs the full scrollback (up to 256 KB per tile).
    pub fn output_history(&self) -> Vec<u8> {
        self.output_history.iter().copied().collect()
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
        self.pty.write(data)
    }

    /// Resize the PTY and the screen buffer.
    pub fn resize(&mut self, cols: u16, rows: u16) -> anyhow::Result<()> {
        self.pty.resize(cols, rows)?;
        self.vte.resize(cols, rows);
        Ok(())
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
}
