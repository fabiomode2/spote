//! API REST bajo `/api/v1`.

use axum::{
    extract::{
        rejection::{JsonRejection, PathRejection, QueryRejection},
        Path, Query, State,
    },
    routing::{delete, get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use spote_protocol::{
    ChatMsg, ControlRequest, CreateRoomRequest, LoginRequest, LoginResponse, Presence, QueueItem,
    Room, TimeResponse, TrackRef, User,
};
use tower_http::{cors::CorsLayer, trace::TraceLayer};

use crate::{auth::AuthUser, error::AppError, service, state::AppState, ws};

pub fn router(state: AppState) -> Router {
    let api = Router::new()
        .route("/time", get(time))
        .route("/auth/login", post(login))
        .route("/me", get(me))
        .route("/users", get(users))
        .route("/friends/{id}", post(add_friend))
        .route("/rooms", get(list_rooms).post(create_room))
        .route("/rooms/{id}", get(get_room))
        .route("/rooms/{id}/join", post(join_room))
        .route("/rooms/{id}/leave", post(leave_room))
        .route("/rooms/{id}/control", post(control))
        .route("/rooms/{id}/queue", post(queue_add))
        .route("/rooms/{id}/queue/{item}", delete(queue_remove))
        .route("/rooms/{id}/chat", get(chat))
        .route("/ws", get(ws::upgrade));

    Router::new()
        .nest("/api/v1", api)
        .with_state(state)
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
}

#[derive(Debug, Deserialize)]
struct QueueAddRequest {
    track: TrackRef,
}

#[derive(Debug, Deserialize)]
struct ChatQuery {
    limit: Option<u32>,
}

#[derive(Debug, Serialize)]
struct OkResponse {
    ok: bool,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn time() -> Json<TimeResponse> {
    Json(TimeResponse {
        t: service::now_ms(),
    })
}

async fn login(
    State(state): State<AppState>,
    body: Result<Json<LoginRequest>, JsonRejection>,
) -> Result<Json<LoginResponse>, AppError> {
    let Json(req) = body.map_err(AppError::from)?;
    Ok(Json(service::login(&state, &req)?))
}

async fn me(auth: AuthUser) -> Json<User> {
    Json(auth.0)
}

async fn users(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> Result<Json<Vec<Presence>>, AppError> {
    Ok(Json(service::users(&state)?))
}

async fn add_friend(
    State(state): State<AppState>,
    path: Result<Path<String>, PathRejection>,
    auth: AuthUser,
) -> Result<Json<User>, AppError> {
    let Path(target_id) = path.map_err(AppError::from)?;
    Ok(Json(service::add_friend(&state, &auth.0.id, &target_id)?))
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
