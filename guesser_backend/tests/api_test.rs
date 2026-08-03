use axum::body::Body;
use axum::http::{Request, StatusCode};
use guesser_backend::api::http::{
    create_room, get_room_info, health_check, join_room, solo_guess, solo_round_result,
    solo_round_state, solo_start, start_room,
};
use guesser_backend::app_state::AppState;
use guesser_backend::config::Config;
use guesser_backend::location::coordinates::{Coordinate, ItemId, Location};
use guesser_backend::resources::pool::PreparedRound;
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

fn setup_test_app() -> axum::Router {
    let config = Config::default();
    let state = AppState::new(config);

    // Pre-populate pool with locations for test
    state.resources.pool.push_batch(vec![
        PreparedRound {
            location: Location {
                item_id: ItemId("Q42".into()),
                coordinate: Coordinate {
                    latitude: 51.5074,
                    longitude: -0.1278,
                },
            },
            images: vec![guesser_backend::image::Image {
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
            images: vec![guesser_backend::image::Image {
                url: "test".into(),
                thumb_url: "test".into(),
                width: 100,
                height: 100,
            }],
        },
    ]);

    axum::Router::new()
        .route("/api/solo/start", axum::routing::post(solo_start))
        .route("/api/solo/guess", axum::routing::post(solo_guess))
        .route("/api/solo/round", axum::routing::get(solo_round_state))
        .route("/api/solo/round/result", axum::routing::get(solo_round_result))
        .route("/api/rooms", axum::routing::post(create_room))
        .route("/api/rooms/{code}", axum::routing::get(get_room_info))
        .route("/api/rooms/{code}/join", axum::routing::post(join_room))
        .route("/api/rooms/{code}/start", axum::routing::post(start_room))
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
async fn join_room_validates_join_code() {
    let app = setup_test_app();

    // Create a room and capture its join code
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

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    let room_code = json["room_code"].as_str().unwrap();
    let join_code = json["join_code"].as_str().unwrap();

    // 1. Reject a bad join code
    let bad_payload = serde_json::json!({
        "name": "Eve",
        "join_code": "9999"
    });
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&format!("/api/rooms/{room_code}/join"))
                .header("content-type", "application/json")
                .body(Body::from(bad_payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // 2. A correct join code succeeds
    let good_payload = serde_json::json!({
        "name": "Bob",
        "join_code": join_code
    });
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&format!("/api/rooms/{room_code}/join"))
                .header("content-type", "application/json")
                .body(Body::from(good_payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn host_start_requires_valid_token_and_starts_round() {
    let app = setup_test_app();

    // Create a room and capture its host token
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

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    let room_code = json["room_code"].as_str().unwrap();
    let host_token = json["host_token"].as_str().unwrap();

    // 1. Invalid host token is rejected
    let bad_payload = serde_json::json!({ "host_token": "wrong_token" });
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&format!("/api/rooms/{room_code}/start"))
                .header("content-type", "application/json")
                .body(Body::from(bad_payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    // 2. Correct host token starts the game and returns images
    let good_payload = serde_json::json!({ "host_token": host_token });
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&format!("/api/rooms/{room_code}/start"))
                .header("content-type", "application/json")
                .body(Body::from(good_payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let start_json: Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(start_json["round"], 1);
    assert!(
        start_json["images"].as_array().unwrap().len() > 0,
        "host start should return round images"
    );
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

    // Start response must include images for round 1 (so the player can render immediately)
    assert!(
        start_json["images"].is_array() && start_json["images"].as_array().unwrap().len() > 0,
        "solo start should return images"
    );
    assert!(
        start_json["deadline_unix_ms"].as_u64().unwrap() > 0,
        "solo start should return a deadline"
    );

    // 2. Solo Guess
    let guess_payload = serde_json::json!({
        "room_code": room_code,
        "player_id": player_id,
        "latitude": 51.5074,
        "longitude": -0.1278
    });

    let response = app
        .clone()
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

    // 3. Fetch round result (actual location reveal) after the guess
    let response = app
        .oneshot(
            Request::builder()
                .uri(&format!(
                    "/api/solo/round/result?room_code={room_code}&player_id={player_id}"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let result_json: Value = serde_json::from_slice(&body).unwrap();

    // The reveal should include the actual location and the item ID
    assert!(
        result_json["item_id"].as_str().is_some(),
        "should reveal the item ID"
    );
    assert_eq!(result_json["score"], 5000);
    assert!(
        result_json["actual_location"]["latitude"].is_number(),
        "should reveal actual location"
    );
    assert!(
        !result_json["game_finished"].as_bool().unwrap(),
        "single round of 5 total should not finish yet"
    );
}

#[tokio::test]
async fn solo_round_state_endpoint_returns_current_round() {
    let app = setup_test_app();

    // Start a solo game (gets round 1 active)
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

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let start_json: Value = serde_json::from_slice(&body).unwrap();
    let room_code = start_json["room_code"].as_str().unwrap();

    // Fetch current round state via /api/solo/round
    let response = app
        .oneshot(
            Request::builder()
                .uri(&format!("/api/solo/round?room_code={room_code}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let round_json: Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(round_json["round_number"], 1);
    assert!(
        round_json["images"].as_array().unwrap().len() > 0,
        "round endpoint should return images"
    );
}
