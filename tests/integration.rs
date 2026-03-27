use std::time::{Duration, Instant};
use termgrid::config::Config;
use termgrid::screen::VteState;

#[test]
fn test_vte_full_session_simulation() {
    let mut vte = VteState::new(80, 24);
    // Simulate a shell session with colors
    vte.process(b"\x1b[32muser@host\x1b[0m:\x1b[34m~/project\x1b[0m$ ");
    vte.process(b"cargo test\r\n");
    vte.process(b"    \x1b[32mCompiling\x1b[0m termgrid v0.1.0\r\n");
    vte.process(b"    \x1b[32mFinished\x1b[0m test target\r\n");
    vte.process(b"test result: \x1b[32mok\x1b[0m. 10 passed; 0 failed\r\n");

    // Verify content is present on the first row
    let line0: String = (0..vte.cols())
        .map(|col| vte.cell_at(0, col).ch)
        .collect::<String>();
    assert!(line0.contains("user@host"));

    // Verify color was applied — 'u' in 'user' should be green
    let first_char = vte.cell_at(0, 0);
    assert_eq!(first_char.fg, ratatui::style::Color::Green);
}

#[test]
fn test_config_default_loads() {
    let config = Config::default();
    assert!(config.layout.default_columns >= 1);
    assert!(config.layout.default_columns <= 3);
}

#[test]
fn test_git_detection_current_dir() {
    // This test works because termgrid itself is a git repo
    let cwd = std::env::current_dir().unwrap();
    let ctx = termgrid::git::detect_git(&cwd);
    if let Some(ctx) = ctx {
        assert_eq!(ctx.project_name, "termgrid");
    }
}

#[test]
fn test_layout_various_configs() {
    use ratatui::layout::Rect;

    // Minimal terminal
    let layout = termgrid::layout::calculate_layout(Rect::new(0, 0, 40, 10), 1, 2, 45, 0);
    assert!(layout.tile_rects.len() <= 2);

    // Wide terminal — detail panel always shown
    let layout = termgrid::layout::calculate_layout(Rect::new(0, 0, 200, 50), 3, 9, 45, 0);
    assert!(layout.detail_panel.is_some());
}

#[test]
fn test_tab_aggregation_round_trip() {
    use std::path::PathBuf;
    use termgrid::git::GitContext;
    use termgrid::tab::{aggregate_tabs, next_tab, TabFilter};

    let contexts = vec![
        Some(GitContext {
            project_name: "alpha".into(),
            branch: Some("main".into()),
            is_worktree: false,
            worktree_name: None,
            repo_root: PathBuf::from("/tmp"),
        }),
        Some(GitContext {
            project_name: "alpha".into(),
            branch: Some("dev".into()),
            is_worktree: false,
            worktree_name: None,
            repo_root: PathBuf::from("/tmp"),
        }),
        None,
    ];

    let tabs = aggregate_tabs(&contexts);
    assert!(!tabs.is_empty());

    // Cycling should work
    let filter = TabFilter::All;
    let next = next_tab(&filter, &tabs);
    assert_ne!(next, TabFilter::All); // moved somewhere
}

#[test]
fn test_session_round_trip() {
    use std::path::PathBuf;
    use termgrid::session::{Session, TileSession};

    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("test_session.json");

    let session = Session {
        tiles: vec![
            TileSession {
                cwd: PathBuf::from("/tmp/a"),
                scrollback_index: None,
                tmux_session: None,
            },
            TileSession {
                cwd: PathBuf::from("/tmp/b"),
                scrollback_index: None,
                tmux_session: None,
            },
        ],
        columns: 3,
        active_tab: "alpha".into(),
    };

    session.save(&path).unwrap();
    let loaded = Session::load(&path).unwrap();
    assert_eq!(loaded.tiles.len(), 2);
    assert_eq!(loaded.columns, 3);
}

// =============================================================================
// Burst+Silence unread detection tests
// =============================================================================
//
// These tests simulate the detection logic from App::poll_tile_states without
// requiring the full async App. The detection condition is:
//   tile.burst_bytes >= UNREAD_BURST_THRESHOLD AND last_active.elapsed() >= UNREAD_SILENCE_DURATION AND !tile.has_unread
//
// burst_bytes is only incremented for non-selected tiles (by app.rs PtyOutput handler).

