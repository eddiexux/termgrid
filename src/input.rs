use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::{AppMode, OverlayKind};
use crate::tile_manager::TileManager;

/// Handle key events in Overlay mode (help, confirm close, project selector).
pub fn handle_overlay_key(
    key: KeyEvent,
    mode: &mut AppMode,
    tile_manager: &mut TileManager,
) {
    let kind = match mode {
        AppMode::Overlay(ref k) => k.clone(),
        _ => return,
    };

    match kind {
        OverlayKind::Help => {
            // Any key closes the help overlay
            *mode = AppMode::Normal;
        }
        OverlayKind::ConfirmClose(id) => {
            if key.code == KeyCode::Char('y') {
                tile_manager.remove(id);
            }
            *mode = AppMode::Normal;
        }
        OverlayKind::ProjectSelector { .. } => {
            if key.code == KeyCode::Esc {
                *mode = AppMode::Normal;
            }
        }
    }
}

/// Convert a crossterm KeyEvent into the bytes to send to the PTY.
pub fn key_event_to_bytes(key: &KeyEvent) -> Vec<u8> {
    match key.code {
        KeyCode::Char(c) => {
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                // Control codes: Ctrl+a = 1, Ctrl+b = 2, ...
                let lower = c.to_ascii_lowercase();
                if lower.is_ascii_lowercase() {
                    vec![lower as u8 - b'a' + 1]
                } else if lower == '[' {
                    vec![0x1b]
                } else {
                    vec![c as u8]
                }
            } else {
                let mut buf = [0u8; 4];
                let s = c.encode_utf8(&mut buf);
                s.as_bytes().to_vec()
            }
        }
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::Esc => vec![0x1b],
        KeyCode::Up => b"\x1b[A".to_vec(),
        KeyCode::Down => b"\x1b[B".to_vec(),
        KeyCode::Right => b"\x1b[C".to_vec(),
        KeyCode::Left => b"\x1b[D".to_vec(),
        KeyCode::Delete => b"\x1b[3~".to_vec(),
        KeyCode::Home => b"\x1b[H".to_vec(),
        KeyCode::End => b"\x1b[F".to_vec(),
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    fn ctrl_key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    #[test]
    fn test_key_to_bytes_char() {
        let k = key(KeyCode::Char('a'));
        assert_eq!(key_event_to_bytes(&k), b"a");
    }

    #[test]
    fn test_key_to_bytes_ctrl_c() {
        let k = ctrl_key(KeyCode::Char('c'));
        assert_eq!(key_event_to_bytes(&k), vec![3]);
    }

    #[test]
    fn test_key_to_bytes_enter() {
        let k = key(KeyCode::Enter);
        assert_eq!(key_event_to_bytes(&k), b"\r");
    }

    #[test]
    fn test_key_to_bytes_arrow() {
        let k = key(KeyCode::Up);
        assert_eq!(key_event_to_bytes(&k), b"\x1b[A");
    }

    #[test]
    fn test_key_to_bytes_unicode() {
        let k = key(KeyCode::Char('中'));
        let expected = '中'.to_string().into_bytes();
        assert_eq!(key_event_to_bytes(&k), expected);
    }

    #[test]
    fn test_overlay_help_any_key_closes() {
        let mut mode = AppMode::Overlay(OverlayKind::Help);
        let mut tm = TileManager::new();
        handle_overlay_key(key(KeyCode::Char('a')), &mut mode, &mut tm);
        assert_eq!(mode, AppMode::Normal);
    }

    #[test]
    fn test_overlay_confirm_close_y_removes() {
        let mut mode =
            AppMode::Overlay(OverlayKind::ConfirmClose(crate::tile::TileId(999)));
        let mut tm = TileManager::new();
        // 'y' on a nonexistent tile still transitions to Normal
        handle_overlay_key(key(KeyCode::Char('y')), &mut mode, &mut tm);
        assert_eq!(mode, AppMode::Normal);
    }

    #[test]
    fn test_overlay_confirm_close_n_cancels() {
        let mut mode =
            AppMode::Overlay(OverlayKind::ConfirmClose(crate::tile::TileId(999)));
        let mut tm = TileManager::new();
        handle_overlay_key(key(KeyCode::Char('n')), &mut mode, &mut tm);
        assert_eq!(mode, AppMode::Normal);
    }

    #[test]
    fn test_noop_on_normal_mode() {
        let mut mode = AppMode::Normal;
        let mut tm = TileManager::new();
        // Should not panic or change anything
        handle_overlay_key(key(KeyCode::Char('q')), &mut mode, &mut tm);
        assert_eq!(mode, AppMode::Normal);
    }

    #[test]
    fn test_key_to_bytes_backspace() {
        assert_eq!(key_event_to_bytes(&key(KeyCode::Backspace)), vec![0x7f]);
    }

    #[test]
    fn test_key_to_bytes_tab() {
        assert_eq!(key_event_to_bytes(&key(KeyCode::Tab)), b"\t");
    }

    #[test]
    fn test_key_to_bytes_esc() {
        assert_eq!(key_event_to_bytes(&key(KeyCode::Esc)), vec![0x1b]);
    }

    #[test]
    fn test_key_to_bytes_delete() {
        assert_eq!(key_event_to_bytes(&key(KeyCode::Delete)), b"\x1b[3~");
    }

    #[test]
    fn test_key_to_bytes_home_end() {
        assert_eq!(key_event_to_bytes(&key(KeyCode::Home)), b"\x1b[H");
        assert_eq!(key_event_to_bytes(&key(KeyCode::End)), b"\x1b[F");
    }

    #[test]
    fn test_key_to_bytes_all_arrows() {
        assert_eq!(key_event_to_bytes(&key(KeyCode::Up)), b"\x1b[A");
        assert_eq!(key_event_to_bytes(&key(KeyCode::Down)), b"\x1b[B");
        assert_eq!(key_event_to_bytes(&key(KeyCode::Right)), b"\x1b[C");
        assert_eq!(key_event_to_bytes(&key(KeyCode::Left)), b"\x1b[D");
    }

    #[test]
    fn test_key_to_bytes_ctrl_a_to_z() {
        // Ctrl+A = 1, Ctrl+Z = 26
        for (i, ch) in ('a'..='z').enumerate() {
            let k = ctrl_key(KeyCode::Char(ch));
            assert_eq!(
                key_event_to_bytes(&k),
                vec![(i + 1) as u8],
                "Ctrl+{} should be byte {}",
                ch,
                i + 1
            );
        }
    }

    #[test]
    fn test_key_to_bytes_ctrl_bracket_is_escape() {
        let k = ctrl_key(KeyCode::Char('['));
        assert_eq!(key_event_to_bytes(&k), vec![0x1b]);
    }

    #[test]
    fn test_key_to_bytes_unknown_returns_empty() {
        let k = key(KeyCode::F(1));
        assert_eq!(key_event_to_bytes(&k), Vec::<u8>::new());
    }

    #[test]
    fn test_overlay_help_closes_on_any_key() {
        // Test multiple different keys all close help
        for code in [KeyCode::Esc, KeyCode::Enter, KeyCode::Char('q')] {
            let mut mode = AppMode::Overlay(OverlayKind::Help);
            let mut tm = TileManager::new();
            handle_overlay_key(key(code), &mut mode, &mut tm);
            assert_eq!(mode, AppMode::Normal);
        }
    }

    #[test]
    fn test_overlay_project_selector_esc_closes() {
        let mut mode = AppMode::Overlay(OverlayKind::ProjectSelector {
            query: String::new(),
            items: vec![],
            selected: 0,
        });
        let mut tm = TileManager::new();
        handle_overlay_key(key(KeyCode::Esc), &mut mode, &mut tm);
        assert_eq!(mode, AppMode::Normal);
    }
}
