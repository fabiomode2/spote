//! Tests de la lógica de negocio (sin red): login, salas, control y cola.

use axum::http::StatusCode;
use spote_protocol::{
    ControlAction, ControlRequest, CreateRoomRequest, LoginRequest, LoginResponse, Role, TrackRef,
};
use spote_server::{config::Config, service, service::ReadyOutcome, state::AppState};

const INVITE: &str = "test-invite-0123456789abcdef";
const KEY_A: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";
const KEY_B: &str = "b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90a1";

fn test_state(dir: &tempfile::TempDir) -> AppState {
    let config = Config::from_lookup(|key| match key {
        "SPOTE_INVITE" => Some(INVITE.to_string()),
        "SPOTE_CLIENT_KEYS" => Some(format!("ana:{KEY_A},bea:{KEY_B}")),
        "SPOTE_DB" => Some(dir.path().join("service-test.db").display().to_string()),
        "SPOTE_BIND" => Some("127.0.0.1:0".to_string()),
        _ => None,
    })
    .expect("config de test");
    AppState::new(config).expect("estado")
}

fn login_with(state: &AppState, name: &str, key: &str) -> LoginResponse {
    let fp = state
        .config
        .client_key_fingerprint(key)
        .expect("clave de test válida");
    service::login(
        state,
        &LoginRequest {
            name: name.to_string(),
            invite: INVITE.to_string(),
        },
        &fp,
    )
    .expect("login")
}

fn login(state: &AppState, name: &str) -> LoginResponse {
    login_with(state, name, KEY_A)
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
fn friends_are_mutual_and_listed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = test_state(&dir);

    let ana = login(&state, "ana");
    let bea = login_with(&state, "bea", KEY_B);
    assert!(service::friends(&state, &ana.user.id)
        .expect("amigos")
        .is_empty());

    let added = service::add_friend(&state, &ana.user.id, &bea.user.id).expect("añadir amigo");
    assert_eq!(added.id, bea.user.id);

    for (owner, friend) in [(&ana, &bea), (&bea, &ana)] {
        let friends = service::friends(&state, &owner.user.id).expect("amigos");
        assert_eq!(friends.len(), 1);
        assert_eq!(friends[0].id, friend.user.id);
    }
}

#[test]
fn login_creates_user_and_is_idempotent_by_name() {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = test_state(&dir);

    let first = login(&state, "ana");
    assert_eq!(first.user.name, "ana");
    assert!(!first.user.id.is_empty());
    assert!(!first.token.is_empty());
    assert!(first.invite.is_empty(), "la invitación no se devuelve");

    let second = login(&state, "ana");
    assert_eq!(first.user.id, second.user.id, "mismo usuario por nombre");
    assert_ne!(first.token, second.token, "token nuevo en cada login");

    let fp = state.config.client_key_fingerprint(KEY_A).expect("huella");
    let resolved = state
        .db
        .user_by_token(&second.token, &fp)
        .expect("consulta")
        .expect("sesión válida");
    assert_eq!(resolved.id, first.user.id);

    let other = login(&state, "bea");
    assert_ne!(other.user.id, first.user.id);
}

#[test]
fn login_rejects_bad_invite_empty_name_and_other_client_key() {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = test_state(&dir);
    let fp = state.config.client_key_fingerprint(KEY_A).expect("huella");

    let bad_invite = service::login(
        &state,
        &LoginRequest {
            name: "ana".to_string(),
            invite: "otra-invitacion-larga".to_string(),
        },
        &fp,
    )
    .expect_err("invite inválida");
    assert_eq!(bad_invite.status(), StatusCode::UNAUTHORIZED);

    let empty = service::login(
        &state,
        &LoginRequest {
            name: "   ".to_string(),
            invite: INVITE.to_string(),
        },
        &fp,
    )
    .expect_err("nombre vacío");
    assert_eq!(empty.status(), StatusCode::BAD_REQUEST);

    let control_char = service::login(
        &state,
        &LoginRequest {
            name: "ana\r\x1b]0;pwned".to_string(),
            invite: INVITE.to_string(),
        },
        &fp,
    )
    .expect_err("caracteres de control");
    assert_eq!(control_char.status(), StatusCode::BAD_REQUEST);
}

