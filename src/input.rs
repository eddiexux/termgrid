use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::{AppMode, OverlayKind};
use crate::tab::{self, TabEntry, TabFilter};
use crate::tile::TileStatus;
use crate::tile_manager::{Direction, TileManager};

pub enum InputResult {
    Continue,
    Quit,
}

/// Dispatch a key event based on current app mode.
pub fn handle_key(
    key: KeyEvent,
    mode: &mut AppMode,
    tile_manager: &mut TileManager,
    active_tab: &mut TabFilter,
    tab_entries: &[TabEntry],
    columns: u8,
) -> InputResult {
    match mode.clone() {
        AppMode::Normal => handle_normal_key(key, mode, tile_manager, active_tab, tab_entries, columns),
        AppMode::Insert => handle_insert_key(key, mode, tile_manager),
        AppMode::Overlay(ref kind) => handle_overlay_key(key, mode, tile_manager, kind.clone()),
    }
}

fn handle_normal_key(
    key: KeyEvent,
    mode: &mut AppMode,
    tile_manager: &mut TileManager,
    active_tab: &mut TabFilter,
    tab_entries: &[TabEntry],
    columns: u8,
) -> InputResult {
    match key.code {
        KeyCode::Char('q') => return InputResult::Quit,
        KeyCode::Char('?') => {
            *mode = AppMode::Overlay(OverlayKind::Help);
        }
        KeyCode::Up | KeyCode::Char('k') => {
            tile_manager.select_direction(active_tab, columns as usize, Direction::Up);
        }
        KeyCode::Down | KeyCode::Char('j') => {
            tile_manager.select_direction(active_tab, columns as usize, Direction::Down);
        }
        KeyCode::Left | KeyCode::Char('h') => {
            tile_manager.select_direction(active_tab, columns as usize, Direction::Left);
        }
        KeyCode::Right | KeyCode::Char('l') => {
            tile_manager.select_direction(active_tab, columns as usize, Direction::Right);
        }
        KeyCode::Char('i') | KeyCode::Enter => {
            if tile_manager.selected_id().is_some() {
                *mode = AppMode::Insert;
            }
        }
        KeyCode::Esc => {
            tile_manager.deselect();
        }
        KeyCode::Char('n') => {
            *mode = AppMode::Overlay(OverlayKind::ProjectSelector {
                query: String::new(),
                items: Vec::new(),
                selected: 0,
            });
        }
        KeyCode::Char('x') => {
            if let Some(id) = tile_manager.selected_id() {
                let needs_confirm = tile_manager
                    .get(id)
                    .map(|t| t.status == TileStatus::Running)
                    .unwrap_or(false);
                if needs_confirm {
                    *mode = AppMode::Overlay(OverlayKind::ConfirmClose(id));
                } else {
                    tile_manager.remove(id);
                }
            }
        }
        KeyCode::Tab => {
            *active_tab = tab::next_tab(active_tab, tab_entries);
        }
        KeyCode::BackTab => {
            *active_tab = tab::prev_tab(active_tab, tab_entries);
        }
        _ => {}
    }
    InputResult::Continue
}

fn handle_insert_key(
    key: KeyEvent,
    mode: &mut AppMode,
    tile_manager: &mut TileManager,
) -> InputResult {
    // Esc → back to Normal mode
    // Also support Ctrl+] (reported as Char(']')+CONTROL or raw Char('\x1d'))
    let is_exit = key.code == KeyCode::Esc
        || (key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char(']'))
        || key.code == KeyCode::Char('\x1d');
    if is_exit {
        *mode = AppMode::Normal;
        return InputResult::Continue;
    }

    // Forward all other keys to the selected PTY
    let bytes = key_event_to_bytes(&key);
    if !bytes.is_empty() {
        if let Some(tile) = tile_manager.selected_mut() {
            let _ = tile.write_input(&bytes);
        }
    }

    InputResult::Continue
}

fn handle_overlay_key(
    key: KeyEvent,
    mode: &mut AppMode,
    tile_manager: &mut TileManager,
    kind: OverlayKind,
) -> InputResult {
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
            // Other keys could filter the project list — handled by App
        }
    }
    InputResult::Continue
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
}
