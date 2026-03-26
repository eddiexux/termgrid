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
    let layout = termgrid::layout::calculate_layout(
        Rect::new(0, 0, 40, 10),
        1,
        2,
        false,
        45,
        0,
    );
    assert!(layout.tile_rects.len() <= 2);

    // Wide terminal with detail panel
    let layout = termgrid::layout::calculate_layout(
        Rect::new(0, 0, 200, 50),
        3,
        9,
        true,
        45,
        0,
    );
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
            },
            TileSession {
                cwd: PathBuf::from("/tmp/b"),
                scrollback_index: None,
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
