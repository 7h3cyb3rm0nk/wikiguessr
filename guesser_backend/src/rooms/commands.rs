use crate::ids::PlayerId;
use crate::image::Image;
use crate::location::coordinates::{Coordinate, ItemId};
use crate::rooms::events::{PlayerStanding, RoomEvent};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoundStateSnapshot {
    pub round_number: u8,
    pub total_rounds: u8,
    pub images: Vec<Image>,
    pub deadline_unix_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoundResultSnapshot {
    pub round: u8,
    pub score: u32,
    pub distance_meters: f64,
    pub actual_location: Coordinate,
    pub item_id: ItemId,
    pub leaderboard: Vec<PlayerStanding>,
    pub game_finished: bool,
    pub final_standings: Option<Vec<PlayerStanding>>,
}

/// Messages sent into a RoomActor's mailbox.
#[derive(Debug)]
pub enum RoomCommand {
    Join {
        player_id: PlayerId,
        name: String,
        /// Optional join code for multiplayer rooms (validated against the room's join_code).
        join_code: Option<String>,
        /// Optional channel to receive acknowledgment (used by REST handlers).
        response_tx: Option<oneshot::Sender<Result<(), String>>>,
    },
    Leave {
        player_id: PlayerId,
    },
    Ready {
        player_id: PlayerId,
        /// Optional channel to receive the round state when round starts (used by REST solo flow).
        response_tx: Option<oneshot::Sender<Result<RoundStateSnapshot, String>>>,
    },
    Guess {
        player_id: PlayerId,
        seq: u64,
        coordinate: Coordinate,
        /// Optional channel to receive immediate guess ack/score (used by REST solo flow).
        response_tx: Option<oneshot::Sender<Result<u32, String>>>,
    },
    Ping {
        player_id: PlayerId,
    },
    RegisterSink {
        player_id: PlayerId,
        sink_tx: mpsc::UnboundedSender<RoomEvent>,
    },
    /// Fetch the current round state (images, deadline, round number).
    /// Used by solo REST players who don't have a WebSocket.
    GetRoundState {
        response_tx: oneshot::Sender<Option<RoundStateSnapshot>>,
    },
    /// Fetch the round result (actual location, score, leaderboard) for solo players.
    GetRoundResult {
        player_id: PlayerId,
        response_tx: oneshot::Sender<Option<RoundResultSnapshot>>,
    },
    /// Host force-starts the game, readying all current players.
    HostStart {
        host_token: String,
        /// Optional channel to receive the round state when round starts (used by REST host flow).
        response_tx: Option<oneshot::Sender<Result<RoundStateSnapshot, String>>>,
    },
}
