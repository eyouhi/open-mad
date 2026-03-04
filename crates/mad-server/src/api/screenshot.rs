use axum::Json;
use mad_core::ScreenCapture;
use serde_json::{Value, json};

pub async fn screenshot_handler() -> Json<Value> {
    match ScreenCapture::capture_main_base64() {
        Ok(img) => Json(json!({ "status": "ok", "image": img })),
        Err(e) => Json(json!({ "status": "error", "message": e.to_string() })),
    }
}
