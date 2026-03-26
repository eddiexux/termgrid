use crate::git::{detect_git, GitContext};
use crate::pty::{PtyHandle, PtyReader};
use crate::screen::VteState;
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
        let vte = VteState::new(cols as usize, rows as usize);
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
        };

        Ok((tile, reader))
    }

    /// Feed bytes into the VTE parser and update last_active.
    pub fn process_output(&mut self, bytes: &[u8]) {
        self.vte.process(bytes);
        self.last_active = Instant::now();
    }

    /// Update cwd; if it changed, re-detect git context.
    pub fn update_cwd(&mut self, new_cwd: PathBuf) {
        if new_cwd != self.cwd {
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
        self.vte.screen.resize(cols as usize, rows as usize);
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
        assert_eq!(vte.screen.cursor.col, 5);
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
        assert_eq!(tile.vte.screen.cursor.col, 5);
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
        assert_eq!(tile.vte.screen.cols(), 120);
        assert_eq!(tile.vte.screen.rows(), 40);
    }
}
