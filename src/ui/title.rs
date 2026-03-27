use crate::tile::{Tile, TileStatus};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// Parsed remote session info from terminal title (OSC 0/2).
struct RemoteInfo {
    user: String,
    host: String,
    path: String,
}

/// Try to parse `user@host: path` or `user@host:path` from a terminal title string.
/// Returns `None` if the title doesn't match or the host is local.
fn parse_remote_title(title: &str) -> Option<RemoteInfo> {
    let title = title.trim();
    if title.is_empty() {
        return None;
    }

    // Match patterns: "user@host: path", "user@host:path", "user@host:~", etc.
    let at_pos = title.find('@')?;
    let user = &title[..at_pos];
    let after_at = &title[at_pos + 1..];

    // Find the colon separator between host and path
    let colon_pos = after_at.find(':')?;
    let host = after_at[..colon_pos].trim();
    let path = after_at[colon_pos + 1..].trim();

    if user.is_empty() || host.is_empty() {
        return None;
    }

    // Check if host is local
    if is_local_host(host) {
        return None;
    }

    Some(RemoteInfo {
        user: user.to_string(),
        host: host.to_string(),
        path: path.to_string(),
    })
}

/// Get the local hostname via libc.
fn local_hostname() -> Option<String> {
    let mut buf = [0u8; 256];
    let ret = unsafe { libc::gethostname(buf.as_mut_ptr() as *mut libc::c_char, buf.len()) };
    if ret != 0 {
        return None;
    }
    let len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    std::str::from_utf8(&buf[..len]).ok().map(String::from)
}

/// Check if a hostname refers to the local machine.
fn is_local_host(host: &str) -> bool {
    let host_lower = host.to_lowercase();
    if host_lower == "localhost" || host_lower == "127.0.0.1" || host_lower == "::1" {
        return true;
    }
    if let Some(local_str) = local_hostname() {
        // Compare case-insensitively; also match short hostname vs FQDN
        let host_lower = host.to_lowercase();
        let local_lower = local_str.to_lowercase();
        if host_lower == local_lower {
            return true;
        }
        // "myhost" matches "myhost.local" or "myhost.domain.com"
        if local_lower.starts_with(&format!("{}.", host_lower))
            || host_lower.starts_with(&format!("{}.", local_lower))
        {
            return true;
        }
    }
    false
}

