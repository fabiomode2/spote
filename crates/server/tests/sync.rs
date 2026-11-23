//! Sincronización por cuenta: aislamiento, cursores, tombstones y conflictos.

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use spote_protocol::{
    LoginResponse, SyncOp, SyncPullResponse, SyncPushRequest, SyncPushResponse, TrackRef,
};
use spote_server::{config::Config, router, state::AppState};
use tower::ServiceExt;

const INVITE: &str = "test-invite-0123456789abcdef";
const KEY_A: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";
const KEY_B: &str = "b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90a1";

fn test_state(dir: &tempfile::TempDir) -> AppState {
    let config = Config::from_lookup(|key| match key {
        "SPOTE_INVITE" => Some(INVITE.to_string()),
        "SPOTE_CLIENT_KEYS" => Some(format!("ana:{KEY_A},bea:{KEY_B}")),
        "SPOTE_DB" => Some(dir.path().join("sync-test.db").display().to_string()),
        "SPOTE_BIND" => Some("127.0.0.1:0".to_string()),
        _ => None,
    })
    .expect("config");
    AppState::new(config).expect("estado")
}

async fn login(app: &axum::Router, name: &str, key: &str) -> LoginResponse {
    let body = serde_json::json!({ "name": name, "invite": INVITE }).to_string();
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/login")
                .header("content-type", "application/json")
                .header("x-spote-key", key)
                .body(Body::from(body))
                .expect("request"),
        )
        .await
        .expect("respuesta");
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("body");
    serde_json::from_slice(&bytes).expect("json")
}

async fn snapshot(app: &axum::Router, token: &str, key: &str) -> SyncPullResponse {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/sync/snapshot")
                .header("authorization", format!("Bearer {token}"))
                .header("x-spote-key", key)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("respuesta");
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("body");
    serde_json::from_slice(&bytes).expect("json")
}

async fn pull(app: &axum::Router, token: &str, key: &str, since: i64) -> SyncPullResponse {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/sync/pull?since={since}"))
                .header("authorization", format!("Bearer {token}"))
                .header("x-spote-key", key)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("respuesta");
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("body");
    serde_json::from_slice(&bytes).expect("json")
}

async fn push(
    app: &axum::Router,
    token: &str,
    key: &str,
    req: &SyncPushRequest,
) -> (StatusCode, Option<SyncPushResponse>) {
    let body = serde_json::to_string(req).expect("json");
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/sync/push")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .header("x-spote-key", key)
                .body(Body::from(body))
                .expect("request"),
        )
        .await
        .expect("respuesta");
    let status = response.status();
    if status != StatusCode::OK {
        return (status, None);
    }
    let bytes = to_bytes(response.into_body(), 256 * 1024)
        .await
        .expect("body");
    (status, serde_json::from_slice(&bytes).ok())
}

fn track(key: &str, title: &str) -> TrackRef {
    TrackRef::new(key, title, "Artista")
}