#[test]
fn a_name_cannot_be_reclaimed_with_another_client_key() {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = test_state(&dir);
    let fp_a = state
        .config
        .client_key_fingerprint(KEY_A)
        .expect("huella A");
    let fp_b = state
        .config
        .client_key_fingerprint(KEY_B)
        .expect("huella B");

    let first = service::login(
        &state,
        &LoginRequest {
            name: "ana".to_string(),
            invite: INVITE.to_string(),
        },
        &fp_a,
    )
    .expect("login A");

    let rejected = service::login(
        &state,
        &LoginRequest {
            name: "ana".to_string(),
            invite: INVITE.to_string(),
        },
        &fp_b,
    )
    .expect_err("otra clave no puede usar el nombre");
    assert_eq!(rejected.status(), StatusCode::UNAUTHORIZED);

    let wrong_session = state
        .db
        .user_by_token(&first.token, &fp_b)
        .expect("consulta");
    assert!(wrong_session.is_none(), "el token está atado a la clave");
}

#[test]
fn chat_and_track_metadata_reject_terminal_control_characters() {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = test_state(&dir);
    let host = login(&state, "ana").user;
    let room = service::create_room(
        &state,
        &host,
        &CreateRoomRequest {
            name: "chat".to_string(),
            is_public: true,
        },
    )
    .expect("crear sala");

    let escape = service::chat_add(&state, &host, &room.id, "hola\x1b]52;c;cHduZWQ=\x07");
    assert_eq!(escape.expect_err("ANSI").status(), StatusCode::BAD_REQUEST);
    let ok = service::chat_add(&state, &host, &room.id, "hola").expect("chat");
    assert_eq!(ok.text, "hola");

    let bad_url = service::queue_add(
        &state,
        &host.id,
        &room.id,
        TrackRef {
            key: "isrc:X".into(),
            title: "T".into(),
            artist: "A".into(),
            art_url: Some("javascript:alert(1)".into()),
            ..Default::default()
        },
    );
    assert_eq!(bad_url.expect_err("url").status(), StatusCode::BAD_REQUEST);

    let bad_title = service::queue_add(
        &state,
        &host.id,
        &room.id,
        TrackRef {
            key: "isrc:X".into(),
            title: "T\r\nX".into(),
            artist: "A".into(),
            ..Default::default()
        },
    );
    assert_eq!(
        bad_title.expect_err("título").status(),
        StatusCode::BAD_REQUEST
    );
}

#[test]
fn avatar_is_validated_persisted_and_reflected_in_presence() {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = test_state(&dir);
    let user = login(&state, "ana").user;
    assert!(user.avatar.is_none());

    let jpeg = format!("data:image/jpeg;base64,{}", "A".repeat(64));
    let updated = service::set_avatar(&state, &user, Some(jpeg.clone())).expect("avatar");
    assert_eq!(updated.avatar.as_deref(), Some(jpeg.as_str()));
    assert_eq!(
        state
            .db
            .user_by_id(&user.id)
            .expect("consulta")
            .expect("usuario")
            .avatar
            .as_deref(),
        Some(jpeg.as_str())
    );
    let presence = state.hub.presence_of(&user.id).expect("presencia");
    assert_eq!(presence.user.avatar.as_deref(), Some(jpeg.as_str()));

    // Una conexión WS abierta antes del cambio lleva el usuario cacheado; su
    // siguiente latido de presencia no debe revertir el avatar.
    service::touch_presence(&state, &user, None, false, true).expect("presencia");
    assert_eq!(
        state
            .hub
            .presence_of(&user.id)
            .expect("presencia")
            .user
            .avatar
            .as_deref(),
        Some(jpeg.as_str())
    );

    let cleared = service::set_avatar(&state, &user, None).expect("borrar avatar");
    assert!(cleared.avatar.is_none());
    assert!(state
        .db
        .user_by_id(&user.id)
        .expect("consulta")
        .expect("usuario")
        .avatar
        .is_none());
    assert!(state
        .hub
        .presence_of(&user.id)
        .expect("presencia")
        .user
        .avatar
        .is_none());

    let missing = service::set_avatar(
        &state,
        &spote_protocol::User {
            id: "no-existe".to_string(),
            name: "ghost".to_string(),
            avatar: None,
        },
        None,
    );
    assert_eq!(
        missing.expect_err("usuario inexistente").status(),
        StatusCode::NOT_FOUND
    );
}

