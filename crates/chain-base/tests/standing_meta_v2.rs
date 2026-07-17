use chain_base::{
    autonomous_bounty_create_from_terms, build_autonomous_bounty_terms_record,
    AutonomousBountyTxPlanner,
};
use chrono::{DateTime, Utc};
use domain::AutonomousBountyTermsDocument;
use serde::Deserialize;
use std::{collections::HashSet, fs, path::PathBuf};

const VERIFIER: &str = "0xe573cb4f471d38b5bf10ce82237251ac902c9867";
const ACCEPTANCE_HASH: &str = "0x25c41d7d51e2c807754b901733de17cdb1778dbd353f86347ff33e10289fcb54";

#[derive(Deserialize)]
struct Manifest {
    schema_version: String,
    creator: String,
    created_at: String,
    bounties: Vec<ManifestBounty>,
}

#[derive(Deserialize)]
struct ManifestBounty {
    issue: u64,
    document: String,
    commitments: Commitments,
    creation_nonce: String,
    bounty_id: String,
    predicted_bounty_contract: String,
}

#[derive(Deserialize)]
struct Commitments {
    terms_hash: String,
    policy_hash: String,
    acceptance_criteria_hash: String,
    benchmark_hash: String,
    evidence_schema_hash: String,
}

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn standing_meta_v2_terms_match_five_creation_vectors() {
    let root = repository_root();
    let manifest: Manifest = serde_json::from_slice(
        &fs::read(root.join("bounties/autonomous-v1/standing-meta-v2-manifest.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        manifest.schema_version,
        "agent-bounties/standing-meta-v2-manifest-v1"
    );
    assert_eq!(manifest.bounties.len(), 5);
    let created_at = DateTime::parse_from_rfc3339(&manifest.created_at)
        .unwrap()
        .with_timezone(&Utc);
    let planner = AutonomousBountyTxPlanner::new(
        "0x082c52131aaf0c56e76b075f895eab6fcab6d2f9",
        "0x2fa36d2b2327642db3a6cc8cdd91544ad7484eb9",
    )
    .unwrap();
    let mut issues = HashSet::new();
    let mut contracts = HashSet::new();

    for bounty in manifest.bounties {
        assert!(issues.insert(bounty.issue));
        let document: AutonomousBountyTermsDocument =
            serde_json::from_slice(&fs::read(root.join(&bounty.document)).unwrap()).unwrap();
        let record =
            build_autonomous_bounty_terms_record(&manifest.creator, document, created_at).unwrap();
        assert_eq!(record.terms_hash, bounty.commitments.terms_hash);
        assert_eq!(record.policy_hash, bounty.commitments.policy_hash);
        assert_eq!(
            record.acceptance_criteria_hash,
            bounty.commitments.acceptance_criteria_hash
        );
        assert_eq!(record.acceptance_criteria_hash, ACCEPTANCE_HASH);
        assert_eq!(record.benchmark_hash, bounty.commitments.benchmark_hash);
        assert_eq!(
            record.evidence_schema_hash,
            bounty.commitments.evidence_schema_hash
        );

        let create = autonomous_bounty_create_from_terms(&record).unwrap();
        assert_eq!(create.initial_funding.amount, 1_000_000);
        assert_eq!(create.solver_reward.amount, 900_000);
        assert_eq!(create.verifier_reward.amount, 100_000);
        assert_eq!(create.verifier_module.as_deref(), Some(VERIFIER));
        assert_eq!(create.creation_nonce, bounty.creation_nonce);
        let plan = planner.plan_creation("base-mainnet", &create).unwrap();
        assert_eq!(plan.bounty_id, bounty.bounty_id);
        assert_eq!(
            plan.predicted_bounty_contract,
            bounty.predicted_bounty_contract
        );
        assert!(contracts.insert(plan.predicted_bounty_contract));
    }
}
