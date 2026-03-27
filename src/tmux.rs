use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use anyhow::{bail, Context, Result};

use crate::pty::PtyBackend;

pub const FIFO_DIR: &str = "/tmp/termgrid";
pub const SESSION_PREFIX: &str = "tg";

// ---------------------------------------------------------------------------
// Utility functions
// ---------------------------------------------------------------------------

/// 检测 tmux 是否可用
pub fn is_tmux_available() -> bool {
    Command::new("tmux")
        .arg("-V")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// 检测当前是否运行在 tmux 内部
pub fn is_inside_tmux() -> bool {
    std::env::var("TMUX").is_ok()
}

/// 列出所有带 `@termgrid=1` 标记的 tg* session，返回 (session_name, cwd)
pub fn list_termgrid_sessions() -> Vec<(String, PathBuf)> {
    let output = Command::new("tmux")
        .args(["list-sessions", "-F", "#{session_name}"])
        .output();
    let output = match output {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };
    let names = String::from_utf8_lossy(&output.stdout);
    let mut sessions = Vec::new();
    for name in names.lines() {
        let name = name.trim();
        if !name.starts_with(SESSION_PREFIX) {
            continue;
        }
        // 检查 @termgrid 标记
        let check = Command::new("tmux")
            .args(["show-options", "-t", name, "-v", "@termgrid"])
            .output();
        match check {
            Ok(o) if o.status.success() && String::from_utf8_lossy(&o.stdout).trim() == "1" => {}
            _ => continue,
        }
        // 获取 cwd
        let cwd_out = Command::new("tmux")
            .args(["display-message", "-t", name, "-p", "#{pane_current_path}"])
            .output();
        let cwd = match cwd_out {
            Ok(o) if o.status.success() => {
                PathBuf::from(String::from_utf8_lossy(&o.stdout).trim().to_string())
            }
            _ => PathBuf::from("."),
        };
        sessions.push((name.to_string(), cwd));
    }
    sessions
}

/// 捕获 tmux pane 的最近 1000 行内容
pub fn capture_pane(session_name: &str) -> Option<Vec<u8>> {
    let output = Command::new("tmux")
        .args(["capture-pane", "-t", session_name, "-p", "-S", "-1000"])
        .output()
        .ok()?;
    if output.status.success() {
        Some(output.stdout)
    } else {
        None
    }
}

/// 扫描已有 tg* session，返回最小可用 ID
pub fn next_session_id() -> u64 {
    let output = Command::new("tmux")
        .args(["list-sessions", "-F", "#{session_name}"])
        .output();
    let output = match output {
        Ok(o) if o.status.success() => o,
        _ => return 0,
    };
    let names = String::from_utf8_lossy(&output.stdout);
    let mut used: Vec<u64> = names
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            line.strip_prefix(SESSION_PREFIX)
                .and_then(|s| s.parse::<u64>().ok())
        })
        .collect();
    used.sort_unstable();
    // 找最小可用 ID
    let mut next = 0u64;
    for id in &used {
        if *id == next {
            next += 1;
        } else {
            break;
        }
    }
    next
}

/// Kill tmux session 并清理 FIFO
pub fn kill_session(session_name: &str) {
    let _ = Command::new("tmux")
        .args(["kill-session", "-t", session_name])
        .output();
    let pipe_path = fifo_path(session_name);
    let _ = std::fs::remove_file(&pipe_path);
}

