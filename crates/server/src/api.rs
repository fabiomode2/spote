//! API REST bajo `/api/v1`.
//!
//! Todas las rutas de esta API pasan por el middleware de clave de cliente.
//! `/healthz` queda fuera y solo devuelve un latido sin datos.

use axum::{
    extract::{
        rejection::{JsonRejection, PathRejection, QueryRejection},
        Path, Query, State,
    },
    http::{header, HeaderName, HeaderValue, Method, Request},
    middleware,
    routing::{delete, get, post, put},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use spote_protocol::{
    ChatMsg, ControlRequest, CreateRoomRequest, LoginRequest, LoginResponse, Presence, QueueItem,
    Room, SyncPullResponse, SyncPushRequest, SyncPushResponse, TimeResponse, TrackRef, User,
};
use tower_http::{
    cors::CorsLayer, limit::RequestBodyLimitLayer, set_header::SetResponseHeaderLayer,
    trace::TraceLayer,
};

use crate::{
    auth::{self, AuthUser, ClientKeyId, SessionTokenHash},
    error::AppError,
    service,
    state::AppState,
    ws,
};

/// Un push de sincronización lleva metadatos de varias pistas y playlists.
/// El límite lógico real lo pone `sync::MAX_SYNC_OPS`.
pub const MAX_BODY_BYTES: usize = 1024 * 1024;

pub fn router(state: AppState) -> Router {
    let mut api = Router::new()
        .route("/time", get(time))
        .route("/auth/login", post(login))
        .route("/auth/logout", post(logout))
        .route("/me", get(me))
        .route("/me/avatar", put(update_avatar))
        .route("/users", get(users))
        .route("/friends", get(list_friends))
        .route("/friends/{id}", post(add_friend))
        .route("/sync/snapshot", get(sync_snapshot))
        .route("/sync/pull", get(sync_pull))
        .route("/sync/push", post(sync_push))
        .route("/rooms", get(list_rooms).post(create_room))
        .route("/rooms/{id}", get(get_room))
        .route("/rooms/{id}/join", post(join_room))
        .route("/rooms/{id}/leave", post(leave_room))
        .route("/rooms/{id}/control", post(control))
        .route("/rooms/{id}/queue", post(queue_add))
        .route("/rooms/{id}/queue/{item}", delete(queue_remove))
        .route("/rooms/{id}/chat", get(chat))
        .route("/rooms/{id}/members", post(add_member))
        .route("/rooms/{id}/members/{user}", delete(kick_member))
        .route("/ws", get(ws::upgrade))
        .fallback(api_not_found)
        // El orden importa: las cabeceras envuelven a todo (también los 401),
        // la clave se comprueba antes de leer el cuerpo y el límite de cuerpo
        // queda por dentro. Cualquier ruta no encontrada bajo /api/v1 también
        // pasa por el middleware y responde 401 sin clave, no 404.
        .layer(RequestBodyLimitLayer::new(MAX_BODY_BYTES))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::require_client_key,
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-store"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::X_FRAME_OPTIONS,
            HeaderValue::from_static("DENY"),
        ))
        .with_state(state.clone());

    if let Some(origin) = state
        .config
        .cors_origin
        .as_deref()
        .and_then(|origin| origin.parse::<HeaderValue>().ok())
    {
        api = api.layer(
            CorsLayer::new()
                .allow_origin(origin)
                .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE])
                .allow_headers([
                    header::AUTHORIZATION,
                    header::CONTENT_TYPE,
                    HeaderName::from_static(auth::CLIENT_KEY_HEADER),
                ]),
        );
    }

    Router::new()
        .route("/healthz", get(healthz))
        .nest("/api/v1", api)
        .layer(
            TraceLayer::new_for_http().make_span_with(|request: &Request<axum::body::Body>| {
                // Nunca loguear query strings ni cabeceras: por ahí viajan
                // tokens y claves en clientes mal escritos.
                tracing::info_span!(
                    "http",
                    method = %request.method(),
                    path = %request.uri().path()
                )
            }),
        )
        .with_state(state)
}

#[derive(Debug, Deserialize)]
struct QueueAddRequest {
    track: TrackRef,
}

#[derive(Debug, Deserialize)]
struct ChatQuery {
    limit: Option<u32>,
}

