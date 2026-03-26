use crate::tile::TileId;
use crossterm::event::Event as CrosstermEvent;

#[derive(Debug)]
pub enum AppEvent {
    Crossterm(CrosstermEvent),
    PtyOutput(TileId, Vec<u8>),
    CwdChanged(TileId, std::path::PathBuf),
    Tick,
}
