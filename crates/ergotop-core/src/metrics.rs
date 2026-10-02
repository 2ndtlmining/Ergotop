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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::test_util::{bx, tx};
    use crate::model::Input;

    const ALICE: &str = "9guaDYhHCxtfAdRTKr8xXaDuXtdB8gdGB7WwnB5zTBw93Ym3Rsq";
    const BOB: &str = "4MQyMKvMbnCJG3aJ";

    #[test]
    fn fee_is_sum_of_fee_outputs() {
        let t = tx("a", 300, vec![bx(ALICE, 10)], vec![bx(BOB, 5), bx(FEE_ADDRESS, 2), bx(FEE_ADDRESS, 1)]);
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
        assert_eq!(tx_metrics(&t), TxMetrics { fee: 1, value: 40, approx: false });
    }

    #[test]
    fn unresolved_inputs_give_approx_value() {
        let mut t = tx("a", 300, vec![], vec![bx(BOB, 40), bx(ALICE, 59), bx(FEE_ADDRESS, 1)]);
        t.inputs.push(Input { box_id: "x".into(), resolved: None });
        assert_eq!(tx_metrics(&t), TxMetrics { fee: 1, value: 99, approx: true });
    }
}
