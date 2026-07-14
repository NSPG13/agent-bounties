use chain_base::{
    autonomous_bounty_create_from_terms, build_autonomous_bounty_terms_record,
    keccak256_canonical_json, AutonomousBountyTxPlanner,
};
use chrono::{DateTime, Utc};
use domain::AutonomousBountyTermsDocument;
use serde::Deserialize;
use serde_json::Value;
use std::{fs, path::PathBuf};

#[derive(Deserialize)]
struct Manifest {
    creator: String,
    created_at: String,
    bounties: Vec<ManifestBounty>,
}

#[derive(Deserialize)]
struct ManifestBounty {
    issue: u64,
    document: String,
    #[serde(default)]
    initial_funding: Option<i64>,
    commitments: Commitments,
    creation_nonce: String,
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
fn direct_canary_terms_and_unsigned_batch_match_committed_artifacts() {
    let root = repository_root();
    let manifest_path = root.join("bounties/autonomous-v1/direct-canaries-manifest.json");
    let manifest_value: Value = serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    let manifest: Manifest = serde_json::from_value(manifest_value.clone()).unwrap();
    let created_at = DateTime::parse_from_rfc3339(&manifest.created_at)
        .unwrap()
        .with_timezone(&Utc);

    let mut creates = Vec::new();
    for bounty in &manifest.bounties {
        let document: AutonomousBountyTermsDocument =
            serde_json::from_slice(&fs::read(root.join(&bounty.document)).unwrap()).unwrap();
        let record =
            build_autonomous_bounty_terms_record(&manifest.creator, document, created_at).unwrap();
        assert_eq!(
            record.terms_hash, bounty.commitments.terms_hash,
            "issue {} terms",
            bounty.issue
        );
        assert_eq!(
            record.policy_hash, bounty.commitments.policy_hash,
            "issue {} policy",
            bounty.issue
        );
        assert_eq!(
            record.acceptance_criteria_hash, bounty.commitments.acceptance_criteria_hash,
            "issue {} criteria",
            bounty.issue
        );
        assert_eq!(
            record.benchmark_hash, bounty.commitments.benchmark_hash,
            "issue {} benchmark",
            bounty.issue
        );
        assert_eq!(
            record.evidence_schema_hash, bounty.commitments.evidence_schema_hash,
            "issue {} evidence schema",
            bounty.issue
        );
        let create = autonomous_bounty_create_from_terms(&record).unwrap();
        assert_eq!(create.creation_nonce, bounty.creation_nonce);
        if let Some(initial_funding) = bounty.initial_funding {
            assert_eq!(create.initial_funding.amount, initial_funding);
        }
        creates.push(create);
    }

    let planner = AutonomousBountyTxPlanner::new(
        "0x082c52131aaf0c56e76b075f895eab6fcab6d2f9",
        "0x2fa36d2b2327642db3a6cc8cdd91544ad7484eb9",
    )
    .unwrap();
    let batch = planner
        .plan_creation_batch("base-mainnet", &creates)
        .unwrap();
    assert_eq!(batch.total_initial_funding, "7890000");
    assert_eq!(batch.creations.len(), 4);
    assert_eq!(batch.wallet_calls.len(), 5);

    let bundle: Value = serde_json::from_slice(
        &fs::read(root.join("deployments/direct-canaries-base-mainnet.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        bundle["manifest_canonical_json_keccak256"],
        keccak256_canonical_json(&manifest_value).unwrap()
    );
    assert_eq!(bundle["creation_batch"]["total_initial_funding"], "7890000");
    for (index, creation) in batch.creations.iter().enumerate() {
        assert_eq!(
            bundle["bounties"][index]["issue"],
            manifest.bounties[index].issue
        );
        assert_eq!(bundle["bounties"][index]["bounty_id"], creation.bounty_id);
        assert_eq!(
            bundle["bounties"][index]["predicted_bounty_contract"],
            creation.predicted_bounty_contract
        );
        assert_eq!(
            bundle["creation_batch"]["creations"][index]["create_bounty"]["data"],
            creation.create_bounty.data
        );
    }
}
