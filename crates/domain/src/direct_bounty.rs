use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// A concise, machine-readable evidence checklist for direct coding bounties.
///
/// Binds the repository commit, benchmark/check run, artifact digest, and
/// canonical settlement boundary into three clearly separated evidence phases:
/// submission, verification, and payment.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub struct DirectBountyEvidenceChecklist {
    pub submission: SubmissionEvidence,
    pub verification: VerificationEvidence,
    pub payment: PaymentEvidence,
}

/// Evidence that the code change was submitted.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub struct SubmissionEvidence {
    /// Full `owner/repo` repository identifier.
    pub repository: String,
    /// Exact source commit SHA (hex, 40 chars).
    pub commit_sha: String,
    /// Optional subdirectory scope within the repo.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subdirectory: Option<String>,
    /// HTTPS pull-request URL.
    pub pull_request_url: String,
}

/// Evidence that automated checks passed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub struct VerificationEvidence {
    /// One or more HTTPS check-run or CI pipeline URLs.
    pub check_run_urls: Vec<String>,
    /// Content-addressed artifact digest (e.g. `sha256:abcdef…`).
    pub artifact_digest: String,
}

/// Evidence of canonical on-chain settlement.
///
/// A PR, test result, or verifier response is **not** payment evidence;
/// only a canonical `BountySettled` event proves payment.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub struct PaymentEvidence {
    /// Canonical settlement boundary string (e.g. `BountySettled(<tx_hash>)`).
    pub settlement_boundary: String,
}

/// Validation errors for [`DirectBountyEvidenceChecklist`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvidenceValidationError {
    EmptyField(&'static str),
    NonHttpsUrl(String),
    InvalidUrl(String),
    MutableArtifactReference(String),
}

impl std::fmt::Display for EvidenceValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyField(name) => write!(f, "required field is empty: {name}"),
            Self::NonHttpsUrl(url) => write!(f, "URL must use HTTPS scheme: {url}"),
            Self::InvalidUrl(url) => write!(f, "invalid URL format: {url}"),
            Self::MutableArtifactReference(r) => {
                write!(f, "artifact reference must be immutable (content-addressed): {r}")
            }
        }
    }
}

impl std::error::Error for EvidenceValidationError {}

impl DirectBountyEvidenceChecklist {
    /// Validate the entire checklist.
    ///
    /// Rejects empty fields, mutable artifact references, and non-HTTPS URLs.
    pub fn validate(&self) -> Result<(), EvidenceValidationError> {
        // ── Submission ──
        reject_empty(&self.submission.repository, "repository")?;
        reject_empty(&self.submission.commit_sha, "commit_sha")?;
        validate_https_url(&self.submission.pull_request_url)?;

        // ── Verification ──
        if self.verification.check_run_urls.is_empty() {
            return Err(EvidenceValidationError::EmptyField("check_run_urls"));
        }
        for url in &self.verification.check_run_urls {
            validate_https_url(url)?;
        }
        reject_empty(&self.verification.artifact_digest, "artifact_digest")?;
        // Reject mutable references: plain HTTP URLs are mutable.
        if self.verification.artifact_digest.starts_with("http://") {
            return Err(EvidenceValidationError::MutableArtifactReference(
                self.verification.artifact_digest.clone(),
            ));
        }

        // ── Payment ──
        reject_empty(&self.payment.settlement_boundary, "settlement_boundary")?;

        Ok(())
    }
}

fn reject_empty(value: &str, field: &'static str) -> Result<(), EvidenceValidationError> {
    if value.trim().is_empty() {
        Err(EvidenceValidationError::EmptyField(field))
    } else {
        Ok(())
    }
}

