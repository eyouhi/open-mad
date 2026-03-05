use anyhow::Result;
use clap::{Parser, Subcommand};
use xshell::{Shell, cmd};

#[derive(Parser)]
#[command(name = "xtask")]
#[command(about = "Development automation tasks", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Bundle the application
    Bundle,
    /// Run checks (clippy, fmt)
    Check {
        /// Automatically apply fixes where possible
        #[arg(long)]
        fix: bool,
    },
    /// Run the application (defaults to mad-ui)
    Run {
        /// Run the CLI instead of the UI
        #[arg(long)]
        cli: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let sh = Shell::new()?;

    match cli.command {
        Commands::Check { fix } => {
            if fix {
                cmd!(sh, "cargo fmt --all").run()?;
                cmd!(
                    sh,
                    "cargo clippy --workspace --all-targets --all-features --fix --allow-dirty --allow-staged"
                )
                .run()?;
            } else {
                cmd!(sh, "cargo fmt --all -- --check").run()?;
                cmd!(
                    sh,
                    "cargo clippy --workspace --all-targets --all-features -- -D warnings"
                )
                .run()?;
            }
        }
        Commands::Bundle => {
            println!("Bundling application...");
            cmd!(
                sh,
                "dx bundle --package mad-ui --platform desktop --release"
            )
            .run()?;
            println!("Bundle complete!");
        }
        Commands::Run { cli } => {
            if cli {
                println!("Running mad-cli...");
                cmd!(sh, "cargo run -p mad-cli --bin mad-cli").run()?;
            } else {
                println!("Running mad-ui (Dioxus)...");
                sh.change_dir("crates/mad-ui");

                if std::env::var("RUST_LOG").is_err() {
                    let _env = sh.push_env(
                        "RUST_LOG",
                        "mad_ui=debug,mad_server=debug,mad_core=debug,mad_skills=debug,tower_http=debug",
                    );
                    cmd!(sh, "cargo run  -p mad-ui").run()?;
                } else {
                    cmd!(sh, "cargo run  -p mad-ui").run()?;
                }
            }
        }
    }

    Ok(())
}
