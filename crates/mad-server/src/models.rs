use serde::{Deserialize, Serialize};

// ControlCommand is now imported from mad_core

#[derive(Deserialize)]
pub struct ChatRequest {
    pub instruction: String,
}

#[derive(Serialize)]
pub struct ChatResponse {
    pub message: String,
    pub actions_performed: Vec<String>,
}
