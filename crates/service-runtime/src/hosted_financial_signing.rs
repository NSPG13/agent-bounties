//! Startup fence for API/MCP financial keys and raw transaction broadcast.
//! Feature switches alone cannot fence retries of previously saved transactions.

#[derive(Clone, Copy, Debug, Default)]
pub struct HostedFinancialSigning(bool);

impl HostedFinancialSigning {
    pub fn from_env() -> Self {
        Self::from_value(
            std::env::var("ENABLE_HOSTED_FINANCIAL_SIGNING")
                .ok()
                .as_deref(),
        )
    }

    /// Only an exact, explicit opt-in authorizes loading financial signing keys.
    pub fn from_value(value: Option<&str>) -> Self {
        Self(value == Some("true"))
    }

    pub fn permits(self, feature_enabled: bool) -> bool {
        self.0 && feature_enabled
    }

    /// Do not even read a credential while held. Absence prevents saved relay
    /// retries and background review finalizers from acquiring a signer.
    pub fn secret(self, name: &str) -> Option<String> {
        self.read_secret(|| std::env::var(name).ok())
    }

    fn read_secret(self, read: impl FnOnce() -> Option<String>) -> Option<String> {
        if self.0 {
            read()
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosted_financial_signing_hold_never_reads_credentials_or_enables_features() {
        for value in [
            None,
            Some("false"),
            Some(""),
            Some("1"),
            Some("TRUE"),
            Some(" true"),
            Some("true\n"),
        ] {
            let policy = HostedFinancialSigning::from_value(value);
            assert!(!policy.permits(true));
            assert!(!policy.permits(false));
            assert!(policy
                .read_secret(|| panic!("held signer must not read a key"))
                .is_none());
        }
    }

    #[test]
    fn hosted_financial_signing_opt_in_preserves_independent_feature_gates() {
        let policy = HostedFinancialSigning::from_value(Some("true"));
        assert!(policy.permits(true));
        assert!(!policy.permits(false));
        assert_eq!(
            policy
                .read_secret(|| Some("synthetic-fixture".into()))
                .as_deref(),
            Some("synthetic-fixture")
        );
        assert!(policy.read_secret(|| None).is_none());
    }
}
