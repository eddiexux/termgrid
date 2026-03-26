use clap::{CommandFactory, Parser};
use clap_complete::{generate, Shell};
use std::path::PathBuf;

use termgrid::app::App;
use termgrid::config::Config;
use termgrid::session::Session;

#[derive(Parser, Debug)]
#[command(
    name = "termgrid",
    about = "Multi-terminal manager with Git context awareness"
)]
struct Cli {
    /// Directory to scan for projects
    #[arg()]
    path: Option<PathBuf>,

    /// Start fresh (ignore saved session)
    #[arg(long)]
    fresh: bool,

    /// Generate shell completions for the given shell
    #[arg(long, value_name = "SHELL")]
    completions: Option<Shell>,
}

fn init_logging() -> PathBuf {
    use tracing_subscriber::EnvFilter;

    let log_dir = dirs::data_local_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
        .join("termgrid");
    std::fs::create_dir_all(&log_dir).ok();

    let file_appender = tracing_appender::rolling::daily(&log_dir, "termgrid.log");

    tracing_subscriber::fmt()
        .with_writer(file_appender)
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_ansi(false)
        .init();

    tracing::info!("termgrid v{} started", env!("CARGO_PKG_VERSION"));

    log_dir
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let log_dir = init_logging();
    let log_path = log_dir.join("termgrid.log");
    eprintln!("termgrid: logging to {}", log_path.display());

    let cli = Cli::parse();

    if let Some(shell) = cli.completions {
        let mut cmd = Cli::command();
        generate(shell, &mut cmd, "termgrid", &mut std::io::stdout());
        return Ok(());
    }

    let config = Config::load(&Config::config_path());
    let mut app = App::new(config);

    if let Some(path) = cli.path {
        if path.exists() {
            app.spawn_tile(&path)?;
        }
    } else if !cli.fresh {
        // Auto-restore last session with scrollback
        if let Some(session) = Session::load(&Session::session_path()) {
            for tile_session in &session.tiles {
                if tile_session.cwd.exists() {
                    if let Ok(id) = app.spawn_tile(&tile_session.cwd) {
                        // Restore scrollback into the tile's VTE
                        if let Some(idx) = tile_session.scrollback_index {
                            if let Some(scrollback_data) = Session::load_scrollback(idx) {
                                app.restore_tile_scrollback(id, &scrollback_data);
                            }
                        }
                    }
                }
            }
            app.set_columns(session.columns);
        }
    }

    app.run().await?;

    // Graceful shutdown: send Ctrl+C twice to Claude Code tiles so they exit
    // and print their session resume command. The output lands in output_history
    // and gets saved with scrollback, so it's visible on next restore.
    {
        let tiles = app.tile_manager_ref().tiles();
        let cc_tile_ids: Vec<_> = tiles
            .iter()
            .filter(|t| t.is_claude_code())
            .map(|t| t.id)
            .collect();

        if !cc_tile_ids.is_empty() {
            tracing::info!(
                "Sending Ctrl+C x2 to {} Claude Code tile(s)",
                cc_tile_ids.len()
            );
            // First Ctrl+C: cancel current task
            for &id in &cc_tile_ids {
                if let Some(tile) = app.tile_manager_ref().get(id) {
                    tile.pty.signal_interrupt();
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;

            // Second Ctrl+C: exit Claude Code
            for &id in &cc_tile_ids {
                if let Some(tile) = app.tile_manager_ref().get(id) {
                    tile.pty.signal_interrupt();
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(1000)).await;

            // Drain pending PTY output into tiles
            while let Ok(event) = app.try_recv_event() {
                if let termgrid::event::AppEvent::PtyOutput(tile_id, data) = event {
                    if let Some(tile) = app.tile_manager_mut().get_mut(tile_id) {
                        tile.process_output(&data);
                    }
                }
            }
        }
    }

    // Save session with scrollback
    Session::clean_scrollback();
    let tiles = app.tile_manager_ref().tiles();
    let tile_sessions: Vec<termgrid::session::TileSession> = tiles
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let scrollback_data = t.output_history();
            let scrollback_index = if !scrollback_data.is_empty() {
                let _ = Session::save_scrollback(i, &scrollback_data);
                Some(i)
            } else {
                None
            };

            termgrid::session::TileSession {
                cwd: t.cwd.clone(),
                scrollback_index,
            }
        })
        .collect();

    let session = Session {
        tiles: tile_sessions,
        columns: app.columns(),
        active_tab: "ALL".into(),
    };
    session.save(&Session::session_path())?;

    Ok(())
}

