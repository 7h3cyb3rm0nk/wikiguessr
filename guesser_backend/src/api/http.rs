use crate::app_state::AppState;
use crate::errors::AppError;
use crate::ids::{PlayerId, RoomCode, RoomId};
use crate::location::coordinates::Coordinate;
use crate::rooms::commands::{RoomCommand, RoundResultSnapshot, RoundStateSnapshot};
use axum::Json;
use axum::extract::{Path, Query, State};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::oneshot;

// ── Single-Player REST Endpoints ─────────────────────────────────────────

#[derive(Serialize)]
pub struct SoloStartResponse {
    pub room_code: RoomCode,
    pub player_id: PlayerId,
    pub player_token: String,
    pub total_rounds: u8,
    pub round_duration_secs: u64,
    pub images: Vec<crate::image::Image>,
    pub deadline_unix_ms: u64,
}

/// `POST /api/solo/start` — Start a single-player game.
/// Spawns a 1-player room actor, auto-joins and readies the player.
pub async fn solo_start(
    State(state): State<AppState>,
) -> Result<Json<SoloStartResponse>, AppError> {
    let (_id, code, _join_code, _host_token, _tx) =
        Arc::clone(&state.rooms).create_room(state.config.game.clone(), state.resources.clone());

    let player_id = PlayerId::new();
    let player_token = format!("p_tok_{player_id}");

    let (join_tx, join_rx) = oneshot::channel();
    state
        .rooms
        .route_by_code(
            &code,
            RoomCommand::Join {
                player_id,
                name: "Solo Player".to_string(),
                join_code: None,
                response_tx: Some(join_tx),
            },
        )
        .await
        .map_err(AppError::Internal)?;

    join_rx
        .await
        .map_err(|_| AppError::Internal("join rx dropped".to_string()))?
        .map_err(AppError::BadRequest)?;

    // Auto-ready to start round 1 immediately, wait for round state
    let (ready_tx, ready_rx) = oneshot::channel();
    state
        .rooms
        .route_by_code(
            &code,
            RoomCommand::Ready {
                player_id,
                response_tx: Some(ready_tx),
            },
        )
        .await
        .map_err(AppError::Internal)?;

    let round_state = ready_rx
        .await
        .map_err(|_| AppError::Internal("ready rx dropped".to_string()))?
        .map_err(AppError::BadRequest)?;

    Ok(Json(SoloStartResponse {
        room_code: code,
        player_id,
        player_token,
        total_rounds: state.config.game.total_rounds,
        round_duration_secs: state.config.game.round_duration_secs,
        images: round_state.images,
        deadline_unix_ms: round_state.deadline_unix_ms,
    }))
}

#[derive(Deserialize)]
pub struct SoloGuessRequest {
    pub room_code: String,
    pub player_id: u64,
    pub latitude: f64,
    pub longitude: f64,
}

#[derive(Serialize)]
pub struct SoloGuessResponse {
    pub score: u32,
    pub status: &'static str,
}

/// `POST /api/solo/guess` — Submit a guess for single-player.
pub async fn solo_guess(
    State(state): State<AppState>,
    Json(payload): Json<SoloGuessRequest>,
) -> Result<Json<SoloGuessResponse>, AppError> {
    let code = RoomCode(payload.room_code);
    let player_id = PlayerId(payload.player_id);
    let coord = Coordinate::new(payload.latitude, payload.longitude)
        .map_err(|_| AppError::BadRequest("invalid coordinates".to_string()))?;

    let (guess_tx, guess_rx) = oneshot::channel();
    state
        .rooms
        .route_by_code(
            &code,
            RoomCommand::Guess {
                player_id,
                seq: 1,
                coordinate: coord,
                response_tx: Some(guess_tx),
            },
        )
        .await
        .map_err(AppError::NotFound)?;

    let score = guess_rx
        .await
        .map_err(|_| AppError::Internal("guess rx dropped".to_string()))?
        .map_err(AppError::BadRequest)?;

    Ok(Json(SoloGuessResponse {
        score,
        status: "success",
    }))
}

/// `GET /api/solo/round?room_code=...` — Fetch current round images for solo player.
#[derive(Deserialize)]
pub struct SoloRoundQuery {
    pub room_code: String,
}

pub async fn solo_round_state(
    State(state): State<AppState>,
    Query(query): Query<SoloRoundQuery>,
) -> Result<Json<RoundStateSnapshot>, AppError> {
    let code = RoomCode(query.room_code);

    let (tx, rx) = oneshot::channel();
    state
        .rooms
        .route_by_code(&code, RoomCommand::GetRoundState { response_tx: tx })
        .await
        .map_err(AppError::NotFound)?;

    let snapshot = rx
        .await
        .map_err(|_| AppError::Internal("round state rx dropped".to_string()))?
        .ok_or_else(|| AppError::NotFound("no active round".to_string()))?;

    Ok(Json(snapshot))
}

/// `GET /api/solo/round/result?room_code=...&player_id=...` — Fetch round result for solo player.
#[derive(Deserialize)]
pub struct SoloRoundResultQuery {
    pub room_code: String,
    pub player_id: u64,
}