#[test]
#[cfg(target_os = "macos")]
fn test_claude_code_detection_live() {
    use termgrid::process::{get_foreground_pid, get_process_name};

    // If a claude process is running, verify the full detection chain works
    let output = std::process::Command::new("pgrep")
        .arg("claude")
        .output();
    if let Ok(out) = output {
        let text = String::from_utf8_lossy(&out.stdout);
        if let Some(line) = text.lines().next() {
            if let Ok(pid) = line.trim().parse::<i32>() {
                let name = get_process_name(pid);
                assert_eq!(
                    name.as_deref(),
                    Some("claude"),
                    "proc_pidpath-based detection should return 'claude' for Claude Code"
                );
            }
        }
    }
}

mod burst_detection {
    use super::*;
    use termgrid::app::{UNREAD_BURST_THRESHOLD, UNREAD_SILENCE_DURATION};
    use termgrid::tab::TabFilter;
    use termgrid::tile::{Tile, TileId};
    use termgrid::tile_manager::TileManager;

    fn make_tile(mgr: &mut TileManager) -> TileId {
        let id = mgr.next_tile_id();
        let dir = std::env::current_dir().unwrap();
        let (tile, _reader) = Tile::spawn(id, "/bin/sh", &dir, 80, 24).unwrap();
        mgr.add(tile);
        id
    }

    /// Simulate the detection logic from poll_tile_states.
    fn run_detection(mgr: &mut TileManager, _selected_id: Option<TileId>) {
        for tile in mgr.tiles_mut() {
            if tile.has_unread {
                continue;
            }
            if tile.burst_bytes >= UNREAD_BURST_THRESHOLD
                && tile.last_active.elapsed() >= UNREAD_SILENCE_DURATION
            {
                tile.has_unread = true;
                tile.burst_bytes = 0;
            }
        }
    }

    #[test]
    fn test_burst_then_silence_triggers_unread() {
        let mut mgr = TileManager::new();
        let id = make_tile(&mut mgr);

        // Simulate non-selected tile receiving a burst
        let tile = mgr.get_mut(id).unwrap();
        tile.process_output(&vec![b'X'; 5000]);
        tile.burst_bytes = 5000; // app.rs would do this for non-selected
        // Simulate 6 seconds of silence
        tile.last_active = Instant::now() - Duration::from_secs(6);

        mgr.deselect(); // nothing selected
        run_detection(&mut mgr, None);

        assert!(mgr.get(id).unwrap().has_unread);
        assert_eq!(mgr.get(id).unwrap().burst_bytes, 0); // reset after trigger
    }

    #[test]
    fn test_small_burst_does_not_trigger() {
        let mut mgr = TileManager::new();
        let id = make_tile(&mut mgr);

        let tile = mgr.get_mut(id).unwrap();
        tile.process_output(b"prompt$ ");
        tile.burst_bytes = 8; // tiny output
        tile.last_active = Instant::now() - Duration::from_secs(10);

        mgr.deselect();
        run_detection(&mut mgr, None);

        assert!(!mgr.get(id).unwrap().has_unread);
    }

    #[test]
    fn test_recent_output_does_not_trigger() {
        let mut mgr = TileManager::new();
        let id = make_tile(&mut mgr);

        let tile = mgr.get_mut(id).unwrap();
        tile.burst_bytes = 5000;
        tile.last_active = Instant::now(); // just now — still outputting

        mgr.deselect();
        run_detection(&mut mgr, None);

        assert!(!mgr.get(id).unwrap().has_unread);
        assert_eq!(mgr.get(id).unwrap().burst_bytes, 5000); // not reset
    }

    #[test]
    fn test_selected_tile_also_triggers_unread() {
        let mut mgr = TileManager::new();
        let id = make_tile(&mut mgr);

        mgr.select(id);
        // select resets burst_bytes, so set after select
        mgr.get_mut(id).unwrap().burst_bytes = 5000;
        mgr.get_mut(id).unwrap().last_active = Instant::now() - Duration::from_secs(10);

        run_detection(&mut mgr, Some(id));

        // Selected tile should still get unread flag
        assert!(mgr.get(id).unwrap().has_unread);
    }

