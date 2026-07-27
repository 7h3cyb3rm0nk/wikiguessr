use crate::app_state::AppState;
use crate::errors::AppError;
use crate::ids::{PlayerId, RoomCode, RoomId};
use crate::location::coordinates::Coordinate;
use crate::rooms::commands::RoomCommand;
use axum::extract::{Path, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

// ── Single-Player REST Endpoints ─────────────────────────────────────────

#[derive(Serialize)]
pub struct SoloStartResponse {
    pub room_code: RoomCode,
    pub player_id: PlayerId,
    pub player_token: String,
    pub total_rounds: u8,
    pub round_duration_secs: u64,
}

/// `POST /api/solo/start` — Start a single-player game.
/// Spawns a 1-player room actor, auto-joins and readies the player.
pub async fn solo_start(
    State(state): State<AppState>,
) -> Result<Json<SoloStartResponse>, AppError> {
    let (_id, code, _join_code, _host_token, _tx) = state
        .rooms
        .create_room(state.config.game.clone(), state.resources.clone());

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
                response_tx: Some(join_tx),
            },
        )
        .await
        .map_err(|e| AppError::Internal(e))?;

    join_rx
        .await
        .map_err(|_| AppError::Internal("join rx dropped".to_string()))?
        .map_err(|e| AppError::BadRequest(e))?;

    // Auto-ready to start round 1 immediately
    state
        .rooms
        .route_by_code(&code, RoomCommand::Ready { player_id })
        .await
        .map_err(|e| AppError::Internal(e))?;

    Ok(Json(SoloStartResponse {
        room_code: code,
        player_id,
        player_token,
        total_rounds: state.config.game.total_rounds,
        round_duration_secs: state.config.game.round_duration_secs,
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
        .map_err(|e| AppError::NotFound(e))?;

    let score = guess_rx
        .await
        .map_err(|_| AppError::Internal("guess rx dropped".to_string()))?
        .map_err(|e| AppError::BadRequest(e))?;

    Ok(Json(SoloGuessResponse {
        score,
        status: "success",
    }))
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
    let (room_id, room_code, join_code, host_token, _tx) = state
        .rooms
        .create_room(state.config.game.clone(), state.resources.clone());

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
                response_tx: Some(join_tx),
            },
        )
        .await
        .map_err(|e| AppError::NotFound(e))?;

    join_rx
        .await
        .map_err(|_| AppError::Internal("join rx dropped".to_string()))?
        .map_err(|e| AppError::BadRequest(e))?;

    Ok(Json(JoinRoomResponse {
        player_id,
        player_token,
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