/// 清理所有 FIFO 文件
pub fn cleanup_fifos() {
    let dir = Path::new(FIFO_DIR);
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map(|e| e == "pipe").unwrap_or(false) {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
}

fn fifo_path(session_name: &str) -> PathBuf {
    PathBuf::from(FIFO_DIR).join(format!("{}.pipe", session_name))
}

// ---------------------------------------------------------------------------
// Input parsing — escape 序列解析
// ---------------------------------------------------------------------------

/// 解析后的输入事件
#[derive(Debug, Clone, PartialEq)]
pub enum InputEvent {
    /// 可打印文本，用 `send-keys -l`
    Literal(String),
    /// tmux 键名，用 `send-keys` (无 -l)
    TmuxKey(String),
    /// 无法识别的原始字节，用 `send-keys -H`
    RawBytes(Vec<u8>),
}

/// 将原始字节流解析为 InputEvent 序列
pub fn parse_input_chunk(data: &[u8]) -> Vec<InputEvent> {
    let mut events = Vec::new();
    let mut literal_buf = String::new();
    let mut i = 0;

    while i < data.len() {
        let b = data[i];

        if b == 0x1b {
            // 先 flush 累积的文本
            flush_literal(&mut literal_buf, &mut events);

            // 尝试解析 escape 序列
            if let Some((key, consumed)) = parse_escape_sequence(&data[i..]) {
                events.push(InputEvent::TmuxKey(key));
                i += consumed;
            } else {
                // 单独的 ESC
                events.push(InputEvent::TmuxKey("Escape".to_string()));
                i += 1;
            }
        } else if b < 0x20 || b == 0x7f {
            // 控制字符
            flush_literal(&mut literal_buf, &mut events);
            if let Some(key) = control_char_to_tmux_key(b) {
                events.push(InputEvent::TmuxKey(key));
            } else {
                events.push(InputEvent::RawBytes(vec![b]));
            }
            i += 1;
        } else {
            // 可打印字符（包括 UTF-8 多字节）
            if b < 0x80 {
                literal_buf.push(b as char);
                i += 1;
            } else {
                // UTF-8 多字节
                let remaining = &data[i..];
                match std::str::from_utf8(remaining) {
                    Ok(s) => {
                        // 只取第一个字符
                        let ch = s.chars().next().unwrap();
                        literal_buf.push(ch);
                        i += ch.len_utf8();
                    }
                    Err(e) => {
                        // 部分有效
                        let valid_len = e.valid_up_to();
                        if valid_len > 0 {
                            let s = std::str::from_utf8(&remaining[..valid_len]).unwrap();
                            let ch = s.chars().next().unwrap();
                            literal_buf.push(ch);
                            i += ch.len_utf8();
                        } else {
                            flush_literal(&mut literal_buf, &mut events);
                            events.push(InputEvent::RawBytes(vec![b]));
                            i += 1;
                        }
                    }
                }
            }
        }
    }
    flush_literal(&mut literal_buf, &mut events);
    events
}

fn flush_literal(buf: &mut String, events: &mut Vec<InputEvent>) {
    if !buf.is_empty() {
        events.push(InputEvent::Literal(std::mem::take(buf)));
    }
}

fn control_char_to_tmux_key(b: u8) -> Option<String> {
    match b {
        0x09 => Some("Tab".to_string()),
        0x0a => Some("Enter".to_string()), // LF
        0x0d => Some("Enter".to_string()), // CR
        0x7f => Some("BSpace".to_string()),
        0x01..=0x1a => {
            let ch = (b'a' + b - 1) as char;
            Some(format!("C-{}", ch))
        }
        _ => None,
    }
}

/// 尝试解析 data[0..] 开头的 escape 序列。
/// 返回 (tmux_key_name, consumed_bytes)。data[0] 必须是 0x1b。
fn parse_escape_sequence(data: &[u8]) -> Option<(String, usize)> {
    if data.len() < 2 {
        return None;
    }

    match data[1] {
        b'[' => parse_csi_sequence(data),
        b'O' => parse_ss3_sequence(data),
        _ => None,
    }
}

/// 解析 CSI 序列: ESC [ ...
fn parse_csi_sequence(data: &[u8]) -> Option<(String, usize)> {
    // data[0] = ESC, data[1] = '['
    if data.len() < 3 {
        return None;
    }

    // 收集参数字节和中间字节 (0x30-0x3f, 0x20-0x2f)
    let mut i = 2;
    let mut params = Vec::new();
    let mut current_param = String::new();

    // 解析参数部分
    while i < data.len() {
        let b = data[i];
        if b >= b'0' && b <= b'9' {
            current_param.push(b as char);
            i += 1;
        } else if b == b';' {
            params.push(current_param.clone());
            current_param.clear();
            i += 1;
        } else if b >= 0x40 && b <= 0x7e {
            // 终止字节
            if !current_param.is_empty() {
                params.push(current_param);
            }
            let final_byte = b;
            let consumed = i + 1;

            let key = match final_byte {
                b'A' => modifier_key(&params, "Up"),
                b'B' => modifier_key(&params, "Down"),
                b'C' => modifier_key(&params, "Right"),
                b'D' => modifier_key(&params, "Left"),
                b'H' => modifier_key(&params, "Home"),
                b'F' => modifier_key(&params, "End"),
                b'~' => {
                    // ESC [ N ~ 形式
                    let num = params.first().and_then(|s| s.parse::<u32>().ok());
                    let modifier = params.get(1).and_then(|s| s.parse::<u32>().ok());
                    let base = match num {
                        Some(2) => "IC",     // Insert
                        Some(3) => "DC",     // Delete
                        Some(5) => "PPage",  // PageUp
                        Some(6) => "NPage",  // PageDown
                        Some(15) => "F5",
                        Some(17) => "F6",
                        Some(18) => "F7",
                        Some(19) => "F8",
                        Some(20) => "F9",
                        Some(21) => "F10",
                        Some(23) => "F11",
                        Some(24) => "F12",
                        _ => return None,
                    };
                    apply_modifier(modifier, base)
                }
                _ => return None,
            };

            return key.map(|k| (k, consumed));
        } else {
            // 未知字节
            return None;
        }
    }
    None
}

/// 处理带修饰符的方向键等: ESC [ 1 ; modifier key
fn modifier_key(params: &[String], base: &str) -> Option<String> {
    if params.is_empty() || (params.len() == 1 && params[0].is_empty()) {
        return Some(base.to_string());
    }
    let modifier = if params.len() >= 2 {
        params[1].parse::<u32>().ok()
    } else {
        None
    };
    apply_modifier(modifier, base)
}

fn apply_modifier(modifier: Option<u32>, base: &str) -> Option<String> {
    match modifier {
        None | Some(1) => Some(base.to_string()),
        Some(2) => Some(format!("S-{}", base)),   // Shift
        Some(3) => Some(format!("M-{}", base)),   // Alt
        Some(4) => Some(format!("M-S-{}", base)), // Alt+Shift
        Some(5) => Some(format!("C-{}", base)),   // Ctrl
        Some(6) => Some(format!("C-S-{}", base)), // Ctrl+Shift
        Some(7) => Some(format!("M-C-{}", base)), // Alt+Ctrl
        Some(8) => Some(format!("M-C-S-{}", base)),
        _ => Some(base.to_string()),
    }
}

/// 解析 SS3 序列: ESC O ...
fn parse_ss3_sequence(data: &[u8]) -> Option<(String, usize)> {
    if data.len() < 3 {
        return None;
    }
    let key = match data[2] {
        b'P' => "F1",
        b'Q' => "F2",
        b'R' => "F3",
        b'S' => "F4",
        b'A' => "Up",
        b'B' => "Down",
        b'C' => "Right",
        b'D' => "Left",
        b'H' => "Home",
        b'F' => "End",
        _ => return None,
    };
    Some((key.to_string(), 3))
}

// ---------------------------------------------------------------------------
// TmuxReader
// ---------------------------------------------------------------------------

/// FIFO 读取端
pub struct TmuxReader {
    pub pipe_path: PathBuf,
}

// ---------------------------------------------------------------------------
// TmuxPtyBackend
// ---------------------------------------------------------------------------

pub struct TmuxPtyBackend {
    session_name: String,
    input_buffer: Arc<Mutex<Vec<u8>>>,
    _flush_cancel: tokio::sync::oneshot::Sender<()>,
}

impl TmuxPtyBackend {
    /// 创建新 tmux session 和 FIFO
    pub fn spawn(
        session_name: &str,
        cwd: &Path,
        cols: u16,
        rows: u16,
    ) -> Result<(Self, TmuxReader)> {
        // 确保 FIFO 目录存在
        std::fs::create_dir_all(FIFO_DIR)
            .with_context(|| format!("创建 FIFO 目录失败: {}", FIFO_DIR))?;

        let pipe = fifo_path(session_name);

        // 删除旧 FIFO（如存在）
        let _ = std::fs::remove_file(&pipe);

        // 创建 FIFO
        let pipe_cstr = std::ffi::CString::new(pipe.to_str().unwrap())?;
        let ret = unsafe { libc::mkfifo(pipe_cstr.as_ptr(), 0o600) };
        if ret != 0 {
            bail!(
                "mkfifo 失败: {}",
                std::io::Error::last_os_error()
            );
        }

        // 创建 tmux session
        let status = Command::new("tmux")
            .args([
                "new-session",
                "-d",
                "-s",
                session_name,
                "-c",
                cwd.to_str().unwrap_or("."),
                "-x",
                &cols.to_string(),
                "-y",
                &rows.to_string(),
            ])
            .status()
            .context("执行 tmux new-session 失败")?;
        if !status.success() {
            let _ = std::fs::remove_file(&pipe);
            bail!("tmux new-session 失败");
        }

        // 设置 @termgrid 标记
        let _ = Command::new("tmux")
            .args(["set-option", "-t", session_name, "@termgrid", "1"])
            .status();

        // 挂载 pipe-pane
        let pipe_cmd = format!("cat > {}", pipe.display());
        let status = Command::new("tmux")
            .args(["pipe-pane", "-t", session_name, "-O", &pipe_cmd])
            .status()
            .context("执行 tmux pipe-pane 失败")?;
        if !status.success() {
            kill_session(session_name);
            bail!("tmux pipe-pane 失败");
        }

        let reader = TmuxReader { pipe_path: pipe };
        let backend = Self::start_flush_task(session_name.to_string());
        Ok((backend, reader))
    }

    /// 重连已有 tmux session
    pub fn reconnect(session_name: &str) -> Result<(Self, TmuxReader)> {
        // 先关闭旧的 pipe-pane
        let _ = Command::new("tmux")
            .args(["pipe-pane", "-t", session_name])
            .status();

        // 确保 FIFO 目录存在
        std::fs::create_dir_all(FIFO_DIR)
            .with_context(|| format!("创建 FIFO 目录失败: {}", FIFO_DIR))?;

        let pipe = fifo_path(session_name);

        // 删除旧 FIFO
        let _ = std::fs::remove_file(&pipe);

        // 创建新 FIFO
        let pipe_cstr = std::ffi::CString::new(pipe.to_str().unwrap())?;
        let ret = unsafe { libc::mkfifo(pipe_cstr.as_ptr(), 0o600) };
        if ret != 0 {
            bail!(
                "mkfifo 失败: {}",
                std::io::Error::last_os_error()
            );
        }

        // 重新挂 pipe-pane
        let pipe_cmd = format!("cat > {}", pipe.display());
        let status = Command::new("tmux")
            .args(["pipe-pane", "-t", session_name, "-O", &pipe_cmd])
            .status()
            .context("执行 tmux pipe-pane 失败")?;
        if !status.success() {
            let _ = std::fs::remove_file(&pipe);
            bail!("tmux pipe-pane 重连失败");
        }

        let reader = TmuxReader { pipe_path: pipe };
        let backend = Self::start_flush_task(session_name.to_string());
        Ok((backend, reader))
    }

    pub fn session_name(&self) -> &str {
        &self.session_name
    }

    fn start_flush_task(session_name: String) -> Self {
        let input_buffer: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel::<()>();

        let buf_clone = Arc::clone(&input_buffer);
        let name_clone = session_name.clone();
        tokio::spawn(async move {
            tokio::pin!(let cancel = cancel_rx;);
            loop {
                tokio::select! {
                    _ = &mut cancel => break,
                    _ = tokio::time::sleep(std::time::Duration::from_millis(10)) => {
                        let data = {
                            let mut buf = buf_clone.lock().unwrap();
                            if buf.is_empty() {
                                continue;
                            }
                            std::mem::take(&mut *buf)
                        };
                        flush_input_to_tmux(&name_clone, &data);
                    }
                }
            }
        });

        Self {
            session_name,
            input_buffer,
            _flush_cancel: cancel_tx,
        }
    }
}

/// 将解析后的输入事件发送到 tmux session
fn flush_input_to_tmux(session_name: &str, data: &[u8]) {
    let events = parse_input_chunk(data);
    for event in events {
        match event {
            InputEvent::Literal(text) => {
                let _ = Command::new("tmux")
                    .args(["send-keys", "-t", session_name, "-l", &text])
                    .output();
            }
            InputEvent::TmuxKey(key) => {
                let _ = Command::new("tmux")
                    .args(["send-keys", "-t", session_name, &key])
                    .output();
            }
            InputEvent::RawBytes(bytes) => {
                let hex_args: Vec<String> = bytes.iter().map(|b| format!("{:02x}", b)).collect();
                let mut cmd = Command::new("tmux");
                cmd.args(["send-keys", "-t", session_name, "-H"]);
                for h in &hex_args {
                    cmd.arg(h);
                }
                let _ = cmd.output();
            }
        }
    }
}

impl PtyBackend for TmuxPtyBackend {
    fn write_input(&mut self, data: &[u8]) -> anyhow::Result<()> {
        let mut buf = self.input_buffer.lock().unwrap();
        buf.extend_from_slice(data);
        Ok(())
    }

    fn resize(&self, cols: u16, rows: u16) -> anyhow::Result<()> {
        let status = Command::new("tmux")
            .args([
                "resize-window",
                "-t",
                &self.session_name,
                "-x",
                &cols.to_string(),
                "-y",
                &rows.to_string(),
            ])
            .status()
            .context("执行 tmux resize-window 失败")?;
        if !status.success() {
            bail!("tmux resize-window 失败");
        }
        Ok(())
    }

    fn is_alive(&mut self) -> bool {
        Command::new("tmux")
            .args(["has-session", "-t", &self.session_name])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn pid(&self) -> Option<u32> {
        let output = Command::new("tmux")
            .args([
                "display-message",
                "-t",
                &self.session_name,
                "-p",
                "#{pane_pid}",
            ])
            .output()
            .ok()?;
        if output.status.success() {
            String::from_utf8_lossy(&output.stdout)
                .trim()
                .parse::<u32>()
                .ok()
        } else {
            None
        }
    }

    fn signal_interrupt(&self) {
        let _ = Command::new("tmux")
            .args(["send-keys", "-t", &self.session_name, "C-c"])
            .output();
    }
}

impl Drop for TmuxPtyBackend {
    fn drop(&mut self) {
        // 清理 FIFO 文件，但不 kill session
        let pipe = fifo_path(&self.session_name);
        let _ = std::fs::remove_file(&pipe);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fifo_path() {
        let path = fifo_path("tg0");
        assert_eq!(path, PathBuf::from("/tmp/termgrid/tg0.pipe"));

        let path = fifo_path("tg42");
        assert_eq!(path, PathBuf::from("/tmp/termgrid/tg42.pipe"));
    }

    // -----------------------------------------------------------------------
    // parse_input_chunk 测试
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_printable_text() {
        let events = parse_input_chunk(b"hello world");
        assert_eq!(events, vec![InputEvent::Literal("hello world".to_string())]);
    }

    #[test]
    fn test_parse_ctrl_c() {
        let events = parse_input_chunk(b"\x03");
        assert_eq!(events, vec![InputEvent::TmuxKey("C-c".to_string())]);
    }

    #[test]
    fn test_parse_ctrl_d() {
        let events = parse_input_chunk(b"\x04");
        assert_eq!(events, vec![InputEvent::TmuxKey("C-d".to_string())]);
    }

    #[test]
    fn test_parse_ctrl_a() {
        let events = parse_input_chunk(b"\x01");
        assert_eq!(events, vec![InputEvent::TmuxKey("C-a".to_string())]);
    }

    #[test]
    fn test_parse_ctrl_z() {
        let events = parse_input_chunk(b"\x1a");
        assert_eq!(events, vec![InputEvent::TmuxKey("C-z".to_string())]);
    }

    #[test]
    fn test_parse_tab() {
        let events = parse_input_chunk(b"\x09");
        assert_eq!(events, vec![InputEvent::TmuxKey("Tab".to_string())]);
    }

    #[test]
    fn test_parse_enter_cr() {
        let events = parse_input_chunk(b"\x0d");
        assert_eq!(events, vec![InputEvent::TmuxKey("Enter".to_string())]);
    }

    #[test]
    fn test_parse_enter_lf() {
        let events = parse_input_chunk(b"\x0a");
        assert_eq!(events, vec![InputEvent::TmuxKey("Enter".to_string())]);
    }

    #[test]
    fn test_parse_backspace() {
        let events = parse_input_chunk(b"\x7f");
        assert_eq!(events, vec![InputEvent::TmuxKey("BSpace".to_string())]);
    }

    #[test]
    fn test_parse_escape_alone() {
        let events = parse_input_chunk(b"\x1b");
        assert_eq!(events, vec![InputEvent::TmuxKey("Escape".to_string())]);
    }

    #[test]
    fn test_parse_arrow_up() {
        let events = parse_input_chunk(b"\x1b[A");
        assert_eq!(events, vec![InputEvent::TmuxKey("Up".to_string())]);
    }

    #[test]
    fn test_parse_arrow_down() {
        let events = parse_input_chunk(b"\x1b[B");
        assert_eq!(events, vec![InputEvent::TmuxKey("Down".to_string())]);
    }

    #[test]
    fn test_parse_arrow_right() {
        let events = parse_input_chunk(b"\x1b[C");
        assert_eq!(events, vec![InputEvent::TmuxKey("Right".to_string())]);
    }

    #[test]
    fn test_parse_arrow_left() {
        let events = parse_input_chunk(b"\x1b[D");
        assert_eq!(events, vec![InputEvent::TmuxKey("Left".to_string())]);
    }

    #[test]
    fn test_parse_home_end() {
        let events = parse_input_chunk(b"\x1b[H");
        assert_eq!(events, vec![InputEvent::TmuxKey("Home".to_string())]);

        let events = parse_input_chunk(b"\x1b[F");
        assert_eq!(events, vec![InputEvent::TmuxKey("End".to_string())]);
    }

    #[test]
    fn test_parse_insert_delete() {
        let events = parse_input_chunk(b"\x1b[2~");
        assert_eq!(events, vec![InputEvent::TmuxKey("IC".to_string())]);

        let events = parse_input_chunk(b"\x1b[3~");
        assert_eq!(events, vec![InputEvent::TmuxKey("DC".to_string())]);
    }

    #[test]
    fn test_parse_page_up_down() {
        let events = parse_input_chunk(b"\x1b[5~");
        assert_eq!(events, vec![InputEvent::TmuxKey("PPage".to_string())]);

        let events = parse_input_chunk(b"\x1b[6~");
        assert_eq!(events, vec![InputEvent::TmuxKey("NPage".to_string())]);
    }

    #[test]
    fn test_parse_ctrl_arrow() {
        // Ctrl+Up: ESC [ 1 ; 5 A
        let events = parse_input_chunk(b"\x1b[1;5A");
        assert_eq!(events, vec![InputEvent::TmuxKey("C-Up".to_string())]);

        // Ctrl+Down: ESC [ 1 ; 5 B
        let events = parse_input_chunk(b"\x1b[1;5B");
        assert_eq!(events, vec![InputEvent::TmuxKey("C-Down".to_string())]);

        // Ctrl+Right
        let events = parse_input_chunk(b"\x1b[1;5C");
        assert_eq!(events, vec![InputEvent::TmuxKey("C-Right".to_string())]);

        // Ctrl+Left
        let events = parse_input_chunk(b"\x1b[1;5D");
        assert_eq!(events, vec![InputEvent::TmuxKey("C-Left".to_string())]);
    }

    #[test]
    fn test_parse_f1_to_f4() {
        let events = parse_input_chunk(b"\x1bOP");
        assert_eq!(events, vec![InputEvent::TmuxKey("F1".to_string())]);

        let events = parse_input_chunk(b"\x1bOQ");
        assert_eq!(events, vec![InputEvent::TmuxKey("F2".to_string())]);

        let events = parse_input_chunk(b"\x1bOR");
        assert_eq!(events, vec![InputEvent::TmuxKey("F3".to_string())]);

        let events = parse_input_chunk(b"\x1bOS");
        assert_eq!(events, vec![InputEvent::TmuxKey("F4".to_string())]);
    }

    #[test]
    fn test_parse_f5_to_f12() {
        let cases = [
            (b"\x1b[15~".as_slice(), "F5"),
            (b"\x1b[17~".as_slice(), "F6"),
            (b"\x1b[18~".as_slice(), "F7"),
            (b"\x1b[19~".as_slice(), "F8"),
            (b"\x1b[20~".as_slice(), "F9"),
            (b"\x1b[21~".as_slice(), "F10"),
            (b"\x1b[23~".as_slice(), "F11"),
            (b"\x1b[24~".as_slice(), "F12"),
        ];
        for (input, expected) in &cases {
            let events = parse_input_chunk(input);
            assert_eq!(
                events,
                vec![InputEvent::TmuxKey(expected.to_string())],
                "failed for {:?}",
                expected
            );
        }
    }

    #[test]
    fn test_parse_mixed_input() {
        // "hello" + Enter + Ctrl-C
        let input = b"hello\x0d\x03";
        let events = parse_input_chunk(input);
        assert_eq!(
            events,
            vec![
                InputEvent::Literal("hello".to_string()),
                InputEvent::TmuxKey("Enter".to_string()),
                InputEvent::TmuxKey("C-c".to_string()),
            ]
        );
    }

    #[test]
    fn test_parse_text_with_arrows() {
        // "ls" + Left + Left + "cd "
        let mut input = Vec::new();
        input.extend_from_slice(b"ls");
        input.extend_from_slice(b"\x1b[D"); // Left
        input.extend_from_slice(b"\x1b[D"); // Left
        input.extend_from_slice(b"cd ");
        let events = parse_input_chunk(&input);
        assert_eq!(
            events,
            vec![
                InputEvent::Literal("ls".to_string()),
                InputEvent::TmuxKey("Left".to_string()),
                InputEvent::TmuxKey("Left".to_string()),
                InputEvent::Literal("cd ".to_string()),
            ]
        );
    }

    #[test]
    fn test_parse_shift_arrow() {
        // Shift+Right: ESC [ 1 ; 2 C
        let events = parse_input_chunk(b"\x1b[1;2C");
        assert_eq!(events, vec![InputEvent::TmuxKey("S-Right".to_string())]);
    }

    #[test]
    fn test_parse_alt_arrow() {
        // Alt+Up: ESC [ 1 ; 3 A
        let events = parse_input_chunk(b"\x1b[1;3A");
        assert_eq!(events, vec![InputEvent::TmuxKey("M-Up".to_string())]);
    }

    #[test]
    fn test_parse_empty_input() {
        let events = parse_input_chunk(b"");
        assert!(events.is_empty());
    }

    #[test]
    fn test_parse_utf8() {
        let events = parse_input_chunk("你好".as_bytes());
        assert_eq!(events, vec![InputEvent::Literal("你好".to_string())]);
    }

    // -----------------------------------------------------------------------
    // 辅助函数测试（不依赖 tmux）
    // -----------------------------------------------------------------------

    #[test]
    fn test_is_inside_tmux_no_panic() {
        // 只检查不 panic，不断言具体值
        let _ = is_inside_tmux();
    }

    #[test]
    fn test_is_tmux_available_no_panic() {
        let _ = is_tmux_available();
    }

    #[test]
    fn test_next_session_id_reasonable() {
        // 不依赖 tmux 运行，仅验证不 panic 且返回合理值
        let id = next_session_id();
        assert!(id < 10000, "session id 应该是合理值，得到: {}", id);
    }

    #[test]
    fn test_control_char_coverage() {
        // 验证所有 Ctrl 字母映射
        for b in 0x01u8..=0x1a {
            let key = control_char_to_tmux_key(b);
            assert!(key.is_some(), "Ctrl char 0x{:02x} 应该有映射", b);
            let key = key.unwrap();
            if b == 0x09 {
                assert_eq!(key, "Tab");
            } else if b == 0x0a {
                assert_eq!(key, "Enter");
            } else if b == 0x0d {
                assert_eq!(key, "Enter");
            } else {
                assert!(key.starts_with("C-"), "0x{:02x} 应映射为 C-x，得到: {}", b, key);
            }
        }
        assert_eq!(control_char_to_tmux_key(0x7f), Some("BSpace".to_string()));
        assert_eq!(control_char_to_tmux_key(0x00), None);
    }
}