    #[test]
    fn test_already_unread_not_re_triggered() {
        let mut mgr = TileManager::new();
        let id = make_tile(&mut mgr);

        let tile = mgr.get_mut(id).unwrap();
        tile.has_unread = true;
        tile.burst_bytes = 5000; // new output arrived while already yellow
        tile.last_active = Instant::now() - Duration::from_secs(10);

        mgr.deselect();
        run_detection(&mut mgr, None);

        // has_unread stays true, burst_bytes NOT reset (skipped by continue)
        assert!(mgr.get(id).unwrap().has_unread);
        assert_eq!(mgr.get(id).unwrap().burst_bytes, 5000);
    }

    #[test]
    fn test_select_resets_for_next_cycle() {
        let mut mgr = TileManager::new();
        let id = make_tile(&mut mgr);

        // First cycle: trigger unread
        let tile = mgr.get_mut(id).unwrap();
        tile.burst_bytes = 5000;
        tile.last_active = Instant::now() - Duration::from_secs(6);
        mgr.deselect();
        run_detection(&mut mgr, None);
        assert!(mgr.get(id).unwrap().has_unread);

        // User clicks — clears unread and burst
        mgr.select(id);
        assert!(!mgr.get(id).unwrap().has_unread);
        assert_eq!(mgr.get(id).unwrap().burst_bytes, 0);

        // Second cycle: no new output yet → should not trigger
        mgr.deselect();
        mgr.get_mut(id).unwrap().last_active = Instant::now() - Duration::from_secs(10);
        run_detection(&mut mgr, None);
        assert!(!mgr.get(id).unwrap().has_unread); // burst_bytes is 0
    }

    #[test]
    fn test_multiple_tiles_independent() {
        let mut mgr = TileManager::new();
        let id_a = make_tile(&mut mgr);
        let id_b = make_tile(&mut mgr);

        // Tile A: big burst, long silence → should trigger
        let tile_a = mgr.get_mut(id_a).unwrap();
        tile_a.burst_bytes = UNREAD_BURST_THRESHOLD + 1000;
        tile_a.last_active = Instant::now() - Duration::from_secs(7);

        // Tile B: small burst → should not trigger
        let tile_b = mgr.get_mut(id_b).unwrap();
        tile_b.burst_bytes = 50;
        tile_b.last_active = Instant::now() - Duration::from_secs(7);

        mgr.deselect();
        run_detection(&mut mgr, None);

        assert!(mgr.get(id_a).unwrap().has_unread);
        assert!(!mgr.get(id_b).unwrap().has_unread);
    }

    #[test]
    fn test_deselection_does_not_carry_over_burst() {
        let mut mgr = TileManager::new();
        let id_a = make_tile(&mut mgr);
        let id_b = make_tile(&mut mgr);

        // Select tile A, simulate output while selected
        mgr.select(id_a);
        let tile_a = mgr.get_mut(id_a).unwrap();
        tile_a.process_output(&vec![b'A'; 2000]);
        // burst_bytes should be 0 because app.rs doesn't increment for selected tiles
        assert_eq!(tile_a.burst_bytes, 0);

        // Switch to tile B → tile A becomes non-selected
        mgr.select(id_b);
        // tile A's burst_bytes is still 0 (was never incremented while selected)
        let tile_a = mgr.get(id_a).unwrap();
        assert_eq!(tile_a.burst_bytes, 0);

        // Even with old last_active, detection should not trigger (burst_bytes = 0)
        mgr.get_mut(id_a).unwrap().last_active = Instant::now() - Duration::from_secs(10);
        run_detection(&mut mgr, Some(id_b));
        assert!(!mgr.get(id_a).unwrap().has_unread);
    }

    #[test]
    fn test_gradual_accumulation_triggers() {
        let mut mgr = TileManager::new();
        let id = make_tile(&mut mgr);
        mgr.deselect();

        // Simulate many small outputs (like Claude Code streaming)
        let tile = mgr.get_mut(id).unwrap();
        for _ in 0..1000 {
            tile.process_output(b"token ");
            tile.burst_bytes += 6;
        }
        // Total: 6000 bytes (above UNREAD_BURST_THRESHOLD)
        assert_eq!(tile.burst_bytes, 6000);

        // Still recent — no trigger
        run_detection(&mut mgr, None);
        assert!(!mgr.get(id).unwrap().has_unread);

        // Simulate silence
        mgr.get_mut(id).unwrap().last_active = Instant::now() - Duration::from_secs(6);
        run_detection(&mut mgr, None);
        assert!(mgr.get(id).unwrap().has_unread);
    }
}
