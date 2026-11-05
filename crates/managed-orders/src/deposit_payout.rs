//! Proof that a forfeited review deposit reached the solver: a successful, confirmed receipt
//! with a canonical-USDC `Transfer` of at least the owed amount to exactly the solver. The
//! sender is not constrained, so any operator wallet may pay.

/// keccak256("Transfer(address,address,uint256)")
pub const TRANSFER_TOPIC: &str =
    "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef";
/// Blocks, including the receipt's own, before a payout counts as paid.
pub const PAYOUT_CONFIRMATIONS: u64 = 3;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PayoutError {
    #[error("the transaction is missing, failed, or not yet confirmed")]
    Unconfirmed,
    #[error("the transaction has no USDC transfer of the owed amount to the solver")]
    NoTransfer,
}

fn hex_u64(value: &serde_json::Value) -> Option<u64> {
    u64::from_str_radix(value.as_str()?.strip_prefix("0x")?, 16).ok()
}

/// The receipt's block when it pays `amount_micro_usdc` of `token` to `solver`.
pub fn verify_deposit_payout(
    receipt: &serde_json::Value,
    tx_hash: &str,
    latest_block: u64,
    token: &str,
    solver: &str,
    amount_micro_usdc: u64,
) -> Result<u64, PayoutError> {
    let block = hex_u64(&receipt["blockNumber"]).ok_or(PayoutError::Unconfirmed)?;
    if receipt["status"] != "0x1"
        || !receipt["transactionHash"]
            .as_str()
            .is_some_and(|hash| hash.eq_ignore_ascii_case(tx_hash))
        || latest_block.saturating_sub(block) + 1 < PAYOUT_CONFIRMATIONS
        || latest_block < block
    {
        return Err(PayoutError::Unconfirmed);
    }
    let recipient = format!(
        "0x{:0>64}",
        solver.trim_start_matches("0x").to_ascii_lowercase()
    );
    let paid = receipt["logs"].as_array().into_iter().flatten().any(|log| {
        let topics = log["topics"].as_array();
        log["removed"] != true
            && log["address"]
                .as_str()
                .is_some_and(|address| address.eq_ignore_ascii_case(token))
            && topics.is_some_and(|topics| {
                topics.len() == 3
                    && topics[0]
                        .as_str()
                        .is_some_and(|t| t.eq_ignore_ascii_case(TRANSFER_TOPIC))
                    && topics[2]
                        .as_str()
                        .is_some_and(|t| t.eq_ignore_ascii_case(&recipient))
            })
            && log["data"]
                .as_str()
                .and_then(|data| data.strip_prefix("0x"))
                .filter(|data| data.len() == 64)
                .and_then(|data| u128::from_str_radix(data, 16).ok())
                .is_some_and(|value| value >= u128::from(amount_micro_usdc))
    });
    if !paid {
        return Err(PayoutError::NoTransfer);
    }
    Ok(block)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const TOKEN: &str = "0x036cbd53842c5426634e7929541ec2318f3dcf7e";
    const SOLVER: &str = "0x5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e";
    const TX: &str = "0x00000000000000000000000000000000000000000000000000000000000000a1";

    fn receipt(token: &str, to: &str, value: u64) -> serde_json::Value {
        json!({"transactionHash": TX, "status": "0x1", "blockNumber": "0x64", "logs": [
            {"address": token, "removed": false, "data": format!("0x{value:064x}"),
             "topics": [TRANSFER_TOPIC, format!("0x{:0>64}", "ab".repeat(20)),
                        format!("0x{:0>64}", to.trim_start_matches("0x"))]}]})
    }

    #[test]
    fn only_a_confirmed_usdc_transfer_of_the_owed_amount_to_the_solver_counts() {
        let good = receipt(TOKEN, SOLVER, 10_000_000);
        assert_eq!(
            verify_deposit_payout(&good, TX, 102, TOKEN, SOLVER, 10_000_000),
            Ok(100)
        );
        assert_eq!(
            verify_deposit_payout(&good, TX, 101, TOKEN, SOLVER, 10_000_000),
            Err(PayoutError::Unconfirmed)
        );
        let mut failed = good.clone();
        failed["status"] = json!("0x0");
        let mut other_tx = good.clone();
        other_tx["transactionHash"] = json!(format!("0x{}", "ff".repeat(32)));
        let mut removed = good.clone();
        removed["logs"][0]["removed"] = json!(true);
        for (bad, error) in [
            (failed, PayoutError::Unconfirmed),
            (other_tx, PayoutError::Unconfirmed),
            (removed, PayoutError::NoTransfer),
            (receipt(TOKEN, SOLVER, 9_999_999), PayoutError::NoTransfer),
            (
                receipt(TOKEN, &format!("0x{}", "77".repeat(20)), 10_000_000),
                PayoutError::NoTransfer,
            ),
            (
                receipt(&format!("0x{}", "11".repeat(20)), SOLVER, 10_000_000),
                PayoutError::NoTransfer,
            ),
        ] {
            assert_eq!(
                verify_deposit_payout(&bad, TX, 200, TOKEN, SOLVER, 10_000_000),
                Err(error)
            );
        }
    }
}
