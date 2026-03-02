use env_logger::Target;
use eyre::{Report, Result};
use log::info;
use otto::RuntimeConfig;
use otto::cli::Parser;
use std::env;
use std::fs::OpenOptions;

/// Default maximum log file size before rotation (10 MB).
const DEFAULT_MAX_LOG_BYTES: u64 = 10 * 1024 * 1024;

/// Rotate the log file if it exceeds the size threshold.
///
/// Renames `otto.log` to `otto.log.1` (one backup maximum).
/// Threshold can be overridden via `OTTO_MAX_LOG_BYTES` env var.
fn rotate_log_if_needed(log_file_path: &std::path::Path) {
    let max_bytes = std::env::var("OTTO_MAX_LOG_BYTES")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(DEFAULT_MAX_LOG_BYTES);

    if let Ok(meta) = std::fs::metadata(log_file_path)
        && meta.len() > max_bytes
    {
        let backup = log_file_path.with_extension("log.1");
        // Overwrite any existing backup
        let _ = std::fs::rename(log_file_path, backup);
    }
}

fn setup_logging() -> Result<(), Report> {
    let log_dir = dirs::data_local_dir()
        .ok_or_else(|| eyre::eyre!("Could not determine local data directory"))?
        .join("otto")
        .join("logs");

    std::fs::create_dir_all(&log_dir)?;
    let log_file_path = log_dir.join("otto.log");

    // Rotate log file before opening if it's too large
    rotate_log_if_needed(&log_file_path);

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_rotate_log_no_file() {
        let temp_dir = TempDir::new().unwrap();
        let log_path = temp_dir.path().join("otto.log");
        // Should not panic when file doesn't exist
        rotate_log_if_needed(&log_path);
        assert!(!log_path.exists());
    }

    #[test]
    fn test_rotate_log_small_file() {
        let temp_dir = TempDir::new().unwrap();
        let log_path = temp_dir.path().join("otto.log");
        fs::write(&log_path, "small content").unwrap();
        rotate_log_if_needed(&log_path);
        // File should still exist (not rotated)
        assert!(log_path.exists());
        assert!(!temp_dir.path().join("otto.log.1").exists());
    }

    #[test]
    fn test_rotate_log_oversized_file() {
        let temp_dir = TempDir::new().unwrap();
        let log_path = temp_dir.path().join("otto.log");
        // Create a file larger than 10MB
        let content = vec![b'x'; 11 * 1024 * 1024];
        fs::write(&log_path, &content).unwrap();

        rotate_log_if_needed(&log_path);

        // Original should be gone, backup should exist
        assert!(!log_path.exists());
        let backup = temp_dir.path().join("otto.log.1");
        assert!(backup.exists());
        assert_eq!(fs::metadata(&backup).unwrap().len(), content.len() as u64);
    }

    #[test]
    fn test_rotate_log_overwrites_existing_backup() {
        let temp_dir = TempDir::new().unwrap();
        let log_path = temp_dir.path().join("otto.log");
        let backup_path = temp_dir.path().join("otto.log.1");

        // Create old backup
        fs::write(&backup_path, "old backup").unwrap();
        // Create oversized log
        let content = vec![b'y'; 11 * 1024 * 1024];
        fs::write(&log_path, &content).unwrap();

        rotate_log_if_needed(&log_path);

        // Backup should be overwritten with new content
        assert!(!log_path.exists());
        assert!(backup_path.exists());
        assert_eq!(fs::metadata(&backup_path).unwrap().len(), content.len() as u64);
    }
}
