use crate::ids::PlayerId;
use crate::image::Image;
use crate::location::coordinates::{Coordinate, ItemId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlayerStanding {
    pub player_id: PlayerId,
    pub name: String,
    pub score: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RoundScore {
    pub player_id: PlayerId,
    pub score: u32,
    pub distance_meters: f64,
}

/// Outgoing broadcast events emitted by a room to connected WebSocket clients.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RoomEvent {
    PlayerJoined {
        player_id: PlayerId,
        name: String,
    },
    PlayerLeft {
        player_id: PlayerId,
    },
    PlayerReady {
        player_id: PlayerId,
    },
    RoundStarted {
        round: u8,
        total_rounds: u8,
        images: Vec<Image>,
        deadline_unix_ms: u64,
    },
    GuessAck {
        player_id: PlayerId,
        seq: u64,
    },
    RoundEnded {
        round: u8,
        scores: Vec<RoundScore>,
        actual_location: Coordinate,
        item_id: ItemId,
    },
    Leaderboard {
        standings: Vec<PlayerStanding>,
    },
    GameFinished {
        final_standings: Vec<PlayerStanding>,
    },
    Error {
        code: String,
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn room_event_serializes_to_expected_json() {
        let event = RoomEvent::PlayerJoined {
            player_id: PlayerId(42),
            name: "Alice".to_string(),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains(r#""type":"player_joined""#));
        assert!(json.contains(r#""player_id":42"#));
        assert!(json.contains(r#""name":"Alice""#));
    }
}