#[test]
fn avatar_rejects_unsupported_types_empty_and_oversized_payloads() {
    let prefix = "data:image/png;base64,";
    let near_limit = (service::MAX_AVATAR_CHARS - prefix.len()) / 4 * 4;
    let accepted = format!("{prefix}{}", "A".repeat(near_limit));
    service::validate_avatar(&accepted).expect("cerca del límite");

    let oversized = format!("{prefix}{}", "A".repeat(service::MAX_AVATAR_CHARS));
    let rejected = [
        String::new(),
        "https://spote.example.com/ana.jpg".to_string(),
        "data:image/gif;base64,R0lGODlhAQABAAAAACw=".to_string(),
        "data:image/svg+xml;base64,PHN2Zz48L3N2Zz4=".to_string(),
        prefix.to_string(),
        format!("{prefix}AAA"),
        format!("{prefix}AAAA===="),
        format!("{prefix}AAA=AAAA"),
        format!("{prefix}AA AA"),
        format!("{prefix}AAAA\n"),
        oversized,
    ];
    for avatar in rejected {
        let error = service::validate_avatar(&avatar).expect_err("avatar inválido");
        assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    }
}

#[test]
fn expired_sessions_are_rejected() {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = test_state(&dir);
    let fp = state.config.client_key_fingerprint(KEY_A).expect("huella");
    let (_, token) = state
        .db
        .login("ana", &fp, service::now_ms() - 1)
        .expect("login caducado");
    let resolved = state.db.user_by_token(&token, &fp).expect("consulta");
    assert!(resolved.is_none());
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
fn queue_remove_only_allows_owner_or_host() {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = test_state(&dir);
    let host = login(&state, "ana").user;
    let guest = login(&state, "bea").user;
    let room = service::create_room(
        &state,
        &host,
        &CreateRoomRequest {
            name: "cola".to_string(),
            is_public: true,
        },
    )
    .expect("crear sala");
    service::join_room(&state, &guest, &room.id).expect("join");

    let item =
        service::queue_add(&state, &host.id, &room.id, TrackRef::new("a", "A", "X")).expect("add");
    let denied = service::queue_remove(&state, &guest.id, &room.id, &item.id);
    assert_eq!(
        denied.expect_err("guest no puede").status(),
        StatusCode::FORBIDDEN
    );
    let allowed = service::queue_remove(&state, &host.id, &room.id, &item.id).expect("host");
    assert!(allowed.is_empty());
}

#[test]
fn private_rooms_reject_outsiders() {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = test_state(&dir);
    let host = login(&state, "ana").user;
    let guest = login(&state, "bea").user;
    let room = service::create_room(
        &state,
        &host,
        &CreateRoomRequest {
            name: "privada".to_string(),
            is_public: false,
        },
    )
    .expect("crear sala");

    let denied = service::join_room(&state, &guest, &room.id);
    assert_eq!(denied.expect_err("privada").status(), StatusCode::FORBIDDEN);

    let added = service::add_member(&state, &host.id, &room.id, &guest.id).expect("add");
    assert_eq!(added.members.len(), 2);
    service::join_room(&state, &guest, &room.id).expect("ya es miembro");

    let kicked = service::kick_member(&state, &host.id, &room.id, &guest.id).expect("kick");
    assert_eq!(kicked.members.len(), 1);
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
