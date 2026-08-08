use crate::api::bounty::VerifierService;
use crate::types::verifier::VerifierDiagnostics;
use serde_json::{json, Value};

pub struct DiagnosticsMcp {
    verifier_service: VerifierService,
}

impl DiagnosticsMcp {
    pub fn new(verifier_service: VerifierService) -> Self {
        Self { verifier_service }
    }

    pub fn get_verifier_diagnostics(&self, contract: &str) -> Value {
        let diagnostics = self.verifier_service.check_readiness(contract, "expected_hash_placeholder");

        json!({
            "verifier_set_hash": diagnostics.verifier_set_hash,
            "threshold": diagnostics.threshold,
            "runner_identifier": diagnostics.runner_identifier,
            "readiness": diagnostics.readiness,
            "is_ready": diagnostics.readiness.is_ready(),
            "reason": diagnostics.readiness.reason()
        })
    }

    pub fn list_ready_bounties(&self) -> Value {
        let contracts = vec!["0xc710d54d192ffb0b84cd6e051754ab70acf1130c"];
        let mut ready_bounties = Vec::new();

        for contract in contracts {
            let diagnostics = self.verifier_service.check_readiness(contract, "expected_hash_placeholder");

            if diagnostics.readiness.is_ready() {
                ready_bounties.push(json!({
                    "contract": contract,
                    "diagnostics": {
                        "verifier_set_hash": diagnostics.verifier_set_hash,
                        "threshold": diagnostics.threshold,
                        "runner_identifier": diagnostics.runner_identifier,
                        "readiness": "ready"
                    }
                }));
            }
        }

        json!({
            "ready_bounties": ready_bounties
        })
    }
}
