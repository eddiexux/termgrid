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

    /// Restore last session
    #[arg(long)]
    restore: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let config = Config::load(&Config::config_path());
    let mut app = App::new(config);

    if cli.restore {
        if let Some(session) = Session::load(&Session::session_path()) {
            for tile_session in &session.tiles {
                if tile_session.cwd.exists() {
                    let _ = app.spawn_tile(&tile_session.cwd);
                }
            }
        }
    } else if let Some(path) = cli.path {
        if path.exists() {
            app.spawn_tile(&path)?;
        }
    }

    app.run().await?;

    // Save session on exit
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
