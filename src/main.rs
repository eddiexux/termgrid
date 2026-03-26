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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
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
