//! Authenticated operational health, never payment authority or proof of computation.
use chain_base::AutonomousBountyFeedItem;
use chrono::{DateTime, Duration, Utc};
use db::{verifier_checkpoints::VerifierCheckpoint, PostgresStore};
use domain::AutonomousBountyTermsRecord;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_AGE_SECONDS: i64 = 45 * 60;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovedRelease {
    pub schema: String,
    pub release_id: String,
    pub source_revision: String,
    pub network: String,
    pub worker_build_digest: String,
    pub worker_binary_digest: String,
    pub signing_runtime_digest: String,
    pub profile_registry_digest: String,
    pub signers: Vec<String>,
}
fn hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|v| hex(v, 64))
}
impl ApprovedRelease {
    pub fn calculated_id(&self) -> Option<String> {
        let mut value = serde_json::to_value(self).ok()?;
        value.as_object_mut()?.remove("release_id");
        Some(format!(
            "sha256:{:x}",
            Sha256::digest(serde_json::to_vec(&value).ok()?)
        ))
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.schema != "agent-bounties/verifier-release-v1"
            || !matches!(self.network.as_str(), "base-mainnet" | "base-sepolia")
            || !hex(&self.source_revision, 40)
            || [
                &self.worker_build_digest,
                &self.worker_binary_digest,
                &self.signing_runtime_digest,
                &self.profile_registry_digest,
            ]
            .iter()
            .any(|s| !digest(s))
            || self.profile_registry_digest != chain_base::reviewed_regression_registry_digest()
            || !(1..=2).contains(&self.signers.len())
            || self
                .signers
                .iter()
                .any(|s| !s.strip_prefix("0x").is_some_and(|s| hex(s, 40)))
            || self.signers.iter().collect::<BTreeSet<_>>().len() != self.signers.len()
            || self.calculated_id().as_deref() != Some(&self.release_id)
        {
            return Err("verification_release_invalid");
        }
        Ok(())
    }
    pub fn components(&self) -> Vec<String> {
        std::iter::once("runner".into())
            .chain(self.signers.iter().map(|s| format!("signer:{s}")))
            .collect()
    }
}

pub struct CheckpointConfig {
    pub release: ApprovedRelease,
    token_hashes: BTreeMap<String, String>,
}
impl CheckpointConfig {
    pub fn from_env() -> Result<Self, &'static str> {
        let release = std::env::var("REGRESSION_VERIFIER_RELEASE_JSON")
            .map_err(|_| "verification_release_unconfigured")?;
        let keys = std::env::var("REGRESSION_CHECKPOINT_TOKEN_HASHES_JSON")
            .map_err(|_| "verification_health_auth_unconfigured")?;
        Self::parse(&release, &keys)
    }
    pub fn parse(release: &str, keys: &str) -> Result<Self, &'static str> {
        let release: ApprovedRelease =
            serde_json::from_str(release).map_err(|_| "verification_release_invalid")?;
        release.validate()?;
        let token_hashes: BTreeMap<String, String> =
            serde_json::from_str(keys).map_err(|_| "verification_health_auth_invalid")?;
        if release.components().into_iter().collect::<BTreeSet<_>>()
            != token_hashes.keys().cloned().collect()
            || token_hashes.values().any(|s| !hex(s, 64))
            || token_hashes.values().collect::<BTreeSet<_>>().len() != token_hashes.len()
        {
            return Err("verification_health_auth_invalid");
        }
        Ok(Self {
            release,
            token_hashes,
        })
    }
    pub fn authenticate(&self, component: &str, token: &str) -> bool {
        if !(32..=256).contains(&token.len()) {
            return false;
        }
        let Some(expected) = self.token_hashes.get(component) else {
            return false;
        };
        let actual = crate::auth::hash_secret(token);
        actual
            .bytes()
            .zip(expected.bytes())
            .fold(0u8, |difference, (a, b)| difference | (a ^ b))
            == 0
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckpointRequest {
    pub release: ApprovedRelease,
    pub component: String,
    pub issued_at: DateTime<Utc>,
    pub healthy: bool,
}
impl CheckpointRequest {
    pub fn validate(
        &self,
        config: &CheckpointConfig,
        token: &str,
        now: DateTime<Utc>,
    ) -> Result<(), &'static str> {
        if !config.authenticate(&self.component, token) {
            return Err("verification_checkpoint_unauthorized");
        }
        self.release.validate()?;
        if self.release.release_id != config.release.release_id {
            return Err("verification_release_mismatch");
        }
        if self.issued_at > now + Duration::seconds(30)
            || self.issued_at < now - Duration::minutes(2)
        {
            return Err("verification_checkpoint_stale");
        }
        Ok(())
    }
}

