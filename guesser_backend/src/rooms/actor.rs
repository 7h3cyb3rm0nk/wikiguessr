use crate::config::GameConfig;
use crate::ids::{PlayerId, RoomCode, RoomId};
use crate::location::coordinates::Coordinate;
use crate::resources::manager::ResourceManager;
use crate::rooms::commands::RoomCommand;
use crate::rooms::events::{PlayerStanding, RoomEvent, RoundScore};
use crate::rooms::state::{PlayerState, RoomState, RoundState};
use crate::rules::{transition, GameStatus, RoomConfig, RoundEvent as PureRoundEvent, RoundNumber};
use crate::scoring::{score_guess, haversine_distance_meters, ScoringConfig};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;
use tokio::time::{sleep, Duration, Instant};
use tracing::{debug, info, warn};

/// Actor that owns all mutable state and game loop logic for a single room.
pub struct RoomActor {
    pub state: RoomState,
    pub resources: Arc<ResourceManager>,
    pub mailbox: mpsc::Receiver<RoomCommand>,
    /// Sinks for broadcasting events to connected clients.
    pub broadcast_sinks: HashMap<PlayerId, mpsc::UnboundedSender<RoomEvent>>,
}

impl RoomActor {
    pub fn new(
        id: RoomId,
        join_code: String,
        host_token: String,
        config: GameConfig,
        resources: Arc<ResourceManager>,
        mailbox: mpsc::Receiver<RoomCommand>,
    ) -> Self {
        Self {
            state: RoomState::new(id, join_code, host_token, config),
            resources,
            mailbox,
            broadcast_sinks: HashMap::new(),
        }
    }

    /// Broadcast a `RoomEvent` to all connected player channels.
    pub fn broadcast(&mut self, event: RoomEvent) {
        self.broadcast_sinks.retain(|player_id, tx| {
            if let Err(_) = tx.send(event.clone()) {
                debug!(%player_id, "dropping disconnected player broadcast sink");
                false
            } else {
                true
            }
        });
    }

    /// Send an event to a specific player's broadcast channel.
    pub fn send_to(&mut self, player_id: &PlayerId, event: RoomEvent) {
        if let Some(tx) = self.broadcast_sinks.get(player_id) {
            let _ = tx.send(event);
        }
    }

    /// Register a player's broadcast channel.
    pub fn register_sink(&mut self, player_id: PlayerId, tx: mpsc::UnboundedSender<RoomEvent>) {
        self.broadcast_sinks.insert(player_id, tx);
    }

    /// Main actor loop. Runs inside its own Tokio task.
    pub async fn run(mut self) {
        info!(room_id = %self.state.id, room_code = %self.state.code, "started room actor");

        let inactivity_duration = Duration::from_secs(self.state.config.inactivity_timeout_secs);
        let mut inactivity_timer = tokio::time::interval(inactivity_duration);
        inactivity_timer.reset();

        loop {
            tokio::select! {
                cmd_option = self.mailbox.recv() => {
                    match cmd_option {
                        Some(cmd) => {
                            inactivity_timer.reset();
                            let should_continue = self.handle_command(cmd).await;
                            if !should_continue {
                                break;
                            }
                        }
                        None => {
                            debug!(room_id = %self.state.id, "all mailbox senders dropped, terminating room actor");
                            break;
                        }
                    }
                }
                _ = inactivity_timer.tick() => {
                    info!(room_id = %self.state.id, "room inactivity timeout reached, destroying room actor");
                    break;
                }
            }
        }

        info!(room_id = %self.state.id, "room actor shut down");
    }

