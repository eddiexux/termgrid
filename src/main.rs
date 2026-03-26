use clap::Parser;
use std::path::PathBuf;

use termgrid::app::App;
use termgrid::config::Config;
use termgrid::session::Session;

#[derive(Parser, Debug)]
#[command(name = "termgrid", about = "Multi-terminal manager with Git context awareness")]
struct Cli {
    /// Directory to scan for projects
    #[arg()]
    path: Option<PathBuf>,

    /// Start fresh (ignore saved session)
    #[arg(long)]
    fresh: bool,
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
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))
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
    let config = Config::load(&Config::config_path());
    let mut app = App::new(config);

    if let Some(path) = cli.path {
        // Explicit path: open tile there
        if path.exists() {
            app.spawn_tile(&path)?;
        }
    } else if !cli.fresh {
        // No path, no --fresh: auto-restore last session
        if let Some(session) = Session::load(&Session::session_path()) {
            for tile_session in &session.tiles {
                if tile_session.cwd.exists() {
                    let _ = app.spawn_tile(&tile_session.cwd);
                }
            }
            app.set_columns(session.columns);
        }
    }
    // --fresh or no session: empty dashboard, press 'n' to create

    app.run().await?;

    // Save session on exit (capture tile CWDs before they're dropped)
    let session = Session {
        tiles: app
            .tile_manager_ref()
            .tiles()
            .iter()
            .map(|t| termgrid::session::TileSession { cwd: t.cwd.clone() })
            .collect(),
        columns: app.columns(),
        active_tab: "ALL".into(),
    };
    session.save(&Session::session_path())?;

    Ok(())
}
