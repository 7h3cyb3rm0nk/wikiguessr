mod api;
mod app_state;
mod config;
mod errors;
mod ids;
mod image;
mod location;
mod resources;
mod rooms;
mod rules;
mod scoring;

use api::http::{create_room, get_room_info, health_check, join_room, solo_guess, solo_start};
use api::websocket::ws_handler;
use app_state::AppState;
use axum::routing::{get, post};
use axum::Router;
use config::Config;
use std::net::SocketAddr;
use tower_http::cors::CorsLayer;
use tracing::info;
use tracing_subscriber::fmt::format::FmtSpan;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize structured logging
    tracing_subscriber::fmt()
        .with_span_events(FmtSpan::CLOSE)
        .init();

    let config = Config::from_env();
    info!(
        host = %config.server.host,
        port = config.server.port,
        "starting wikiguessr backend server"
    );

    let state = AppState::new(config.clone());

    // Start background location pool refill worker
    state
        .resources
        .clone()
        .start_refill_worker(config.game.clone());

    // Configure CORS for web frontend
    let cors = CorsLayer::permissive();

    // Build router
    let app = Router::new()
        // Single-player REST routes
        .route("/api/solo/start", post(solo_start))
        .route("/api/solo/guess", post(solo_guess))
        // Multiplayer REST routes
        .route("/api/rooms", post(create_room))
        .route("/api/rooms/{code}", get(get_room_info))
        .route("/api/rooms/{code}/join", post(join_room))
        // WebSocket multiplayer route
        .route("/api/ws", get(ws_handler))
        // Health check
        .route("/api/health", get(health_check))
        .layer(cors)
        .with_state(state);

    let addr: SocketAddr = format!("{}:{}", config.server.host, config.server.port).parse()?;
    info!("server listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