    /// Process a single incoming `RoomCommand`.
    /// Returns `true` to continue loop, `false` to terminate actor.
    async fn handle_command(&mut self, cmd: RoomCommand) -> bool {
        match cmd {
            RoomCommand::Join {
                player_id,
                name,
                response_tx,
            } => {
                let token = format!("p_tok_{player_id}");
                let is_host = self.state.players.is_empty();
                self.state.add_player(player_id, name.clone(), token, is_host);

                self.broadcast(RoomEvent::PlayerJoined {
                    player_id,
                    name: name.clone(),
                });

                if let Some(tx) = response_tx {
                    let _ = tx.send(Ok(()));
                }
            }
            RoomCommand::Leave { player_id } => {
                self.state.remove_player(&player_id);
                self.broadcast_sinks.remove(&player_id);
                self.broadcast(RoomEvent::PlayerLeft { player_id });

                if self.state.players.is_empty() {
                    info!(room_id = %self.state.id, "last player left, destroying room");
                    return false;
                }
            }
            RoomCommand::Ready { player_id } => {
                self.state.ready_players.insert(player_id);
                self.broadcast(RoomEvent::PlayerReady { player_id });

                if self.state.status == GameStatus::Lobby && self.state.all_players_ready() {
                    self.start_next_round().await;
                }
            }
            RoomCommand::Guess {
                player_id,
                seq,
                coordinate,
                response_tx,
            } => {
                match self.state.record_guess(player_id, coordinate) {
                    Ok(()) => {
                        self.broadcast(RoomEvent::GuessAck { player_id, seq });

                        // Compute immediate score for REST response if requested
                        let score = if let Some(ref round) = self.state.current_round {
                            score_guess(
                                coordinate,
                                round.location.coordinate,
                                ScoringConfig {
                                    max_score: self.state.config.max_score,
                                    decay_m: self.state.config.scoring_decay_m,
                                },
                            )
                        } else {
                            0
                        };

                        if let Some(tx) = response_tx {
                            let _ = tx.send(Ok(score));
                        }

                        // If all players submitted guesses, advance round immediately!
                        if self.state.all_players_guessed() {
                            self.end_round().await;
                        }
                    }
                    Err(err_msg) => {
                        if let Some(tx) = response_tx {
                            let _ = tx.send(Err(err_msg.to_string()));
                        }
                        self.send_to(
                            &player_id,
                            RoomEvent::Error {
                                code: "guess_error".to_string(),
                                message: err_msg.to_string(),
                            },
                        );
                    }
                }
            }
            RoomCommand::Ping { .. } => {
                // Keep-alive command, resets inactivity timer
            }
            RoomCommand::RegisterSink { player_id, sink_tx } => {
                self.register_sink(player_id, sink_tx);
            }
            RoomCommand::Tick => {
                // Internal timer tick to check round timeout
                if let Some(ref round) = self.state.current_round {
                    let now_ms = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as u64;

                    if now_ms >= round.deadline_unix_ms {
                        self.end_round().await;
                    }
                }
            }
        }
        true
    }

    /// Advance the state machine to start the next round.
    async fn start_next_round(&mut self) {
        let rule_config = RoomConfig {
            total_rounds: self.state.config.total_rounds,
        };
        self.state.status = transition(self.state.status, PureRoundEvent::AllPlayersReady, rule_config);

        let round_number = match self.state.status {
            GameStatus::InRound(RoundNumber(n)) => n,
            _ => return,
        };

        // Fetch location from pool or fallback
        let location = match self.resources.next_location() {
            Some(loc) => loc,
            None => {
                warn!(room_id = %self.state.id, "location pool empty when starting round");
                return;
            }
        };

        // Fetch images for this location
        let images = self
            .resources
            .images_for(&location.item_id, location.coordinate)
            .await;

        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let duration_ms = self.state.config.round_duration_secs * 1000;
        let deadline_unix_ms = now_ms + duration_ms;

        self.state.current_round = Some(RoundState {
            round_number,
            location: location.clone(),
            images: images.clone(),
            deadline_unix_ms,
            guesses: HashMap::new(),
        });

        self.broadcast(RoomEvent::RoundStarted {
            round: round_number,
            total_rounds: self.state.config.total_rounds,
            images,
            deadline_unix_ms,
        });

        info!(
            room_id = %self.state.id,
            round = round_number,
            item_id = %location.item_id.0,
            "started round"
        );
    }

