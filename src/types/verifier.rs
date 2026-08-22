use serde::{Deserialize, Serialize};
use std::fmt;

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

    pub fn reason(&self) -> Option<&str> {
        match self {
            VerifierReadiness::Ready => None,
            VerifierReadiness::MissingSigner { reason } => Some(reason),
            VerifierReadiness::StaleRunner { reason } => Some(reason),
            VerifierReadiness::VerifierSetMismatch { reason } => Some(reason),
        }
    }
}

impl fmt::Display for VerifierReadiness {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VerifierReadiness::Ready => write!(f, "ready"),
            VerifierReadiness::MissingSigner { reason } => write!(f, "missing_signer: {}", reason),
            VerifierReadiness::StaleRunner { reason } => write!(f, "stale_runner: {}", reason),
            VerifierReadiness::VerifierSetMismatch { reason } => write!(f, "verifier_set_mismatch: {}", reason),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifierDiagnostics {
    pub verifier_set_hash: String,
    pub threshold: u32,
    pub runner_identifier: String,
    pub readiness: VerifierReadiness,
}
