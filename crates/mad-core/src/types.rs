use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ControlCommand {
    MoveMouse(i32, i32),
    Click,
    Type(String),
    Paste(String),
    KeySequence(Vec<String>),
    Wait(u64),
    Minimize,
    ClickComponent(String),
    Inspect,
    Screenshot,
}
