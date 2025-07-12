use std::env;
use eyre::Report;
use log::info;
use env_logger::Target;
use std::fs::OpenOptions;

use otto::{
    cli::demo::run_demo,
    cli::NomParser,
};

fn setup_logging() -> Result<(), Report> {
    let log_dir = dirs::data_local_dir()
        .ok_or_else(|| eyre::eyre!("Could not determine local data directory"))?
        .join("otto")
        .join("logs");

    std::fs::create_dir_all(&log_dir)?;
    let log_file_path = log_dir.join("otto.log");

    let log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_file_path)?;

    env_logger::Builder::from_env(env_logger::Env::default().filter_or("RUST_LOG", "info"))
        .target(Target::Pipe(Box::new(log_file)))
        .init();

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Report> {
    // Setup logging first
    setup_logging()?;
    info!("Starting otto");

    let args: Vec<String> = env::args().collect();

    return main_nom(args).await;
}

async fn main_nom(args: Vec<String>) -> Result<(), Report> {
    // For now, just run the demo to show that nom-cli is working
    // In a full implementation, this would:
    // 1. Parse command line arguments using NomParser
    // 2. Load configuration from ottofile
    // 3. Execute tasks like the clap version does

    println!("Otto CLI (nom-based) - Command: {:?}", args.get(1).unwrap_or(&"(none)".to_string()));

    // Check if this is a demo request
    if args.get(1).map(|s| s.as_str()) == Some("demo") {
        run_demo().await;
        return Ok(());
    }

    // For now, just show that we received the arguments
    if args.len() > 1 {
        println!("Received arguments: {:?}", &args[1..]);
        println!("This would be parsed by the nom-based CLI parser.");

        // Example of how the nom parser would be used:
        let input = args[1..].join(" ");
        println!("Input to parse: '{}'", input);

        // Create a basic parser (without config for now)
        let mut parser = NomParser::new(None).map_err(|e| eyre::eyre!("Failed to create parser: {}", e))?;

        match parser.parse(&input) {
            Ok(parsed) => {
                println!("✓ Parsed successfully:");
                println!("  Global options: {:?}", parsed.global_options);
                println!("  Tasks: {:?}", parsed.tasks);
            }
            Err(e) => {
                println!("✗ Parse error: {}", e);
            }
        }
    } else {
        println!("No arguments provided. Try 'otto demo' to see the nom-based parser demo.");
    }

    Ok(())
}
