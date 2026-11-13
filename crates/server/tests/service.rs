//! Tests de la lógica de negocio (sin red): login, salas, control y cola.

use axum::http::StatusCode;
use spote_protocol::{
    ControlAction, ControlRequest, CreateRoomRequest, LoginRequest, LoginResponse, Role, TrackRef,
};
use spote_server::{config::Config, service, service::ReadyOutcome, state::AppState};

fn test_state(dir: &tempfile::TempDir) -> AppState {
    let config = Config {
        bind: "127.0.0.1:0".parse().expect("addr válida"),
        db_path: dir.path().join("service-test.db"),
        invite: "spote".to_string(),
    };
    AppState::new(config).expect("estado")
}

fn login(state: &AppState, name: &str) -> LoginResponse {
    service::login(
        state,
        &LoginRequest {
            name: name.to_string(),
            invite: "spote".to_string(),
        },
    )
    .expect("login")
}

fn control(
    state: &AppState,
    user_id: &str,
    room_id: &str,
    action: ControlAction,
    position_ms: Option<u64>,
) -> service::ControlOutcome {
    service::control(
        state,
        user_id,
        room_id,
        &ControlRequest {
            action,
            position_ms,
        },
    )
    .expect("control")
}

#[test]
fn login_creates_user_and_is_idempotent_by_name() {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = test_state(&dir);

    let first = login(&state, "ana");
    assert_eq!(first.user.name, "ana");
    assert!(!first.user.id.is_empty());
    assert!(!first.token.is_empty());
    assert_eq!(first.invite, "spote");

    let second = login(&state, "ana");
    assert_eq!(first.user.id, second.user.id, "mismo usuario por nombre");
    assert_ne!(first.token, second.token, "token nuevo en cada login");

    let resolved = state
        .db
        .user_by_token(&second.token)
        .expect("consulta")
        .expect("sesión válida");
    assert_eq!(resolved.id, first.user.id);

    let other = login(&state, "bea");
    assert_ne!(other.user.id, first.user.id);
}

#[test]
fn login_rejects_bad_invite_and_empty_name() {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = test_state(&dir);

    let bad_invite = service::login(
        &state,
        &LoginRequest {
            name: "ana".to_string(),
            invite: "otra".to_string(),
        },
    )
    .expect_err("invite inválida");
    assert_eq!(bad_invite.status(), StatusCode::UNAUTHORIZED);

    let empty = service::login(
        &state,
        &LoginRequest {
            name: "   ".to_string(),
            invite: "spote".to_string(),
        },
    )
    .expect_err("nombre vacío");
    assert_eq!(empty.status(), StatusCode::BAD_REQUEST);
}

#[test]
fn create_room_join_and_control_updates_state() {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = test_state(&dir);
    let host = login(&state, "ana").user;
    let guest = login(&state, "bea").user;

    let room = service::create_room(
        &state,
        &host,
        &CreateRoomRequest {
            name: "sesión".to_string(),
            is_public: true,
        },
    )
    .expect("crear sala");
    assert_eq!(room.host_id, host.id);
    assert_eq!(room.members.len(), 1);
    assert_eq!(room.members[0].role, Role::Host);
    assert!(!room.state.playing);

    let joined = service::join_room(&state, &guest, &room.id).expect("join");
    assert_eq!(joined.members.len(), 2);
    assert!(joined
        .members
        .iter()
        .any(|m| m.user.id == guest.id && m.role == Role::Guest));

    service::queue_add(
        &state,
        &host.id,
        &room.id,
        TrackRef::new("isrc:AAA", "Uno", "Artista"),
    )
    .expect("queue add A");
    service::queue_add(
        &state,
        &guest.id,
        &room.id,
        TrackRef::new("isrc:BBB", "Dos", "Artista"),
    )
    .expect("queue add B");

    let playing = control(&state, &host.id, &room.id, ControlAction::Play, None);
    assert!(playing.state.playing);
    assert!(playing.queue_changed);
    assert_eq!(
        playing.state.track.as_ref().map(|t| t.key.as_str()),
        Some("isrc:AAA")
    );
    assert!(playing.state.anchor_ms > 0);

    let paused = control(&state, &guest.id, &room.id, ControlAction::Pause, None);
    assert!(!paused.state.playing);
    let frozen = paused.state.position_ms;

    let seeked = control(
        &state,
        &guest.id,
        &room.id,
        ControlAction::Seek,
        Some(9_000),
    );
    assert_eq!(seeked.state.position_ms, 9_000);
    assert!(!seeked.state.playing);
    assert!(seeked.state.anchor_ms >= paused.state.anchor_ms);
    assert!(frozen <= 5_000, "pausa congela una posición razonable");

    let next = control(&state, &host.id, &room.id, ControlAction::Next, None);
    assert_eq!(
        next.state.track.as_ref().map(|t| t.key.as_str()),
        Some("isrc:BBB"),
        "next avanza la cola"
    );
    assert_eq!(next.state.position_ms, 0);
    assert!(next.state.playing);
    assert!(next.queue.is_empty());

    let stopped = control(&state, &host.id, &room.id, ControlAction::Next, None);
    assert!(stopped.state.track.is_none(), "cola vacía detiene la sala");
    assert!(!stopped.state.playing);

    let prev = control(&state, &host.id, &room.id, ControlAction::Prev, None);
    assert_eq!(
        prev.state.track.as_ref().map(|t| t.key.as_str()),
        Some("isrc:BBB"),
        "prev recupera del historial"
    );

    let stored = state.db.room(&room.id).expect("consulta").expect("sala");
    assert_eq!(
        stored.state.track.as_ref().map(|t| t.key.as_str()),
        Some("isrc:BBB")
    );
}

