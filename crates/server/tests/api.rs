//! Integración con el router de axum mediante `tower::ServiceExt::oneshot`
//! (sin abrir puertos).

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use spote_protocol::{LoginResponse, TimeResponse};
use spote_server::{config::Config, router, state::AppState};
use tower::ServiceExt;

const INVITE: &str = "test-invite-0123456789abcdef";
const KEY_A: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";
const KEY_B: &str = "b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90a1";

fn test_state(dir: &tempfile::TempDir) -> AppState {
    let config = Config::from_lookup(|key| match key {
        "SPOTE_INVITE" => Some(INVITE.to_string()),
        "SPOTE_CLIENT_KEYS" => Some(format!("ana:{KEY_A},bea:{KEY_B}")),
        "SPOTE_DB" => Some(dir.path().join("api-test.db").display().to_string()),
        "SPOTE_BIND" => Some("127.0.0.1:0".to_string()),
        "SPOTE_SESSION_DAYS" => Some("1".to_string()),
        _ => None,
    })
    .expect("config de test");
    AppState::new(config).expect("estado")
}

fn login_request(name: &str, invite: &str, key: &str) -> Request<Body> {
    let body = serde_json::json!({ "name": name, "invite": invite }).to_string();
    Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header("content-type", "application/json")
        .header("x-spote-key", key)
        .body(Body::from(body))
        .expect("request")
}

async fn login(app: &axum::Router, name: &str, key: &str) -> LoginResponse {
    let response = app
        .clone()
        .oneshot(login_request(name, INVITE, key))
        .await
        .expect("respuesta");
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("body");
    serde_json::from_slice(&bytes).expect("json")
}

fn avatar_request(token: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method("PUT")
        .uri("/api/v1/me/avatar")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .header("x-spote-key", KEY_A)
        .body(Body::from(body.to_string()))
        .expect("request")
}

#[tokio::test]
async fn api_is_closed_without_client_key() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = router(test_state(&dir));

    let missing = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/time")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("respuesta");
    assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);

    let wrong = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/time")
                .header("x-spote-key", "no-es-una-clave")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("respuesta");
    assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);

    // Incluso las rutas inexistentes bajo /api/v1 quedan cerradas.
    let unknown = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/no-existe")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("respuesta");
    assert_eq!(unknown.status(), StatusCode::UNAUTHORIZED);

    let ok = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/time")
                .header("x-spote-key", KEY_A)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("respuesta");
    assert_eq!(ok.status(), StatusCode::OK);
    let bytes = to_bytes(ok.into_body(), 64 * 1024).await.expect("body");
    let body: TimeResponse = serde_json::from_slice(&bytes).expect("json");
    assert!(body.t > 1_600_000_000_000, "epoch en ms");
}

#[tokio::test]
async fn healthz_is_public_but_does_not_expose_data() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = router(test_state(&dir));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("respuesta");
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn login_returns_token_and_me_works() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = router(test_state(&dir));

    let login = login(&app, "ana", KEY_A).await;
    assert_eq!(login.user.name, "ana");
    assert!(!login.token.is_empty());

    let me = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/me")
                .header("authorization", format!("Bearer {}", login.token))
                .header("x-spote-key", KEY_A)
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
                .header("x-spote-key", KEY_A)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("respuesta");
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);

    let bad_invite = app
        .oneshot(login_request("bea", "invitacion-incorrecta", KEY_B))
        .await
        .expect("respuesta");
    assert_eq!(bad_invite.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn logout_revokes_the_session() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = router(test_state(&dir));
    let login = login(&app, "ana", KEY_A).await;
    let auth = format!("Bearer {}", login.token);

    let logout = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/logout")
                .header("authorization", &auth)
                .header("x-spote-key", KEY_A)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("respuesta");
    assert_eq!(logout.status(), StatusCode::OK);

    let me = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/me")
                .header("authorization", auth)
                .header("x-spote-key", KEY_A)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("respuesta");
    assert_eq!(me.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn session_and_name_are_bound_to_the_client_key() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = router(test_state(&dir));

    let login = login(&app, "ana", KEY_A).await;

    // El token de A no sirve con la clave de B.
    let wrong_key = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/me")
                .header("authorization", format!("Bearer {}", login.token))
                .header("x-spote-key", KEY_B)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("respuesta");
    assert_eq!(wrong_key.status(), StatusCode::UNAUTHORIZED);

    // B tampoco puede reclamar el nombre de A.
    let impersonation = app
        .oneshot(login_request("ana", INVITE, KEY_B))
        .await
        .expect("respuesta");
    assert_eq!(impersonation.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn private_rooms_cannot_be_joined_without_an_invite_from_the_host() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = router(test_state(&dir));

    let ana = login(&app, "ana", KEY_A).await;
    let bea = login(&app, "bea", KEY_B).await;

    let created = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/rooms")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {}", ana.token))
                .header("x-spote-key", KEY_A)
                .body(Body::from(r#"{"name":"privada","is_public":false}"#))
                .expect("request"),
        )
        .await
        .expect("respuesta");
    assert_eq!(created.status(), StatusCode::OK);
    let bytes = to_bytes(created.into_body(), 256 * 1024)
        .await
        .expect("body");
    let room: spote_protocol::Room = serde_json::from_slice(&bytes).expect("json");

    let rejected = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/rooms/{}/join", room.id))
                .header("authorization", format!("Bearer {}", bea.token))
                .header("x-spote-key", KEY_B)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("respuesta");
    assert_eq!(rejected.status(), StatusCode::FORBIDDEN);

    let added = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/rooms/{}/members", room.id))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {}", ana.token))
                .header("x-spote-key", KEY_A)
                .body(Body::from(
                    serde_json::json!({ "user_id": bea.user.id }).to_string(),
                ))
                .expect("request"),
        )
        .await
        .expect("respuesta");
    assert_eq!(added.status(), StatusCode::OK);

    let joined = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/rooms/{}/join", room.id))
                .header("authorization", format!("Bearer {}", bea.token))
                .header("x-spote-key", KEY_B)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("respuesta");
    assert_eq!(joined.status(), StatusCode::OK);
}

