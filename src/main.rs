use env_logger::Target;
use eyre::{Report, Result};
use log::info;
use otto::RuntimeConfig;
use otto::cli::Parser;
use std::env;
use std::fs::OpenOptions;

fn setup_logging() -> Result<(), Report> {
    let log_dir = dirs::data_local_dir()
        .ok_or_else(|| eyre::eyre!("Could not determine local data directory"))?
        .join("otto")
        .join("logs");

    std::fs::create_dir_all(&log_dir)?;
    let log_file_path = log_dir.join("otto.log");

    let log_file = OpenOptions::new().create(true).append(true).open(&log_file_path)?;

    env_logger::Builder::from_env(env_logger::Env::default().filter_or("RUST_LOG", "info"))
        .target(Target::Pipe(Box::new(log_file)))
        .init();

    Ok(())
}

#[tokio::main]
async fn main() {
    if let Err(e) = setup_logging() {
        eprintln!("Failed to setup logging: {e}");
        std::process::exit(1);
    }
    info!("Starting otto");

    let args: Vec<String> = env::args().collect();

    // Handle hidden --is-valid-ottofile arg early (before normal parsing)
    if let Some(exit_code) = handle_is_valid_ottofile(&args) {
        std::process::exit(exit_code);
    }

    // Handle subcommands that use their own clap parsers
    if args.len() > 1
        && let Some(result) = handle_subcommand(&args).await
    {
        if let Err(e) = result {
            eprintln!("{e}");
            std::process::exit(1);
        }
        return;
    }

    // Parse and run main command
    let mut parser = match Parser::new(args) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };

    let config = match RuntimeConfig::from_parser(&mut parser) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };

    if let Err(e) = otto::run(config).await {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

/// Handle --is-valid-ottofile argument. Returns Some(exit_code) if handled.
fn handle_is_valid_ottofile(args: &[String]) -> Option<i32> {
    for (i, arg) in args.iter().enumerate() {
        if arg == "--is-valid-ottofile" {
            if let Some(path_arg) = args.get(i + 1) {
                let filename = std::path::Path::new(path_arg)
                    .file_name()
                    .and_then(|f| f.to_str())
                    .unwrap_or(path_arg);
                if otto::cli::is_valid_ottofile_name(filename) {
                    return Some(0);
                }
            }
            return Some(1);
        } else if let Some(path_arg) = arg.strip_prefix("--is-valid-ottofile=") {
            let filename = std::path::Path::new(path_arg)
                .file_name()
                .and_then(|f| f.to_str())
                .unwrap_or(path_arg);
            if otto::cli::is_valid_ottofile_name(filename) {
                return Some(0);
            }
            return Some(1);
        }
    }
    None
}

/// Handle subcommands that use their own clap parsers. Returns Some(result) if handled.
async fn handle_subcommand(args: &[String]) -> Option<Result<(), Report>> {
    match args[1].as_str() {
        "Clean" => Some(otto::app::execute_clean_command(&args[1..]).await),
        "Convert" => Some(otto::app::execute_convert_command(&args[1..])),
        "History" => Some(otto::app::execute_history_command(&args[1..])),
        "Stats" => Some(otto::app::execute_stats_command(&args[1..])),
        "Upgrade" => Some(otto::app::execute_upgrade_command(&args[1..]).await),
        _ => None,
    }
}
