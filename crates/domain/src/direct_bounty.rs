use serde::{Deserialize, Serialize};
use url::Url;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DirectBountyEvidenceChecklist {
    pub submission_evidence: SubmissionEvidence,
    pub verification_evidence: VerificationEvidence,
    pub payment_evidence: PaymentEvidence,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SubmissionEvidence {
    pub repository: String,
    pub commit_sha: String,
    pub subdirectory: Option<String>,
    pub pull_request_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VerificationEvidence {
    pub check_run_urls: Vec<String>,
    pub artifact_digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PaymentEvidence {
    pub settlement_boundary: String,
}

impl DirectBountyEvidenceChecklist {
    pub fn validate(&self) -> Result<(), String> {
        if self.submission_evidence.repository.trim().is_empty() {
            return Err("Repository cannot be empty".to_string());
        }
        
        if self.submission_evidence.commit_sha.trim().is_empty() {
            return Err("Commit SHA cannot be empty".to_string());
        }

        Self::validate_https_url(&self.submission_evidence.pull_request_url)?;

        if self.verification_evidence.check_run_urls.is_empty() {
            return Err("At least one check-run URL is required".to_string());
        }

        for url in &self.verification_evidence.check_run_urls {
            Self::validate_https_url(url)?;
        }

        if self.verification_evidence.artifact_digest.trim().is_empty() {
            return Err("Artifact digest cannot be empty".to_string());
        }
        
        if self.payment_evidence.settlement_boundary.trim().is_empty() {
            return Err("Settlement boundary cannot be empty".to_string());
        }

        Ok(())
    }

    fn validate_https_url(url_str: &str) -> Result<(), String> {
        if url_str.trim().is_empty() {
            return Err("URL cannot be empty".to_string());
        }
        
        let parsed_url = Url::parse(url_str).map_err(|_| format!("Invalid URL format: {}", url_str))?;
        if parsed_url.scheme() != "https" {
            return Err(format!("URL must use HTTPS scheme: {}", url_str));
        }
        
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_checklist() {
        let checklist = DirectBountyEvidenceChecklist {
            submission_evidence: SubmissionEvidence {
                repository: "NSPG13/agent-bounties".to_string(),
                commit_sha: "7d8251e605d398d5c4146c3b6831d102e1b816ab".to_string(),
                subdirectory: None,
                pull_request_url: "https://github.com/NSPG13/agent-bounties/pull/123".to_string(),
            },
            verification_evidence: VerificationEvidence {
                check_run_urls: vec!["https://github.com/NSPG13/agent-bounties/runs/123456".to_string()],
                artifact_digest: "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".to_string(),
            },
            payment_evidence: PaymentEvidence {
                settlement_boundary: "BountySettled(tx_hash)".to_string(),
            },
        };

        assert!(checklist.validate().is_ok());
    }

    #[test]
    fn test_invalid_http_url() {
        let checklist = DirectBountyEvidenceChecklist {
            submission_evidence: SubmissionEvidence {
                repository: "NSPG13/agent-bounties".to_string(),
                commit_sha: "7d8251e605d398d5".to_string(),
                subdirectory: None,
                pull_request_url: "http://github.com/NSPG13/agent-bounties/pull/123".to_string(),
            },
            verification_evidence: VerificationEvidence {
                check_run_urls: vec!["https://github.com/NSPG13/agent-bounties/runs/123456".to_string()],
                artifact_digest: "sha256:xyz".to_string(),
            },
            payment_evidence: PaymentEvidence {
                settlement_boundary: "BountySettled".to_string(),
            },
        };

        let result = checklist.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("HTTPS"));
    }

    #[test]
    fn test_missing_fields() {
        let checklist = DirectBountyEvidenceChecklist {
            submission_evidence: SubmissionEvidence {
                repository: "".to_string(),
                commit_sha: "7d8251e605d398d5".to_string(),
                subdirectory: None,
                pull_request_url: "https://github.com/NSPG13/agent-bounties/pull/123".to_string(),
            },
            verification_evidence: VerificationEvidence {
                check_run_urls: vec!["https://github.com/NSPG13/agent-bounties/runs/123456".to_string()],
                artifact_digest: "sha256:xyz".to_string(),
            },
            payment_evidence: PaymentEvidence {
                settlement_boundary: "BountySettled".to_string(),
            },
        };

        assert!(checklist.validate().is_err());
    }
}
