//! Built-in classification data embedded at compile time.
use std::collections::HashMap;

use serde::Deserialize;

use crate::config::LocalAddress;

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Rule {
    pub name: String,
    pub kind: String,
    pub address_prefix: String,
}

#[derive(Debug, Clone, Default)]
pub struct Builtin {
    pub addresses: Vec<LocalAddress>,
    pub colors: HashMap<String, String>,
    pub rules: Vec<Rule>,
}

#[derive(Deserialize)]
struct AddressesToml {
    #[serde(default)]
    address: Vec<LocalAddress>,
    #[serde(default)]
    colors: HashMap<String, String>,
}

#[derive(Deserialize)]
struct RulesToml {
    #[serde(default)]
    rule: Vec<Rule>,
}

const ADDRESSES_TOML: &str = include_str!("../../../../assets/builtin-addresses.toml");
const RULES_TOML: &str = include_str!("../../../../assets/rules.toml");

impl Builtin {
    /// Parses the embedded assets. Panics only if the shipped assets are malformed (covered by tests).
    pub fn load() -> Builtin {
        let a: AddressesToml =
            toml::from_str(ADDRESSES_TOML).expect("assets/builtin-addresses.toml");
        let r: RulesToml = toml::from_str(RULES_TOML).expect("assets/rules.toml");
        Builtin {
            addresses: a.address,
            colors: a.colors,
            rules: r.rule,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_assets_parse() {
        let b = Builtin::load();
        assert!(b.addresses.len() >= 150, "got {}", b.addresses.len());
        assert_eq!(b.rules.len(), 3);
        assert_eq!(
            b.colors.get("Spectrum").map(String::as_str),
            Some("#3498db")
        );
    }
}
