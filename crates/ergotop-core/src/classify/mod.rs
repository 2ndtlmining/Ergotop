//! Address and transaction classification.
mod builtin;

pub use builtin::{Builtin, Rule};

use std::collections::HashMap;

use crate::config::LocalAddress;
use crate::metrics::FEE_ADDRESS;
use crate::model::{BoxData, Tx};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Exchange,
    Service,
    MiningPool,
    Meme,
    Local,
    P2P,
    Contract,
    Unknown,
}

impl Kind {
    /// Parses ergexplorer `type` values and our own TOML `kind` values.
    pub fn parse(s: &str) -> Kind {
        match s.trim().to_ascii_lowercase().replace(' ', "").as_str() {
            "exchange" => Kind::Exchange,
            "miningpool" => Kind::MiningPool,
            "meme" => Kind::Meme,
            "local" => Kind::Local,
            _ => Kind::Service,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::Exchange => "Exchange",
            Kind::Service => "Service",
            Kind::MiningPool => "Mining pool",
            Kind::Meme => "Meme",
            Kind::Local => "Local",
            Kind::P2P => "P2P",
            Kind::Contract => "Contract",
            Kind::Unknown => "Unknown",
        }
    }

    fn base_color(self) -> Rgb {
        match self {
            Kind::Exchange => Rgb(0xf3, 0x9c, 0x12),
            Kind::Service => Rgb(0x34, 0x98, 0xdb),
            Kind::MiningPool => Rgb(0x8b, 0x45, 0x13),
            Kind::Meme => Rgb(0xe9, 0x1e, 0x63),
            Kind::Local => Rgb(0xf1, 0xc4, 0x0f),
            Kind::P2P => Rgb(0x7a, 0xb8, 0x7a),
            Kind::Contract => Rgb(0x8a, 0x6a, 0x9a),
            Kind::Unknown => Rgb(0x55, 0x66, 0x55),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub fn parse_hex(s: &str) -> Option<Rgb> {
        let h = s.strip_prefix('#')?;
        if h.len() != 6 {
            return None;
        }
        let v = u32::from_str_radix(h, 16).ok()?;
        Some(Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
    }

    fn shift(self, delta: i16) -> Rgb {
        let f = |c: u8| (c as i16 + delta).clamp(0, 255) as u8;
        Rgb(f(self.0), f(self.1), f(self.2))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Classification {
    pub name: String,
    pub kind: Kind,
    pub color: Rgb,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TxClass {
    pub class: Classification,
    /// Input-side name when it differs from the output-side match (`from → class`).
    pub from: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BookEntry {
    pub address: String,
    pub name: String,
    pub kind: Kind,
}

#[derive(Clone, Debug)]
struct Entry {
    name: String,
    kind: Kind,
    color: Option<Rgb>,
}

pub struct Classifier {
    exact: HashMap<String, Entry>,
    rules: Vec<(String, Entry)>,
    colors: HashMap<String, Rgb>,
}

fn fnv1a(s: &str) -> u32 {
    let mut h: u32 = 0x811c9dc5;
    for b in s.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x01000193);
    }
    h
}

fn local_entry(a: &LocalAddress, default_kind: Kind) -> Entry {
    Entry {
        name: a.name.clone(),
        kind: a.kind.as_deref().map(Kind::parse).unwrap_or(default_kind),
        color: a.color.as_deref().and_then(Rgb::parse_hex),
    }
}

fn is_p2pk(address: &str) -> bool {
    address.starts_with('9') && address.len() == 51
}

impl Classifier {
    pub fn new(builtin: &Builtin, book: &[BookEntry], local: &[LocalAddress]) -> Self {
        let mut exact = HashMap::new();
        for a in &builtin.addresses {
            exact.insert(a.address.clone(), local_entry(a, Kind::Service));
        }
        for b in book {
            exact.insert(
                b.address.clone(),
                Entry {
                    name: b.name.clone(),
                    kind: b.kind,
                    color: None,
                },
            );
        }
        for a in local {
            exact.insert(a.address.clone(), local_entry(a, Kind::Local));
        }
        let rules = builtin
            .rules
            .iter()
            .map(|r| {
                (
                    r.address_prefix.clone(),
                    Entry {
                        name: r.name.clone(),
                        kind: Kind::parse(&r.kind),
                        color: None,
                    },
                )
            })
            .collect();
        let colors = builtin
            .colors
            .iter()
            .filter_map(|(name, hex)| Rgb::parse_hex(hex).map(|c| (name.clone(), c)))
            .collect();
        Classifier {
            exact,
            rules,
            colors,
        }
    }

    fn finish(&self, e: &Entry) -> Classification {
        let color = e
            .color
            .or_else(|| self.colors.get(&e.name).copied())
            .unwrap_or_else(|| e.kind.base_color().shift((fnv1a(&e.name) % 81) as i16 - 40));
        Classification {
            name: e.name.clone(),
            kind: e.kind,
            color,
        }
    }

    pub fn lookup(&self, address: &str) -> Option<Classification> {
        if let Some(e) = self.exact.get(address) {
            return Some(self.finish(e));
        }
        self.rules
            .iter()
            .find(|(prefix, _)| address.starts_with(prefix.as_str()))
            .map(|(_, e)| self.finish(e))
    }

    pub fn classify_tx(&self, tx: &Tx) -> TxClass {
        let outs: Vec<&BoxData> = tx
            .outputs
            .iter()
            .filter(|o| o.address != FEE_ADDRESS)
            .collect();
        let out_named = outs.iter().find_map(|o| self.lookup(&o.address));
        let in_named = tx
            .inputs
            .iter()
            .filter_map(|i| i.resolved.as_ref())
            .find_map(|b| self.lookup(&b.address));
        match (out_named, in_named) {
            (Some(o), Some(i)) if o.name != i.name => TxClass {
                class: o,
                from: Some(i.name),
            },
            (Some(o), _) => TxClass {
                class: o,
                from: None,
            },
            (None, Some(i)) => TxClass {
                class: i,
                from: None,
            },
            (None, None) => TxClass {
                class: heuristic(&outs),
                from: None,
            },
        }
    }
}

fn heuristic(outs: &[&BoxData]) -> Classification {
    let kind = if outs.is_empty() {
        Kind::Unknown
    } else if outs.iter().all(|o| is_p2pk(&o.address)) {
        Kind::P2P
    } else {
        Kind::Contract
    };
    Classification {
        name: kind.label().to_string(),
        kind,
        color: kind.base_color(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LocalAddress;
    use crate::metrics::FEE_ADDRESS;
    use crate::model::test_util::{bx, tx};

    const WALLET_A: &str = "9guaDYhHCxtfAdRTKr8xXaDuXtdB8gdGB7WwnB5zTBw93Ym3Rsq";
    const WALLET_B: &str = "9fyeEQBXvJzRYpRmrNy2eaB2kDqQGDk3KoSQGUB62db3tVDw2Z1";
    const CONTRACT: &str = "4MQyMKvMbnCJG3aJ";

    fn local(address: &str, name: &str) -> LocalAddress {
        LocalAddress {
            address: address.into(),
            name: name.into(),
            kind: None,
            color: None,
        }
    }

    fn book(address: &str, name: &str, kind: Kind) -> BookEntry {
        BookEntry {
            address: address.into(),
            name: name.into(),
            kind,
        }
    }

    #[test]
    fn local_beats_book_beats_builtin() {
        let builtin = Builtin {
            addresses: vec![local(WALLET_A, "Builtin")],
            ..Default::default()
        };
        let c = Classifier::new(&builtin, &[], &[]);
        assert_eq!(c.lookup(WALLET_A).unwrap().name, "Builtin");

        let books = [book(WALLET_A, "Book", Kind::Exchange)];
        let c = Classifier::new(&builtin, &books, &[]);
        let hit = c.lookup(WALLET_A).unwrap();
        assert_eq!((hit.name.as_str(), hit.kind), ("Book", Kind::Exchange));

        let c = Classifier::new(&builtin, &books, &[local(WALLET_A, "Mine")]);
        let hit = c.lookup(WALLET_A).unwrap();
        assert_eq!((hit.name.as_str(), hit.kind), ("Mine", Kind::Local));
    }

    #[test]
    fn prefix_rules_match_after_exact_miss() {
        let builtin = Builtin {
            rules: vec![Rule {
                name: "Spectrum".into(),
                kind: "Service".into(),
                address_prefix: "4MQy".into(),
            }],
            ..Default::default()
        };
        let c = Classifier::new(&builtin, &[], &[]);
        assert_eq!(c.lookup(CONTRACT).unwrap().name, "Spectrum");
        assert!(c.lookup(WALLET_A).is_none());
    }

    #[test]
    fn tx_uses_output_match_and_reports_input_side() {
        let books = [
            book(WALLET_A, "Kucoin", Kind::Exchange),
            book(CONTRACT, "Spectrum", Kind::Service),
        ];
        let c = Classifier::new(&Builtin::default(), &books, &[]);
        let t = tx(
            "t",
            300,
            vec![bx(WALLET_A, 10)],
            vec![bx(CONTRACT, 9), bx(FEE_ADDRESS, 1)],
        );
        let tc = c.classify_tx(&t);
        assert_eq!(tc.class.name, "Spectrum");
        assert_eq!(tc.from.as_deref(), Some("Kucoin"));
    }

    #[test]
    fn tx_falls_back_to_input_match() {
        let books = [book(WALLET_A, "Kucoin", Kind::Exchange)];
        let c = Classifier::new(&Builtin::default(), &books, &[]);
        let t = tx(
            "t",
            300,
            vec![bx(WALLET_A, 10)],
            vec![bx(WALLET_B, 9), bx(FEE_ADDRESS, 1)],
        );
        let tc = c.classify_tx(&t);
        assert_eq!(tc.class.name, "Kucoin");
        assert_eq!(tc.from, None);
    }

    #[test]
    fn heuristics_p2p_contract_unknown() {
        let c = Classifier::new(&Builtin::default(), &[], &[]);
        let p2p = tx("t", 1, vec![], vec![bx(WALLET_B, 9), bx(FEE_ADDRESS, 1)]);
        assert_eq!(c.classify_tx(&p2p).class.kind, Kind::P2P);
        let contract = tx("t", 1, vec![], vec![bx(WALLET_B, 9), bx(CONTRACT, 1)]);
        assert_eq!(c.classify_tx(&contract).class.kind, Kind::Contract);
        let only_fee = tx("t", 1, vec![], vec![bx(FEE_ADDRESS, 1)]);
        assert_eq!(c.classify_tx(&only_fee).class.kind, Kind::Unknown);
    }

    #[test]
    fn colors_prefer_entry_then_override_then_kind_shade() {
        let mut builtin = Builtin::default();
        builtin.colors.insert("Spectrum".into(), "#3498db".into());
        let mut mine = local(WALLET_B, "Mine");
        mine.color = Some("#ff00ff".into());
        let books = [
            book(CONTRACT, "Spectrum", Kind::Service),
            book(WALLET_A, "Other", Kind::Service),
        ];
        let c = Classifier::new(&builtin, &books, &[mine]);
        assert_eq!(c.lookup(WALLET_B).unwrap().color, Rgb(0xff, 0x00, 0xff));
        assert_eq!(c.lookup(CONTRACT).unwrap().color, Rgb(0x34, 0x98, 0xdb));
        let shade = c.lookup(WALLET_A).unwrap().color;
        assert_eq!(
            shade,
            c.lookup(WALLET_A).unwrap().color,
            "stable across calls"
        );
    }

    #[test]
    fn parses_kinds_and_hex() {
        assert_eq!(Kind::parse("Mining pool"), Kind::MiningPool);
        assert_eq!(Kind::parse("Exchange"), Kind::Exchange);
        assert_eq!(Kind::parse("Meme"), Kind::Meme);
        assert_eq!(Kind::parse("something new"), Kind::Service);
        assert_eq!(Rgb::parse_hex("#0a0B0c"), Some(Rgb(10, 11, 12)));
        assert_eq!(Rgb::parse_hex("nope"), None);
    }
}