#[test]
fn queue_add_and_remove_compact_positions() {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = test_state(&dir);
    let host = login(&state, "ana").user;
    let room = service::create_room(
        &state,
        &host,
        &CreateRoomRequest {
            name: "cola".to_string(),
            is_public: true,
        },
    )
    .expect("crear sala");

    let a = service::queue_add(&state, &host.id, &room.id, TrackRef::new("a", "A", "X"))
        .expect("add a");
    let b = service::queue_add(&state, &host.id, &room.id, TrackRef::new("b", "B", "X"))
        .expect("add b");
    let c = service::queue_add(&state, &host.id, &room.id, TrackRef::new("c", "C", "X"))
        .expect("add c");
    assert_eq!((a.position, b.position, c.position), (0, 1, 2));

    let left = service::queue_remove(&state, &host.id, &room.id, &b.id).expect("remove b");
    assert_eq!(left.len(), 2);
    assert_eq!(left[0].track.key, "a");
    assert_eq!(left[0].position, 0);
    assert_eq!(left[1].track.key, "c");
    assert_eq!(left[1].position, 1);

    let missing = service::queue_remove(&state, &host.id, &room.id, &b.id);
    assert!(missing.is_err(), "quitar dos veces el mismo item falla");
}

#[test]
fn ready_with_all_members_schedules_start_at() {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = test_state(&dir);
    let host = login(&state, "ana").user;
    let room = service::create_room(
        &state,
        &host,
        &CreateRoomRequest {
            name: "party".to_string(),
            is_public: true,
        },
    )
    .expect("crear sala");
    service::queue_add(
        &state,
        &host.id,
        &room.id,
        TrackRef::new("isrc:AAA", "Uno", "Artista"),
    )
    .expect("queue add");
    control(&state, &host.id, &room.id, ControlAction::Play, None);

    let before = service::now_ms();
    let outcome = service::mark_ready(&state, &host.id, &room.id, "isrc:AAA", 500).expect("ready");
    assert!(matches!(outcome, ReadyOutcome::Started));

    let stored = state.db.room(&room.id).expect("consulta").expect("sala");
    let start_at = stored.state.start_at_ms.expect("start_at programado");
    assert!(start_at >= before + service::START_DELAY_MS);
    assert!(stored.state.playing);
    assert_eq!(stored.state.position_ms, 0);
    assert_eq!(stored.state.anchor_ms, start_at);
    assert!(stored.members.iter().all(|m| !m.ready), "ready se resetea");

    // Un ready tardío no debe reanclar la pista que ya está en marcha.
    let _ = service::mark_ready(&state, &host.id, &room.id, "isrc:AAA", 500).expect("ready tardío");
    let after = state.db.room(&room.id).expect("consulta").expect("sala");
    assert_eq!(after.state.start_at_ms, Some(start_at));
    assert_eq!(after.state.anchor_ms, start_at);

    let mismatch = service::mark_ready(&state, &host.id, &room.id, "otra", 500);
    assert!(mismatch.is_err());
}
