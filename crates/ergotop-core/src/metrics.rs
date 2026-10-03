//! Per-transaction fee and transferred value.
use std::collections::HashSet;

use crate::model::Tx;

pub const FEE_ADDRESS: &str = "2iHkR7CWvD1R4j1yZg5bkeDRQavjAaVPeTDFGGLZduHyfWMuYpmhHocX8GJoaieTx78FntzJbCBVL6rf96ocJoZdmWBL2fci7NqWgAirppPQmZ7fN9V6z13Ay6brPriBKYqLp1bT2Fk4FkFLCfdPpe";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TxMetrics {
    pub fee: u64,
    pub value: u64,
    /// True when some inputs are unresolved, so change could not be excluded.
    pub approx: bool,
}

pub fn tx_metrics(tx: &Tx) -> TxMetrics {
    let fee = tx
        .outputs
        .iter()
        .filter(|o| o.address == FEE_ADDRESS)
        .map(|o| o.value)
        .sum();
    let input_addrs: Option<HashSet<&str>> = tx
        .inputs
        .iter()
        .map(|i| i.resolved.as_ref().map(|b| b.address.as_str()))
        .collect();
    let non_fee = tx.outputs.iter().filter(|o| o.address != FEE_ADDRESS);
    match input_addrs {
        Some(ins) => TxMetrics {
            fee,
            value: non_fee
                .filter(|o| !ins.contains(o.address.as_str()))
                .map(|o| o.value)
                .sum(),
            approx: false,
        },
        None => TxMetrics {
            fee,
            value: non_fee.map(|o| o.value).sum(),
            approx: true,
        },
    }
}

/// Fee rate in nanoERG per byte (what block packing ranks by).
pub fn fee_rate(fee: u64, size: u32) -> u64 {
    fee / u64::from(size.max(1))
}

/// Fee-rate distribution of a mempool, in nanoERG per byte.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RateStats {
    pub min: u64,
    pub median: u64,
    pub p90: u64,
    /// Lowest rate that made the next block when the mempool overflows it;
    /// `None` when everything fits (any fee gets in).
    pub entry: Option<u64>,
}

/// Stats over `(fee, size)` pairs; the block is filled highest rate first, as `packing` does.
pub fn rate_stats(txs: impl Iterator<Item = (u64, u32)>, capacity: u64) -> Option<RateStats> {
    let mut txs: Vec<(u64, u32)> = txs.collect();
    if txs.is_empty() {
        return None;
    }
    let precise = |&(fee, size): &(u64, u32)| fee as u128 * 1000 / size.max(1) as u128;
    txs.sort_by_key(|t| std::cmp::Reverse(precise(t)));
    let rates: Vec<u64> = txs.iter().map(|&(f, s)| fee_rate(f, s)).collect();
    // Nearest-rank percentile over rates in ascending order.
    let pct = |p: usize| rates[rates.len() - (p * rates.len()).div_ceil(100)];
    let mut used = 0u64;
    let mut entry: Option<u64> = None;
    let mut overflowed = false;
    for (&(_, size), &rate) in txs.iter().zip(&rates) {
        if used + u64::from(size) <= capacity {
            used += u64::from(size);
            entry = Some(rate);
        } else {
            overflowed = true;
        }
    }
    Some(RateStats {
        min: *rates.last().unwrap(),
        median: pct(50),
        p90: pct(90),
        entry: entry.filter(|_| overflowed),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::test_util::{bx, tx};
    use crate::model::Input;

    const ALICE: &str = "9guaDYhHCxtfAdRTKr8xXaDuXtdB8gdGB7WwnB5zTBw93Ym3Rsq";
    const BOB: &str = "4MQyMKvMbnCJG3aJ";

    #[test]
    fn fee_rate_is_nanoerg_per_byte() {
        assert_eq!(fee_rate(1_100_000, 300), 3_666);
        assert_eq!(fee_rate(5, 0), 5, "zero size does not divide by zero");
    }

    #[test]
    fn rate_stats_percentiles_and_entry_rate() {
        // Rates: 500, 930, 3_640, 3_666 n/B.
        let txs = [
            (1_500_000, 412),
            (2_000_000, 2_150),
            (1_100_000, 300),
            (10_000_000, 20_000),
        ];
        let all_fit = rate_stats(txs.iter().copied(), 1_000_000).unwrap();
        assert_eq!(
            all_fit,
            RateStats {
                min: 500,
                median: 930,
                p90: 3_666,
                entry: None
            }
        );
        // 3_000 bytes: c3 (300) + a1 (412) + b2 (2_150) fit, d4 does not.
        let full = rate_stats(txs.iter().copied(), 3_000).unwrap();
        assert_eq!(full.entry, Some(930));
        assert_eq!(rate_stats(std::iter::empty(), 1_000), None);
    }

    #[test]
    fn fee_is_sum_of_fee_outputs() {
        let t = tx(
            "a",
            300,
            vec![bx(ALICE, 10)],
            vec![bx(BOB, 5), bx(FEE_ADDRESS, 2), bx(FEE_ADDRESS, 1)],
        );
        assert_eq!(tx_metrics(&t).fee, 3);
    }

    #[test]
    fn no_fee_output_means_zero_fee() {
        let t = tx("a", 300, vec![bx(ALICE, 10)], vec![bx(BOB, 10)]);
        assert_eq!(tx_metrics(&t).fee, 0);
    }

    #[test]
    fn value_excludes_change_and_fee() {
        let t = tx(
            "a",
            300,
            vec![bx(ALICE, 100)],
            vec![bx(BOB, 40), bx(ALICE, 59), bx(FEE_ADDRESS, 1)],
        );
        assert_eq!(
            tx_metrics(&t),
            TxMetrics {
                fee: 1,
                value: 40,
                approx: false
            }
        );
    }

    #[test]
    fn unresolved_inputs_give_approx_value() {
        let mut t = tx(
            "a",
            300,
            vec![],
            vec![bx(BOB, 40), bx(ALICE, 59), bx(FEE_ADDRESS, 1)],
        );
        t.inputs.push(Input {
            box_id: "x".into(),
            resolved: None,
        });
        assert_eq!(
            tx_metrics(&t),
            TxMetrics {
                fee: 1,
                value: 99,
                approx: true
            }
        );
    }
}
