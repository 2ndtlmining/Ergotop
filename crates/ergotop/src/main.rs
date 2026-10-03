use std::path::PathBuf;

use clap::Parser;
use ergotop_core::config::{config_dir, load_from_dir, AddressesFile, Config};

#[derive(Parser)]
#[command(version, about = "Real-time Ergo mempool visualizer")]
struct Args {
    /// Directory containing ergotop.toml and addresses.toml
    #[arg(long)]
    config: Option<PathBuf>,
    /// Print mempool activity as text instead of the TUI
    #[arg(long)]
    headless: bool,
    /// Write logs to this file (nothing is logged to the terminal)
    #[arg(long)]
    log: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    if let Some(path) = &args.log {
        let file = std::fs::File::create(path)?;
        tracing_subscriber::fmt()
            .with_writer(std::sync::Mutex::new(file))
            .with_ansi(false)
            .init();
    }
    let (mut cfg, addrs, warnings) = match args.config.clone().or_else(config_dir) {
        Some(dir) => load_from_dir(&dir),
        None => (Config::default(), AddressesFile::default(), vec![]),
    };
    cfg.apply_env(
        std::env::var("ERGO_NODE_URL").ok(),
        std::env::var("ERGO_API_URL").ok(),
    );
    if args.headless {
        for w in &warnings {
            eprintln!("warning: {w}");
        }
        return ergotop::headless::run(cfg, addrs).await;
    }
    ergotop::tui::run(cfg, addrs, warnings).await
}
