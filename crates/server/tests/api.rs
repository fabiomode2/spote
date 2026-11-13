//! Integración con el router de axum mediante `tower::ServiceExt::oneshot`
//! (sin abrir puertos).

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use spote_protocol::{LoginResponse, TimeResponse};
use spote_server::{config::Config, router, state::AppState};
use tower::ServiceExt;

fn test_state(dir: &tempfile::TempDir) -> AppState {
    let config = Config {
        bind: "127.0.0.1:0".parse().expect("addr válida"),
        db_path: dir.path().join("api-test.db"),
        invite: "spote".to_string(),
    };
    AppState::new(config).expect("estado")
}

fn login_request(name: &str, invite: &str) -> Request<Body> {
    let body = serde_json::json!({ "name": name, "invite": invite }).to_string();
    Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header("content-type", "application/json")
        .body(Body::from(body))
        .expect("request")
}

#[tokio::test]
async fn get_time_returns_server_clock() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = router(test_state(&dir));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/time")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("respuesta");

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("body");
    let body: TimeResponse = serde_json::from_slice(&bytes).expect("json");
    assert!(body.t > 1_600_000_000_000, "epoch en ms");
}

#[tokio::test]
async fn login_returns_token_and_me_works() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = router(test_state(&dir));

    let response = app
        .clone()
        .oneshot(login_request("ana", "spote"))
        .await
        .expect("respuesta");
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("body");
    let login: LoginResponse = serde_json::from_slice(&bytes).expect("json");
    assert_eq!(login.user.name, "ana");
    assert!(!login.token.is_empty());

    let me = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/me")
                .header("authorization", format!("Bearer {}", login.token))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("respuesta");
    assert_eq!(me.status(), StatusCode::OK);
    let bytes = to_bytes(me.into_body(), 64 * 1024).await.expect("body");
    let user: spote_protocol::User = serde_json::from_slice(&bytes).expect("json");
    assert_eq!(user.id, login.user.id);

    let anonymous = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/me")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("respuesta");
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);

    let bad_invite = app
        .oneshot(login_request("bea", "nope"))
        .await
        .expect("respuesta");
    assert_eq!(bad_invite.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn rooms_are_listed_after_creation() {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = test_state(&dir);
    let app = router(state);

    let login = app
        .clone()
        .oneshot(login_request("ana", "spote"))
        .await
        .expect("respuesta");
    let bytes = to_bytes(login.into_body(), 64 * 1024).await.expect("body");
    let login: LoginResponse = serde_json::from_slice(&bytes).expect("json");
    let auth = format!("Bearer {}", login.token);

    let created = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/rooms")
                .header("content-type", "application/json")
                .header("authorization", &auth)
                .body(Body::from(r#"{"name":"sesión","is_public":true}"#))
                .expect("request"),
        )
        .await
        .expect("respuesta");
    assert_eq!(created.status(), StatusCode::OK);

    let listed = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/rooms")
                .header("authorization", &auth)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("respuesta");
    assert_eq!(listed.status(), StatusCode::OK);
    let bytes = to_bytes(listed.into_body(), 256 * 1024)
        .await
        .expect("body");
    let rooms: Vec<spote_protocol::Room> = serde_json::from_slice(&bytes).expect("json");
    assert_eq!(rooms.len(), 1);
    assert_eq!(rooms[0].name, "sesión");
    assert_eq!(rooms[0].members.len(), 1);
}
