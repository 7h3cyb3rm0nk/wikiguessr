/// Room-wide game phase. Round numbers are 1-indexed to match what's
/// shown to players; `total_rounds` comes from RoomConfig, not hardcoded,
/// so game length is tunable without touching this state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameStatus {
    Lobby,
    InRound(RoundNumber),
    Finished,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct RoundNumber(pub u8);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoomConfig {
    pub total_rounds: u8,
}

impl Default for RoomConfig {
    fn default() -> Self {
        Self { total_rounds: 5 }
    }
}

/// Events that can drive a state transition. Deliberately narrow —
/// this is not the full RoomCommand enum (that's network-facing and
/// lives in room/command.rs); this is just what the pure rules engine
/// needs to know happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoundEvent {
    AllPlayersReady,
    RoundEnded, // fired by the actor on timer expiry OR all-players-guessed
}

/// Pure state transition. No side effects, no I/O — the actor calls
/// this and then separately handles the consequences (start timer,
/// fetch next location, broadcast event) based on the returned status.
pub fn transition(current: GameStatus, event: RoundEvent, config: RoomConfig) -> GameStatus {
    match (current, event) {
        (GameStatus::Lobby, RoundEvent::AllPlayersReady) => GameStatus::InRound(RoundNumber(1)),

        (GameStatus::InRound(RoundNumber(n)), RoundEvent::RoundEnded) => {
            if n >= config.total_rounds {
                GameStatus::Finished
            } else {
                GameStatus::InRound(RoundNumber(n + 1))
            }
        }

        // Any other (state, event) pair is a no-op: e.g. AllPlayersReady
        // fired again mid-round, or RoundEnded fired while still in Lobby.
        // The actor should treat unexpected events as ignorable rather
        // than panicking — untrusted client commands can arrive out of
        // order relative to actor-internal timer events.
        (state, _) => state,
    }
}

pub fn is_terminal(status: GameStatus) -> bool {
    matches!(status, GameStatus::Finished)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lobby_to_round_one_on_ready() {
        let config = RoomConfig::default();
        let next = transition(GameStatus::Lobby, RoundEvent::AllPlayersReady, config);
        assert_eq!(next, GameStatus::InRound(RoundNumber(1)));
    }

    #[test]
    fn advances_through_all_rounds_then_finishes() {
        let config = RoomConfig { total_rounds: 3 };
        let mut status = GameStatus::Lobby;
        status = transition(status, RoundEvent::AllPlayersReady, config);
        assert_eq!(status, GameStatus::InRound(RoundNumber(1)));

        status = transition(status, RoundEvent::RoundEnded, config);
        assert_eq!(status, GameStatus::InRound(RoundNumber(2)));

        status = transition(status, RoundEvent::RoundEnded, config);
        assert_eq!(status, GameStatus::InRound(RoundNumber(3)));

        status = transition(status, RoundEvent::RoundEnded, config);
        assert_eq!(status, GameStatus::Finished);
        assert!(is_terminal(status));
    }

    #[test]
    fn ready_event_ignored_once_already_in_round() {
        let config = RoomConfig::default();
        let status = GameStatus::InRound(RoundNumber(2));
        let next = transition(status, RoundEvent::AllPlayersReady, config);
        assert_eq!(next, status, "unexpected event should be a no-op, not a panic");
    }

    #[test]
    fn round_ended_ignored_while_in_lobby() {
        let config = RoomConfig::default();
        let next = transition(GameStatus::Lobby, RoundEvent::RoundEnded, config);
        assert_eq!(next, GameStatus::Lobby);
    }

    #[test]
    fn finished_is_terminal_and_absorbs_further_events() {
        let config = RoomConfig::default();
        let status = GameStatus::Finished;
        assert_eq!(transition(status, RoundEvent::RoundEnded, config), status);
        assert_eq!(transition(status, RoundEvent::AllPlayersReady, config), status);
    }

    #[test]
    fn single_round_game_finishes_immediately_after_round_one() {
        let config = RoomConfig { total_rounds: 1 };
        let mut status = transition(GameStatus::Lobby, RoundEvent::AllPlayersReady, config);
        assert_eq!(status, GameStatus::InRound(RoundNumber(1)));
        status = transition(status, RoundEvent::RoundEnded, config);
        assert_eq!(status, GameStatus::Finished);
    }
}
