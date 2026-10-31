//! Serialize all hosted transaction senders with the API and legacy keepers.
use crate::OpenCompetitionV2BrokerChainConfig;
use chain_base::{BaseRelayedTransaction, ChainBaseError, EvmTransactionIntent};
use db::PostgresStore;

pub(crate) async fn broadcast(
    store: &PostgresStore,
    network: &str,
    chain: &OpenCompetitionV2BrokerChainConfig,
    intent: &EvmTransactionIntent,
) -> Result<BaseRelayedTransaction, ChainBaseError> {
    let (_, rpc, _) = chain
        .rpc(network)
        .map_err(|_| ChainBaseError::RelayerProvider("Worker RPC unavailable".into()))?;
    let chain_id = chain_base::base_network_descriptor(network)?.chain_id;
    if !chain.relayer.has_gas_budget() {
        return Err(ChainBaseError::RelayerProvider(
            "Shared gas allowance is required".into(),
        ));
    }
    let lease = store
        .acquire_x402_relayer_lease(network, 60)
        .await
        .map_err(|_| ChainBaseError::RelayerProvider("Shared sender lease unavailable".into()))?
        .ok_or_else(|| ChainBaseError::RelayerProvider("Shared sender is busy".into()))?;
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(45),
        chain.relayer.simulate_and_broadcast(
            &rpc,
            chain_id,
            intent,
            chain.max_gas,
            chain.max_fee_per_gas_wei,
        ),
    )
    .await;
    match result {
        Ok(result) => {
            // An acknowledged pending transaction has consumed its pending nonce.
            // On provider errors retain the lease until expiry; its reply may
            // have been lost after the node accepted the transaction.
            if result.is_ok() {
                let _ = store.release_x402_relayer_lease(network, lease).await;
            }
            result
        }
        Err(_) => Err(ChainBaseError::RelayerProvider(
            "Worker relay timed out; reconcile before retrying".into(),
        )),
    }
}
