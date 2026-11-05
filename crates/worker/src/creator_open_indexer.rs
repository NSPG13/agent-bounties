//! Receipt-verified indexing for hosted creator-open terms, including transactions
//! sent outside the API. A reorg discards the cached projection before replay.
use super::*;
use chain_base::{
    creator_open as protocol, creator_open_service as service, BaseBlockIdentity, EvmLog,
};
use std::collections::BTreeMap;

pub async fn poll_creator_open_indexer(
    store: &PostgresStore,
    release: &service::Release,
    rpc: &str,
) -> anyhow::Result<usize> {
    tokio::time::timeout(Duration::from_secs(90), poll_inner(store, release, rpc))
        .await
        .map_err(|_| ChainBaseError::RpcTransport("creator-open index poll timed out".into()))?
}
async fn poll_inner(
    store: &PostgresStore,
    release: &service::Release,
    rpc: &str,
) -> anyhow::Result<usize> {
    release.validate()?;
    let head = fetch_exact_block_identity(rpc, fetch_block_number(rpc, 1).await?, 2).await?;
    service::verify_release(
        &chain_base::ReqwestJsonRpcTransport::default(),
        rpc,
        release,
        &head,
        Utc::now().timestamp() as u64,
    )
    .await?;
    let safe = fetch_safe_block_identity(rpc, 3).await?;
    if safe.number > head.number
        || safe.timestamp > head.timestamp
        || head.timestamp.saturating_sub(safe.timestamp) > 3600
    {
        anyhow::bail!("creator-open safe index head unavailable");
    }
    store.seed_creator_open_indexes(&release.network).await?;
    let mut updated = 0;
    for item in store
        .creator_open_index_batch(&release.network, None)
        .await?
    {
        let result = tokio::time::timeout(
            Duration::from_secs(45),
            poll_one(store, release, rpc, &safe, &item),
        )
        .await;
        match result {
            Ok(Ok(true)) => updated += 1,
            Ok(Ok(false)) => {}
            Ok(Err(error)) => {
                if let Err(persistence) = store.invalidate_creator_open_index(&item, false).await {
                    return Err(indexer_persistence_failure(&error, persistence));
                }
                // Stop the batch on a provider failure; do not hammer the same
                // provider once for every remaining bounty.
                return Err(error);
            }
            Err(_) => {
                store.invalidate_creator_open_index(&item, false).await?;
                return Err(chain_base::ChainBaseError::RpcTransport(
                    "creator-open item timed out".into(),
                )
                .into());
            }
        }
    }
    Ok(updated)
}
async fn receipt_logs(
    rpc: &str,
    hash: &str,
    safe: &BaseBlockIdentity,
) -> anyhow::Result<Option<(BaseBlockIdentity, Vec<EvmLog>)>> {
    let Some(receipt) = chain_base::fetch_transaction_receipt(rpc, hash, 4)
        .await?
        .result
    else {
        return Ok(None);
    };
    if !receipt.transaction_hash.eq_ignore_ascii_case(hash) || receipt.succeeded()? != Some(true) {
        return Ok(None);
    }
    let Some(number) = receipt.block_number()? else {
        return Ok(None);
    };
    if number > safe.number {
        return Ok(None);
    }
    let block = fetch_exact_block_identity(rpc, number, 5).await?;
    if receipt.block_hash.as_ref() != Some(&block.hash) {
        return Ok(None);
    }
    let mut logs = receipt.logs_to_evm_logs()?;
    for l in &mut logs {
        if l.block_number != number || !l.tx_hash.eq_ignore_ascii_case(hash) {
            anyhow::bail!("inconsistent creator-open receipt log");
        }
        l.occurred_at = DateTime::from_timestamp(i64::try_from(block.timestamp)?, 0);
    }
    Ok(Some((block, logs)))
}
async fn poll_one(
    store: &PostgresStore,
    release: &service::Release,
    rpc: &str,
    safe: &BaseBlockIdentity,
    item: &db::CreatorOpenIndex,
) -> anyhow::Result<bool> {
    let p = service::prepare(release, &item.request, &service::Signatures::default())?;
    if item.bounty_contract != format!("{:#x}", p.bounty_contract) {
        anyhow::bail!("index identity conflict");
    }
    if let Some(checkpoint) = &item.checkpoint {
        if checkpoint.number > safe.number {
            store.invalidate_creator_open_index(item, true).await?;
            return Ok(false);
        }
        let actual = fetch_exact_block_identity(rpc, checkpoint.number, 6).await?;
        if actual.hash != checkpoint.hash {
            store.invalidate_creator_open_index(item, true).await?;
            return Ok(false);
        }
        if checkpoint.number > safe.number {
            anyhow::bail!("safe head moved behind checkpoint");
        }
    }
    let Some((creation, creation_logs)) = receipt_logs(rpc, &item.transaction_hash, safe).await?
    else {
        store.invalidate_creator_open_index(item, true).await?;
        return Ok(false);
    };
    protocol::project(
        &item.request.original_terms,
        release.factory,
        release.implementation,
        item.request.user_salt,
        creation.timestamp,
        &creation_logs,
    )?;
    let mut logs = if item.checkpoint.is_some() {
        item.logs.clone()
    } else {
        creation_logs
    };
    let from = item
        .checkpoint
        .as_ref()
        .map_or(creation.number, |b| b.number.saturating_add(1));
    // The public Base endpoint limits eth_getLogs to 1,000 inclusive blocks.
    // Persist each verified page so downtime cannot trap this cursor behind an
    // oversized request. Partial catch-up remains unavailable to public reads.
    let to = safe.number.min(from.saturating_add(999));
    if from <= to {
        let observed = fetch_base_contract_logs(
            rpc,
            &BaseContractLogQuery {
                contract: item.bounty_contract.clone(),
                from_block: from,
                to_block: Some(to),
                topic0: vec![],
            },
            7,
        )
        .await?;
        let mut receipts = BTreeMap::new();
        for raw in observed.result {
            let candidate = raw.to_evm_log()?;
            if candidate.block_number < from
                || candidate.block_number > to
                || !candidate
                    .address
                    .eq_ignore_ascii_case(&item.bounty_contract)
            {
                anyhow::bail!("log escaped creator-open query");
            }
            if !receipts.contains_key(&candidate.tx_hash) {
                let value = receipt_logs(rpc, &candidate.tx_hash, safe)
                    .await?
                    .ok_or_else(|| anyhow!("unconfirmed creator-open event"))?;
                receipts.insert(candidate.tx_hash.clone(), value);
            }
            let (_, verified) = &receipts[&candidate.tx_hash];
            let exact = verified
                .iter()
                .find(|l| {
                    l.log_index == candidate.log_index
                        && l.block_number == candidate.block_number
                        && l.address.eq_ignore_ascii_case(&candidate.address)
                        && l.topics == candidate.topics
                        && l.data == candidate.data
                })
                .ok_or_else(|| anyhow!("log absent from canonical receipt"))?;
            if let Some(previous) = logs
                .iter()
                .find(|l| l.block_number == exact.block_number && l.log_index == exact.log_index)
            {
                if serde_json::to_value(previous)? != serde_json::to_value(exact)? {
                    anyhow::bail!("conflicting creator-open log position");
                }
            } else {
                logs.push(exact.clone());
            }
        }
    }
    let checkpoint = fetch_exact_block_identity(rpc, to.max(creation.number), 8).await?;
    // Recheck the old prefix and selected safe head after the RPC reads. Its hash
    // commits to all ancestors; no mixed-fork snapshot can become public.
    if fetch_exact_block_identity(rpc, safe.number, 9).await?.hash != safe.hash {
        anyhow::bail!("safe head changed during scan");
    }
    if let Some(old) = &item.checkpoint {
        if fetch_exact_block_identity(rpc, old.number, 10).await?.hash != old.hash {
            store.invalidate_creator_open_index(item, true).await?;
            return Ok(false);
        }
    }
    let projection = protocol::project(
        &item.request.original_terms,
        release.factory,
        release.implementation,
        item.request.user_salt,
        creation.timestamp,
        &logs,
    )?;
    service::verify_projection(
        &chain_base::ReqwestJsonRpcTransport::default(),
        rpc,
        &projection,
        &checkpoint,
    )
    .await?;
    store
        .save_creator_open_index(
            item,
            &checkpoint,
            &logs,
            &projection,
            checkpoint.number == safe.number,
        )
        .await
        .map_err(Into::into)
}
