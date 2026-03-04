use crate::state::AppState;
use axum::{
    Router,
    routing::{get, post},
};
use tower_http::cors::CorsLayer;

pub mod chat;
pub mod screenshot;

use chat::{chat_handler, chat_stream_handler};
use screenshot::screenshot_handler;

pub fn create_router(state: AppState) -> Router {
    Router::new()
        .route("/api/chat", post(chat_handler))
        .route("/api/chat/stream", post(chat_stream_handler))
        .route("/api/screenshot", get(screenshot_handler))
        .layer(CorsLayer::permissive())
        .with_state(state)
}
