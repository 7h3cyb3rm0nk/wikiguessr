use axum::body::Body;
use axum::http::{Request, StatusCode};
use guesser_backend::api::http::{
    create_room, get_room_info, health_check, join_room, solo_guess, solo_start,
};
use guesser_backend::app_state::AppState;
use guesser_backend::config::Config;
use guesser_backend::location::coordinates::{Coordinate, ItemId, Location};
use http_body_util::BodyExt;
use serde_json::Value;
use std::sync::Arc;
use tower::ServiceExt;

fn setup_test_app() -> axum::Router {
    let config = Config::default();
    let state = AppState::new(config);

    // Pre-populate pool with locations for test
    state.resources.pool.push_batch(vec![
        Location {
            item_id: ItemId("Q42".into()),
            coordinate: Coordinate {
                latitude: 51.5074,
                longitude: -0.1278,
            },
        },
        Location {
            item_id: ItemId("Q100".into()),
            coordinate: Coordinate {
                latitude: 48.8566,
                longitude: 2.3522,
            },
        },
    ]);

    axum::Router::new()
        .route("/api/solo/start", axum::routing::post(solo_start))
        .route("/api/solo/guess", axum::routing::post(solo_guess))
        .route("/api/rooms", axum::routing::post(create_room))
        .route("/api/rooms/{code}", axum::routing::get(get_room_info))
        .route("/api/rooms/{code}/join", axum::routing::post(join_room))
        .route("/api/health", axum::routing::get(health_check))
        .with_state(state)
}

#[tokio::test]
async fn health_check_returns_200_ok() {
    let app = setup_test_app();
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "ok");
}

#[tokio::test]
async fn create_and_get_room_info() {
    let app = setup_test_app();

    // 1. Create Room
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/rooms")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();

    let room_code = json["room_code"].as_str().unwrap();
    let join_code = json["join_code"].as_str().unwrap();

    // 2. Get Room Info
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(&format!("/api/rooms/{room_code}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    // 3. Join Room
    let join_payload = serde_json::json!({
        "name": "Bob",
        "join_code": join_code
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&format!("/api/rooms/{room_code}/join"))
                .header("content-type", "application/json")
                .body(Body::from(join_payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn solo_start_and_guess_flow() {
    let app = setup_test_app();

    // 1. Solo Start
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/solo/start")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let start_json: Value = serde_json::from_slice(&body).unwrap();

    let room_code = start_json["room_code"].as_str().unwrap();
    let player_id = start_json["player_id"].as_u64().unwrap();

    // 2. Solo Guess
    let guess_payload = serde_json::json!({
        "room_code": room_code,
        "player_id": player_id,
        "latitude": 51.5074,
        "longitude": -0.1278
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/solo/guess")
                .header("content-type", "application/json")
                .body(Body::from(guess_payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let guess_json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(guess_json["score"], 5000);
}
