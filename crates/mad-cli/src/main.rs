use clap::Parser;
use mad_server::{process_chat, run_controller_loop, run_server, setup_app};
use std::io::{self, Write};

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    /// Run in interactive mode
    #[arg(short, long)]
    interactive: bool,
}

fn main() {
    // Initialize environment variables (load .env, set HF_ENDPOINT)
    // This must be done before any threads/runtime start.
    mad_server::config::init_env();

    let cli = Cli::parse();

    if cli.interactive {
        // Only show errors in interactive mode to keep UI clean
        tracing_subscriber::fmt().with_env_filter("error").init();
    } else {
        tracing_subscriber::fmt::init();
    }

    // Start tokio runtime
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            run_cli(cli).await;
        });
}

async fn run_cli(cli: Cli) {
    if cli.interactive {
        println!("Starting MAD CLI in interactive mode...");
        let (state, rx) = match setup_app().await {
            Ok(v) => v,
            Err(e) => {
                eprintln!("Failed to setup app: {:?}", e);
                std::process::exit(1);
            }
        };

        // Spawn controller loop in background thread for CLI mode
        std::thread::spawn(async move || {
            let _ = run_controller_loop(rx).await;
        });

        println!("Type your instruction and press Enter. Type 'exit' to quit.");

        loop {
            print!("User: ");
            io::stdout().flush().unwrap();

            let mut input = String::new();
            if io::stdin().read_line(&mut input).is_err() {
                break;
            }

            let input = input.trim();
            if input.eq_ignore_ascii_case("exit") {
                break;
            }
            if input.is_empty() {
                continue;
            }

            let response = process_chat(state.clone(), input.to_string()).await;

            // Format the output
            if !response.actions_performed.is_empty() {
                println!("AI: Executed actions:");
                for action in response.actions_performed {
                    println!("  - {}", action);
                }
            } else {
                // Try to see if it's just a message
                println!("AI: {}", response.message);
            }
            println!();
        }
    } else {
        println!("Starting MAD CLI server...");
        let _ = run_server().await;
    }
}
