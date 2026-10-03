//! ergotop.toml / addresses.toml loading and source list resolution.
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::Deserialize;

use crate::model::{SourceId, SourceKind};

pub const DEFAULT_NODE_URL: &str = "http://127.0.0.1:9053";
pub const PUBLIC_EXPLORER: &str = "https://api.ergoplatform.com";
pub const P2P_EXPLORER: &str = "https://api-p2p.ergoplatform.com";

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    pub node: Vec<NodeConfig>,
    pub explorers: ExplorersConfig,
    pub ui: UiConfig,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct NodeConfig {
    pub url: String,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default)]
pub struct ExplorersConfig {
    pub enabled: Vec<String>,
}

impl Default for ExplorersConfig {
    fn default() -> Self {
        Self {
            enabled: vec!["p2p".into(), "public".into()],
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default)]
pub struct UiConfig {
    pub theme: String,
    pub fps: u32,
    pub start_view: String,
    pub motion: bool,
    /// Highlight txs moving at least this many ERG; 0 disables.
    pub whale_erg: f64,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            theme: "neon-green".into(),
            fps: 30,
            start_view: "packing".into(),
            motion: true,
            whale_erg: 10_000.0,
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct LocalAddress {
    pub address: String,
    pub name: String,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct AddressesFile {
    pub address: Vec<LocalAddress>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SourceSpec {
    pub id: SourceId,
    pub kind: SourceKind,
    pub url: String,
}

impl Config {
    pub fn apply_env(&mut self, node_url: Option<String>, api_url: Option<String>) {
        if let Some(url) = node_url {
            self.node = vec![NodeConfig { url, name: None }];
        }
        if let Some(url) = api_url {
            self.explorers.enabled = vec![url];
        }
    }

    /// All sources in priority order: nodes (config order), then explorers.
    pub fn sources(&self) -> Vec<SourceSpec> {
        let nodes = if self.node.is_empty() {
            vec![NodeConfig {
                url: DEFAULT_NODE_URL.into(),
                name: Some("local".into()),
            }]
        } else {
            self.node.clone()
        };
        let mut out: Vec<SourceSpec> = nodes
            .into_iter()
            .map(|n| {
                let url = n.url.trim_end_matches('/').to_string();
                SourceSpec {
                    id: SourceId(n.name.unwrap_or_else(|| url.clone())),
                    kind: SourceKind::Node,
                    url,
                }
            })
            .collect();
        for e in &self.explorers.enabled {
            let (id, url) = match e.as_str() {
                "p2p" => ("p2p", P2P_EXPLORER),
                "public" => ("public", PUBLIC_EXPLORER),
                other => (other, other),
            };
            out.push(SourceSpec {
                id: SourceId(id.to_string()),
                kind: SourceKind::Explorer,
                url: url.trim_end_matches('/').to_string(),
            });
        }
        out
    }
}

pub fn config_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("ergotop"))
}

pub fn cache_dir() -> Option<PathBuf> {
    dirs::cache_dir().map(|d| d.join("ergotop"))
}

/// Loads `ergotop.toml` and `addresses.toml` from `dir`.
/// Missing files give defaults; unparsable files give defaults plus a warning.
pub fn load_from_dir(dir: &Path) -> (Config, AddressesFile, Vec<String>) {
    let mut warnings = Vec::new();
    let config = read_toml(&dir.join("ergotop.toml"), &mut warnings);
    let addresses = read_toml(&dir.join("addresses.toml"), &mut warnings);
    (config, addresses, warnings)
}

fn read_toml<T: DeserializeOwned + Default>(path: &Path, warnings: &mut Vec<String>) -> T {
    match std::fs::read_to_string(path) {
        Ok(text) => toml::from_str(&text).unwrap_or_else(|e| {
            warnings.push(format!("{}: {e}", path.display()));
            T::default()
        }),
        Err(_) => T::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ergotop-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn parses_full_config() {
        let cfg: Config = toml::from_str(
            r#"
            [[node]]
            url = "http://192.168.1.50:9053/"
            name = "node-a"
            [[node]]
            url = "http://192.168.1.51:9053"
            [explorers]
            enabled = ["public"]
            [ui]
            theme = "amber-terminal"
            fps = 60
            start_view = "dashboard"
            motion = false
            "#,
        )
        .unwrap();
        let ids: Vec<String> = cfg.sources().into_iter().map(|s| s.id.0).collect();
        assert_eq!(ids, vec!["node-a", "http://192.168.1.51:9053", "public"]);
        assert_eq!(cfg.sources()[0].url, "http://192.168.1.50:9053");
        assert_eq!(cfg.ui.fps, 60);
        assert!(!cfg.ui.motion);
    }

    #[test]
    fn defaults_use_local_node_and_both_explorers() {
        let specs = Config::default().sources();
        assert_eq!(specs.len(), 3);
        assert_eq!(specs[0].id, SourceId("local".into()));
        assert_eq!(specs[0].kind, SourceKind::Node);
        assert_eq!(specs[0].url, DEFAULT_NODE_URL);
        assert_eq!(specs[1].url, P2P_EXPLORER);
        assert_eq!(specs[2].url, PUBLIC_EXPLORER);
        assert_eq!(Config::default().ui.theme, "neon-green");
        assert!(Config::default().ui.motion);
    }

    #[test]
    fn env_overrides_replace_lists() {
        let mut cfg = Config::default();
        cfg.apply_env(
            Some("http://10.0.0.5:9053".into()),
            Some("https://my-explorer.example".into()),
        );
        let specs = cfg.sources();
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].url, "http://10.0.0.5:9053");
        assert_eq!(specs[1].kind, SourceKind::Explorer);
        assert_eq!(specs[1].url, "https://my-explorer.example");
    }

    #[test]
    fn load_from_dir_reads_both_files() {
        let dir = temp_dir("both");
        std::fs::write(
            dir.join("ergotop.toml"),
            "[[node]]\nurl = \"http://n:9053\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("addresses.toml"),
            "[[address]]\naddress = \"9f\"\nname = \"Mine\"\n",
        )
        .unwrap();
        let (cfg, addrs, warnings) = load_from_dir(&dir);
        assert!(warnings.is_empty());
        assert_eq!(cfg.node[0].url, "http://n:9053");
        assert_eq!(addrs.address[0].name, "Mine");
        assert_eq!(addrs.address[0].kind, None);
    }

    #[test]
    fn missing_files_give_defaults_and_bad_file_gives_warning() {
        let dir = temp_dir("bad");
        let (cfg, addrs, warnings) = load_from_dir(&dir);
        assert_eq!(cfg, Config::default());
        assert!(addrs.address.is_empty());
        assert!(warnings.is_empty());

        std::fs::write(dir.join("ergotop.toml"), "this is = = not toml").unwrap();
        let (cfg, _, warnings) = load_from_dir(&dir);
        assert_eq!(cfg, Config::default());
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn shipped_example_files_parse() {
        let cfg: Config = toml::from_str(include_str!("../../../examples/ergotop.toml")).unwrap();
        let ids: Vec<String> = cfg.sources().into_iter().map(|s| s.id.0).collect();
        assert_eq!(ids, vec!["node-a", "node-b", "p2p", "public"]);
        assert_eq!(cfg.sources()[0].url, "http://192.168.1.50:9053");
        assert!(cfg.ui.motion);
        let addrs: AddressesFile =
            toml::from_str(include_str!("../../../examples/addresses.toml")).unwrap();
        assert_eq!(addrs.address.len(), 2);
        assert_eq!(addrs.address[0].kind.as_deref(), Some("Local"));
    }
}
