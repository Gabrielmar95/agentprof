mod analyzer;
mod hook;
mod installer;
mod logger;
mod tokens;
mod web;

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "agentprof", about = "Claude Code tool call profiler")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Analyze a JSONL log file and print a profiling report
    Analyze {
        /// Path to the JSONL log file
        log_file: PathBuf,
    },
    /// Log a Claude Code hook event (reads hook JSON from stdin)
    Hook {
        /// Path to the JSONL log file
        #[arg(short, long)]
        log: PathBuf,
    },
    /// Launch a web dashboard to explore profiling data
    Web {
        /// Path to the JSONL log file
        log_file: PathBuf,

        /// Port for the web server
        #[arg(short, long, default_value_t = 8080)]
        port: u16,
    },
    /// Install agentprof hooks into Claude Code settings
    Install {
        /// Path to the JSONL log file
        #[arg(short, long)]
        log: PathBuf,

        /// Install into global ~/.claude/settings.json
        #[arg(long, conflicts_with = "local")]
        global: bool,

        /// Install into local .claude/settings.local.json (default)
        #[arg(long)]
        local: bool,
    },
    /// Remove agentprof hooks from Claude Code settings
    Uninstall {
        /// Remove from global ~/.claude/settings.json
        #[arg(long, conflicts_with = "local")]
        global: bool,

        /// Remove from local .claude/settings.local.json (default)
        #[arg(long)]
        local: bool,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    match cli.command {
        Commands::Analyze { log_file } => match analyzer::analyze(&log_file) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("agentprof: analysis failed: {}", e);
                ExitCode::from(1)
            }
        },
        Commands::Hook { log } => hook::run_hook(&log),
        Commands::Web { log_file, port } => {
            let rt = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
            match rt.block_on(web::run_server(&log_file, port)) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("agentprof: web server failed: {}", e);
                    ExitCode::from(1)
                }
            }
        }
        Commands::Install {
            log,
            global,
            local: _,
        } => match installer::install(&log, global) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("agentprof: install failed: {}", e);
                ExitCode::from(1)
            }
        },
        Commands::Uninstall { global, local: _ } => match installer::uninstall(global) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("agentprof: uninstall failed: {}", e);
                ExitCode::from(1)
            }
        },
    }
}
