use async_trait::async_trait;
use chrono::Utc;
use domain::{Id, Submission, VerificationDecision, VerifierKind, VerifierResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum VerifierError {
    #[error("invalid verifier input: {0}")]
    InvalidInput(String),
    #[error("verification failed: {0}")]
    Failed(String),
}

pub type VerifierResultType<T> = Result<T, VerifierError>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationInput {
    pub bounty_id: Id,
    pub submission: Submission,
    pub expected_artifact_digest: Option<String>,
    pub rubric: Option<String>,
    pub evidence: Option<Value>,
}

#[async_trait]
pub trait Verifier: Send + Sync {
    fn kind(&self) -> VerifierKind;
    async fn verify(&self, input: VerificationInput) -> VerifierResultType<VerifierResult>;
}

#[derive(Debug, Clone)]
pub struct ManualVerifier {
    pub verifier_agent_id: Option<Id>,
}

#[async_trait]
impl Verifier for ManualVerifier {
    fn kind(&self) -> VerifierKind {
        VerifierKind::Manual
    }

    async fn verify(&self, input: VerificationInput) -> VerifierResultType<VerifierResult> {
        Ok(make_result(
            input.bounty_id,
            input.submission.id,
            self.verifier_agent_id,
            VerifierKind::Manual,
            VerificationDecision::NeedsReview,
            "manual review required",
            0.5,
        ))
    }
}

#[derive(Debug, Clone)]
pub struct DigestVerifier;

#[async_trait]
impl Verifier for DigestVerifier {
    fn kind(&self) -> VerifierKind {
        VerifierKind::JsonSchema
    }

    async fn verify(&self, input: VerificationInput) -> VerifierResultType<VerifierResult> {
        let expected = input.expected_artifact_digest.ok_or_else(|| {
            VerifierError::InvalidInput("expected_artifact_digest is required".to_string())
        })?;

        let accepted = expected == input.submission.artifact_digest;
        Ok(make_result(
            input.bounty_id,
            input.submission.id,
            None,
            VerifierKind::JsonSchema,
            if accepted {
                VerificationDecision::Accepted
            } else {
                VerificationDecision::Rejected
            },
            if accepted {
                "artifact digest matched"
            } else {
                "artifact digest did not match"
            },
            if accepted { 1.0 } else { 0.0 },
        ))
    }
}

#[derive(Debug, Clone)]
pub struct AiJudgeFilter;

#[async_trait]
impl Verifier for AiJudgeFilter {
    fn kind(&self) -> VerifierKind {
        VerifierKind::AiJudgeFilter
    }

    async fn verify(&self, input: VerificationInput) -> VerifierResultType<VerifierResult> {
        let rubric = input.rubric.unwrap_or_default();
        let low_confidence = rubric.len() < 20 || input.submission.artifact_digest.len() < 16;
        Ok(make_result(
            input.bounty_id,
            input.submission.id,
            None,
            VerifierKind::AiJudgeFilter,
            VerificationDecision::NeedsReview,
            "AI judge filter is advisory and cannot settle funds",
            if low_confidence { 0.4 } else { 0.72 },
        ))
    }
}

#[derive(Debug, Clone)]
pub struct GitHubCiVerifier;

#[async_trait]
impl Verifier for GitHubCiVerifier {
    fn kind(&self) -> VerifierKind {
        VerifierKind::GitHubCi
    }

    async fn verify(&self, input: VerificationInput) -> VerifierResultType<VerifierResult> {
        let evidence = required_evidence(&input)?;
        let conclusion = evidence_string(evidence, "check_conclusion")
            .or_else(|| evidence_string(evidence, "conclusion"))
            .unwrap_or_default()
            .to_ascii_lowercase();
        let status = evidence_string(evidence, "status")
            .unwrap_or_default()
            .to_ascii_lowercase();
        let accepted = matches!(conclusion.as_str(), "success" | "passed")
            || matches!(status.as_str(), "success" | "passed" | "completed");

        Ok(make_result(
            input.bounty_id,
            input.submission.id,
            None,
            VerifierKind::GitHubCi,
            if accepted {
                VerificationDecision::Accepted
            } else {
                VerificationDecision::Rejected
            },
            if accepted {
                "GitHub CI evidence passed"
            } else {
                "GitHub CI evidence did not pass"
            },
            if accepted { 0.98 } else { 0.0 },
        ))
    }
}

#[derive(Debug, Clone)]
pub struct DockerCommandVerifier;

#[async_trait]
impl Verifier for DockerCommandVerifier {
    fn kind(&self) -> VerifierKind {
        VerifierKind::DockerCommand
    }

    async fn verify(&self, input: VerificationInput) -> VerifierResultType<VerifierResult> {
        let evidence = required_evidence(&input)?;
        let exit_code = evidence_i64(evidence, "exit_code").ok_or_else(|| {
            VerifierError::InvalidInput("docker evidence requires exit_code".to_string())
        })?;
        let digest_matches = match &input.expected_artifact_digest {
            Some(expected) => expected == &input.submission.artifact_digest,
            None => true,
        };
        let accepted = exit_code == 0 && digest_matches;

        Ok(make_result(
            input.bounty_id,
            input.submission.id,
            None,
            VerifierKind::DockerCommand,
            if accepted {
                VerificationDecision::Accepted
            } else {
                VerificationDecision::Rejected
            },
            if accepted {
                "Docker command exited successfully"
            } else {
                "Docker command evidence failed"
            },
            if accepted { 0.96 } else { 0.0 },
        ))
    }
}

#[derive(Debug, Clone)]
pub struct HttpCallbackVerifier;

#[async_trait]
impl Verifier for HttpCallbackVerifier {
    fn kind(&self) -> VerifierKind {
        VerifierKind::HttpCallback
    }

    async fn verify(&self, input: VerificationInput) -> VerifierResultType<VerifierResult> {
        let evidence = required_evidence(&input)?;
        let status_code = evidence_i64(evidence, "status_code").ok_or_else(|| {
            VerifierError::InvalidInput("http callback evidence requires status_code".to_string())
        })?;
        let decision = evidence_string(evidence, "decision")
            .unwrap_or_default()
            .to_ascii_lowercase();
        let signature_valid = evidence_bool(evidence, "signature_valid").unwrap_or(false);
        let accepted =
            (200..300).contains(&status_code) && decision == "accepted" && signature_valid;

        Ok(make_result(
            input.bounty_id,
            input.submission.id,
            None,
            VerifierKind::HttpCallback,
            if accepted {
                VerificationDecision::Accepted
            } else {
                VerificationDecision::Rejected
            },
            if accepted {
                "HTTP callback accepted signed evidence"
            } else {
                "HTTP callback evidence failed"
            },
            if accepted { 0.94 } else { 0.0 },
        ))
    }
}

pub async fn verify_with_builtin(
    kind: VerifierKind,
    input: VerificationInput,
    verifier_agent_id: Option<Id>,
) -> VerifierResultType<VerifierResult> {
    match kind {
        VerifierKind::Manual => ManualVerifier { verifier_agent_id }.verify(input).await,
        VerifierKind::JsonSchema => DigestVerifier.verify(input).await,
        VerifierKind::DockerCommand => DockerCommandVerifier.verify(input).await,
        VerifierKind::GitHubCi => GitHubCiVerifier.verify(input).await,
        VerifierKind::HttpCallback => HttpCallbackVerifier.verify(input).await,
        VerifierKind::AiJudgeFilter => AiJudgeFilter.verify(input).await,
    }
}

fn make_result(
    bounty_id: Id,
    submission_id: Id,
    verifier_agent_id: Option<Id>,
    kind: VerifierKind,
    decision: VerificationDecision,
    summary: impl Into<String>,
    confidence: f32,
) -> VerifierResult {
    let summary = summary.into();
    let mut hasher = Sha256::new();
    hasher.update(format!(
        "{bounty_id}:{submission_id}:{kind:?}:{decision:?}:{summary}:{confidence}"
    ));

    VerifierResult {
        id: Uuid::new_v4(),
        bounty_id,
        submission_id,
        verifier_agent_id,
        kind,
        decision,
        summary,
        confidence,
        signed_payload_hash: hex::encode(hasher.finalize()),
        created_at: Utc::now(),
    }
}

fn required_evidence(input: &VerificationInput) -> VerifierResultType<&Value> {
    input
        .evidence
        .as_ref()
        .ok_or_else(|| VerifierError::InvalidInput("structured evidence is required".to_string()))
}

fn evidence_string(evidence: &Value, key: &str) -> Option<String> {
    evidence.get(key)?.as_str().map(ToString::to_string)
}

fn evidence_i64(evidence: &Value, key: &str) -> Option<i64> {
    evidence.get(key)?.as_i64()
}

fn evidence_bool(evidence: &Value, key: &str) -> Option<bool> {
    evidence.get(key)?.as_bool()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use domain::Submission;

    #[tokio::test]
    async fn digest_verifier_accepts_matching_artifact() {
        let bounty_id = Uuid::new_v4();
        let submission = Submission {
            id: Uuid::new_v4(),
            bounty_id,
            solver_agent_id: Uuid::new_v4(),
            artifact_digest: "abc123abc123abc123".to_string(),
            artifact_uri: "s3://bucket/artifact".to_string(),
            submitted_at: Utc::now(),
        };

        let result = DigestVerifier
            .verify(VerificationInput {
                bounty_id,
                submission,
                expected_artifact_digest: Some("abc123abc123abc123".to_string()),
                rubric: None,
                evidence: None,
            })
            .await
            .unwrap();

        assert_eq!(result.decision, VerificationDecision::Accepted);
    }

    #[tokio::test]
    async fn ai_judge_filter_does_not_authorize_settlement() {
        let bounty_id = Uuid::new_v4();
        let submission = Submission {
            id: Uuid::new_v4(),
            bounty_id,
            solver_agent_id: Uuid::new_v4(),
            artifact_digest: "short".to_string(),
            artifact_uri: "s3://bucket/artifact".to_string(),
            submitted_at: Utc::now(),
        };

        let result = AiJudgeFilter
            .verify(VerificationInput {
                bounty_id,
                submission,
                expected_artifact_digest: None,
                rubric: Some("unclear".to_string()),
                evidence: None,
            })
            .await
            .unwrap();

        assert_eq!(result.decision, VerificationDecision::NeedsReview);
    }

    #[tokio::test]
    async fn github_ci_verifier_accepts_success_evidence() {
        let bounty_id = Uuid::new_v4();
        let submission = submission_for(bounty_id, "abc123abc123abc123");

        let result = GitHubCiVerifier
            .verify(VerificationInput {
                bounty_id,
                submission,
                expected_artifact_digest: None,
                rubric: None,
                evidence: Some(serde_json::json!({
                    "check_conclusion": "success",
                    "check_name": "test"
                })),
            })
            .await
            .unwrap();

        assert_eq!(result.kind, VerifierKind::GitHubCi);
        assert_eq!(result.decision, VerificationDecision::Accepted);
    }

    #[tokio::test]
    async fn docker_verifier_requires_zero_exit_and_digest_match() {
        let bounty_id = Uuid::new_v4();
        let submission = submission_for(bounty_id, "abc123abc123abc123");

        let result = DockerCommandVerifier
            .verify(VerificationInput {
                bounty_id,
                submission,
                expected_artifact_digest: Some("abc123abc123abc123".to_string()),
                rubric: None,
                evidence: Some(serde_json::json!({ "exit_code": 0 })),
            })
            .await
            .unwrap();

        assert_eq!(result.kind, VerifierKind::DockerCommand);
        assert_eq!(result.decision, VerificationDecision::Accepted);
    }

    #[tokio::test]
    async fn http_callback_verifier_requires_signed_acceptance() {
        let bounty_id = Uuid::new_v4();
        let submission = submission_for(bounty_id, "abc123abc123abc123");

        let result = HttpCallbackVerifier
            .verify(VerificationInput {
                bounty_id,
                submission,
                expected_artifact_digest: None,
                rubric: None,
                evidence: Some(serde_json::json!({
                    "status_code": 200,
                    "decision": "accepted",
                    "signature_valid": true
                })),
            })
            .await
            .unwrap();

        assert_eq!(result.kind, VerifierKind::HttpCallback);
        assert_eq!(result.decision, VerificationDecision::Accepted);
    }

    fn submission_for(bounty_id: Uuid, digest: &str) -> Submission {
        Submission {
            id: Uuid::new_v4(),
            bounty_id,
            solver_agent_id: Uuid::new_v4(),
            artifact_digest: digest.to_string(),
            artifact_uri: "s3://bucket/artifact".to_string(),
            submitted_at: Utc::now(),
        }
    }
}