pub async fn record(
    store: Option<&PostgresStore>,
    token: &str,
    request: &CheckpointRequest,
) -> Result<(), &'static str> {
    let config = CheckpointConfig::from_env()?;
    request.validate(&config, token, Utc::now())?;
    let accepted = store
        .ok_or("verification_checkpoint_store_unavailable")?
        .record_verifier_checkpoint(
            &config.release.release_id,
            &VerifierCheckpoint {
                component: request.component.clone(),
                issued_at: request.issued_at,
                healthy: request.healthy,
            },
        )
        .await
        .map_err(|_| "verification_checkpoint_store_unavailable")?;
    if !accepted {
        return Err("verification_checkpoint_replayed");
    }
    Ok(())
}

pub struct HealthSnapshot {
    release: Option<ApprovedRelease>,
    rows: Vec<VerifierCheckpoint>,
    blocker: Option<&'static str>,
    now: DateTime<Utc>,
}
impl HealthSnapshot {
    pub async fn load(store: Option<&PostgresStore>, network: &str) -> Self {
        let now = Utc::now();
        let config = match CheckpointConfig::from_env() {
            Ok(config) => config,
            Err(code) => {
                return Self {
                    release: None,
                    rows: vec![],
                    blocker: Some(code),
                    now,
                }
            }
        };
        if config.release.network != network {
            return Self {
                release: Some(config.release),
                rows: vec![],
                blocker: Some("verification_health_network_mismatch"),
                now,
            };
        }
        let rows = match store {
            Some(store) => store
                .verifier_checkpoints(&config.release.release_id)
                .await
                .ok(),
            None => None,
        };
        Self {
            release: Some(config.release),
            blocker: rows
                .is_none()
                .then_some("verification_checkpoint_store_unavailable"),
            rows: rows.unwrap_or_default(),
            now,
        }
    }
    pub fn details(&self, terms: &AutonomousBountyTermsRecord) -> serde_json::Value {
        self.benchmark_details(
            &terms.document.benchmark,
            &terms.document.verification_policy,
        )
    }
    pub fn benchmark_details(
        &self,
        benchmark: &serde_json::Value,
        policy: &serde_json::Value,
    ) -> serde_json::Value {
        let mut blockers = Vec::new();
        if let Some(code) = self.blocker {
            blockers.push(code);
        }
        let mut profile_id = None;
        if benchmark["engine"] == "sandboxed_regression_v1" {
            match chain_base::reviewed_regression_profile_id(benchmark) {
                Ok(id) => profile_id = Some(id),
                Err(_) => blockers.push("verification_profile_unapproved"),
            }
        }
        if benchmark["engine"] == "standing_meta_v3_routed_parent" {
            match chain_base::reviewed_routed_child_profile_id(benchmark) {
                Ok(id) => profile_id = Some(id.to_string()),
                Err(_) => blockers.push("verification_child_profile_unapproved"),
            }
        }
        let mut oldest = None;
        if let Some(release) = &self.release {
            let parent = benchmark["required_child_engine"] == "sandboxed_regression_v1";
            let signers = if parent {
                let expected = chain_base::BASE_MAINNET_STANDING_META_V2_VERIFIERS
                    .iter()
                    .map(|s| s.to_ascii_lowercase())
                    .collect::<Vec<_>>();
                if release.signers.iter().collect::<BTreeSet<_>>()
                    != expected.iter().collect::<BTreeSet<_>>()
                {
                    blockers.push("verification_signer_unapproved");
                }
                expected
            } else {
                policy["verifiers"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(str::to_ascii_lowercase))
                            .collect()
                    })
                    .unwrap_or_default()
            };
            if signers.is_empty()
                || signers.iter().collect::<BTreeSet<_>>().len() != signers.len()
                || signers.iter().any(|s| !release.signers.contains(s))
            {
                blockers.push("verification_signer_unapproved");
            }
            for component in std::iter::once("runner".into())
                .chain(signers.iter().map(|s| format!("signer:{s}")))
            {
                match self.rows.iter().find(|r| r.component == component) {
                    None => blockers.push("verification_checkpoint_missing"),
                    Some(row) => {
                        oldest =
                            Some(oldest.map_or(row.issued_at, |old: DateTime<Utc>| {
                                old.min(row.issued_at)
                            }));
                        if !row.healthy {
                            blockers.push("verification_checkpoint_failed");
                        }
                        if row.issued_at > self.now + Duration::seconds(30)
                            || self.now - row.issued_at > Duration::seconds(MAX_AGE_SECONDS)
                        {
                            blockers.push("verification_checkpoint_stale");
                        }
                    }
                }
            }
        }
        blockers.sort_unstable();
        blockers.dedup();
        let operational_ready = blockers.iter().all(|code| {
            matches!(
                *code,
                "verification_profile_unapproved" | "verification_child_profile_unapproved"
            )
        });
        serde_json::json!({"ready":blockers.is_empty(),"operational_ready":operational_ready,"blocker_codes":blockers,"release_id":self.release.as_ref().map(|r|&r.release_id),"profile_id":profile_id,"profile_registry_digest":chain_base::reviewed_regression_registry_digest(),"last_check_at":oldest,"evaluated_at":self.now,"maximum_age_seconds":MAX_AGE_SECONDS,"claim":"Authenticated service health only; not a verdict, payment approval or cryptographic proof of computation."})
    }
    pub fn apply(&self, feed: &mut [AutonomousBountyFeedItem]) {
        for item in feed {
            let Some(terms) = &item.terms else {
                continue;
            };
            if !crate::verifier_readiness::needs_hosted_regression(terms) {
                continue;
            }
            let mut details = self.details(terms);
            if item.verification_ready && details["ready"] != true {
                item.verification_ready = false;
                let blocker = details["blocker_codes"][0]
                    .as_str()
                    .unwrap_or("verification_unavailable");
                item.verification_readiness_reason = if blocker
                    == "verification_child_profile_unapproved"
                {
                    chain_base::ROUTED_CHILD_PROFILE_UNAVAILABLE.into()
                } else {
                    format!("{blocker}: Verification unavailable. The approved runner and required signers must report healthy before new hosted work can start.")
                };
            }
            if !item.verification_ready && details["ready"] == true {
                details["blocker_codes"] = serde_json::json!(["verification_terms_not_ready"]);
            }
            details["ready"] = serde_json::json!(item.verification_ready);
            details["reason"] = serde_json::json!(item.verification_readiness_reason);
            item.verification_details = Some(details);
        }
    }
}

