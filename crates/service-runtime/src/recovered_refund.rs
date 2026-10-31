//! Read-only canonical qualification for recovered card-bounty principal. No wallet key.
use alloy::primitives::keccak256;
use chain_base::JsonRpcTransport;
use chain_base::{base_network_descriptor, decode_autonomous_bounty_logs, EvmLog};
use db::managed_service_orders::ManagedRecoveryTarget;
use managed_orders::{
    deposit_payout::TRANSFER_TOPIC,
    recovery::{verify_recovered_principal, RecoveredPrincipal, RecoveryBlock, RecoveryState},
};
use serde_json::{json, Value};

#[derive(Debug, thiserror::Error)]
#[error("canonical recovery qualification unavailable or inconsistent")]
pub struct RecoveryReadError;
type Result<T> = std::result::Result<T, RecoveryReadError>;
fn qty(v: &Value) -> Result<u64> {
    u64::from_str_radix(
        v.as_str()
            .and_then(|s| s.strip_prefix("0x"))
            .ok_or(RecoveryReadError)?,
        16,
    )
    .map_err(|_| RecoveryReadError)
}
fn text(v: &Value) -> Result<String> {
    v.as_str()
        .map(str::to_ascii_lowercase)
        .ok_or(RecoveryReadError)
}
fn digest(v: &Value) -> Result<String> {
    let bytes = hex::decode(text(v)?.strip_prefix("0x").ok_or(RecoveryReadError)?)
        .map_err(|_| RecoveryReadError)?;
    Ok(format!("0x{}", hex::encode(keccak256(bytes))))
}
fn call_data(signature: &str, address: Option<&str>) -> String {
    let mut data = format!("0x{}", hex::encode(&keccak256(signature.as_bytes())[..4]));
    if let Some(a) = address {
        data.push_str(&format!("{:0>64}", a.trim_start_matches("0x")))
    }
    data
}
async fn rpc(
    transport: &impl JsonRpcTransport,
    url: &str,
    method: &str,
    params: Value,
) -> Result<Value> {
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(12),
        transport.post_json_value(
            url,
            &json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}),
        ),
    )
    .await
    .map_err(|_| RecoveryReadError)?
    .map_err(|_| RecoveryReadError)?;
    if result.get("error").is_some() {
        return Err(RecoveryReadError);
    }
    result
        .get("result")
        .filter(|v| !v.is_null())
        .cloned()
        .ok_or(RecoveryReadError)
}
async fn call(
    transport: &impl JsonRpcTransport,
    url: &str,
    to: &str,
    signature: &str,
    address: Option<&str>,
    tag: &str,
) -> Result<Value> {
    rpc(
        transport,
        url,
        "eth_call",
        json!([{"to":to,"data":call_data(signature,address)},tag]),
    )
    .await
}
fn pin(network: &str) -> Result<(&'static str, &'static str, &'static str)> {
    match network {
        "base-mainnet" => Ok((
            "0xc33e2ae33bb9580837ea59df18e57fa1039ae58a",
            "0x8420c9bd1ff8a6b1abc4230b349538612c58f3a6",
            "0x3a706b2753ee4c6c05379cb17a3db21f5a7eb5aca6b207166611fed8baf1dc57",
        )),
        // Identity read-verified during the 27 October qualification; this is not a claim of
        // an actual testnet refund rehearsal. Both chains remain behind execution gates.
        "base-sepolia" => Ok((
            "0x44de6074440fc006c20087991364ce65c87e6d88",
            "0x9482786192e16ca4eb55f61787af664acd6b8e53",
            "0xff87a0ebc272fd18828ae50fd448d4112677b5b749f8aff01c40f0fec8956413",
        )),
        _ => Err(RecoveryReadError),
    }
}
/// Re-read exact receipts against safe-chain headers and immutable runtime pins before every
/// attempt. Indexed history supplies only transaction locators, never confirmation authority.
pub async fn qualify_recovery(
    url: &str,
    target: &ManagedRecoveryTarget,
    hashes: &[String],
) -> Result<RecoveredPrincipal> {
    let (_, _, factory_hash) = pin(&target.funding_plan.network)?;
    qualify_with(
        &chain_base::ReqwestJsonRpcTransport::default(),
        url,
        target,
        hashes,
        factory_hash,
        "0x7420d074883da31643c5069b63dd9c1bd75e405ebd9a3d76bf513099c3d0314e",
    )
    .await
}
async fn qualify_with(
    transport: &impl JsonRpcTransport,
    url: &str,
    target: &ManagedRecoveryTarget,
    hashes: &[String],
    factory_hash: &str,
    implementation_hash: &str,
) -> Result<RecoveredPrincipal> {
    if hashes.is_empty() || hashes.len() > 3 || !hashes.contains(&target.funding_tx) {
        return Err(RecoveryReadError);
    }
    let plan = &target.funding_plan;
    let (factory, implementation, _) = pin(&plan.network)?;
    let network = base_network_descriptor(&plan.network).map_err(|_| RecoveryReadError)?;
    if plan.factory != factory
        || qty(&rpc(transport, url, "eth_chainId", json!([])).await?)? != network.chain_id
    {
        return Err(RecoveryReadError);
    }
    let safe = rpc(
        transport,
        url,
        "eth_getBlockByNumber",
        json!(["safe", false]),
    )
    .await?;
    let safe_number = qty(&safe["number"])?;
    let safe_hash = text(&safe["hash"])?;
    let tag = format!("0x{safe_number:x}");
    if digest(&rpc(transport, url, "eth_getCode", json!([factory, tag])).await?)? != factory_hash
        || digest(&rpc(transport, url, "eth_getCode", json!([implementation, tag])).await?)?
            != implementation_hash
        || text(
            &rpc(
                transport,
                url,
                "eth_getCode",
                json!([plan.predicted_bounty, tag]),
            )
            .await?,
        )? != format!(
            "0x363d3d373d3d3d363d73{}5af43d82803e903d91602b57fd5bf3",
            implementation.trim_start_matches("0x")
        )
        || qty(&call(
            transport,
            url,
            factory,
            "isCanonicalBounty(address)",
            Some(&plan.predicted_bounty),
            &tag,
        )
        .await?)?
            != 1
    {
        return Err(RecoveryReadError);
    }
    let token = network.native_usdc_token_address.to_ascii_lowercase();
    if text(
        &call(
            transport,
            url,
            &plan.predicted_bounty,
            "settlementToken()",
            None,
            &tag,
        )
        .await?,
    )? != format!("0x{:0>64}", token.trim_start_matches("0x"))
    {
        return Err(RecoveryReadError);
    }
    let state = RecoveryState {
        cancelled: qty(&call(
            transport,
            url,
            &plan.predicted_bounty,
            "status()",
            None,
            &tag,
        )
        .await?)?
            == 5,
        active_claim_bond: qty(&call(
            transport,
            url,
            &plan.predicted_bounty,
            "activeClaimBond()",
            None,
            &tag,
        )
        .await?)?,
        pending_bond_refund_total: qty(&call(
            transport,
            url,
            &plan.predicted_bounty,
            "pendingBondRefundTotal()",
            None,
            &tag,
        )
        .await?)?,
        funding_wallet_contribution: qty(&call(
            transport,
            url,
            &plan.predicted_bounty,
            "contributions(address)",
            Some(&plan.funding_wallet),
            &tag,
        )
        .await?)?,
    };
    if !state.cancelled
        || state.pending_bond_refund_total != 0
        || state.active_claim_bond != 0
        || state.funding_wallet_contribution != 0
    {
        return Err(RecoveryReadError);
    }
    let mut events = vec![];
    let mut receipts = vec![];
    let mut seen = std::collections::HashSet::new();
    for hash in hashes {
        if hash.len() != 66 || !hash.starts_with("0x") || !seen.insert(hash) {
            return Err(RecoveryReadError);
        }
        let receipt = rpc(transport, url, "eth_getTransactionReceipt", json!([hash])).await?;
        let number = qty(&receipt["blockNumber"])?;
        let block_hash = text(&receipt["blockHash"])?;
        let block = rpc(
            transport,
            url,
            "eth_getBlockByNumber",
            json!([format!("0x{number:x}"), false]),
        )
        .await?;
        if receipt["status"] != "0x1"
            || text(&receipt["transactionHash"])? != *hash
            || text(&block["hash"])? != block_hash
            || qty(&block["number"])? != number
            || safe_number < number.saturating_add(2)
            || number < target.funded_block
        {
            return Err(RecoveryReadError);
        }
        let logs = receipt["logs"]
            .as_array()
            .filter(|l| l.len() <= 512)
            .ok_or(RecoveryReadError)?;
        let mut parsed = vec![];
        for log in logs {
            if log["removed"] == true
                || text(&log["transactionHash"])? != *hash
                || text(&log["blockHash"])? != block_hash
                || qty(&log["blockNumber"])? != number
            {
                return Err(RecoveryReadError);
            }
            let address = text(&log["address"])?;
            if address != factory && address != plan.predicted_bounty {
                continue;
            }
            parsed.push(EvmLog {
                address,
                topics: serde_json::from_value(log["topics"].clone())
                    .map_err(|_| RecoveryReadError)?,
                data: text(&log["data"])?,
                tx_hash: hash.clone(),
                block_number: number,
                log_index: qty(&log["logIndex"])?,
                occurred_at: None,
            });
        }
        events.extend(decode_autonomous_bounty_logs(parsed).map_err(|_| RecoveryReadError)?);
        receipts.push(receipt);
    }
    if !matches!(managed_orders::funding::observe_funding(plan,&events),
        managed_orders::funding::FundingObservation::Funded { tx_hash, block_number }
        if tx_hash==target.funding_tx && block_number==target.funded_block)
    {
        return Err(RecoveryReadError);
    }
    let withdrawal = events
        .iter()
        .find(|e| {
            e.kind == chain_base::AutonomousBountyEventKind::RefundWithdrawn
                && e.data["contributor"] == plan.funding_wallet
        })
        .ok_or(RecoveryReadError)?;
    let receipt = receipts
        .iter()
        .find(|r| r["transactionHash"] == withdrawal.tx_hash)
        .ok_or(RecoveryReadError)?;
    let amount = withdrawal.data["amount"]
        .as_u64()
        .ok_or(RecoveryReadError)?;
    let from = format!("0x{:0>64}", plan.predicted_bounty.trim_start_matches("0x"));
    let to = format!("0x{:0>64}", plan.funding_wallet.trim_start_matches("0x"));
    let transfers = receipt["logs"]
        .as_array()
        .ok_or(RecoveryReadError)?
        .iter()
        .filter(|l| {
            l["address"]
                .as_str()
                .is_some_and(|v| v.eq_ignore_ascii_case(&token))
                && l["topics"] == json!([TRANSFER_TOPIC, from, to])
                && l["data"] == format!("0x{amount:064x}")
        })
        .count();
    // Reject a safe checkpoint that changed while the dependent reads were in progress.
    if text(&rpc(transport, url, "eth_getBlockByNumber", json!([tag, false])).await?["hash"])?
        != safe_hash
    {
        return Err(RecoveryReadError);
    }
    verify_recovered_principal(
        &target.order.quote,
        plan,
        &events,
        RecoveryBlock {
            number: safe_number,
            hash: safe_hash,
        },
        RecoveryBlock {
            number: withdrawal.block_number,
            hash: text(&receipt["blockHash"])?,
        },
        state,
        transfers == 1,
    )
    .map_err(|_| RecoveryReadError)
}

#[cfg(test)]
mod tests;
