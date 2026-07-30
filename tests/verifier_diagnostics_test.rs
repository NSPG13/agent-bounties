use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

mod types {
    pub mod verifier {
        use serde::{Deserialize, Serialize};

        #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
        pub struct VerifierSet {
            pub hash: String,
            pub threshold: u32,
            pub signers: Vec<String>,
        }

        #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
        pub struct RunnerInfo {
            pub identifier: String,
            pub version: String,
            pub last_seen: i64,
        }

        #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
        #[serde(rename_all = "snake_case")]
        pub enum VerifierReadiness {
            Ready,
            MissingSigner { reason: String },
            StaleRunner { reason: String },
            VerifierSetMismatch { reason: String },
        }

        impl VerifierReadiness {
            pub fn is_ready(&self) -> bool {
                matches!(self, VerifierReadiness::Ready)
            }
        }

        #[derive(Debug, Clone, Serialize, Deserialize)]
        pub struct VerifierDiagnostics {
            pub verifier_set_hash: String,
            pub threshold: u32,
            pub runner_identifier: String,
            pub readiness: VerifierReadiness,
        }
    }
}

use types::verifier::{RunnerInfo, VerifierDiagnostics, VerifierReadiness, VerifierSet};

const RUNNER_STALE_THRESHOLD_SECS: i64 = 300;

struct TestVerifierService {
    verifier_sets: Arc<RwLock<HashMap<String, VerifierSet>>>,
    runners: Arc<RwLock<HashMap<String, RunnerInfo>>>,
}

impl TestVerifierService {
    fn new() -> Self {
        Self {
            verifier_sets: Arc::new(RwLock::new(HashMap::new())),
            runners: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    fn register_verifier_set(&self, contract: &str, verifier_set: VerifierSet) {
        self.verifier_sets.write().unwrap().insert(contract.to_string(), verifier_set);
    }

    fn register_runner(&self, contract: &str, runner: RunnerInfo) {
        self.runners.write().unwrap().insert(contract.to_string(), runner);
    }

    fn check_readiness(&self, contract: &str, expected_hash: &str) -> VerifierDiagnostics {
        let verifier_sets = self.verifier_sets.read().unwrap();
        let runners = self.runners.read().unwrap();

        let verifier_set = verifier_sets.get(contract);
        let runner = runners.get(contract);

        let (verifier_set_hash, threshold, runner_identifier, readiness) = match (verifier_set, runner) {
            (Some(vs), Some(r)) => {
                let current_time = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs() as i64;

                let readiness = if vs.hash != expected_hash {
                    VerifierReadiness::VerifierSetMismatch {
                        reason: format!("expected {}, got {}", expected_hash, vs.hash),
                    }
                } else if vs.signers.len() < vs.threshold as usize {
                    VerifierReadiness::MissingSigner {
                        reason: format!("{} of {} signers available", vs.signers.len(), vs.threshold),
                    }
                } else if current_time - r.last_seen > RUNNER_STALE_THRESHOLD_SECS {
                    VerifierReadiness::StaleRunner {
                        reason: format!("last seen {} seconds ago", current_time - r.last_seen),
                    }
                } else {
                    VerifierReadiness::Ready
                };

                (vs.hash.clone(), vs.threshold, r.identifier.clone(), readiness)
            }
            (Some(vs), None) => (
                vs.hash.clone(),
                vs.threshold,
                "unknown".to_string(),
                VerifierReadiness::StaleRunner {
                    reason: "no runner registered".to_string(),
                },
            ),
            (None, _) => (
                "unknown".to_string(),
                0,
                "unknown".to_string(),
                VerifierReadiness::VerifierSetMismatch {
                    reason: "no verifier set registered".to_string(),
                },
            ),
        };

        VerifierDiagnostics {
            verifier_set_hash,
            threshold,
            runner_identifier,
            readiness,
        }
    }
}

#[test]
fn test_healthy_verifier_state() {
    let service = TestVerifierService::new();
    let contract = "0xtest123";
    let expected_hash = "hash_abc";

    let current_time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    service.register_verifier_set(
        contract,
        VerifierSet {
            hash: expected_hash.to_string(),
            threshold: 2,
            signers: vec!["signer1".to_string(), "signer2".to_string()],
        },
    );

    service.register_runner(
        contract,
        RunnerInfo {
            identifier: "runner_v1".to_string(),
            version: "1.0.0".to_string(),
            last_seen: current_time,
        },
    );

    let diagnostics = service.check_readiness(contract, expected_hash);

    assert_eq!(diagnostics.verifier_set_hash, expected_hash);
    assert_eq!(diagnostics.threshold, 2);
    assert_eq!(diagnostics.runner_identifier, "runner_v1");
    assert!(matches!(diagnostics.readiness, VerifierReadiness::Ready));
    assert!(diagnostics.readiness.is_ready());
}

#[test]
fn test_missing_signer_state() {
    let service = TestVerifierService::new();
    let contract = "0xtest456";
    let expected_hash = "hash_def";

    let current_time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    service.register_verifier_set(
        contract,
        VerifierSet {
            hash: expected_hash.to_string(),
            threshold: 3,
            signers: vec!["signer1".to_string()],
        },
    );

    service.register_runner(
        contract,
        RunnerInfo {
            identifier: "runner_v1".to_string(),
            version: "1.0.0".to_string(),
            last_seen: current_time,
        },
    );

    let diagnostics = service.check_readiness(contract, expected_hash);

    assert_eq!(diagnostics.threshold, 3);
    assert!(matches!(
        diagnostics.readiness,
        VerifierReadiness::MissingSigner { .. }
    ));
    assert!(!diagnostics.readiness.is_ready());
}

#[test]
fn test_stale_runner_state() {
    let service = TestVerifierService::new();
    let contract = "0xtest789";
    let expected_hash = "hash_ghi";

    let stale_time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
        - 600;

    service.register_verifier_set(
        contract,
        VerifierSet {
            hash: expected_hash.to_string(),
            threshold: 2,
            signers: vec!["signer1".to_string(), "signer2".to_string()],
        },
    );

    service.register_runner(
        contract,
        RunnerInfo {
            identifier: "runner_v1".to_string(),
            version: "1.0.0".to_string(),
            last_seen: stale_time,
        },
    );

    let diagnostics = service.check_readiness(contract, expected_hash);

    assert!(matches!(
        diagnostics.readiness,
        VerifierReadiness::StaleRunner { .. }
    ));
    assert!(!diagnostics.readiness.is_ready());
}

#[test]
fn test_verifier_set_mismatch_state() {
    let service = TestVerifierService::new();
    let contract = "0xtestmismatch";
    let expected_hash = "hash_expected";
    let actual_hash = "hash_actual";

    let current_time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    service.register_verifier_set(
        contract,
        VerifierSet {
            hash: actual_hash.to_string(),
            threshold: 2,
            signers: vec!["signer1".to_string(), "signer2".to_string()],
        },
    );

    service.register_runner(
        contract,
        RunnerInfo {
            identifier: "runner_v1".to_string(),
            version: "1.0.0".to_string(),
            last_seen: current_time,
        },
    );

    let diagnostics = service.check_readiness(contract, expected_hash);

    assert_eq!(diagnostics.verifier_set_hash, actual_hash);
    assert!(matches!(
        diagnostics.readiness,
        VerifierReadiness::VerifierSetMismatch { .. }
    ));
    assert!(!diagnostics.readiness.is_ready());
}
