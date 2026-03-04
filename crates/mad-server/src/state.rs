use crate::memory::MemoryStore;
use anyhow::Context;
use mad_core::types::ControlCommand;
use mad_core::{AccessibilityScanner, ComputerController, DeepseekClient};
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio::time::{Duration, timeout};
use tracing::{debug, error, info};

pub struct ControlRequest {
    pub cmd: ControlCommand,
    pub done_tx: oneshot::Sender<Result<(), String>>,
}

#[derive(Clone)]
pub struct AppState {
    pub client: Arc<DeepseekClient>,
    pub command_tx: mpsc::Sender<ControlRequest>,
    pub model: String,
    pub port: Option<u16>,
    pub memory: Option<Arc<Mutex<MemoryStore>>>,
}

pub async fn setup_app() -> anyhow::Result<(AppState, mpsc::Receiver<ControlRequest>)> {
    // Create channel for communication
    let (tx, rx) = mpsc::channel::<ControlRequest>(100);

    // Load config from ~/.open-mad/config.toml
    let config = crate::config::load_config();

    // Initialize Memory Store (Optional)
    let memory_model = config.get_memory_model();
    let memory_model_path = config.get_memory_model_path();
    let memory = if let Some(path) = memory_model_path {
        match tokio::task::spawn_blocking(move || MemoryStore::new(&memory_model, Some(path))).await
        {
            Ok(Ok(store)) => Some(Arc::new(Mutex::new(store))),
            Ok(Err(e)) => {
                error!("Failed to initialize MemoryStore: {}", e);
                None
            }
            Err(e) => {
                error!("Failed to join MemoryStore task: {}", e);
                None
            }
        }
    } else {
        info!("MemoryStore disabled: no local model path provided.");
        None
    };

    // Get API Key from env or config
    let api_key = std::env::var("DEEPSEEK_API_KEY")
        .ok()
        .or(config.get_api_key())
        .context("Missing API Key. Please set DEEPSEEK_API_KEY env var or configure it in ~/.open-mad/config.toml")?;

    let base_url = std::env::var("MAD_BASE_URL").ok().or(config.get_base_url());

    let model = std::env::var("MAD_MODEL")
        .ok()
        .or(config.get_model())
        .unwrap_or_else(|| "deepseek-chat".to_string());

    let client = Arc::new(DeepseekClient::new(api_key, base_url, Some(model.clone())));

    let port = std::env::var("MAD_PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .or(config.port);

    Ok((
        AppState {
            client,
            command_tx: tx,
            model,
            port,
            memory,
        },
        rx,
    ))
}

pub async fn run_controller_loop(mut rx: mpsc::Receiver<ControlRequest>) {
    info!("Starting controller loop in current thread...");

    // Initialize controller once
    let mut controller = match ComputerController::new() {
        Ok(c) => c,
        Err(e) => {
            error!("Failed to initialize controller: {}", e);
            return;
        }
    };

    loop {
        debug!("Waiting for next command...");
        match timeout(Duration::from_secs(20), rx.recv()).await {
            Ok(Some(req)) => {
                debug!("Received command: {:?}", req.cmd);
                let result = process_command(&mut controller, req.cmd).await;
                if req.done_tx.send(result).is_err() {
                    debug!("Command completion receiver dropped");
                }
                debug!("Finished processing command");
            }
            Ok(None) => {
                info!("Command channel closed, exiting controller loop");
                break;
            }
            Err(_) => {
                debug!("Controller idle: no command received for 20s");
            }
        }
    }
}

pub async fn process_command(
    controller: &mut ComputerController,
    cmd: ControlCommand,
) -> Result<(), String> {
    debug!("Processing command: {:?}", cmd);

    match cmd {
        ControlCommand::MoveMouse(x, y) => {
            info!("Executing MoveMouse({}, {})", x, y);
            // Small delay to ensure mouse is positioned
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            if let Err(e) = controller.move_mouse(x, y) {
                error!("Move mouse failed: {}", e);
                return Err(format!("MoveMouse failed: {}", e));
            }
        }
        ControlCommand::Click => {
            info!("Executing Click");
            if let Err(e) = controller.click() {
                error!("Click failed: {}", e);
                return Err(format!("Click failed: {}", e));
            }
        }
        ControlCommand::Type(text) => {
            info!("Executing Type('{}')", text);
            if let Err(e) = controller.type_text(&text).await {
                error!("Type failed: {}", e);
                return Err(format!("Type failed: {}", e));
            }
        }
        ControlCommand::Paste(text) => {
            info!("Executing Paste('{}')", text);
            if let Err(e) = controller.paste_text(&text).await {
                error!("Paste failed: {}", e);
                return Err(format!("Paste failed: {}", e));
            }
        }
        ControlCommand::KeySequence(keys) => {
            info!("Executing KeySequence({:?})", keys);
            if let Err(e) = controller.key_sequence(keys).await {
                error!("KeySequence failed: {}", e);
                return Err(format!("KeySequence failed: {}", e));
            }
        }
        ControlCommand::Wait(s) => {
            let max_wait_seconds = std::env::var("MAD_MAX_WAIT_SECONDS")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(5);
            let wait_seconds = s.min(max_wait_seconds);
            info!("Executing Wait({}s), clamped from {}s", wait_seconds, s);
            tokio::time::sleep(Duration::from_secs(wait_seconds)).await;
        }
        ControlCommand::Minimize => {
            info!("Executing Minimize");
            match AccessibilityScanner::minimize_active_window() {
                Ok(_) => info!("Minimized active window"),
                Err(e) => {
                    error!("Minimize failed: {}", e);
                    return Err(format!("Minimize failed: {}", e));
                }
            }
        }
        ControlCommand::ClickComponent(component) => {
            info!("Executing ClickComponent('{}')", component);
            if let Some((x, y)) = AccessibilityScanner::find_element_center(&component) {
                info!("Found component '{}' at ({}, {})", component, x, y);
                // Move mouse
                if let Err(e) = controller.move_mouse(x, y) {
                    error!("Move mouse failed: {}", e);
                    return Err(format!("ClickComponent move failed: {}", e));
                }

                // Small delay to ensure mouse is positioned
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;

                // Click
                if let Err(e) = controller.click() {
                    error!("Click failed: {}", e);
                    return Err(format!("ClickComponent click failed: {}", e));
                }
            } else {
                error!("Component '{}' not found", component);
                return Err(format!("Component '{}' not found", component));
            }
        }
        ControlCommand::Inspect => {
            info!("Executing Inspect (acknowledged)");
        }
        ControlCommand::Screenshot => {
            info!("Executing Screenshot");
            // Just logging for now as the capture happens in the chat loop
        }
    }
    Ok(())
}
