use crate::types::verifier::{RunnerInfo, VerifierDiagnostics, VerifierReadiness, VerifierSet};
use actix_web::{get, web, HttpResponse, Result};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

const RUNNER_STALE_THRESHOLD_SECS: i64 = 300;

#[derive(Debug, Serialize, Deserialize)]
pub struct BountyResponse {
    pub contract: String,
    pub amount: String,
    pub status: String,
    pub diagnostics: VerifierDiagnostics,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BountyListResponse {
    pub bounties: Vec<BountyResponse>,
}

pub struct VerifierService {
    verifier_sets: Arc<std::sync::RwLock<std::collections::HashMap<String, VerifierSet>>>,
    runners: Arc<std::sync::RwLock<std::collections::HashMap<String, RunnerInfo>>>,
}

impl VerifierService {
    pub fn new() -> Self {
        Self {
            verifier_sets: Arc::new(std::sync::RwLock::new(std::collections::HashMap::new())),
            runners: Arc::new(std::sync::RwLock::new(std::collections::HashMap::new())),
        }
    }

    pub fn check_readiness(&self, contract: &str, expected_hash: &str) -> VerifierDiagnostics {
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
            (None, Some(r)) => (
                "unknown".to_string(),
                0,
                r.identifier.clone(),
                VerifierReadiness::VerifierSetMismatch {
                    reason: "no verifier set registered".to_string(),
                },
            ),
            (None, None) => (
                "unknown".to_string(),
                0,
                "unknown".to_string(),
                VerifierReadiness::VerifierSetMismatch {
                    reason: "no verifier configuration found".to_string(),
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

#[get("/api/bounty/{contract}")]
pub async fn get_bounty(
    contract: web::Path<String>,
    verifier_service: web::Data<VerifierService>,
) -> Result<HttpResponse> {
    let diagnostics = verifier_service.check_readiness(&contract, "expected_hash_placeholder");

    let response = BountyResponse {
        contract: contract.to_string(),
        amount: "2.00".to_string(),
        status: "claimable".to_string(),
        diagnostics,
    };

    Ok(HttpResponse::Ok().json(response))
}

#[get("/api/bounties")]
pub async fn list_bounties(
    verifier_service: web::Data<VerifierService>,
    query: web::Query<std::collections::HashMap<String, String>>,
) -> Result<HttpResponse> {
    let ready_only = query.get("ready_only").map(|v| v == "true").unwrap_or(false);

    let contracts = vec!["0xc710d54d192ffb0b84cd6e051754ab70acf1130c"];
    let mut bounties = Vec::new();

    for contract in contracts {
        let diagnostics = verifier_service.check_readiness(contract, "expected_hash_placeholder");

        if ready_only && !diagnostics.readiness.is_ready() {
            continue;
        }

        bounties.push(BountyResponse {
            contract: contract.to_string(),
            amount: "2.00".to_string(),
            status: "claimable".to_string(),
            diagnostics,
        });
    }

    Ok(HttpResponse::Ok().json(BountyListResponse { bounties }))
}