#[tokio::test]
async fn avatar_can_be_set_read_and_cleared() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = router(test_state(&dir));
    let login = login(&app, "ana", KEY_A).await;
    let jpeg = format!("data:image/jpeg;base64,{}", "A".repeat(64));

    let set = app
        .clone()
        .oneshot(avatar_request(
            &login.token,
            &serde_json::json!({ "avatar": jpeg }).to_string(),
        ))
        .await
        .expect("respuesta");
    assert_eq!(set.status(), StatusCode::OK);
    let bytes = to_bytes(set.into_body(), 64 * 1024).await.expect("body");
    let user: spote_protocol::User = serde_json::from_slice(&bytes).expect("json");
    assert_eq!(user.avatar.as_deref(), Some(jpeg.as_str()));

    let me = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/me")
                .header("authorization", format!("Bearer {}", login.token))
                .header("x-spote-key", KEY_A)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("respuesta");
    assert_eq!(me.status(), StatusCode::OK);
    let bytes = to_bytes(me.into_body(), 64 * 1024).await.expect("body");
    let me: spote_protocol::User = serde_json::from_slice(&bytes).expect("json");
    assert_eq!(me.avatar.as_deref(), Some(jpeg.as_str()));

    let invalid = app
        .clone()
        .oneshot(avatar_request(
            &login.token,
            r#"{"avatar":"data:image/gif;base64,R0lGODlhAQABAAAAACw="}"#,
        ))
        .await
        .expect("respuesta");
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);

    let missing = app
        .clone()
        .oneshot(avatar_request(&login.token, "{}"))
        .await
        .expect("respuesta");
    assert_eq!(missing.status(), StatusCode::BAD_REQUEST);

    let wrong_type = app
        .clone()
        .oneshot(avatar_request(&login.token, r#"{"avatar":42}"#))
        .await
        .expect("respuesta");
    assert_eq!(wrong_type.status(), StatusCode::BAD_REQUEST);

    let anonymous = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v1/me/avatar")
                .header("content-type", "application/json")
                .header("x-spote-key", KEY_A)
                .body(Body::from(r#"{"avatar":null}"#))
                .expect("request"),
        )
        .await
        .expect("respuesta");
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);

    let cleared = app
        .oneshot(avatar_request(&login.token, r#"{"avatar":null}"#))
        .await
        .expect("respuesta");
    assert_eq!(cleared.status(), StatusCode::OK);
    let bytes = to_bytes(cleared.into_body(), 64 * 1024)
        .await
        .expect("body");
    let user: spote_protocol::User = serde_json::from_slice(&bytes).expect("json");
    assert!(user.avatar.is_none());
}

#[tokio::test]
async fn rooms_are_listed_after_creation() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = router(test_state(&dir));
    let login = login(&app, "ana", KEY_A).await;
    let auth = format!("Bearer {}", login.token);

    let created = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/rooms")
                .header("content-type", "application/json")
                .header("authorization", &auth)
                .header("x-spote-key", KEY_A)
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
                .header("x-spote-key", KEY_A)
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
