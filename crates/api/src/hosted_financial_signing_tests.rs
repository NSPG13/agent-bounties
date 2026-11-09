use super::*;
use service_runtime::hosted_financial_signing::HostedFinancialSigning;

#[test]
fn held_startup_does_not_load_any_financial_key() {
    // Isolated subprocesses exercise the real startup constructors without
    // mutating the environment shared with other tests. Deliberately invalid
    // synthetic credentials make any accidental key read/parse fail closed.
    for value in [None, Some("false"), Some("TRUE"), Some("invalid")] {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap());
        child
            .env_clear()
            .env("HOSTED_SIGNING_HOLD_TEST_CHILD", "1")
            .env("ENABLE_X402_HOSTED_RELAY", "true")
            .env("ENABLE_BOND_SPONSORSHIP", "true")
            .env("ENABLE_BASE_TX_BROADCAST", "true")
            .env("X402_RELAYER_PRIVATE_KEY", "invalid-fixture")
            .env("BOND_SPONSOR_GRANT_SIGNER_PRIVATE_KEY", "invalid-fixture")
            .env("CUSTOMER_REVIEW_SIGNER_PRIVATE_KEY", "invalid-fixture")
            .args([
                "--exact",
                "hosted_financial_signing_tests::held_child",
                "--test-threads=1",
            ]);
        if let Some(value) = value {
            child.env("ENABLE_HOSTED_FINANCIAL_SIGNING", value);
        }
        let output = child.output().unwrap();
        assert!(
            output.status.success(),
            "held startup constructor failed: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed; 0 failed"));
    }
}

#[test]
fn held_child() {
    if std::env::var("HOSTED_SIGNING_HOLD_TEST_CHILD").as_deref() != Ok("1") {
        return;
    }
    let policy = HostedFinancialSigning::from_env();
    assert!(!policy.permits(env_flag("ENABLE_BASE_TX_BROADCAST")));
    let relayer = X402HostedRelayerConfig::from_env(policy).unwrap();
    assert!(!relayer.enabled);
    assert!(
        relayer.relayer.is_none(),
        "saved transaction retries must have no relayer"
    );
    let sponsor = BondSponsorConfig::from_env(policy).unwrap();
    assert!(!sponsor.enabled);
    assert!(sponsor.grant_signer.is_none());
    let managed = managed_payments::ManagedPaymentsConfig::from_env(policy).unwrap();
    assert!(
        managed.review_signer.is_none(),
        "review routes and finalizer must have no signer"
    );
}