    /// End the current round, score guesses, and broadcast results.
    async fn end_round(&mut self) {
        let round = match self.state.current_round.take() {
            Some(r) => r,
            None => return,
        };

        let scoring_cfg = ScoringConfig {
            max_score: self.state.config.max_score,
            decay_m: self.state.config.scoring_decay_m,
        };

        let actual = round.location.coordinate;
        let mut round_scores = Vec::new();

        for (player_id, player_state) in &mut self.state.players {
            let guess_coord = round.guesses.get(player_id);
            let (score, distance_meters) = match guess_coord {
                Some(&g) => (
                    score_guess(g, actual, scoring_cfg),
                    haversine_distance_meters(g, actual),
                ),
                None => (0, 20_000_000.0), // No guess submitted
            };

            player_state.cumulative_score += score;
            round_scores.push(RoundScore {
                player_id: *player_id,
                score,
                distance_meters,
            });
        }

        self.broadcast(RoomEvent::RoundEnded {
            round: round.round_number,
            scores: round_scores,
            actual_location: actual,
            item_id: round.location.item_id,
        });

        // Compute standings
        let mut standings: Vec<PlayerStanding> = self
            .state
            .players
            .values()
            .map(|p| PlayerStanding {
                player_id: p.id,
                name: p.name.clone(),
                score: p.cumulative_score,
            })
            .collect();
        standings.sort_by(|a, b| b.score.cmp(&a.score));

        self.broadcast(RoomEvent::Leaderboard {
            standings: standings.clone(),
        });

        // Transition pure rules engine
        let rule_config = RoomConfig {
            total_rounds: self.state.config.total_rounds,
        };
        self.state.status = transition(self.state.status, PureRoundEvent::RoundEnded, rule_config);

        if self.state.status == GameStatus::Finished {
            self.broadcast(RoomEvent::GameFinished {
                final_standings: standings,
            });
            info!(room_id = %self.state.id, "game finished");
        } else {
            // Automatically advance to next round
            self.start_next_round().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::location::coordinates::{ItemId, Location};
    use tokio::sync::oneshot;

    fn setup_actor_with_location() -> (RoomActor, mpsc::Sender<RoomCommand>, mpsc::UnboundedReceiver<RoomEvent>, PlayerId) {
        let config = GameConfig::default();
        let resources = Arc::new(ResourceManager::new(&config));
        resources.pool.push_batch(vec![
            Location {
                item_id: ItemId("Q42".into()),
                coordinate: Coordinate { latitude: 51.5074, longitude: -0.1278 },
            },
            Location {
                item_id: ItemId("Q100".into()),
                coordinate: Coordinate { latitude: 48.8566, longitude: 2.3522 },
            },
        ]);

        let (tx, rx) = mpsc::channel(10);
        let mut actor = RoomActor::new(
            RoomId::new(),
            "1234".into(),
            "host_tok".into(),
            config,
            resources,
            rx,
        );

        let p1 = PlayerId::new();
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        actor.register_sink(p1, event_tx);

        (actor, tx, event_rx, p1)
    }

    #[tokio::test]
    async fn actor_join_and_leave_lifecycle() {
        let (mut actor, _tx, mut event_rx, p1) = setup_actor_with_location();

        // Join p1
        let (join_tx, join_rx) = oneshot::channel();
        actor.handle_command(RoomCommand::Join {
            player_id: p1,
            name: "Alice".into(),
            response_tx: Some(join_tx),
        }).await;

        assert!(join_rx.await.unwrap().is_ok());
        let event = event_rx.recv().await.unwrap();
        assert_eq!(event, RoomEvent::PlayerJoined { player_id: p1, name: "Alice".into() });

        // Leave p1 -> actor should report false (terminate)
        let continue_loop = actor.handle_command(RoomCommand::Leave { player_id: p1 }).await;
        assert!(!continue_loop, "actor should terminate when last player leaves");
    }

    #[tokio::test]
    async fn actor_ready_starts_round_and_broadcasts() {
        let (mut actor, _tx, mut event_rx, p1) = setup_actor_with_location();

        actor.handle_command(RoomCommand::Join { player_id: p1, name: "Alice".into(), response_tx: None }).await;
        let _ = event_rx.recv().await; // Drain PlayerJoined

        actor.handle_command(RoomCommand::Ready { player_id: p1 }).await;

        let ready_event = event_rx.recv().await.unwrap();
        assert_eq!(ready_event, RoomEvent::PlayerReady { player_id: p1 });

        let round_event = event_rx.recv().await.unwrap();
        match round_event {
            RoomEvent::RoundStarted { round, total_rounds, .. } => {
                assert_eq!(round, 1);
                assert_eq!(total_rounds, 5);
            }
            other => panic!("expected RoundStarted, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn actor_guess_and_scoring() {
        let (mut actor, _tx, mut event_rx, p1) = setup_actor_with_location();

        actor.handle_command(RoomCommand::Join { player_id: p1, name: "Alice".into(), response_tx: None }).await;
        actor.handle_command(RoomCommand::Ready { player_id: p1 }).await;
        let _ = event_rx.recv().await; // Joined
        let _ = event_rx.recv().await; // Ready
        let _ = event_rx.recv().await; // RoundStarted

        // Submit guess for exact location (51.5074, -0.1278) -> should score 5000 max score
        let (guess_tx, guess_rx) = oneshot::channel();
        actor.handle_command(RoomCommand::Guess {
            player_id: p1,
            seq: 1,
            coordinate: Coordinate { latitude: 51.5074, longitude: -0.1278 },
            response_tx: Some(guess_tx),
        }).await;

        let score = guess_rx.await.unwrap().unwrap();
        assert_eq!(score, 5000);

        let ack = event_rx.recv().await.unwrap();
        assert_eq!(ack, RoomEvent::GuessAck { player_id: p1, seq: 1 });

        let round_ended = event_rx.recv().await.unwrap();
        assert!(matches!(round_ended, RoomEvent::RoundEnded { .. }));
    }
}
