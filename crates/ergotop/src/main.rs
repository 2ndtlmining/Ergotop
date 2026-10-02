mod headless;

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
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let (mut cfg, addrs, warnings) = match args.config.clone().or_else(config_dir) {
        Some(dir) => load_from_dir(&dir),
        None => (Config::default(), AddressesFile::default(), vec![]),
    };
    cfg.apply_env(
        std::env::var("ERGO_NODE_URL").ok(),
        std::env::var("ERGO_API_URL").ok(),
    );
    for w in &warnings {
        eprintln!("warning: {w}");
    }
    if !args.headless {
        eprintln!("The TUI arrives in Plan 2; run with --headless for now.");
        return Ok(());
    }
    headless::run(cfg, addrs).await
}
