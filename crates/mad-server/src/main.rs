use mad_server::run_server;
use tracing::info;

fn main() {
    // Initialize environment variables (load .env, set HF_ENDPOINT)
    // This must be done before any threads/runtime start.
    mad_server::config::init_env();

    // Initialize tracing
    tracing_subscriber::fmt::init();

    info!("Starting MAD Server...");

    // Start tokio runtime
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            if let Err(e) = run_server().await {
                tracing::error!("MAD Server exited with error: {:?}", e);
            }
        });
}
