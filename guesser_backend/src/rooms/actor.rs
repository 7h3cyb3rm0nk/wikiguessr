use crate::config::GameConfig;
use crate::ids::{PlayerId, RoomId};
use crate::resources::manager::ResourceManager;
use crate::rooms::commands::RoomCommand;
use crate::rooms::events::{PlayerStanding, RoomEvent, RoundScore};
use crate::rooms::state::{RoomState, RoundState};
use crate::rules::{GameStatus, RoomConfig, RoundEvent as PureRoundEvent, RoundNumber, transition};
use crate::scoring::{ScoringConfig, haversine_distance_meters, score_guess};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;
use tokio::time::Duration;
use tracing::{debug, info};

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
            if tx.send(event.clone()).is_err() {
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

        // Lobby timeout: destroys a lobby that never starts a game.
        let lobby_duration = Duration::from_secs(self.state.config.lobby_timeout_secs);
        let mut lobby_timer = tokio::time::interval(lobby_duration);
        lobby_timer.reset();

        // 1-second ticker for round deadline checks; no Tick command needed.
        let mut round_timer = tokio::time::interval(Duration::from_secs(1));
        round_timer.reset();

        loop {
            tokio::select! {
                cmd_option = self.mailbox.recv() => {
                    match cmd_option {
                        Some(cmd) => {
                            inactivity_timer.reset();
                            // While in the lobby, any command (join/ready/leave) resets the lobby timer.
                            if self.state.status == GameStatus::Lobby {
                                lobby_timer.reset();
                            }
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
                _ = round_timer.tick() => {
                    // Round deadline check — fires every second while a round is active.
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
                _ = lobby_timer.tick() => {
                    if self.state.status == GameStatus::Lobby {
                        info!(room_id = %self.state.id, "lobby timeout reached, destroying room actor");
                        break;
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
                join_code,
                response_tx,
            } => {
                // Validate the join code for multiplayer rooms (solo passes None).
                if let Some(provided) = join_code
                    && !provided.eq_ignore_ascii_case(&self.state.join_code)
                {
                    if let Some(tx) = response_tx {
                        let _ = tx.send(Err("invalid_join_code".to_string()));
                    }
                    self.send_to(
                        &player_id,
                        RoomEvent::Error {
                            code: "join_error".to_string(),
                            message: "invalid_join_code".to_string(),
                        },
                    );
                    return true;
                }

                let token = format!("p_tok_{player_id}");
                let is_host = self.state.players.is_empty();
                self.state
                    .add_player(player_id, name.clone(), token, is_host);

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
            RoomCommand::Ready {
                player_id,
                response_tx,
            } => {
                self.state.ready_players.insert(player_id);
                self.broadcast(RoomEvent::PlayerReady { player_id });

                if self.state.status == GameStatus::Lobby && self.state.all_players_ready() {
                    self.start_next_round().await;

                    // If there's a response channel (solo REST), send the round state
                    if let Some(tx) = response_tx {
                        if let Some(ref round) = self.state.current_round {
                            use crate::rooms::commands::RoundStateSnapshot;
                            let snapshot = RoundStateSnapshot {
                                round_number: round.round_number,
                                total_rounds: self.state.config.total_rounds,
                                images: round.images.clone(),
                                deadline_unix_ms: round.deadline_unix_ms,
                            };
                            let _ = tx.send(Ok(snapshot));
                        } else {
                            let _ = tx.send(Err("round not started".to_string()));
                        }
                    }
                }
            }
            RoomCommand::HostStart {
                host_token,
                response_tx,
            } => {
                // Validate the host token before allowing a force-start.
                if !host_token.eq_ignore_ascii_case(&self.state.host_token) {
                    if let Some(tx) = response_tx {
                        let _ = tx.send(Err("invalid_host_token".to_string()));
                    }
                    return true;
                }

                if self.state.status != GameStatus::Lobby {
                    if let Some(tx) = response_tx {
                        let _ = tx.send(Err("game_already_started".to_string()));
                    }
                    return true;
                }

                // Ready every current player, then start the round.
                let player_ids: Vec<PlayerId> = self.state.players.keys().copied().collect();
                for pid in &player_ids {
                    self.state.ready_players.insert(*pid);
                    self.broadcast(RoomEvent::PlayerReady { player_id: *pid });
                }

                self.start_next_round().await;

                if let Some(tx) = response_tx {
                    if let Some(ref round) = self.state.current_round {
                        use crate::rooms::commands::RoundStateSnapshot;
                        let snapshot = RoundStateSnapshot {
                            round_number: round.round_number,
                            total_rounds: self.state.config.total_rounds,
                            images: round.images.clone(),
                            deadline_unix_ms: round.deadline_unix_ms,
                        };
                        let _ = tx.send(Ok(snapshot));
                    } else {
                        let _ = tx.send(Err("round not started".to_string()));
                    }
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
            RoomCommand::GetRoundState { response_tx } => {
                let snapshot = self.state.current_round.as_ref().map(|r| {
                    use crate::rooms::commands::RoundStateSnapshot;
                    RoundStateSnapshot {
                        round_number: r.round_number,
                        total_rounds: self.state.config.total_rounds,
                        images: r.images.clone(),
                        deadline_unix_ms: r.deadline_unix_ms,
                    }
                });
                let _ = response_tx.send(snapshot);
            }
            RoomCommand::GetRoundResult {
                player_id,
                response_tx,
            } => {
                let snapshot = self.state.last_round_result.as_ref().and_then(|rr| {
                    // Find the score for this specific player
                    let player_score = rr.scores.iter().find(|s| s.player_id == player_id);
                    player_score.map(|s| {
                        use crate::rooms::commands::RoundResultSnapshot;
                        RoundResultSnapshot {
                            round: rr.round,
                            score: s.score,
                            distance_meters: s.distance_meters,
                            actual_location: rr.actual_location,
                            item_id: rr.item_id.clone(),
                            leaderboard: rr.leaderboard.clone(),
                            game_finished: rr.game_finished,
                            final_standings: rr.final_standings.clone(),
                        }
                    })
                });
                let _ = response_tx.send(snapshot);
            }
        }
        true
    }

    /// Advance the state machine to start the next round.
    async fn start_next_round(&mut self) {
        let rule_config = RoomConfig {
            total_rounds: self.state.config.total_rounds,
        };

        // If we're still in Lobby, this is the first round — transition via AllPlayersReady.
        // If we're in InRound(prev), end_round() already advanced us — no transition needed.
        if self.state.status == GameStatus::Lobby {
            self.state.status = transition(
                self.state.status,
                PureRoundEvent::AllPlayersReady,
                rule_config,
            );
        }

        let round_number = match self.state.status {
            GameStatus::InRound(RoundNumber(n)) => n,
            _ => return,
        };

        // Fetch prepared round from pool (now async and waits up to 10s)
        let prepared = self.resources.next_location().await;
        let location = prepared.location;
        let images = prepared.images;

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
            scores: round_scores.clone(),
            actual_location: actual,
            item_id: round.location.item_id.clone(),
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
        standings.sort_by_key(|p| std::cmp::Reverse(p.score));

        self.broadcast(RoomEvent::Leaderboard {
            standings: standings.clone(),
        });

        // Transition pure rules engine
        let rule_config = RoomConfig {
            total_rounds: self.state.config.total_rounds,
        };
        self.state.status = transition(self.state.status, PureRoundEvent::RoundEnded, rule_config);

        // Store round result for solo REST players
        let finished = self.state.status == GameStatus::Finished;
        self.state.last_round_result = Some(crate::rooms::state::RoundResult {
            round: round.round_number,
            scores: round_scores,
            actual_location: actual,
            item_id: round.location.item_id,
            leaderboard: standings.clone(),
            game_finished: finished,
            final_standings: if finished {
                Some(standings.clone())
            } else {
                None
            },
        });

        if finished {
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
    use crate::location::coordinates::{Coordinate, ItemId, Location};
    use crate::resources::pool::PreparedRound;
    use tokio::sync::oneshot;

    fn setup_actor_with_location() -> (
        RoomActor,
        mpsc::Sender<RoomCommand>,
        mpsc::UnboundedReceiver<RoomEvent>,
        PlayerId,
    ) {
        let config = GameConfig::default();
        let resources = Arc::new(ResourceManager::new(&config));
        resources.pool.push_batch(vec![
            PreparedRound {
                location: Location {
                    item_id: ItemId("Q42".into()),
                    coordinate: Coordinate {
                        latitude: 51.5074,
                        longitude: -0.1278,
                    },
                },
                images: vec![crate::image::Image {
                    url: "test".into(),
                    thumb_url: "test".into(),
                    width: 100,
                    height: 100,
                }],
            },
            PreparedRound {
                location: Location {
                    item_id: ItemId("Q100".into()),
                    coordinate: Coordinate {
                        latitude: 48.8566,
                        longitude: 2.3522,
                    },
                },
                images: vec![crate::image::Image {
                    url: "test".into(),
                    thumb_url: "test".into(),
                    width: 100,
                    height: 100,
                }],
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
        actor
            .handle_command(RoomCommand::Join {
                player_id: p1,
                name: "Alice".into(),
                join_code: None,
                response_tx: Some(join_tx),
            })
            .await;

        assert!(join_rx.await.unwrap().is_ok());
        let event = event_rx.recv().await.unwrap();
        assert_eq!(
            event,
            RoomEvent::PlayerJoined {
                player_id: p1,
                name: "Alice".into()
            }
        );

        // Leave p1 -> actor should report false (terminate)
        let continue_loop = actor
            .handle_command(RoomCommand::Leave { player_id: p1 })
            .await;
        assert!(
            !continue_loop,
            "actor should terminate when last player leaves"
        );
    }

    #[tokio::test]
    async fn actor_ready_starts_round_and_broadcasts() {
        let (mut actor, _tx, mut event_rx, p1) = setup_actor_with_location();

        actor
            .handle_command(RoomCommand::Join {
                player_id: p1,
                name: "Alice".into(),
                join_code: None,
                response_tx: None,
            })
            .await;
        let _ = event_rx.recv().await; // Drain PlayerJoined

        actor
            .handle_command(RoomCommand::Ready {
                player_id: p1,
                response_tx: None,
            })
            .await;

        let ready_event = event_rx.recv().await.unwrap();
        assert_eq!(ready_event, RoomEvent::PlayerReady { player_id: p1 });

        let round_event = event_rx.recv().await.unwrap();
        match round_event {
            RoomEvent::RoundStarted {
                round,
                total_rounds,
                images,
                ..
            } => {
                assert_eq!(round, 1);
                assert_eq!(total_rounds, 5);
                assert!(
                    !images.is_empty(),
                    "round should include fallback images for guessing"
                );
            }
            other => panic!("expected RoundStarted, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn actor_guess_and_scoring() {
        let (mut actor, _tx, mut event_rx, p1) = setup_actor_with_location();

        actor
            .handle_command(RoomCommand::Join {
                player_id: p1,
                name: "Alice".into(),
                join_code: None,
                response_tx: None,
            })
            .await;
        actor
            .handle_command(RoomCommand::Ready {
                player_id: p1,
                response_tx: None,
            })
            .await;
        let _ = event_rx.recv().await; // Joined
        let _ = event_rx.recv().await; // Ready
        let _ = event_rx.recv().await; // RoundStarted

        // Submit guess for exact location (51.5074, -0.1278) -> should score 5000 max score
        let (guess_tx, guess_rx) = oneshot::channel();
        actor
            .handle_command(RoomCommand::Guess {
                player_id: p1,
                seq: 1,
                coordinate: Coordinate {
                    latitude: 51.5074,
                    longitude: -0.1278,
                },
                response_tx: Some(guess_tx),
            })
            .await;

        let score = guess_rx.await.unwrap().unwrap();
        assert_eq!(score, 5000);

        let ack = event_rx.recv().await.unwrap();
        assert_eq!(
            ack,
            RoomEvent::GuessAck {
                player_id: p1,
                seq: 1
            }
        );

        let round_ended = event_rx.recv().await.unwrap();
        assert!(matches!(round_ended, RoomEvent::RoundEnded { .. }));
    }
}
