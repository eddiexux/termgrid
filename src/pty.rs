use std::io::Write;
use std::path::Path;

use anyhow::Result;
use portable_pty::{CommandBuilder, PtySize};

pub struct PtyReader(pub Box<dyn std::io::Read + Send>);

pub struct PtyHandle {
    master: Box<dyn portable_pty::MasterPty + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    writer: Box<dyn std::io::Write + Send>,
}

impl PtyHandle {
    pub fn spawn(shell: &str, cwd: &Path, cols: u16, rows: u16) -> Result<(Self, PtyReader)> {
        tracing::debug!(
            "PTY spawned: shell={}, cwd={:?}, size={}x{}",
            shell,
            cwd,
            cols,
            rows
        );
        let pty_system = portable_pty::native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| {
                tracing::error!("PTY spawn failed: {}", e);
                e
            })?;

        let mut cmd = CommandBuilder::new(shell);
        cmd.cwd(cwd);

        let child = pair.slave.spawn_command(cmd).map_err(|e| {
            tracing::error!("PTY spawn failed: {}", e);
            e
        })?;
        drop(pair.slave);

        let reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;

        let handle = PtyHandle {
            master: pair.master,
            child,
            writer,
        };

        Ok((handle, PtyReader(reader)))
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        Ok(())
    }

    pub fn write(&mut self, data: &[u8]) -> Result<()> {
        self.writer.write_all(data)?;
        self.writer.flush()?;
        Ok(())
    }

    pub fn pid(&self) -> Option<u32> {
        self.child.process_id()
    }

    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    #[cfg(unix)]
    pub fn master_fd(&self) -> Option<i32> {
        self.master.as_raw_fd()
    }

    pub fn wait(&mut self) -> Result<bool> {
        let status = self.child.wait()?;
        Ok(status.success())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::path::PathBuf;
    use std::time::Duration;

    fn tmp_dir() -> PathBuf {
        std::env::temp_dir()
    }

    #[test]
    fn test_spawn_and_read_output() {
        let (mut handle, PtyReader(mut reader)) =
            PtyHandle::spawn("/bin/sh", &tmp_dir(), 80, 24).expect("spawn failed");

        // Send command and exit
        handle
            .write(b"echo hello_termgrid\n")
            .expect("write failed");
        handle.write(b"exit\n").expect("write failed");

        // Read until EOF
        let mut output = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => output.extend_from_slice(&buf[..n]),
                Err(_) => break,
            }
        }

        let text = String::from_utf8_lossy(&output);
        assert!(
            text.contains("hello_termgrid"),
            "expected 'hello_termgrid' in output, got: {:?}",
            text
        );

        let _ = handle.wait();
    }

    #[test]
    fn test_resize() {
        let (handle, _reader) =
            PtyHandle::spawn("/bin/sh", &tmp_dir(), 80, 24).expect("spawn failed");

        handle.resize(120, 40).expect("resize failed");
    }

    #[test]
    fn test_wait_for_exit() {
        let (mut handle, PtyReader(mut reader)) =
            PtyHandle::spawn("/bin/sh", &tmp_dir(), 80, 24).expect("spawn failed");

        handle.write(b"exit 0\n").expect("write failed");

        // Read until EOF to drain output and allow the shell to exit cleanly
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
        }

        // Poll is_alive with timeout
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            if !handle.is_alive() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }

        assert!(!handle.is_alive(), "expected process to have exited");
    }
}
