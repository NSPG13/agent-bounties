#[cfg(test)]
mod integration_fixtures {
    use std::time::Duration;

    /// Known-good Base mainnet RPC endpoints for integration tests
    pub const BASE_MAINNET_ENDPOINTS: &[&str] = &[
        "https://mainnet.base.org",
        "https://base.llamarpc.com",
        "https://base-rpc.publicnode.com",
    ];

    /// Expected Base mainnet chain ID as hex
    pub const BASE_CHAIN_ID_HEX: &str = "0x2105";

    /// Expected Base mainnet chain ID as u64
    pub const BASE_CHAIN_ID: u64 = 8453;

    /// Test fixture: valid retry configuration
    pub fn default_retry_config() -> RetryConfig {
        RetryConfig {
            max_retries_per_endpoint: 3,
            base_backoff: Duration::from_secs(1),
            max_backoff: Duration::from_secs(15),
            request_timeout: Duration::from_secs(30),
            jitter_factor: 0.1,
        }
    }

    /// Test fixture: aggressive retry config for CI (short timeouts)
    pub fn ci_retry_config() -> RetryConfig {
        RetryConfig {
            max_retries_per_endpoint: 1,
            base_backoff: Duration::from_millis(100),
            max_backoff: Duration::from_secs(1),
            request_timeout: Duration::from_secs(5),
            jitter_factor: 0.0,
        }
    }

    /// Test fixture: simulated HTTP 429 rate-limit response
    pub fn rate_limit_response() -> String {
        r#"{"jsonrpc":"2.0","error":{"code":-32000,"message":"rate limit exceeded"},"id":1}"#.to_string()
    }

    /// Test fixture: simulated HTTP 503 server error response
    pub fn server_error_response() -> String {
        r#"{"jsonrpc":"2.0","error":{"code":-32000,"message":"internal server error"},"id":1}"#.to_string()
    }

    /// Test fixture: valid eth_chainId response for Base mainnet
    pub fn valid_chain_id_response() -> String {
        r#"{"jsonrpc":"2.0","id":1,"result":"0x2105"}"#.to_string()
    }

    /// Test fixture: wrong chain ID response (Sepolia testnet)
    pub fn wrong_chain_id_response() -> String {
        r#"{"jsonrpc":"2.0","id":1,"result":"0x14a34"}"#.to_string()
    }

    /// Endpoint validation helper: checks endpoint URL format
    pub fn is_valid_https_endpoint(url: &str) -> bool {
        url.starts_with("https://") && url.len() > 10 && !url.contains('@')
    }
}

#[derive(Debug, Clone)]
pub struct RetryConfig {
    pub max_retries_per_endpoint: u32,
    pub base_backoff: Duration,
    pub max_backoff: Duration,
    pub request_timeout: Duration,
    pub jitter_factor: f64,
}