pub async fn solo_round_result(
    State(state): State<AppState>,
    Query(query): Query<SoloRoundResultQuery>,
) -> Result<Json<RoundResultSnapshot>, AppError> {
    let code = RoomCode(query.room_code);
    let player_id = PlayerId(query.player_id);

    let (tx, rx) = oneshot::channel();
    state
        .rooms
        .route_by_code(
            &code,
            RoomCommand::GetRoundResult {
                player_id,
                response_tx: tx,
            },
        )
        .await
        .map_err(AppError::NotFound)?;

    let snapshot = rx
        .await
        .map_err(|_| AppError::Internal("round result rx dropped".to_string()))?
        .ok_or_else(|| AppError::NotFound("no round result available".to_string()))?;

    Ok(Json(snapshot))
}

// ── Multiplayer REST Endpoints ───────────────────────────────────────────

#[derive(Serialize)]
pub struct CreateRoomResponse {
    pub room_id: RoomId,
    pub room_code: RoomCode,
    pub join_code: String,
    pub host_token: String,
}

/// `POST /api/rooms` — Create a multiplayer room.
pub async fn create_room(
    State(state): State<AppState>,
) -> Result<Json<CreateRoomResponse>, AppError> {
    let (room_id, room_code, join_code, host_token, _tx) =
        Arc::clone(&state.rooms).create_room(state.config.game.clone(), state.resources.clone());

    Ok(Json(CreateRoomResponse {
        room_id,
        room_code,
        join_code,
        host_token,
    }))
}

#[derive(Serialize)]
pub struct RoomInfoResponse {
    pub room_code: RoomCode,
    pub active: bool,
}

/// `GET /api/rooms/:code` — Check room status.
pub async fn get_room_info(
    State(state): State<AppState>,
    Path(code_str): Path<String>,
) -> Result<Json<RoomInfoResponse>, AppError> {
    let code = RoomCode(code_str);
    let active = state.rooms.get_id_by_code(&code).is_some();

    if !active {
        return Err(AppError::NotFound("room not found".to_string()));
    }

    Ok(Json(RoomInfoResponse {
        room_code: code,
        active: true,
    }))
}

#[derive(Deserialize)]
pub struct JoinRoomRequest {
    pub name: String,
    pub join_code: String,
}

#[derive(Serialize)]
pub struct JoinRoomResponse {
    pub player_id: PlayerId,
    pub player_token: String,
}

/// `POST /api/rooms/:code/join` — Join a room.
pub async fn join_room(
    State(state): State<AppState>,
    Path(code_str): Path<String>,
    Json(payload): Json<JoinRoomRequest>,
) -> Result<Json<JoinRoomResponse>, AppError> {
    let code = RoomCode(code_str);
    let player_id = PlayerId::new();
    let player_token = format!("p_tok_{player_id}");

    let (join_tx, join_rx) = oneshot::channel();
    state
        .rooms
        .route_by_code(
            &code,
            RoomCommand::Join {
                player_id,
                name: payload.name,
                join_code: Some(payload.join_code),
                response_tx: Some(join_tx),
            },
        )
        .await
        .map_err(AppError::NotFound)?;

    join_rx
        .await
        .map_err(|_| AppError::Internal("join rx dropped".to_string()))?
        .map_err(AppError::BadRequest)?;

    Ok(Json(JoinRoomResponse {
        player_id,
        player_token,
    }))
}

#[derive(Deserialize)]
pub struct HostStartRequest {
    pub host_token: String,
}

#[derive(Serialize)]
pub struct HostStartResponse {
    pub round: u8,
    pub total_rounds: u8,
    pub images: Vec<crate::image::Image>,
    pub deadline_unix_ms: u64,
}

/// `POST /api/rooms/:code/start` — Host force-starts the game.
pub async fn start_room(
    State(state): State<AppState>,
    Path(code_str): Path<String>,
    Json(payload): Json<HostStartRequest>,
) -> Result<Json<HostStartResponse>, AppError> {
    let code = RoomCode(code_str);

    let (tx, rx) = oneshot::channel();
    state
        .rooms
        .route_by_code(
            &code,
            RoomCommand::HostStart {
                host_token: payload.host_token,
                response_tx: Some(tx),
            },
        )
        .await
        .map_err(AppError::NotFound)?;

    let snapshot = rx
        .await
        .map_err(|_| AppError::Internal("start rx dropped".to_string()))?
        .map_err(AppError::Unauthorized)?;

    Ok(Json(HostStartResponse {
        round: snapshot.round_number,
        total_rounds: snapshot.total_rounds,
        images: snapshot.images,
        deadline_unix_ms: snapshot.deadline_unix_ms,
    }))
}

// ── Health Check ─────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub active_rooms: usize,
    pub pool_size: usize,
}

/// `GET /api/health` — Health status.
pub async fn health_check(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        active_rooms: state.rooms.len(),
        pool_size: state.resources.pool.len(),
    })
}
