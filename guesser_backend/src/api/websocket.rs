use crate::app_state::AppState;
use crate::errors::AppError;
use crate::ids::{PlayerId, RoomCode};
use crate::location::coordinates::Coordinate;
use crate::rooms::commands::RoomCommand;
use crate::rooms::events::RoomEvent;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::response::IntoResponse;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

#[derive(Deserialize)]
pub struct WsQuery {
    pub room_code: String,
    pub player_id: u64,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientWsMessage {
    Ready,
    Guess {
        seq: u64,
        latitude: f64,
        longitude: f64,
    },
    Ping,
    Leave,
}

/// `GET /api/ws` — WebSocket handshake and upgrade endpoint.
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    Query(query): Query<WsQuery>,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let code = RoomCode(query.room_code);
    let player_id = PlayerId(query.player_id);

    let room_id = state
        .rooms
        .get_id_by_code(&code)
        .ok_or_else(|| AppError::NotFound("room_not_found".to_string()))?;

    Ok(ws.on_upgrade(move |socket| handle_socket(socket, room_id, player_id, state)))
}

async fn handle_socket(socket: WebSocket, room_id: crate::ids::RoomId, player_id: PlayerId, state: AppState) {
    debug!(%room_id, %player_id, "websocket connected");

    let (mut ws_sender, mut ws_receiver) = socket.split();
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<RoomEvent>();

    // Register broadcast sink with RoomActor
    let _ = state
        .rooms
        .route(
            room_id,
            RoomCommand::RegisterSink {
                player_id,
                sink_tx: event_tx,
            },
        )
        .await;

    // Task 1: Forward outbound RoomEvents -> WebSocket sender
    let send_task = tokio::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            match serde_json::to_string(&event) {
                Ok(json) => {
                    if let Err(e) = ws_sender.send(Message::Text(json.into())).await {
                        debug!(%player_id, error = %e, "websocket send failed");
                        break;
                    }
                }
                Err(e) => {
                    error!(error = %e, "failed to serialize RoomEvent");
                }
            }
        }
    });

    // Task 2: Receive WebSocket messages -> route RoomCommands to RoomActor
    let rooms_clone = state.rooms.clone();
    let recv_task = tokio::spawn(async move {
        while let Some(msg_result) = ws_receiver.next().await {
            let msg = match msg_result {
                Ok(m) => m,
                Err(e) => {
                    debug!(%player_id, error = %e, "websocket receive error");
                    break;
                }
            };

            let text = match msg {
                Message::Text(t) => t,
                Message::Close(_) => break,
                _ => continue,
            };

            let client_msg: ClientWsMessage = match serde_json::from_str(&text) {
                Ok(m) => m,
                Err(e) => {
                    warn!(%player_id, error = %e, "malformed websocket message");
                    continue;
                }
            };

            let command = match client_msg {
                ClientWsMessage::Ready => RoomCommand::Ready { player_id },
                ClientWsMessage::Guess {
                    seq,
                    latitude,
                    longitude,
                } => {
                    match Coordinate::new(latitude, longitude) {
                        Ok(coordinate) => RoomCommand::Guess {
                            player_id,
                            seq,
                            coordinate,
                            response_tx: None,
                        },
                        Err(_) => continue,
                    }
                }
                ClientWsMessage::Ping => RoomCommand::Ping { player_id },
                ClientWsMessage::Leave => RoomCommand::Leave { player_id },
            };

            if let Err(_) = rooms_clone.route(room_id, command).await {
                break;
            }
        }

        // On disconnect/leave, inform room actor
        let _ = rooms_clone.route(room_id, RoomCommand::Leave { player_id }).await;
    });

    // Wait for either send or receive task to complete
    tokio::select! {
        _ = send_task => {},
        _ = recv_task => {},
    }

    info!(%room_id, %player_id, "websocket connection closed");
}
