//! Platform approval binds the executable policy, not just a benchmark label.
use crate::{RegressionSandboxPolicy, VerifierError, VerifierResultType};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::sync::OnceLock;

pub const REGRESSION_PROFILE_REGISTRY: &str = include_str!("../regression-profiles-v1.json");

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegressionProfileRegistry {
    pub schema: String,
    pub version: u64,
    pub profiles: Vec<RegressionProfile>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegressionProfile {
    pub id: String,
    pub status: String,
    pub sources: Vec<Value>,
    pub runner_manifest: Value,
    pub claim: String,
    pub limitations: String,
}

pub fn regression_profile_registry() -> VerifierResultType<&'static RegressionProfileRegistry> {
    static REGISTRY: OnceLock<Result<RegressionProfileRegistry, String>> = OnceLock::new();
    REGISTRY
        .get_or_init(|| {
            let registry: RegressionProfileRegistry =
                serde_json::from_str(REGRESSION_PROFILE_REGISTRY)
                    .map_err(|error| error.to_string())?;
            if registry.schema != "agent-bounties/regression-profiles-v1" || registry.version != 1 {
                return Err("unsupported regression profile registry".into());
            }
            let mut ids = std::collections::HashSet::new();
            for profile in &registry.profiles {
                if !ids.insert(&profile.id)
                    || profile.sources.is_empty()
                    || !["approved", "held"].contains(&profile.status.as_str())
                {
                    return Err("invalid regression profile registry".into());
                }
                let policy: RegressionSandboxPolicy =
                    serde_json::from_value(profile.runner_manifest.clone())
                        .map_err(|error| error.to_string())?;
                policy.validate().map_err(|error| error.to_string())?;
            }
            Ok(registry)
        })
        .as_ref()
        .map_err(|error| VerifierError::InvalidInput(error.clone()))
}

pub fn regression_profile_registry_digest() -> String {
    format!(
        "sha256:{}",
        hex::encode(Sha256::digest(REGRESSION_PROFILE_REGISTRY.as_bytes()))
    )
}

pub fn approved_regression_profile(
    benchmark: &Value,
) -> VerifierResultType<&'static RegressionProfile> {
    let unavailable = || {
        VerifierError::InvalidInput(
        "verification_profile_unapproved: Verification unavailable. The exact immutable source and complete runner manifest must be independently reconciled and approved.".into()
    )
    };
    if benchmark.get("engine").and_then(Value::as_str) != Some("sandboxed_regression_v1") {
        return Err(unavailable());
    }
    let mut source = benchmark.get("source").cloned().ok_or_else(unavailable)?;
    // GitHub repository names and commit hex are case-insensitive; paths are not.
    for key in ["repository", "commit"] {
        let value = source
            .get(key)
            .and_then(Value::as_str)
            .ok_or_else(unavailable)?
            .to_ascii_lowercase();
        source[key] = Value::String(value);
    }
    let runner = benchmark.get("runner_manifest").ok_or_else(unavailable)?;
    regression_profile_registry()?
        .profiles
        .iter()
        .find(|profile| {
            profile.status == "approved"
                && profile.sources.contains(&source)
                && &profile.runner_manifest == runner
        })
        .ok_or_else(unavailable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn cross_language_approval_cases() {
        let cases: Value =
            serde_json::from_str(include_str!("../regression-profile-cases-v1.json")).unwrap();
        let profile = &regression_profile_registry().unwrap().profiles[0];
        for case in cases.as_array().unwrap() {
            let mut benchmark = json!({"engine":"sandboxed_regression_v1", "source":profile.sources[0], "runner_manifest":profile.runner_manifest});
            for (path, replacement) in case["changes"].as_object().unwrap() {
                let (parent, key) = path.rsplit_once('/').unwrap();
                benchmark.pointer_mut(parent).unwrap()[key] = replacement.clone();
            }
            assert_eq!(
                approved_regression_profile(&benchmark).is_ok(),
                case["approved"].as_bool().unwrap(),
                "{}",
                case["name"]
            );
        }
    }

    #[test]
    fn approved_profile_binds_every_execution_field_and_source() {
        for profile in &regression_profile_registry().unwrap().profiles {
            let benchmark = json!({"engine":"sandboxed_regression_v1", "source":profile.sources[0], "runner_manifest":profile.runner_manifest});
            if profile.status != "approved" {
                assert!(approved_regression_profile(&benchmark).is_err());
                continue;
            }
            assert_eq!(
                approved_regression_profile(&benchmark).unwrap().id,
                profile.id
            );
            for key in profile.runner_manifest.as_object().unwrap().keys() {
                let mut missing = benchmark.clone();
                missing["runner_manifest"]
                    .as_object_mut()
                    .unwrap()
                    .remove(key);
                assert!(
                    approved_regression_profile(&missing).is_err(),
                    "missing {key}"
                );
                let mut changed = benchmark.clone();
                changed["runner_manifest"][key] = json!("substituted");
                assert!(
                    approved_regression_profile(&changed).is_err(),
                    "changed {key}"
                );
            }
            for command in [
                json!(["true"]),
                json!(["python", "-c", "exit(0)"]),
                json!(["python", "/workspace/check.py"]),
            ] {
                let mut changed = benchmark.clone();
                changed["runner_manifest"]["command"] = command;
                assert!(approved_regression_profile(&changed).is_err());
            }
            for key in ["kind", "repository", "commit", "subdirectory"] {
                let mut changed = benchmark.clone();
                changed["source"][key] = json!("substituted");
                assert!(approved_regression_profile(&changed).is_err());
            }
            for source in &profile.sources {
                let mut alias = benchmark.clone();
                alias["source"] = source.clone();
                assert!(approved_regression_profile(&alias).is_ok());
            }
        }
    }
}