/// Build the standard title line used by both tile card and detail panel.
pub fn build_title_line(tile: &Tile, index_label: Option<&str>) -> Line<'static> {
    let mut spans = Vec::new();

    // Status tag
    let (status_label, status_color) = match &tile.status {
        TileStatus::Running => ("▶ RUN", Color::Green),
        TileStatus::Waiting => ("◉ WAIT", Color::Yellow),
        TileStatus::Idle(_) => ("◌ IDLE", Color::DarkGray),
        TileStatus::Exited => ("✕ EXIT", Color::Red),
        TileStatus::Error(_) => ("! ERR", Color::Red),
    };
    spans.push(Span::styled(
        format!("[{}] ", status_label),
        Style::default().fg(status_color),
    ));

    // tmux session name (e.g. "tg0")
    if let Some(ref sname) = tile.session_name {
        spans.push(Span::styled(
            format!("{} ", sname),
            Style::default().fg(Color::DarkGray),
        ));
    }

    // Claude Code indicator
    if tile.is_claude_code() {
        spans.push(Span::styled(
            "[CC] ",
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ));
    }

    // Index label for disambiguation (e.g. "[2]")
    if let Some(label) = index_label {
        spans.push(Span::styled(
            format!("{} ", label),
            Style::default().fg(Color::DarkGray),
        ));
    }

    // Check for remote session via VTE terminal title
    if let Some(remote) = parse_remote_title(tile.vte.title()) {
        // Remote: show "user@host:path" format
        spans.push(Span::styled(
            format!("{}@{}", remote.user, remote.host),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            format!(":{}", remote.path),
            Style::default().fg(Color::Gray),
        ));
    } else {
        // Local: show git context + cwd
        if let Some(ref git_ctx) = tile.git_context {
            spans.push(Span::styled(
                git_ctx.project_name.clone(),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ));

            // Branch tag
            if let Some(ref branch) = git_ctx.branch {
                spans.push(Span::styled(
                    format!(" ⑂{}", branch),
                    Style::default().fg(Color::Blue),
                ));
            }

            // Worktree tag
            if git_ctx.is_worktree {
                if let Some(ref wt_name) = git_ctx.worktree_name {
                    spans.push(Span::styled(
                        format!(" ⑃{}", wt_name),
                        Style::default().fg(Color::Magenta),
                    ));
                }
            }

            spans.push(Span::raw(" "));
        }

        // Path (gray)
        let path_str = tile.cwd.display().to_string();
        spans.push(Span::styled(path_str, Style::default().fg(Color::Gray)));
    }

    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_remote_title_standard() {
        let info = parse_remote_title("user@prod-server: ~/deploy/app").unwrap();
        assert_eq!(info.user, "user");
        assert_eq!(info.host, "prod-server");
        assert_eq!(info.path, "~/deploy/app");
    }

    #[test]
    fn test_parse_remote_title_no_space_after_colon() {
        let info = parse_remote_title("root@10.0.1.5:/var/log").unwrap();
        assert_eq!(info.user, "root");
        assert_eq!(info.host, "10.0.1.5");
        assert_eq!(info.path, "/var/log");
    }

    #[test]
    fn test_parse_remote_title_home_tilde() {
        let info = parse_remote_title("deploy@web01:~").unwrap();
        assert_eq!(info.host, "web01");
        assert_eq!(info.path, "~");
    }

    #[test]
    fn test_parse_remote_title_localhost_returns_none() {
        assert!(parse_remote_title("user@localhost: ~/code").is_none());
    }

    #[test]
    fn test_parse_remote_title_empty() {
        assert!(parse_remote_title("").is_none());
    }

    #[test]
    fn test_parse_remote_title_no_at() {
        assert!(parse_remote_title("/usr/local/bin").is_none());
    }

    #[test]
    fn test_parse_remote_title_no_colon() {
        assert!(parse_remote_title("user@host").is_none());
    }

    #[test]
    fn test_is_local_host_loopback() {
        assert!(is_local_host("localhost"));
        assert!(is_local_host("127.0.0.1"));
        assert!(is_local_host("::1"));
    }

    #[test]
    fn test_is_local_host_remote() {
        assert!(!is_local_host("prod-server"));
        assert!(!is_local_host("10.0.1.5"));
    }

    #[test]
    fn test_parse_remote_title_multiple_at_signs() {
        // "user@host@extra: /path" — first @ is the split point
        let info = parse_remote_title("user@host@extra: /path");
        // Should parse user="user", after_at="host@extra: /path", host="host@extra", path="/path"
        assert!(info.is_some());
        let info = info.unwrap();
        assert_eq!(info.user, "user");
    }

    #[test]
    fn test_parse_remote_title_multiple_colons() {
        // "user@host:/path:/with/colons" — first colon after host is split point
        let info = parse_remote_title("user@host:/path:/with/colons").unwrap();
        assert_eq!(info.host, "host");
        assert_eq!(info.path, "/path:/with/colons");
    }

    #[test]
    fn test_parse_remote_title_whitespace_only() {
        assert!(parse_remote_title("   ").is_none());
    }

    #[test]
    fn test_parse_remote_title_empty_user() {
        // "@host:path" — empty user should return None
        assert!(parse_remote_title("@host:path").is_none());
    }

    #[test]
    fn test_parse_remote_title_empty_host() {
        // "user@:path" — empty host should return None
        assert!(parse_remote_title("user@:path").is_none());
    }

    #[test]
    fn test_is_local_host_case_insensitive() {
        // Should match regardless of case
        assert!(is_local_host("LOCALHOST"));
        assert!(is_local_host("Localhost"));
    }

    #[test]
    fn test_is_local_host_fqdn_matching() {
        // Get real local hostname and test FQDN suffix matching
        if let Some(hostname) = local_hostname() {
            assert!(is_local_host(&hostname));
            // hostname.local should also match
            let fqdn = format!("{}.local", hostname);
            assert!(is_local_host(&fqdn));
        }
    }

    #[test]
    fn test_local_hostname_returns_some() {
        // On any real system, hostname should be retrievable
        let hostname = local_hostname();
        assert!(hostname.is_some());
        assert!(!hostname.unwrap().is_empty());
    }
}