pub async fn apply_current(
    store: Option<&PostgresStore>,
    network: &str,
    feed: &mut [AutonomousBountyFeedItem],
) {
    HealthSnapshot::load(store, network).await.apply(feed);
}
pub async fn require_available(
    store: Option<&PostgresStore>,
    network: &str,
    terms: &AutonomousBountyTermsRecord,
) -> Result<(), &'static str> {
    if !crate::verifier_readiness::needs_hosted_regression(terms) {
        return Ok(());
    }
    let details = HealthSnapshot::load(store, network).await.details(terms);
    if details["blocker_codes"].as_array().is_some_and(|codes| {
        codes
            .iter()
            .any(|code| code == "verification_child_profile_unapproved")
    }) {
        return Err(chain_base::ROUTED_CHILD_PROFILE_UNAVAILABLE);
    }
    if details["ready"] != true {
        return Err(crate::verifier_readiness::UNAVAILABLE);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (CheckpointConfig, AutonomousBountyTermsRecord, Vec<String>) {
        let mut release = ApprovedRelease {
            schema: "agent-bounties/verifier-release-v1".into(),
            release_id: String::new(),
            source_revision: "a".repeat(40),
            network: "base-mainnet".into(),
            worker_build_digest: format!("sha256:{}", "1".repeat(64)),
            worker_binary_digest: format!("sha256:{}", "2".repeat(64)),
            signing_runtime_digest: format!("sha256:{}", "3".repeat(64)),
            profile_registry_digest: chain_base::reviewed_regression_registry_digest(),
            signers: chain_base::BASE_MAINNET_STANDING_META_V2_VERIFIERS
                .iter()
                .map(|s| s.to_string())
                .collect(),
        };
        release.release_id = release.calculated_id().unwrap();
        let tokens = vec!["r".repeat(40), "a".repeat(40), "b".repeat(40)];
        let keys = release
            .components()
            .into_iter()
            .zip(tokens.iter().map(|s| crate::auth::hash_secret(s)))
            .collect::<BTreeMap<_, _>>();
        let config = CheckpointConfig::parse(
            &serde_json::to_string(&release).unwrap(),
            &serde_json::to_string(&keys).unwrap(),
        )
        .unwrap();
        let registry: serde_json::Value = serde_json::from_str(include_str!(
            "../../verifier-sdk/regression-profiles-v1.json"
        ))
        .unwrap();
        let p = &registry["profiles"][0];
        let terms=serde_json::from_value(serde_json::json!({
            "terms_hash":"hash","policy_hash":"policy","acceptance_criteria_hash":"criteria","benchmark_hash":"benchmark","evidence_schema_hash":"evidence","creator_wallet":"wallet","created_at":"2026-09-30T00:00:00Z",
            "document":{"schema_version":"v1","contract_terms":{},"title":"test","goal":"test","acceptance_criteria":[],"benchmark":{"engine":"sandboxed_regression_v1","source":p["sources"][0],"runner_manifest":p["runner_manifest"]},"evidence_schema":{},"verification_policy":{"verifiers":release.signers,"threshold":2}}
        })).unwrap();
        (config, terms, tokens)
    }
    #[test]
    fn release_identity_matches_the_python_fixture() {
        let release: ApprovedRelease =
            serde_json::from_str(include_str!("../fixtures/verifier-release-test-v1.json"))
                .unwrap();
        release.validate().unwrap();
    }
    #[test]
    fn release_identity_binds_every_digest_and_scoped_credentials() {
        let (config, _, tokens) = fixture();
        let now = Utc::now();
        let mut request = CheckpointRequest {
            release: config.release.clone(),
            component: "runner".into(),
            issued_at: now,
            healthy: true,
        };
        assert!(request.validate(&config, &tokens[0], now).is_ok());
        assert!(request.validate(&config, &tokens[1], now).is_err());
        request.component = format!("signer:{}", config.release.signers[0]);
        assert!(request.validate(&config, &tokens[0], now).is_err());
        request.component = "runner".into();
        request.issued_at = now - Duration::minutes(3);
        assert_eq!(
            request.validate(&config, &tokens[0], now),
            Err("verification_checkpoint_stale")
        );
        request.issued_at = now + Duration::seconds(31);
        assert!(request.validate(&config, &tokens[0], now).is_err());
        request.issued_at = now;
        for field in [
            "source_revision",
            "worker_build_digest",
            "worker_binary_digest",
            "signing_runtime_digest",
            "profile_registry_digest",
            "network",
        ] {
            let mut value = serde_json::to_value(&config.release).unwrap();
            value[field] = serde_json::json!("substituted");
            request.release = serde_json::from_value(value).unwrap();
            assert!(
                request.validate(&config, &tokens[0], now).is_err(),
                "{field}"
            );
        }
        request.release = config.release.clone();
        request.release.worker_binary_digest = format!("sha256:{}", "f".repeat(64));
        request.release.release_id = request.release.calculated_id().unwrap();
        assert_eq!(
            request.validate(&config, &tokens[0], now),
            Err("verification_release_mismatch")
        );
    }
    #[test]
    fn each_required_signer_and_runner_must_be_fresh_healthy_and_approved() {
        let (config, terms, _) = fixture();
        let now = Utc::now();
        let rows = config
            .release
            .components()
            .into_iter()
            .map(|component| VerifierCheckpoint {
                component,
                issued_at: now - Duration::seconds(MAX_AGE_SECONDS),
                healthy: true,
            })
            .collect();
        let mut snapshot = HealthSnapshot {
            release: Some(config.release),
            rows,
            blocker: None,
            now,
        };
        assert_eq!(
            snapshot.details(&terms)["ready"],
            true,
            "exact45minute boundary remains valid"
        );
        snapshot.now = now + Duration::seconds(1);
        assert_eq!(snapshot.details(&terms)["ready"], false);
        snapshot.now = now;
        for index in 0..3 {
            snapshot.rows[index].healthy = false;
            assert_eq!(snapshot.details(&terms)["ready"], false);
            snapshot.rows[index].healthy = true;
            let row = snapshot.rows.remove(index);
            assert_eq!(snapshot.details(&terms)["ready"], false);
            snapshot.rows.insert(index, row);
        }
        let mut bad = terms.clone();
        bad.document.benchmark["runner_manifest"]["command"] = serde_json::json!(["true"]);
        assert_eq!(snapshot.details(&bad)["ready"], false);
        snapshot.blocker = Some("verification_checkpoint_store_unavailable");
        assert_eq!(snapshot.details(&terms)["ready"], false);
    }
    #[test]
    fn healthy_components_cannot_override_unready_terms() {
        let (config, terms, _) = fixture();
        let now = Utc::now();
        let snapshot = HealthSnapshot {
            rows: config
                .release
                .components()
                .into_iter()
                .map(|component| VerifierCheckpoint {
                    component,
                    issued_at: now,
                    healthy: true,
                })
                .collect(),
            release: Some(config.release),
            blocker: None,
            now,
        };
        let item: AutonomousBountyFeedItem = serde_json::from_value(serde_json::json!({
            "bounty_id":"id", "bounty_contract":"contract", "creator":"creator", "status":"claimable", "solver_reward":"990000", "verifier_reward":"10000", "claim_bond":"10000", "timeout_bond_pool":"0", "target_amount":"1000000", "funded_amount":"1000000", "terms_hash":"hash", "terms":terms, "terms_valid":false, "verification_mode":"signed_quorum", "verifier_module":null, "verification_ready":false, "verification_readiness_reason":"Terms are not valid", "validation_errors":["invalid terms"], "events":[]
        })).unwrap();
        let mut feed = vec![item.clone()];
        snapshot.apply(&mut feed);
        assert!(!feed[0].verification_ready);
        let details = feed[0].verification_details.as_ref().unwrap();
        assert_eq!(details["operational_ready"], true);
        assert_eq!(details["ready"], false);
        assert_eq!(details["blocker_codes"][0], "verification_terms_not_ready");
        assert_eq!(
            feed[0].verification_readiness_reason,
            item.verification_readiness_reason
        );
        assert_eq!(feed[0].terms, item.terms);
    }

    #[test]
    fn healthy_services_cannot_advertise_an_unreviewed_routed_child_profile() {
        let (config, mut terms, _) = fixture();
        let now = Utc::now();
        let mut snapshot = HealthSnapshot {
            rows: config
                .release
                .components()
                .into_iter()
                .map(|component| VerifierCheckpoint {
                    component,
                    issued_at: now,
                    healthy: true,
                })
                .collect(),
            release: Some(config.release),
            blocker: None,
            now,
        };
        for lane in ["mcp", "mcp_interoperability", "unknown"] {
            terms.document.benchmark = serde_json::json!({
                "engine":"standing_meta_v3_routed_parent", "required_child_engine":"sandboxed_regression_v1", "lane":lane,
                "profile_id":"a2a-agent-card-v1"
            });
            let details = snapshot.details(&terms);
            assert_eq!(details["operational_ready"], true);
            assert_eq!(details["ready"], false);
            assert_eq!(details["profile_id"], serde_json::Value::Null);
            assert_eq!(
                details["blocker_codes"],
                serde_json::json!(["verification_child_profile_unapproved"])
            );
        }
        terms
            .document
            .benchmark
            .as_object_mut()
            .unwrap()
            .remove("lane");
        assert_eq!(snapshot.details(&terms)["ready"], false);
        let original: Vec<AutonomousBountyFeedItem> = serde_json::from_str(include_str!(
            "../../chain-base/fixtures/routed-mcp-readiness-v1.json"
        ))
        .unwrap();
        let mut projected = original.clone();
        snapshot.apply(&mut projected);
        for (before, after) in original.iter().zip(projected.iter()) {
            assert!(!after.verification_ready);
            assert_eq!(
                after.verification_readiness_reason,
                chain_base::ROUTED_CHILD_PROFILE_UNAVAILABLE
            );
            assert_eq!(
                after.verification_details.as_ref().unwrap()["operational_ready"],
                true
            );
            let mut restored = after.clone();
            restored.verification_ready = before.verification_ready;
            restored.verification_readiness_reason = before.verification_readiness_reason.clone();
            restored.verification_details = before.verification_details.clone();
            assert_eq!(
                serde_json::to_value(restored).unwrap(),
                serde_json::to_value(before).unwrap()
            );
        }
        snapshot.rows[0].healthy = false;
        let details = snapshot.details(&terms);
        assert_eq!(details["operational_ready"], false);
        assert_eq!(details["ready"], false);
        assert!(details["blocker_codes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|code| code == "verification_checkpoint_failed"));
    }
}