fn validate_https_url(url: &str) -> Result<(), EvidenceValidationError> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return Err(EvidenceValidationError::EmptyField("url"));
    }
    if !trimmed.starts_with("https://") {
        return Err(EvidenceValidationError::NonHttpsUrl(url.to_string()));
    }
    // Minimal structural check: must have a host after the scheme.
    let after_scheme = &trimmed["https://".len()..];
    if after_scheme.is_empty() || after_scheme.starts_with('/') {
        return Err(EvidenceValidationError::InvalidUrl(url.to_string()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_checklist() -> DirectBountyEvidenceChecklist {
        DirectBountyEvidenceChecklist {
            submission: SubmissionEvidence {
                repository: "NSPG13/agent-bounties".to_string(),
                commit_sha: "7d8251e605d398d5c4146c3b6831d102e1b816ab".to_string(),
                subdirectory: None,
                pull_request_url: "https://github.com/NSPG13/agent-bounties/pull/123".to_string(),
            },
            verification: VerificationEvidence {
                check_run_urls: vec![
                    "https://github.com/NSPG13/agent-bounties/actions/runs/123456".to_string(),
                ],
                artifact_digest:
                    "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
                        .to_string(),
            },
            payment: PaymentEvidence {
                settlement_boundary: "BountySettled(0xabc123)".to_string(),
            },
        }
    }

    #[test]
    fn accepts_valid_evidence() {
        assert!(valid_checklist().validate().is_ok());
    }

    #[test]
    fn rejects_empty_repository() {
        let mut c = valid_checklist();
        c.submission.repository = "".to_string();
        assert_eq!(
            c.validate().unwrap_err(),
            EvidenceValidationError::EmptyField("repository")
        );
    }

    #[test]
    fn rejects_empty_commit_sha() {
        let mut c = valid_checklist();
        c.submission.commit_sha = "   ".to_string();
        assert_eq!(
            c.validate().unwrap_err(),
            EvidenceValidationError::EmptyField("commit_sha")
        );
    }

    #[test]
    fn rejects_http_pull_request_url() {
        let mut c = valid_checklist();
        c.submission.pull_request_url =
            "http://github.com/NSPG13/agent-bounties/pull/1".to_string();
        assert!(matches!(
            c.validate().unwrap_err(),
            EvidenceValidationError::NonHttpsUrl(_)
        ));
    }

    #[test]
    fn rejects_empty_check_run_urls() {
        let mut c = valid_checklist();
        c.verification.check_run_urls = vec![];
        assert_eq!(
            c.validate().unwrap_err(),
            EvidenceValidationError::EmptyField("check_run_urls")
        );
    }

    #[test]
    fn rejects_non_https_check_run_url() {
        let mut c = valid_checklist();
        c.verification.check_run_urls = vec!["http://ci.example.com/run/1".to_string()];
        assert!(matches!(
            c.validate().unwrap_err(),
            EvidenceValidationError::NonHttpsUrl(_)
        ));
    }

    #[test]
    fn rejects_empty_artifact_digest() {
        let mut c = valid_checklist();
        c.verification.artifact_digest = "".to_string();
        assert_eq!(
            c.validate().unwrap_err(),
            EvidenceValidationError::EmptyField("artifact_digest")
        );
    }

    #[test]
    fn rejects_mutable_http_artifact_reference() {
        let mut c = valid_checklist();
        c.verification.artifact_digest = "http://example.com/build/latest.tar.gz".to_string();
        assert!(matches!(
            c.validate().unwrap_err(),
            EvidenceValidationError::MutableArtifactReference(_)
        ));
    }

    #[test]
    fn rejects_empty_settlement_boundary() {
        let mut c = valid_checklist();
        c.payment.settlement_boundary = "".to_string();
        assert_eq!(
            c.validate().unwrap_err(),
            EvidenceValidationError::EmptyField("settlement_boundary")
        );
    }

    #[test]
    fn rejects_bare_https_scheme_without_host() {
        let mut c = valid_checklist();
        c.submission.pull_request_url = "https://".to_string();
        assert!(matches!(
            c.validate().unwrap_err(),
            EvidenceValidationError::InvalidUrl(_)
        ));
    }

    #[test]
    fn serialization_round_trip() {
        let original = valid_checklist();
        let json = serde_json::to_string(&original).unwrap();
        let deserialized: DirectBountyEvidenceChecklist = serde_json::from_str(&json).unwrap();
        assert_eq!(original, deserialized);
    }

    #[test]
    fn compact_json_contains_no_secrets_or_personal_data() {
        let json = serde_json::to_string(&valid_checklist()).unwrap();
        // Must not contain common PII/secret markers.
        for marker in ["password", "secret", "api_key", "private_key", "ssn", "email"] {
            assert!(
                !json.to_lowercase().contains(marker),
                "JSON output must not contain '{marker}'"
            );
        }
    }

    #[test]
    fn parses_and_validates_valid_fixture() {
        let json = include_str!("../../../fixtures/evidence/valid.json");
        let checklist: DirectBountyEvidenceChecklist = serde_json::from_str(json).unwrap();
        assert!(checklist.validate().is_ok());
    }

    #[test]
    fn parses_and_rejects_invalid_fixture() {
        let json = include_str!("../../../fixtures/evidence/invalid.json");
        let checklist: DirectBountyEvidenceChecklist = serde_json::from_str(json).unwrap();
        assert!(checklist.validate().is_err());
    }
}
