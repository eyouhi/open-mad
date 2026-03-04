pub mod api;
pub mod config;
pub mod memory;
pub mod models;
pub mod state;

pub use api::chat::process_chat;
pub use api::create_router;
pub use mad_core::{ComputerController, ControlCommand};
pub use models::{ChatRequest, ChatResponse};
pub use state::{AppState, process_command, run_controller_loop, setup_app};

use tracing::{error, info};

pub async fn run_server() -> anyhow::Result<()> {
    let (state, rx) = match setup_app().await {
        Ok(v) => v,
        Err(e) => {
            tracing::error!("Failed to setup app: {:?}", e);
            return Err(e);
        }
    };

    let config = config::load_config();
    let socket_path = std::env::var("MAD_SOCKET_PATH")
        .ok()
        .or_else(|| config.get_socket_path())
        .unwrap_or_else(config::default_socket_path);
    let socket_path_buf = std::path::PathBuf::from(&socket_path);
    if let Some(parent) = socket_path_buf.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    if socket_path_buf.exists() {
        match tokio::fs::remove_file(&socket_path_buf).await {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }

    let app = create_router(state);

    let listener = match tokio::net::UnixListener::bind(&socket_path_buf) {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("Failed to bind unix socket {}: {}", socket_path, e);
            return Err(e.into());
        }
    };
    info!("Listening on unix socket {}", socket_path);

    let _controller_thread = std::thread::Builder::new()
        .name("mad-controller".to_string())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    error!("Failed to build controller runtime: {}", e);
                    return;
                }
            };
            runtime.block_on(run_controller_loop(rx));
        })
        .map_err(|e| anyhow::anyhow!("Failed to spawn controller thread: {}", e))?;

    match axum::serve(listener, app).await {
        Ok(_) => info!("Server shut down gracefully"),
        Err(e) => error!("Server error: {}", e),
    }

    Ok(())
}
