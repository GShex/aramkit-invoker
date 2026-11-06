mod client;
mod creds;
mod model;
mod watcher;

pub use creds::LcuCredentials;
pub use watcher::WatcherHandle;

/// Status snapshot sent from the background LCU watcher to the UI.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Status {
    pub connected: bool,
    pub phase: String,
    pub queue_id: i64,
    pub champion_id: i64,
    pub champion_name: String,
    pub message: String,
}

impl Status {
    pub fn disconnected(message: impl Into<String>) -> Self {
        Self {
            connected: false,
            message: message.into(),
            ..Self::default()
        }
    }
}

/// The ARAMKit page that was opened after a champion was finalized.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChampionPage {
    pub champion_id: i64,
    pub champion_name: String,
    pub url: String,
    pub error: Option<String>,
}

/// Events produced by the LCU watcher thread.
#[derive(Debug, Clone)]
pub enum LcuEvent {
    Status(Status),
    ChampionOpened(ChampionPage),
    Log(String),
}