/// `{"avatar": "<data URL>"}` o `{"avatar": null}`. El `null` explícito borra
/// el avatar; un campo ausente o de otro tipo es un 400.
#[derive(Debug, Deserialize)]
struct AvatarRequest {
    avatar: AvatarValue,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum AvatarValue {
    Set(String),
    Clear,
}

#[derive(Debug, Serialize)]
struct OkResponse {
    ok: bool,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    ok: bool,
    version: &'static str,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn api_not_found() -> AppError {
    AppError::NotFound("not found".to_string())
}

async fn healthz(State(state): State<AppState>) -> Result<Json<HealthResponse>, AppError> {
    state.db.ping()?;
    Ok(Json(HealthResponse {
        ok: true,
        version: env!("CARGO_PKG_VERSION"),
    }))
}

async fn time() -> Json<TimeResponse> {
    Json(TimeResponse {
        t: service::now_ms(),
    })
}

async fn login(
    State(state): State<AppState>,
    key: ClientKeyId,
    body: Result<Json<LoginRequest>, JsonRejection>,
) -> Result<Json<LoginResponse>, AppError> {
    let Json(req) = body.map_err(AppError::from)?;
    Ok(Json(service::login(&state, &req, &key.0)?))
}

async fn logout(
    State(state): State<AppState>,
    _auth: AuthUser,
    session: SessionTokenHash,
) -> Result<Json<OkResponse>, AppError> {
    state.db.revoke_session(&session.0)?;
    Ok(Json(OkResponse { ok: true }))
}

async fn me(auth: AuthUser) -> Json<User> {
    Json(auth.0)
}

async fn update_avatar(
    State(state): State<AppState>,
    auth: AuthUser,
    body: Result<Json<AvatarRequest>, JsonRejection>,
) -> Result<Json<User>, AppError> {
    let Json(req) = body.map_err(AppError::from)?;
    let avatar = match req.avatar {
        AvatarValue::Set(avatar) => Some(avatar),
        AvatarValue::Clear => None,
    };
    Ok(Json(service::set_avatar(&state, &auth.0, avatar)?))
}

async fn users(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> Result<Json<Vec<Presence>>, AppError> {
    Ok(Json(service::users(&state)?))
}

async fn list_friends(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Vec<User>>, AppError> {
    Ok(Json(service::friends(&state, &auth.0.id)?))
}

async fn add_friend(
    State(state): State<AppState>,
    path: Result<Path<String>, PathRejection>,
    auth: AuthUser,
) -> Result<Json<User>, AppError> {
    let Path(target_id) = path.map_err(AppError::from)?;
    Ok(Json(service::add_friend(&state, &auth.0.id, &target_id)?))
}

async fn sync_snapshot(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<SyncPullResponse>, AppError> {
    Ok(Json(crate::sync::snapshot(&state, &auth.0.id)?))
}

#[derive(Debug, Deserialize)]
struct SyncSinceQuery {
    since: i64,
}

async fn sync_pull(
    State(state): State<AppState>,
    query: Result<Query<SyncSinceQuery>, QueryRejection>,
    auth: AuthUser,
) -> Result<Json<SyncPullResponse>, AppError> {
    let Query(query) = query.map_err(AppError::from)?;
    Ok(Json(crate::sync::pull(&state, &auth.0.id, query.since)?))
}

async fn sync_push(
    State(state): State<AppState>,
    auth: AuthUser,
    body: Result<Json<SyncPushRequest>, JsonRejection>,
) -> Result<Json<SyncPushResponse>, AppError> {
    let Json(req) = body.map_err(AppError::from)?;
    Ok(Json(crate::sync::push(&state, &auth.0.id, &req)?))
}

async fn list_rooms(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Vec<Room>>, AppError> {
    Ok(Json(service::list_rooms(&state, &auth.0.id)?))
}

async fn create_room(
    State(state): State<AppState>,
    auth: AuthUser,
    body: Result<Json<CreateRoomRequest>, JsonRejection>,
) -> Result<Json<Room>, AppError> {
    let Json(req) = body.map_err(AppError::from)?;
    Ok(Json(service::create_room(&state, &auth.0, &req)?))
}

async fn get_room(
    State(state): State<AppState>,
    path: Result<Path<String>, PathRejection>,
    auth: AuthUser,
) -> Result<Json<Room>, AppError> {
    let Path(room_id) = path.map_err(AppError::from)?;
    Ok(Json(service::room(&state, &auth.0.id, &room_id)?))
}

async fn join_room(
    State(state): State<AppState>,
    path: Result<Path<String>, PathRejection>,
    auth: AuthUser,
) -> Result<Json<Room>, AppError> {
    let Path(room_id) = path.map_err(AppError::from)?;
    Ok(Json(service::join_room(&state, &auth.0, &room_id)?))
}

async fn leave_room(
    State(state): State<AppState>,
    path: Result<Path<String>, PathRejection>,
    auth: AuthUser,
) -> Result<Json<OkResponse>, AppError> {
    let Path(room_id) = path.map_err(AppError::from)?;
    service::leave_room(&state, &auth.0.id, &room_id)?;
    Ok(Json(OkResponse { ok: true }))
}

async fn add_member(
    State(state): State<AppState>,
    path: Result<Path<String>, PathRejection>,
    auth: AuthUser,
    body: Result<Json<MemberRequest>, JsonRejection>,
) -> Result<Json<Room>, AppError> {
    let Path(room_id) = path.map_err(AppError::from)?;
    let Json(req) = body.map_err(AppError::from)?;
    let room = service::add_member(&state, &auth.0.id, &room_id, &req.user_id)?;
    Ok(Json(room))
}

async fn kick_member(
    State(state): State<AppState>,
    path: Result<Path<(String, String)>, PathRejection>,
    auth: AuthUser,
) -> Result<Json<Room>, AppError> {
    let Path((room_id, user_id)) = path.map_err(AppError::from)?;
    Ok(Json(service::kick_member(
        &state, &auth.0.id, &room_id, &user_id,
    )?))
}

#[derive(Debug, Deserialize)]
struct MemberRequest {
    user_id: String,
}

async fn control(
    State(state): State<AppState>,
    auth: AuthUser,
    path: Result<Path<String>, PathRejection>,
    body: Result<Json<ControlRequest>, JsonRejection>,
) -> Result<Json<ControlResponse>, AppError> {
    let Path(room_id) = path.map_err(AppError::from)?;
    let Json(req) = body.map_err(AppError::from)?;
    let outcome = service::control(&state, &auth.0.id, &room_id, &req)?;
    Ok(Json(ControlResponse {
        state: outcome.state,
        queue: outcome.queue,
    }))
}

#[derive(Debug, Serialize)]
struct ControlResponse {
    state: spote_protocol::RoomState,
    queue: Vec<QueueItem>,
}

async fn queue_add(
    State(state): State<AppState>,
    auth: AuthUser,
    path: Result<Path<String>, PathRejection>,
    body: Result<Json<QueueAddRequest>, JsonRejection>,
) -> Result<Json<QueueItem>, AppError> {
    let Path(room_id) = path.map_err(AppError::from)?;
    let Json(req) = body.map_err(AppError::from)?;
    Ok(Json(service::queue_add(
        &state, &auth.0.id, &room_id, req.track,
    )?))
}

async fn queue_remove(
    State(state): State<AppState>,
    path: Result<Path<(String, String)>, PathRejection>,
    auth: AuthUser,
) -> Result<Json<Vec<QueueItem>>, AppError> {
    let Path((room_id, item_id)) = path.map_err(AppError::from)?;
    Ok(Json(service::queue_remove(
        &state, &auth.0.id, &room_id, &item_id,
    )?))
}

async fn chat(
    State(state): State<AppState>,
    path: Result<Path<String>, PathRejection>,
    query: Result<Query<ChatQuery>, QueryRejection>,
    auth: AuthUser,
) -> Result<Json<Vec<ChatMsg>>, AppError> {
    let Path(room_id) = path.map_err(AppError::from)?;
    let Query(query) = query.map_err(AppError::from)?;
    service::room(&state, &auth.0.id, &room_id)?;
    let limit = query.limit.unwrap_or(service::CHAT_HISTORY_LIMIT);
    Ok(Json(state.db.chat_history(&room_id, limit)?))
}
