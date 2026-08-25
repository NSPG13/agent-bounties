use alloy::providers::{Provider, RootProvider};
use alloy::rpc::types::request::RpcRequest;
use alloy::rpc::types::error::ProviderError;
use alloy::primitives::U256;
use std::time::Duration;
use tokio::time::sleep;
use thiserror::error_type;

#[derive(Debug, thiserror::Error)]
#[error("Chain mismatch: expected {expected}, got {actual}")]
pub struct ChainMismatchError {
    pub expected: U256,
    pub actual: U256,
}

#[derive(Debug, thiserror::Error)]
#[error("RPC transport error: {0}")]
pub struct RpcTransportError(#[from] anyhow::Error);

#[derive(Debug, Clone)]
pub struct BaseRpcTransport {
    endpoints: Vec<String>,
    chain_id: U256,
    max_retries: u32,
}

impl BaseRpcTransport {
    pub fn new(endpoints: Vec<String>, chain_id: U256) -> Self {
        Self {
            endpoints,
            chain_id,
            max_retries: 3,
        }
    }

    async fn validate_chain<P: Provider<T = RootProvider> + Clone>(&self, provider: &P) -> Result<(), ChainMismatchError> {
        let actual_chain_id = provider.get_chain_id().await.map_err(|e| ChainMismatchError {
            expected: self.chain_id,
            actual: U256::ZERO,
        })?;
        
        if actual_chain_id!= self.chain_id {
            return Err(ChainMismatchError {
                expected: self.chain_id,
                actual: actual_chain_id,
            });
        }
        Ok(())
    }

    pub async fn execute_with_retry<P, T, F, Fut>(&self, provider: &P, f: F) -> Result<T, anyhow::Error>
    where
        P: Provider<T = RootProvider> + Clone,
        F: Fn(P) -> Fut,
        Fut: std::future::Future<Output = Result<T, ProviderError>>,
    {
        let mut last_error: Option<ProviderError> = None;

        for attempt in 0..=self.max_retries {
            if attempt > 0 {
                sleep(Duration::from_millis(200 * 2u64.pow(attempt as u32 - 1))).await;
            }

            // Try endpoints in order
            for endpoint in &self.endpoints {
                // In a real implementation, we'd recreate the provider with the new endpoint
                // For this minimal implementation, we assume the provider is passed in
                // and we handle the retry logic around the call.
                
                match f(provider.clone()).await {
                    Ok(result) => return Ok(result),
                    Err(e) => {
                        // Check if error is retryable (429 or 5xx)
                        let is_retryable = match &e {
                            ProviderError::CallError(_) => false, // JSON-RPC execution error
                            ProviderError::TransportError(te) => {
                                // Simplified check for 429/5xx
                                let msg = te.to_string();
                                msg.contains("429") || msg.contains("500") || msg.contains("502") || msg.contains("503") || msg.contains("504")
                            }
                            _ => false,
                        };

                        if!is_retryable {
                            return Err(anyhow::Error::new(e));
                        }
                        last_error = Some(e);
                    }
                }
            }
        }

        Err(anyhow::Error::new(last_error.unwrap()))
    }
}