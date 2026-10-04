//! Transaction filter: space-separated terms that must all match.
//!
//! `kucoin` (name, kind, origin or tx id prefix) · `>100` / `<1` (value, ERG) ·
//! `fee>0.01` `value>=100` `size<2k` `rate>1000` `age>5m` · `origin:rosen,spectrum` ·
//! `addr:9fyeE` (input/output address prefix) · `token:sigusd` (name or id prefix) ·
//! `!term` negates. Invalid terms are skipped and reported in `errors`.
use std::collections::HashMap;

use ergotop_core::metrics::fee_rate;
use ergotop_core::model::TokenMeta;
use ergotop_core::reconcile::TxEntry;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Field {
    Value,
    Fee,
    Size,
    Rate,
    Age,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Op {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
}

#[derive(Clone, Debug, PartialEq)]
enum Pred {
    Text(String),
    Cmp(Field, Op, f64),
    Origin(Vec<String>),
    Addr(String),
    Token(String),
}

#[derive(Clone, Debug, PartialEq)]
struct Term {
    negate: bool,
    pred: Pred,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Filter {
    terms: Vec<Term>,
    /// One message per term that could not be parsed (and is ignored).
    pub errors: Vec<String>,
}

impl Filter {
    pub fn parse(input: &str) -> Filter {
        let mut f = Filter::default();
        for raw in input.split_whitespace() {
            let (negate, body) = match raw.strip_prefix('!') {
                Some(rest) => (true, rest),
                None => (false, raw),
            };
            match parse_pred(body) {
                Ok(pred) => f.terms.push(Term { negate, pred }),
                Err(e) => f.errors.push(format!("{raw}: {e}")),
            }
        }
        f
    }

    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    pub fn matches(&self, e: &TxEntry, now_ms: u64, tokens: &HashMap<String, TokenMeta>) -> bool {
        self.terms
            .iter()
            .all(|t| pred_matches(&t.pred, e, now_ms, tokens) != t.negate)
    }
}

fn parse_pred(body: &str) -> Result<Pred, String> {
    if body.is_empty() {
        return Err("empty term".into());
    }
    if let Some((key, val)) = body.split_once(':') {
        if val.is_empty() {
            return Err(format!("nothing after {key}:"));
        }
        return match key.to_lowercase().as_str() {
            "origin" => Ok(Pred::Origin(
                val.split(',')
                    .filter(|s| !s.is_empty())
                    .map(str::to_lowercase)
                    .collect(),
            )),
            "addr" => Ok(Pred::Addr(val.to_string())),
            "token" => Ok(Pred::Token(val.to_lowercase())),
            other => Err(format!("unknown prefix {other}: (origin, addr, token)")),
        };
    }
    if let Some(at) = body.find(['<', '>', '=']) {
        let (name, rest) = body.split_at(at);
        let field = match name.to_lowercase().as_str() {
            "" | "value" => Field::Value,
            "fee" => Field::Fee,
            "size" => Field::Size,
            "rate" => Field::Rate,
            "age" => Field::Age,
            other => {
                return Err(format!(
                    "unknown field {other} (value, fee, size, rate, age)"
                ))
            }
        };
        let (op, num) = if let Some(n) = rest.strip_prefix(">=") {
            (Op::Ge, n)
        } else if let Some(n) = rest.strip_prefix("<=") {
            (Op::Le, n)
        } else if let Some(n) = rest.strip_prefix('>') {
            (Op::Gt, n)
        } else if let Some(n) = rest.strip_prefix('<') {
            (Op::Lt, n)
        } else {
            (Op::Eq, rest.trim_start_matches('='))
        };
        return Ok(Pred::Cmp(field, op, parse_amount(field, num)?));
    }
    Ok(Pred::Text(body.to_lowercase()))
}

/// The threshold in the units the entry is compared in: nanoERG, bytes, n/B or ms.
fn parse_amount(field: Field, s: &str) -> Result<f64, String> {
    let s = s.trim().to_lowercase();
    let split = s
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(s.len());
    let (num, unit) = s.split_at(split);
    let n: f64 = num
        .parse()
        .map_err(|_| format!("expected a number, got {s:?}"))?;
    let scale = match (field, unit) {
        (Field::Value | Field::Fee, "" | "erg") => 1e9,
        (Field::Size, "" | "b") => 1.0,
        (Field::Size, "k" | "kb") => 1024.0,
        (Field::Size, "m" | "mb") => 1024.0 * 1024.0,
        (Field::Rate, "") => 1.0,
        (Field::Rate, "k") => 1000.0,
        (Field::Age, "" | "s") => 1000.0,
        (Field::Age, "m") => 60_000.0,
        (Field::Age, "h") => 3_600_000.0,
        _ => return Err(format!("unknown unit {unit:?}")),
    };
    Ok(n * scale)
}

fn pred_matches(
    pred: &Pred,
    e: &TxEntry,
    now_ms: u64,
    tokens: &HashMap<String, TokenMeta>,
) -> bool {
    let class = &e.class;
    match pred {
        Pred::Text(f) => {
            class.class.name.to_lowercase().contains(f)
                || class.class.kind.label().to_lowercase().contains(f)
                || class
                    .from
                    .as_deref()
                    .is_some_and(|x| x.to_lowercase().contains(f))
                || e.tx.id.starts_with(f.as_str())
        }
        Pred::Origin(names) => {
            let name = class.class.name.to_lowercase();
            let from = class.from.as_deref().map(str::to_lowercase);
            names
                .iter()
                .any(|n| name.contains(n) || from.as_deref().is_some_and(|f| f.contains(n)))
        }
        Pred::Addr(prefix) => {
            let ins = e.tx.inputs.iter().filter_map(|i| i.resolved.as_ref());
            ins.chain(&e.tx.outputs)
                .any(|b| b.address.starts_with(prefix.as_str()))
        }
        Pred::Token(q) => {
            let ins = e.tx.inputs.iter().filter_map(|i| i.resolved.as_ref());
            ins.chain(&e.tx.outputs).flat_map(|b| &b.tokens).any(|t| {
                t.token_id.starts_with(q.as_str())
                    || tokens
                        .get(&t.token_id)
                        .and_then(|m| m.name.as_deref())
                        .is_some_and(|n| n.to_lowercase().contains(q))
            })
        }
        Pred::Cmp(field, op, x) => {
            let v = match field {
                Field::Value => e.metrics.value as f64,
                Field::Fee => e.metrics.fee as f64,
                Field::Size => f64::from(e.tx.size),
                Field::Rate => fee_rate(e.metrics.fee, e.tx.size) as f64,
                Field::Age => now_ms.saturating_sub(e.first_seen_ms) as f64,
            };
            match op {
                Op::Lt => v < *x,
                Op::Le => v <= *x,
                Op::Gt => v > *x,
                Op::Ge => v >= *x,
                Op::Eq => (v - x).abs() < 0.5,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::testkit::*;
    use crate::app::App;
    use ergotop_core::model::{BoxData, SourceId, Token};
    use ergotop_core::sources::SourceEvent;

    const SIGUSD: &str = "03faf2cb329f2e90d6d23b58d91bbb6c046aa143261cc21f52fbe2824bfcbf04";

    fn ids(app: &App, filter: &str) -> Vec<String> {
        let f = Filter::parse(filter);
        assert!(f.errors.is_empty(), "{:?}", f.errors);
        let mut v: Vec<String> = app
            .rec
            .pool()
            .values()
            .filter(|e| f.matches(e, NOW, &app.tokens))
            .map(|e| e.tx.id[..2].to_string())
            .collect();
        v.sort();
        v
    }

    /// sample_app plus "e5": a SigUSD transfer.
    fn app_with_token() -> App {
        let mut app = sample_app();
        let out = BoxData {
            box_id: "e5-0".into(),
            value: 1_000_000,
            address: WALLET.into(),
            tokens: vec![Token {
                token_id: SIGUSD.into(),
                amount: 100,
            }],
        };
        let e5 = tx("e5", 500, WALLET, vec![out]);
        let mut all: Vec<String> = app.rec.pool().keys().cloned().collect();
        all.push(e5.id.clone());
        app.on_source_event(
            SourceEvent::Mempool {
                source: SourceId("node-a".into()),
                ids: all,
                new_txs: vec![e5],
                latency_ms: 20,
            },
            NOW - 500,
        );
        app.on_source_event(
            SourceEvent::TokenMeta(TokenMeta {
                token_id: SIGUSD.into(),
                name: Some("SigUSD".into()),
                decimals: 2,
            }),
            NOW,
        );
        app
    }

    #[test]
    fn words_and_value_shortcuts_work_as_before() {
        let app = sample_app();
        assert_eq!(ids(&app, "kucoin"), vec!["a1"]);
        assert_eq!(ids(&app, "exchange"), vec!["a1"]);
        assert_eq!(ids(&app, "b2"), vec!["b2"]);
        assert_eq!(ids(&app, ">10"), vec!["a1", "d4"]);
        assert_eq!(ids(&app, "<2"), vec!["c3"]);
    }

    #[test]
    fn field_comparisons_with_units() {
        let app = sample_app();
        assert_eq!(ids(&app, "fee>0.0019"), vec!["b2", "d4"]);
        assert_eq!(ids(&app, "size<2k"), vec!["a1", "c3"]);
        assert_eq!(ids(&app, "size>=2150"), vec!["b2", "d4"]);
        assert_eq!(ids(&app, "rate>3k"), vec!["a1", "c3"]);
        assert_eq!(ids(&app, "value=12"), vec!["a1"]);
        assert_eq!(ids(&app, "age>20s"), vec!["a1", "b2", "c3", "d4"]);
        assert!(ids(&app, "age>1m").is_empty());
    }

    #[test]
    fn terms_combine_and_negate() {
        let app = sample_app();
        assert_eq!(ids(&app, "contract >10"), vec!["d4"]);
        assert_eq!(ids(&app, "!contract"), vec!["a1", "c3"]);
        assert_eq!(ids(&app, "origin:kucoin,p2p"), vec!["a1", "c3"]);
    }

    #[test]
    fn address_and_token_search() {
        let app = app_with_token();
        assert_eq!(ids(&app, &format!("addr:{}", &KUCOIN[..10])), vec!["a1"]);
        assert_eq!(ids(&app, "addr:4MQyMKvM"), vec!["b2", "d4"]);
        assert_eq!(ids(&app, "token:sigusd"), vec!["e5"]);
        assert_eq!(ids(&app, "token:03faf2"), vec!["e5"]);
        assert!(ids(&app, "token:rsn").is_empty());
    }

    #[test]
    fn invalid_terms_are_reported_and_skipped() {
        let f = Filter::parse("kucoin colour>3 fee>abc addr: size>3q !");
        assert_eq!(f.terms.len(), 1);
        assert_eq!(f.errors.len(), 5, "{:?}", f.errors);
        assert!(f.errors[0].starts_with("colour>3: unknown field colour"));
    }
}
