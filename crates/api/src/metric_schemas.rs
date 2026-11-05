//! OpenAPI mirrors of the immutable proving-input types.
//! Keep schema derivation out of competition-metric-core: its exact source
//! bytes are part of the approved SP1 release identities. Request handling
//! continues to deserialize the original core types; these only document them.

#![allow(dead_code)]

use serde::{Deserialize, Serialize};

#[derive(utoipa::ToSchema, Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicVectorMode {
    AllEqual,
    MaximizeExactMatches,
    MinimizeAbsoluteError,
}

#[derive(utoipa::ToSchema, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicVectorCase {
    pub expected: i64,
    pub observed: i64,
    pub weight: u32,
}

#[derive(utoipa::ToSchema, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ArtifactRequirement {
    Utf8Contains {
        needle: String,
        minimum_occurrences: u32,
        weight: u32,
    },
    Utf8Excludes {
        needle: String,
        weight: u32,
    },
    MaximumBytes {
        maximum: u32,
        weight: u32,
    },
    JsonValid {
        weight: u32,
    },
    JsonPointerExists {
        pointer: String,
        weight: u32,
    },
    JsonPointerStringEquals {
        pointer: String,
        expected: String,
        weight: u32,
    },
    JsonArrayMinimumLength {
        pointer: String,
        minimum: u32,
        weight: u32,
    },
}

#[derive(
    utoipa::ToSchema, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum CanonicalGmvProtocol {
    Autonomous,
    OpenCompetitionV1,
    OpenCompetitionV2,
}

#[derive(utoipa::ToSchema, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalGmvFunding {
    pub contributor: [u8; 20],
    pub amount_base_units: u128,
}

#[derive(utoipa::ToSchema, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalGmvSettlement {
    pub protocol: CanonicalGmvProtocol,
    pub bounty_contract: [u8; 20],
    pub bounty_id: [u8; 32],
    pub creator: [u8; 20],
    pub solver: [u8; 20],
    pub settled_at: u64,
    pub block_number: u64,
    pub transaction_hash: [u8; 32],
    pub log_index: u32,
    pub gmv_base_units: u128,
    pub funding: Vec<CanonicalGmvFunding>,
}

#[derive(utoipa::ToSchema, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForwardCanonicalGmvCampaign {
    pub epoch_id: [u8; 32],
    pub starts_at: u64,
    pub ends_at: u64,
    pub minimum_score_base_units: u128,
    pub excluded_wallets: Vec<[u8; 20]>,
    pub excluded_bounty_contracts: Vec<[u8; 20]>,
    pub snapshot_attesters: Vec<[u8; 20]>,
    pub snapshot_attestation_threshold: u8,
}

#[derive(utoipa::ToSchema, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalGmvSnapshotAttestation {
    pub signer: [u8; 20],
    pub signature: Vec<u8>,
}

#[derive(utoipa::ToSchema, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForwardCanonicalGmvSnapshot {
    pub start_block: u64,
    pub end_safe_block: u64,
    pub end_block_hash: [u8; 32],
    pub settlements: Vec<CanonicalGmvSettlement>,
    pub attestations: Vec<CanonicalGmvSnapshotAttestation>,
}
