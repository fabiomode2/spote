//! Migración desde la base de datos anterior (sin claves de cliente ni
//! caducidad de sesión): no debe perder usuarios ni arrancar rota.

use rusqlite::Connection;
use spote_protocol::LoginRequest;
use spote_server::{config::Config, service, state::AppState};

const INVITE: &str = "test-invite-0123456789abcdef";
const KEY_A: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";
const KEY_B: &str = "b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90a1";

#[test]
fn old_database_is_upgraded_and_old_sessions_are_invalidated() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("old.db");

    {
        let conn = Connection::open(&db_path).expect("sqlite");
        conn.execute_batch(
            r#"
            CREATE TABLE users (
                id          TEXT PRIMARY KEY,
                name        TEXT NOT NULL UNIQUE,
                avatar      TEXT,
                invite_code TEXT NOT NULL,
                created_at  INTEGER NOT NULL
            );
            CREATE TABLE sessions (
                token      TEXT PRIMARY KEY,
                user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                created_at INTEGER NOT NULL
            );
            INSERT INTO users (id, name, avatar, invite_code, created_at)
                VALUES ('u1', 'ana', NULL, 'spote', 1);
            INSERT INTO sessions (token, user_id, created_at)
                VALUES ('old-token', 'u1', 1);
            "#,
        )
        .expect("esquema viejo");
    }

    let config = Config::from_lookup(|key| match key {
        "SPOTE_INVITE" => Some(INVITE.to_string()),
        "SPOTE_CLIENT_KEYS" => Some(format!("ana:{KEY_A},bea:{KEY_B}")),
        "SPOTE_DB" => Some(db_path.display().to_string()),
        "SPOTE_BIND" => Some("127.0.0.1:0".to_string()),
        _ => None,
    })
    .expect("config");
    let state = AppState::new(config).expect("migración");

    let fp_a = state.config.client_key_fingerprint(KEY_A).expect("huella");
    let fp_b = state.config.client_key_fingerprint(KEY_B).expect("huella");

    // La sesión vieja no se puede reutilizar (no estaba hasheada).
    assert!(state
        .db
        .user_by_token("old-token", &fp_a)
        .expect("consulta")
        .is_none());

    // El primer login con la clave A ata el usuario antiguo a esa clave.
    let login = service::login(
        &state,
        &LoginRequest {
            name: "ana".to_string(),
            invite: INVITE.to_string(),
        },
        &fp_a,
    )
    .expect("login migrado");
    assert_eq!(login.user.id, "u1");
    assert!(state
        .db
        .user_by_token(&login.token, &fp_a)
        .expect("consulta")
        .is_some());
    assert!(state
        .db
        .user_by_token(&login.token, &fp_b)
        .expect("consulta")
        .is_none());
}
