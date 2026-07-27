use crate::ids::PlayerId;
use crate::location::coordinates::Coordinate;
use crate::rooms::events::RoomEvent;
use tokio::sync::{mpsc, oneshot};

/// Messages sent into a RoomActor's mailbox.
#[derive(Debug)]
pub enum RoomCommand {
    Join {
        player_id: PlayerId,
        name: String,
        /// Optional channel to receive acknowledgment (used by REST handlers).
        response_tx: Option<oneshot::Sender<Result<(), String>>>,
    },
    Leave {
        player_id: PlayerId,
    },
    Ready {
        player_id: PlayerId,
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
    /// Internal timer tick event.
    Tick,
}
