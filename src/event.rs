use crate::tile::TileId;
use crossterm::event::Event as CrosstermEvent;

#[derive(Debug)]
pub enum AppEvent {
    Crossterm(CrosstermEvent),
    PtyOutput(TileId, Vec<u8>),
    PtyExited(TileId),
    CwdChanged(TileId, std::path::PathBuf),
    Tick,
}
