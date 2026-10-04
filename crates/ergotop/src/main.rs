use std::path::PathBuf;

use clap::Parser;
use ergotop::theme::THEMES;
use ergotop_core::config::{
    config_dir, load_from_dir, load_state, AddressesFile, Config, NodeConfig,
};

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
    /// Use this node instead of the configured ones (repeat for several)
    #[arg(long = "node-url", value_name = "URL")]
    node_url: Vec<String>,
    /// Use this explorer API instead of the configured ones
    #[arg(long = "api-url", value_name = "URL")]
    api_url: Option<String>,
    /// Colour theme
    #[arg(long, value_parser = clap::builder::PossibleValuesParser::new(THEMES.map(|t| t.name)))]
    theme: Option<String>,
    /// View to start in
    #[arg(long, value_parser = ["dashboard", "packing", "sources"])]
    view: Option<String>,
    /// Transaction sort
    #[arg(long, value_parser = ["rate", "fee", "value", "size", "age", "origin"])]
    sort: Option<String>,
    /// Reverse the sort direction
    #[arg(long)]
    reverse: bool,
    /// Pack the next block into the ERG hexagon
    #[arg(long)]
    hexagon: bool,
    /// No animations
    #[arg(long = "no-motion")]
    no_motion: bool,
    /// Frames per second while animating (1-120)
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..=120))]
    fps: Option<u32>,
    /// Highlight txs moving at least this many ERG (0 = off)
    #[arg(long, value_name = "ERG")]
    whale: Option<f64>,
    /// Neither read nor write state.toml (the remembered in-app choices)
    #[arg(long = "no-state")]
    no_state: bool,
}

/// Flags override everything else: defaults < ergotop.toml < state.toml < env < flags.
fn apply_args(args: &Args, cfg: &mut Config) {
    if !args.node_url.is_empty() {
        cfg.node = args
            .node_url
            .iter()
            .map(|url| NodeConfig {
                url: url.clone(),
                name: None,
            })
            .collect();
    }
    if let Some(url) = &args.api_url {
        cfg.explorers.enabled = vec![url.clone()];
    }
    let ui = &mut cfg.ui;
    if let Some(v) = &args.theme {
        ui.theme = v.clone();
    }
    if let Some(v) = &args.view {
        ui.start_view = v.clone();
    }
    if let Some(v) = &args.sort {
        ui.sort = v.clone();
    }
    if args.reverse {
        ui.sort_reversed = true;
    }
    if args.hexagon {
        ui.shape = "hexagon".into();
    }
    if args.no_motion {
        ui.motion = false;
    }
    if let Some(v) = args.fps {
        ui.fps = v;
    }
    if let Some(v) = args.whale {
        ui.whale_erg = v;
    }
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
    let dir = args.config.clone().or_else(config_dir);
    let (mut cfg, addrs, warnings) = match &dir {
        Some(dir) => load_from_dir(dir),
        None => (Config::default(), AddressesFile::default(), vec![]),
    };
    let state_dir = dir.filter(|_| !args.no_state);
    if let Some(d) = &state_dir {
        load_state(d).apply(&mut cfg.ui);
    }
    cfg.apply_env(
        std::env::var("ERGO_NODE_URL").ok(),
        std::env::var("ERGO_API_URL").ok(),
    );
    apply_args(&args, &mut cfg);
    if args.headless {
        for w in &warnings {
            eprintln!("warning: {w}");
        }
        return ergotop::headless::run(cfg, addrs).await;
    }
    ergotop::tui::run(cfg, addrs, warnings, state_dir).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(argv: &[&str]) -> Args {
        Args::try_parse_from(std::iter::once("ergotop").chain(argv.iter().copied())).unwrap()
    }

    #[test]
    fn flags_override_config() {
        let mut cfg = Config::default();
        let args = parse(&[
            "--node-url",
            "http://a:9053",
            "--node-url",
            "http://b:9053",
            "--api-url",
            "https://x",
            "--theme",
            "amber-terminal",
            "--view",
            "sources",
            "--sort",
            "value",
            "--reverse",
            "--hexagon",
            "--no-motion",
            "--fps",
            "10",
            "--whale",
            "500",
        ]);
        apply_args(&args, &mut cfg);
        let urls: Vec<&str> = cfg.node.iter().map(|n| n.url.as_str()).collect();
        assert_eq!(urls, vec!["http://a:9053", "http://b:9053"]);
        assert_eq!(cfg.explorers.enabled, vec!["https://x".to_string()]);
        let ui = &cfg.ui;
        assert_eq!(
            (ui.theme.as_str(), ui.start_view.as_str()),
            ("amber-terminal", "sources")
        );
        assert_eq!(
            (ui.sort.as_str(), ui.sort_reversed, ui.shape.as_str()),
            ("value", true, "hexagon")
        );
        assert_eq!((ui.motion, ui.fps, ui.whale_erg), (false, 10, 500.0));
    }

    #[test]
    fn no_flags_change_nothing_and_bad_values_are_rejected() {
        let mut cfg = Config::default();
        apply_args(&parse(&[]), &mut cfg);
        assert_eq!(cfg, Config::default());
        for bad in [["--theme", "pink"], ["--sort", "colour"], ["--fps", "0"]] {
            assert!(
                Args::try_parse_from(["ergotop", bad[0], bad[1]]).is_err(),
                "{bad:?}"
            );
        }
    }
}