#[tokio::test]
async fn sync_is_per_account_stable_and_incremental() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = router(test_state(&dir));
    let ana = login(&app, "ana", KEY_A).await;
    let bea = login(&app, "bea", KEY_B).await;

    let empty = snapshot(&app, &ana.token, KEY_A).await;
    assert_eq!(empty.revision, 0);
    assert!(empty.tracks.is_empty());

    let req = SyncPushRequest {
        base_revision: 0,
        ops: vec![
            SyncOp::TrackUpsert {
                track: track("isrc:ONE", "Uno"),
                favorite: true,
                play_count: 3,
                last_played: Some(1_700_000_000_000),
            },
            SyncOp::PlaylistUpsert {
                id: "pl-1".to_string(),
                name: "Favoritas".to_string(),
                description: "para el coche".to_string(),
                cover_url: Some("https://example.com/cover.jpg".to_string()),
            },
            SyncOp::PlaylistItemUpsert {
                id: "item-1".to_string(),
                playlist_id: "pl-1".to_string(),
                track_key: "isrc:ONE".to_string(),
                position: 0,
            },
        ],
    };
    let (status, pushed) = push(&app, &ana.token, KEY_A, &req).await;
    assert_eq!(status, StatusCode::OK);
    let pushed = pushed.expect("respuesta");
    assert_eq!(pushed.revision, 1);
    assert_eq!(pushed.applied, 3);

    let changes = pull(&app, &ana.token, KEY_A, 0).await;
    assert_eq!(changes.revision, 1);
    assert_eq!(changes.tracks.len(), 1);
    assert!(changes.tracks[0].favorite);
    assert_eq!(changes.playlists.len(), 1);
    assert_eq!(changes.items.len(), 1);

    // Otro usuario no ve nada aunque use la misma invitación.
    let other = snapshot(&app, &bea.token, KEY_B).await;
    assert_eq!(other.revision, 0);
    assert!(other.tracks.is_empty());

    // Un cambio posterior solo devuelve lo tocado.
    let req = SyncPushRequest {
        base_revision: 1,
        ops: vec![SyncOp::TrackUpsert {
            track: track("isrc:ONE", "Uno"),
            favorite: false,
            play_count: 4,
            last_played: Some(1_700_000_000_001),
        }],
    };
    let (status, pushed) = push(&app, &ana.token, KEY_A, &req).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(pushed.expect("respuesta").revision, 2);

    let changes = pull(&app, &ana.token, KEY_A, 1).await;
    assert_eq!(changes.revision, 2);
    assert_eq!(changes.tracks.len(), 1);
    assert!(!changes.tracks[0].favorite);
    assert!(changes.playlists.is_empty());
    assert!(changes.items.is_empty());

    // Tombstone de playlist: el snapshot deja de verla, el pull la marca.
    let req = SyncPushRequest {
        base_revision: 2,
        ops: vec![SyncOp::PlaylistDelete {
            id: "pl-1".to_string(),
        }],
    };
    let (status, _) = push(&app, &ana.token, KEY_A, &req).await;
    assert_eq!(status, StatusCode::OK);
    let changes = pull(&app, &ana.token, KEY_A, 2).await;
    assert_eq!(changes.playlists.len(), 1);
    assert!(changes.playlists[0].deleted);
    let snap = snapshot(&app, &ana.token, KEY_A).await;
    assert!(snap.playlists.is_empty());
    assert_eq!(snap.revision, 3);
}

#[tokio::test]
async fn stale_revision_returns_conflict_and_does_not_apply() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = router(test_state(&dir));
    let ana = login(&app, "ana", KEY_A).await;

    let req = SyncPushRequest {
        base_revision: 0,
        ops: vec![SyncOp::TrackUpsert {
            track: track("isrc:ONE", "Uno"),
            favorite: true,
            play_count: 0,
            last_played: None,
        }],
    };
    let (status, _) = push(&app, &ana.token, KEY_A, &req).await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = push(&app, &ana.token, KEY_A, &req).await;
    assert_eq!(status, StatusCode::CONFLICT);

    let snap = snapshot(&app, &ana.token, KEY_A).await;
    assert_eq!(snap.revision, 1);
    assert_eq!(snap.tracks.len(), 1);
}

#[tokio::test]
async fn sync_rejects_control_characters_and_bad_urls() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = router(test_state(&dir));
    let ana = login(&app, "ana", KEY_A).await;

    let bad_title = SyncPushRequest {
        base_revision: 0,
        ops: vec![SyncOp::TrackUpsert {
            track: track("isrc:ONE", "Uno\x1b[2J"),
            favorite: true,
            play_count: 0,
            last_played: None,
        }],
    };
    let (status, _) = push(&app, &ana.token, KEY_A, &bad_title).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let bad_cover = SyncPushRequest {
        base_revision: 0,
        ops: vec![SyncOp::PlaylistUpsert {
            id: "pl-1".to_string(),
            name: "Lista".to_string(),
            description: String::new(),
            cover_url: Some("javascript:alert(1)".to_string()),
        }],
    };
    let (status, _) = push(&app, &ana.token, KEY_A, &bad_cover).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Nada se aplicó: el conflicto de base no se gastó.
    let snap = snapshot(&app, &ana.token, KEY_A).await;
    assert_eq!(snap.revision, 0);
    assert!(snap.tracks.is_empty());
}
