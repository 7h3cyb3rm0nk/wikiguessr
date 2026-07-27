use crate::config::GameConfig;
use crate::ids::{PlayerId, RoomCode, RoomId};
use crate::image::Image;
use crate::location::coordinates::{Coordinate, Location};
use crate::rules::GameStatus;
use std::collections::{HashMap, HashSet};

/// State of an individual player in a room.
#[derive(Debug, Clone)]
pub struct PlayerState {
    pub id: PlayerId,
    pub name: String,
    pub token: String,
    pub cumulative_score: u32,
    pub is_host: bool,
}

/// State of an active round.
#[derive(Debug, Clone)]
pub struct RoundState {
    pub round_number: u8,
    pub location: Location,
    pub images: Vec<Image>,
    pub deadline_unix_ms: u64,
    /// Guesses recorded during this round (one per player, idempotent).
    pub guesses: HashMap<PlayerId, Coordinate>,
}

/// Inner state of a room, owned exclusively by its `RoomActor` task.
#[derive(Debug, Clone)]
pub struct RoomState {
    pub id: RoomId,
    pub code: RoomCode,
    pub join_code: String,
    pub host_token: String,
    pub status: GameStatus,
    pub players: HashMap<PlayerId, PlayerState>,
    pub ready_players: HashSet<PlayerId>,
    pub current_round: Option<RoundState>,
    pub config: GameConfig,
}

impl RoomState {
    pub fn new(id: RoomId, join_code: String, host_token: String, config: GameConfig) -> Self {
        let code = id.to_room_code();
        Self {
            id,
            code,
            join_code,
            host_token,
            status: GameStatus::Lobby,
            players: HashMap::new(),
            ready_players: HashSet::new(),
            current_round: None,
            config,
        }
    }

    /// Add a player to the room state.
    pub fn add_player(&mut self, id: PlayerId, name: String, token: String, is_host: bool) {
        self.players.insert(
            id,
            PlayerState {
                id,
                name,
                token,
                cumulative_score: 0,
                is_host,
            },
        );
    }

    /// Remove a player from the room state.
    pub fn remove_player(&mut self, id: &PlayerId) {
        self.players.remove(id);
        self.ready_players.remove(id);
    }

    /// Check if all currently connected players have marked themselves ready.
    pub fn all_players_ready(&self) -> bool {
        !self.players.is_empty() && self.players.keys().all(|id| self.ready_players.contains(id))
    }

    /// Record a player's guess for the active round.
    /// Returns `Ok(())` on success, or `Err("already_guessed")` if duplicate.
    pub fn record_guess(&mut self, player_id: PlayerId, coord: Coordinate) -> Result<(), &'static str> {
        let round = match self.current_round.as_mut() {
            Some(r) => r,
            None => return Err("no_active_round"),
        };

        if round.guesses.contains_key(&player_id) {
            return Err("already_guessed");
        }

        round.guesses.insert(player_id, coord);
        Ok(())
    }

    /// Check if all connected players have submitted guesses for the active round.
    pub fn all_players_guessed(&self) -> bool {
        match &self.current_round {
            Some(round) => {
                !self.players.is_empty()
                    && self.players.keys().all(|id| round.guesses.contains_key(id))
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn room_state_initialization() {
        let id = RoomId::new();
        let state = RoomState::new(id, "SECRET".to_string(), "HOST".to_string(), GameConfig::default());
        assert_eq!(state.status, GameStatus::Lobby);
        assert!(state.players.is_empty());
    }

    #[test]
    fn all_players_ready_check() {
        let id = RoomId::new();
        let mut state = RoomState::new(id, "SECRET".to_string(), "HOST".to_string(), GameConfig::default());

        let p1 = PlayerId::new();
        let p2 = PlayerId::new();
        state.add_player(p1, "Alice".to_string(), "tok1".to_string(), true);
        state.add_player(p2, "Bob".to_string(), "tok2".to_string(), false);

        assert!(!state.all_players_ready());

        state.ready_players.insert(p1);
        assert!(!state.all_players_ready());

        state.ready_players.insert(p2);
        assert!(state.all_players_ready());
    }

    #[test]
    fn record_guess_idempotency() {
        let id = RoomId::new();
        let mut state = RoomState::new(id, "SECRET".to_string(), "HOST".to_string(), GameConfig::default());
        let p1 = PlayerId::new();
        state.add_player(p1, "Alice".to_string(), "tok1".to_string(), true);

        let loc = Location {
            item_id: crate::location::coordinates::ItemId("Q1".to_string()),
            coordinate: Coordinate { latitude: 0.0, longitude: 0.0 },
        };

        state.current_round = Some(RoundState {
            round_number: 1,
            location: loc,
            images: vec![],
            deadline_unix_ms: 0,
            guesses: HashMap::new(),
        });

        let guess = Coordinate { latitude: 10.0, longitude: 10.0 };
        assert!(state.record_guess(p1, guess).is_ok());
        // Duplicate guess must be rejected
        assert_eq!(state.record_guess(p1, guess), Err("already_guessed"));
    }
}
