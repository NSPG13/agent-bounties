//! Optional transport separation for marketplace header reads only.
//! This configuration is never passed to signing, relaying or wallet checks.
use chain_base::{
    base_network_descriptor, BaseNetworkDescriptor, BaseRpcUrlConfig, ChainBaseError,
};

#[derive(Clone, Default)]
pub(super) struct HeadRpcOverrides {
    pub(super) base_mainnet: Option<String>,
    pub(super) base_sepolia: Option<String>,
}

impl HeadRpcOverrides {
    pub(super) fn from_env() -> Self {
        fn value(name: &str) -> Option<String> {
            match std::env::var(name) {
                Ok(value) => Some(value),
                Err(std::env::VarError::NotPresent) => None,
                // Explicit but unusable configuration must not silently fall back.
                Err(std::env::VarError::NotUnicode(_)) => Some(String::new()),
            }
        }
        Self {
            base_mainnet: value("BASE_MAINNET_MARKETPLACE_HEAD_RPC_URL"),
            base_sepolia: value("BASE_SEPOLIA_MARKETPLACE_HEAD_RPC_URL"),
        }
    }

    pub(super) fn resolve(
        &self,
        transaction_rpc: &BaseRpcUrlConfig,
        network: &str,
    ) -> Result<(BaseNetworkDescriptor, String), ChainBaseError> {
        let descriptor = base_network_descriptor(network)?;
        let selected = match descriptor.chain_id {
            8453 => self.base_mainnet.as_ref(),
            84532 => self.base_sepolia.as_ref(),
            _ => None,
        };
        let Some(selected) = selected else {
            return transaction_rpc.resolve(network);
        };
        let unavailable = || {
            ChainBaseError::RpcTransport("marketplace head RPC configuration unavailable".into())
        };
        let url = reqwest::Url::parse(selected).map_err(|_| unavailable())?;
        let secure = url.scheme() == "https";
        #[cfg(test)]
        let secure = secure
            || (url.scheme() == "http"
                && url
                    .host_str()
                    .and_then(|host| host.parse::<std::net::IpAddr>().ok())
                    .is_some_and(|ip| ip.is_loopback()));
        if !secure
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
            || selected.trim() != selected
        {
            return Err(unavailable());
        }
        Ok((descriptor, selected.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_network_aliases_preserve_transaction_configuration() {
        let transaction = BaseRpcUrlConfig {
            base_mainnet: Some("https://transaction.example/mainnet".into()),
            base_sepolia: Some("https://transaction.example/sepolia".into()),
        };
        let defaults = HeadRpcOverrides::default();
        assert_eq!(
            defaults.resolve(&transaction, "base").unwrap().1,
            transaction.base_mainnet.clone().unwrap()
        );
        let heads = HeadRpcOverrides {
            base_mainnet: Some("https://headers.example/mainnet".into()),
            base_sepolia: None,
        };
        for network in ["base-mainnet", "base", "mainnet", "8453"] {
            let (descriptor, url) = heads.resolve(&transaction, network).unwrap();
            assert_eq!(descriptor.chain_id, 8453);
            assert_eq!(url, "https://headers.example/mainnet");
        }
        assert_eq!(
            heads.resolve(&transaction, "sepolia").unwrap().1,
            "https://transaction.example/sepolia"
        );
        assert!(heads.resolve(&transaction, "ethereum").is_err());
        assert_eq!(
            transaction.resolve("base-mainnet").unwrap().1,
            "https://transaction.example/mainnet"
        );
    }

    #[test]
    fn explicit_invalid_configuration_never_falls_back_or_leaks_values() {
        let transaction = BaseRpcUrlConfig {
            base_mainnet: Some("https://valid.example".into()),
            base_sepolia: None,
        };
        for raw in [
            "",
            " ",
            "https://example.org/key#secret",
            "http://example.org",
            "ftp://example.org",
            "https://user:secret@example.org",
            " https://example.org",
        ] {
            let heads = HeadRpcOverrides {
                base_mainnet: Some(raw.into()),
                base_sepolia: None,
            };
            let error = heads.resolve(&transaction, "base-mainnet").unwrap_err();
            assert!(error
                .to_string()
                .contains("marketplace head RPC configuration unavailable"));
            assert!(!error.to_string().contains("secret"));
        }
        assert!(HeadRpcOverrides::default()
            .resolve(&BaseRpcUrlConfig::default(), "base-mainnet")
            .is_err());
    }
}
