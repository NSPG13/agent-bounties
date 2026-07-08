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
        let Some(evidence) = input.evidence.as_ref() else {
            return Ok(make_result(
                input.bounty_id,
                input.submission.id,
                None,
                VerifierKind::GitHubCi,
                VerificationDecision::NeedsReview,
                "GitHub CI evidence needs review: structured evidence is required",
                0.35,
            ));
        };

        let parsed = match GitHubCiEvidence::from_value(evidence, &input.submission) {
            Ok(parsed) => parsed,
            Err(message) => {
                return Ok(make_result(
                    input.bounty_id,
                    input.submission.id,
                    None,
                    VerifierKind::GitHubCi,
                    VerificationDecision::NeedsReview,
                    format!("GitHub CI evidence needs review: {message}"),
                    0.35,
                ));
            }
        };

        if let Err(reason) = parsed.validate_ownership(&input.submission) {
            return Ok(make_result_with_payload(ResultSeed {
                bounty_id: input.bounty_id,
                submission_id: input.submission.id,
                verifier_agent_id: None,
                kind: VerifierKind::GitHubCi,
                decision: VerificationDecision::Rejected,
                summary: format!("GitHub CI evidence rejected: {reason}"),
                confidence: 0.0,
                payload: Some(&parsed.canonical_payload()),
            }));
        }

        let status = parsed.check_status.to_ascii_lowercase();
        let conclusion = parsed.check_conclusion.to_ascii_lowercase();
        let completed = matches!(status.as_str(), "completed" | "success" | "passed");
        let succeeded = matches!(conclusion.as_str(), "success" | "passed");
        let accepted = completed && succeeded;
        let short_sha = short_sha(&parsed.commit_sha);
        let payload = parsed.canonical_payload();
        if accepted {
            if let Some(reason) = parsed.automatic_acceptance_review_reason() {
                return Ok(make_result_with_payload(ResultSeed {
                    bounty_id: input.bounty_id,
                    submission_id: input.submission.id,
                    verifier_agent_id: None,
                    kind: VerifierKind::GitHubCi,
                    decision: VerificationDecision::NeedsReview,
                    summary: format!("GitHub CI evidence needs review: {reason}"),
                    confidence: 0.62,
                    payload: Some(&payload),
                }));
            }
        }
        let summary = if accepted {
            format!(
                "GitHub CI evidence accepted: {} {} commit {} check {}#{} succeeded",
                parsed.repository,
                parsed
                    .pull_request_number()
                    .map(|number| format!("PR #{number}"))
                    .unwrap_or_else(|| "repository evidence".to_string()),
                short_sha,
                parsed.check_name,
                parsed.check_run_id
            )
        } else {
            format!(
                "GitHub CI evidence rejected: check {}#{} for {} commit {} ended with status `{}` and conclusion `{}`",
                parsed.check_name,
                parsed.check_run_id,
                parsed.repository,
                short_sha,
                parsed.check_status,
                parsed.check_conclusion
            )
        };

        Ok(make_result_with_payload(ResultSeed {
            bounty_id: input.bounty_id,
            submission_id: input.submission.id,
            verifier_agent_id: None,
            kind: VerifierKind::GitHubCi,
            decision: if accepted {
                VerificationDecision::Accepted
            } else {
                VerificationDecision::Rejected
            },
            summary,
            confidence: if accepted { 0.98 } else { 0.0 },
            payload: Some(&payload),
        }))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GitHubCiEvidence {
    repository: String,
    pull_request_url: Option<String>,
    pull_request: Option<GitHubPullRequestEvidence>,
    commit_sha: String,
    check_run_id: String,
    check_name: String,
    check_status: String,
    check_conclusion: String,
    check_head_sha: String,
    check_repository: String,
    check_html_url: Option<String>,
}

impl GitHubCiEvidence {
    fn from_value(evidence: &Value, submission: &Submission) -> Result<Self, String> {
        let check_run = evidence.get("check_run").filter(|value| value.is_object());
        let repository = evidence_string(evidence, "repository")
            .or_else(|| evidence_string(evidence, "repo"))
            .or_else(|| {
                check_run.and_then(|value| nested_string(value, &["repository", "full_name"]))
            })
            .or_else(|| github_pr_reference(&submission.artifact_uri).map(|pr| pr.repository))
            .ok_or_else(|| "repository is required".to_string())?;
        let repository = normalize_repository(&repository)
            .ok_or_else(|| "repository must be in owner/name form".to_string())?;

        let pull_request_url = evidence_string(evidence, "pull_request_url")
            .or_else(|| evidence_string(evidence, "pr_url"))
            .or_else(|| github_pr_reference(&submission.artifact_uri).map(|pr| pr.url));
        if let Some(url) = &pull_request_url {
            let pr = github_pr_reference(url)
                .ok_or_else(|| "pull_request_url must be a GitHub pull request URL".to_string())?;
            if pr.repository != repository {
                return Err("pull_request_url repository does not match repository".to_string());
            }
        }

        let pull_request = evidence
            .get("pull_request")
            .filter(|value| value.is_object())
            .map(GitHubPullRequestEvidence::from_value)
            .transpose()?;

        let commit_sha = evidence_string(evidence, "commit_sha")
            .or_else(|| evidence_string(evidence, "head_sha"))
            .or_else(|| check_run.and_then(|value| evidence_string(value, "head_sha")))
            .ok_or_else(|| "commit_sha is required".to_string())?;
        let commit_sha = normalize_git_sha(&commit_sha)
            .ok_or_else(|| "commit_sha must be a 7-64 character hex Git SHA".to_string())?;

        let check_run_id = evidence_string(evidence, "check_run_id")
            .or_else(|| check_run.and_then(|value| evidence_string(value, "id")))
            .or_else(|| {
                check_run.and_then(|value| evidence_i64(value, "id").map(|id| id.to_string()))
            })
            .ok_or_else(|| "check_run_id is required".to_string())?;
        if check_run_id.trim().is_empty() {
            return Err("check_run_id is required".to_string());
        }

        let check_name = evidence_string(evidence, "check_name")
            .or_else(|| check_run.and_then(|value| evidence_string(value, "name")))
            .ok_or_else(|| "check_name is required".to_string())?;
        if check_name.trim().is_empty() {
            return Err("check_name is required".to_string());
        }

        let check_status = evidence_string(evidence, "check_status")
            .or_else(|| evidence_string(evidence, "status"))
            .or_else(|| check_run.and_then(|value| evidence_string(value, "status")))
            .ok_or_else(|| "check_status is required".to_string())?;

        let check_conclusion = evidence_string(evidence, "check_conclusion")
            .or_else(|| evidence_string(evidence, "conclusion"))
            .or_else(|| check_run.and_then(|value| evidence_string(value, "conclusion")))
            .ok_or_else(|| "check_conclusion is required".to_string())?;

        let check_head_sha = evidence_string(evidence, "check_head_sha")
            .or_else(|| check_run.and_then(|value| evidence_string(value, "head_sha")))
            .or_else(|| {
                check_run.and_then(|value| nested_string(value, &["check_suite", "head_sha"]))
            })
            .ok_or_else(|| "check_head_sha is required".to_string())?;
        let check_head_sha = normalize_git_sha(&check_head_sha)
            .ok_or_else(|| "check_head_sha must be a 7-64 character hex Git SHA".to_string())?;

        let check_repository = evidence_string(evidence, "check_repository")
            .or_else(|| {
                check_run.and_then(|value| nested_string(value, &["repository", "full_name"]))
            })
            .unwrap_or_else(|| repository.clone());
        let check_repository = normalize_repository(&check_repository)
            .ok_or_else(|| "check_repository must be in owner/name form".to_string())?;

        let check_html_url = evidence_string(evidence, "check_html_url")
            .or_else(|| check_run.and_then(|value| evidence_string(value, "html_url")));

        Ok(Self {
            repository,
            pull_request_url,
            pull_request,
            commit_sha,
            check_run_id: check_run_id.trim().to_string(),
            check_name: check_name.trim().to_string(),
            check_status: check_status.trim().to_string(),
            check_conclusion: check_conclusion.trim().to_string(),
            check_head_sha,
            check_repository,
            check_html_url,
        })
    }

    fn validate_ownership(&self, submission: &Submission) -> Result<(), String> {
        if self.check_repository != self.repository {
            return Err(format!(
                "check run repository `{}` does not match submitted repository `{}`",
                self.check_repository, self.repository
            ));
        }
        if self.check_head_sha != self.commit_sha {
            return Err(format!(
                "check run head SHA `{}` does not match submitted commit `{}`",
                self.check_head_sha, self.commit_sha
            ));
        }
        if let Some(submission_pr) = github_pr_reference(&submission.artifact_uri) {
            if submission_pr.repository != self.repository {
                return Err(format!(
                    "submission artifact repository `{}` does not match evidence repository `{}`",
                    submission_pr.repository, self.repository
                ));
            }
            if let Some(evidence_pr_url) = &self.pull_request_url {
                let evidence_pr =
                    github_pr_reference(evidence_pr_url).expect("validated pull request URL");
                if evidence_pr.number != submission_pr.number {
                    return Err(format!(
                        "evidence PR #{} does not match submitted PR #{}",
                        evidence_pr.number, submission_pr.number
                    ));
                }
            }
        }
        if let Some(url) = &self.check_html_url {
            if !github_url_belongs_to_repository(url, &self.repository) {
                return Err(format!(
                    "check run URL does not belong to repository `{}`",
                    self.repository
                ));
            }
        }
        Ok(())
    }

    fn automatic_acceptance_review_reason(&self) -> Option<String> {
        self.pull_request_url.as_ref()?;
        let Some(pull_request) = &self.pull_request else {
            return Some(
                "pull_request metadata with author, merge state, merger, and reviews is required"
                    .to_string(),
            );
        };
        if !pull_request.merged {
            return Some(
                "pull request must be merged before automatic bounty acceptance".to_string(),
            );
        }
        let Some(merged_by_login) = &pull_request.merged_by_login else {
            return Some(
                "pull_request.merged_by_login is required to rule out self-merge".to_string(),
            );
        };
        if merged_by_login == &pull_request.author_login {
            return Some(
                "pull request was merged by its author; independent operator review is required"
                    .to_string(),
            );
        }
        if !pull_request.has_independent_approval() {
            return Some(
                "pull request needs at least one APPROVED review from a non-author reviewer"
                    .to_string(),
            );
        }
        None
    }

    fn canonical_payload(&self) -> String {
        format!(
            "github-ci:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}",
            self.repository,
            self.pull_request_url.as_deref().unwrap_or(""),
            self.pull_request
                .as_ref()
                .map(GitHubPullRequestEvidence::canonical_payload)
                .unwrap_or_default(),
            self.commit_sha,
            self.check_run_id,
            self.check_name,
            self.check_status.to_ascii_lowercase(),
            self.check_conclusion.to_ascii_lowercase(),
            self.check_head_sha,
            self.check_repository
        )
    }

    fn pull_request_number(&self) -> Option<u64> {
        self.pull_request_url
            .as_deref()
            .and_then(github_pr_reference)
            .map(|pr| pr.number)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GitHubPullRequestEvidence {
    author_login: String,
    merged: bool,
    merged_by_login: Option<String>,
    reviews: Vec<GitHubReviewEvidence>,
}

impl GitHubPullRequestEvidence {
    fn from_value(value: &Value) -> Result<Self, String> {
        let author_login = evidence_string(value, "author_login")
            .or_else(|| evidence_string(value, "user_login"))
            .or_else(|| nested_string(value, &["author", "login"]))
            .or_else(|| nested_string(value, &["user", "login"]))
            .and_then(|login| normalize_github_login(&login))
            .ok_or_else(|| "pull_request.author_login is required".to_string())?;
        let merged = evidence_bool(value, "merged").unwrap_or_else(|| {
            evidence_string(value, "merged_at").is_some_and(|text| !text.trim().is_empty())
        });
        let merged_by_login = evidence_string(value, "merged_by_login")
            .or_else(|| nested_string(value, &["merged_by", "login"]))
            .and_then(|login| normalize_github_login(&login));
        let reviews = value
            .get("reviews")
            .and_then(Value::as_array)
            .map(|reviews| {
                reviews
                    .iter()
                    .filter_map(GitHubReviewEvidence::from_value)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        Ok(Self {
            author_login,
            merged,
            merged_by_login,
            reviews,
        })
    }

    fn has_independent_approval(&self) -> bool {
        self.reviews.iter().any(|review| {
            review.state == GitHubReviewState::Approved && review.author_login != self.author_login
        })
    }

    fn canonical_payload(&self) -> String {
        let reviews = self
            .reviews
            .iter()
            .map(GitHubReviewEvidence::canonical_payload)
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "pr:{}:{}:{}:{}",
            self.author_login,
            self.merged,
            self.merged_by_login.as_deref().unwrap_or(""),
            reviews
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GitHubReviewEvidence {
    author_login: String,
    state: GitHubReviewState,
}

impl GitHubReviewEvidence {
    fn from_value(value: &Value) -> Option<Self> {
        let author_login = evidence_string(value, "author_login")
            .or_else(|| evidence_string(value, "user_login"))
            .or_else(|| nested_string(value, &["author", "login"]))
            .or_else(|| nested_string(value, &["user", "login"]))
            .and_then(|login| normalize_github_login(&login))?;
        let state = evidence_string(value, "state")
            .and_then(|state| GitHubReviewState::from_str(&state))
            .unwrap_or(GitHubReviewState::Other);
        Some(Self {
            author_login,
            state,
        })
    }

    fn canonical_payload(&self) -> String {
        format!("{}:{:?}", self.author_login, self.state)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum GitHubReviewState {
    Approved,
    Other,
}

impl GitHubReviewState {
    fn from_str(value: &str) -> Option<Self> {
        match value.trim().to_ascii_uppercase().as_str() {
            "APPROVED" => Some(Self::Approved),
            "" => None,
            _ => Some(Self::Other),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GitHubPullRequestRef {
    repository: String,
    number: u64,
    url: String,
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
    make_result_with_payload(ResultSeed {
        bounty_id,
        submission_id,
        verifier_agent_id,
        kind,
        decision,
        summary: summary.into(),
        confidence,
        payload: None,
    })
}

struct ResultSeed<'a> {
    bounty_id: Id,
    submission_id: Id,
    verifier_agent_id: Option<Id>,
    kind: VerifierKind,
    decision: VerificationDecision,
    summary: String,
    confidence: f32,
    payload: Option<&'a str>,
}

fn make_result_with_payload(seed: ResultSeed<'_>) -> VerifierResult {
    let mut hasher = Sha256::new();
    hasher.update(format!(
        "{}:{}:{:?}:{:?}:{}:{}",
        seed.bounty_id, seed.submission_id, seed.kind, seed.decision, seed.summary, seed.confidence
    ));
    if let Some(payload) = seed.payload {
        hasher.update(":");
        hasher.update(payload);
    }

    VerifierResult {
        id: Uuid::new_v4(),
        bounty_id: seed.bounty_id,
        submission_id: seed.submission_id,
        verifier_agent_id: seed.verifier_agent_id,
        kind: seed.kind,
        decision: seed.decision,
        summary: seed.summary,
        confidence: seed.confidence,
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
    let value = evidence.get(key)?;
    if let Some(text) = value.as_str() {
        return Some(text.to_string());
    }
    value.as_u64().map(|number| number.to_string())
}

fn evidence_i64(evidence: &Value, key: &str) -> Option<i64> {
    evidence.get(key)?.as_i64()
}

fn evidence_bool(evidence: &Value, key: &str) -> Option<bool> {
    evidence.get(key)?.as_bool()
}

fn nested_string(value: &Value, path: &[&str]) -> Option<String> {
    let mut cursor = value;
    for key in path {
        cursor = cursor.get(*key)?;
    }
    if let Some(text) = cursor.as_str() {
        return Some(text.to_string());
    }
    cursor.as_u64().map(|number| number.to_string())
}

fn normalize_repository(value: &str) -> Option<String> {
    let trimmed = value.trim().trim_matches('/');
    let parts = trimmed.split('/').collect::<Vec<_>>();
    if parts.len() != 2 || parts.iter().any(|part| part.is_empty()) {
        return None;
    }
    if parts.iter().any(|part| {
        !part
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    }) {
        return None;
    }
    Some(format!(
        "{}/{}",
        parts[0].to_ascii_lowercase(),
        parts[1].to_ascii_lowercase()
    ))
}

fn normalize_git_sha(value: &str) -> Option<String> {
    let sha = value.trim().to_ascii_lowercase();
    if (7..=64).contains(&sha.len()) && sha.chars().all(|ch| ch.is_ascii_hexdigit()) {
        Some(sha)
    } else {
        None
    }
}

fn normalize_github_login(value: &str) -> Option<String> {
    let login = value.trim().trim_start_matches('@').to_ascii_lowercase();
    if login.is_empty()
        || login.len() > 64
        || !login
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '[' | ']'))
    {
        None
    } else {
        Some(login)
    }
}

fn github_pr_reference(url: &str) -> Option<GitHubPullRequestRef> {
    let trimmed = url.trim();
    let path = trimmed
        .strip_prefix("https://github.com/")
        .or_else(|| trimmed.strip_prefix("http://github.com/"))?;
    let path = path.trim_end_matches('/');
    let parts = path.split('/').collect::<Vec<_>>();
    if parts.len() < 4 || parts[2] != "pull" {
        return None;
    }
    let repository = normalize_repository(&format!("{}/{}", parts[0], parts[1]))?;
    let number = parts[3].parse::<u64>().ok()?;
    Some(GitHubPullRequestRef {
        repository,
        number,
        url: format!(
            "https://github.com/{}/{}/pull/{}",
            parts[0].to_ascii_lowercase(),
            parts[1].to_ascii_lowercase(),
            number
        ),
    })
}

fn github_url_belongs_to_repository(url: &str, repository: &str) -> bool {
    let Some(path) = url
        .trim()
        .strip_prefix("https://github.com/")
        .or_else(|| url.trim().strip_prefix("http://github.com/"))
    else {
        return false;
    };
    let parts = path.split('/').collect::<Vec<_>>();
    if parts.len() < 2 {
        return false;
    }
    normalize_repository(&format!("{}/{}", parts[0], parts[1])).as_deref() == Some(repository)
}

fn short_sha(sha: &str) -> String {
    sha.chars().take(7).collect()
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
        let submission = Submission {
            artifact_uri: "https://github.com/agent-bounties/agent-bounties/pull/42".to_string(),
            ..submission_for(bounty_id, "abc123abc123abc123")
        };

        let result = GitHubCiVerifier
            .verify(VerificationInput {
                bounty_id,
                submission,
                expected_artifact_digest: None,
                rubric: None,
                evidence: Some(github_ci_evidence()),
            })
            .await
            .unwrap();

        assert_eq!(result.kind, VerifierKind::GitHubCi);
        assert_eq!(result.decision, VerificationDecision::Accepted);
        assert!(result.summary.contains("PR #42"));
        assert_eq!(result.signed_payload_hash.len(), 64);
    }

    #[tokio::test]
    async fn github_ci_verifier_rejects_mismatched_commit_evidence() {
        let bounty_id = Uuid::new_v4();
        let submission = Submission {
            artifact_uri: "https://github.com/agent-bounties/agent-bounties/pull/42".to_string(),
            ..submission_for(bounty_id, "abc123abc123abc123")
        };
        let mut evidence = github_ci_evidence();
        evidence["check_run"]["head_sha"] =
            serde_json::json!("ffffffffffffffffffffffffffffffffffffffff");

        let result = GitHubCiVerifier
            .verify(VerificationInput {
                bounty_id,
                submission,
                expected_artifact_digest: None,
                rubric: None,
                evidence: Some(evidence),
            })
            .await
            .unwrap();

        assert_eq!(result.decision, VerificationDecision::Rejected);
        assert!(result.summary.contains("does not match submitted commit"));
    }

    #[tokio::test]
    async fn github_ci_verifier_rejects_failed_check_evidence() {
        let bounty_id = Uuid::new_v4();
        let submission = Submission {
            artifact_uri: "https://github.com/agent-bounties/agent-bounties/pull/42".to_string(),
            ..submission_for(bounty_id, "abc123abc123abc123")
        };
        let mut evidence = github_ci_evidence();
        evidence["check_run"]["conclusion"] = serde_json::json!("failure");

        let result = GitHubCiVerifier
            .verify(VerificationInput {
                bounty_id,
                submission,
                expected_artifact_digest: None,
                rubric: None,
                evidence: Some(evidence),
            })
            .await
            .unwrap();

        assert_eq!(result.decision, VerificationDecision::Rejected);
        assert!(result.summary.contains("conclusion `failure`"));
    }

    #[tokio::test]
    async fn github_ci_verifier_needs_review_without_pr_acceptance_metadata() {
        let bounty_id = Uuid::new_v4();
        let submission = Submission {
            artifact_uri: "https://github.com/agent-bounties/agent-bounties/pull/42".to_string(),
            ..submission_for(bounty_id, "abc123abc123abc123")
        };
        let mut evidence = github_ci_evidence();
        evidence.as_object_mut().unwrap().remove("pull_request");

        let result = GitHubCiVerifier
            .verify(VerificationInput {
                bounty_id,
                submission,
                expected_artifact_digest: None,
                rubric: None,
                evidence: Some(evidence),
            })
            .await
            .unwrap();

        assert_eq!(result.decision, VerificationDecision::NeedsReview);
        assert!(result.summary.contains("pull_request metadata"));
    }

    #[tokio::test]
    async fn github_ci_verifier_needs_review_for_self_merged_pr() {
        let bounty_id = Uuid::new_v4();
        let submission = Submission {
            artifact_uri: "https://github.com/agent-bounties/agent-bounties/pull/42".to_string(),
            ..submission_for(bounty_id, "abc123abc123abc123")
        };
        let mut evidence = github_ci_evidence();
        evidence["pull_request"]["merged_by_login"] = serde_json::json!("solver-agent");

        let result = GitHubCiVerifier
            .verify(VerificationInput {
                bounty_id,
                submission,
                expected_artifact_digest: None,
                rubric: None,
                evidence: Some(evidence),
            })
            .await
            .unwrap();

        assert_eq!(result.decision, VerificationDecision::NeedsReview);
        assert!(result.summary.contains("merged by its author"));
    }

    #[tokio::test]
    async fn github_ci_verifier_needs_review_without_independent_approval() {
        let bounty_id = Uuid::new_v4();
        let submission = Submission {
            artifact_uri: "https://github.com/agent-bounties/agent-bounties/pull/42".to_string(),
            ..submission_for(bounty_id, "abc123abc123abc123")
        };
        let mut evidence = github_ci_evidence();
        evidence["pull_request"]["reviews"] = serde_json::json!([
            {
                "author_login": "solver-agent",
                "state": "APPROVED"
            }
        ]);

        let result = GitHubCiVerifier
            .verify(VerificationInput {
                bounty_id,
                submission,
                expected_artifact_digest: None,
                rubric: None,
                evidence: Some(evidence),
            })
            .await
            .unwrap();

        assert_eq!(result.decision, VerificationDecision::NeedsReview);
        assert!(result.summary.contains("non-author reviewer"));
    }

    #[tokio::test]
    async fn github_ci_verifier_needs_review_for_missing_evidence() {
        let bounty_id = Uuid::new_v4();
        let submission = submission_for(bounty_id, "abc123abc123abc123");

        let result = GitHubCiVerifier
            .verify(VerificationInput {
                bounty_id,
                submission,
                expected_artifact_digest: None,
                rubric: None,
                evidence: None,
            })
            .await
            .unwrap();

        assert_eq!(result.decision, VerificationDecision::NeedsReview);
        assert!(result.summary.contains("structured evidence is required"));
    }

    #[tokio::test]
    async fn github_ci_verifier_rejects_replayed_pr_evidence() {
        let bounty_id = Uuid::new_v4();
        let submission = Submission {
            artifact_uri: "https://github.com/agent-bounties/agent-bounties/pull/43".to_string(),
            ..submission_for(bounty_id, "abc123abc123abc123")
        };

        let result = GitHubCiVerifier
            .verify(VerificationInput {
                bounty_id,
                submission,
                expected_artifact_digest: None,
                rubric: None,
                evidence: Some(github_ci_evidence()),
            })
            .await
            .unwrap();

        assert_eq!(result.decision, VerificationDecision::Rejected);
        assert!(result.summary.contains("does not match submitted PR"));
    }

    #[tokio::test]
    async fn github_ci_verifier_hash_binds_check_run_payload() {
        let bounty_id = Uuid::new_v4();
        let submission = Submission {
            artifact_uri: "https://github.com/agent-bounties/agent-bounties/pull/42".to_string(),
            ..submission_for(bounty_id, "abc123abc123abc123")
        };
        let mut replayed = github_ci_evidence();
        replayed["check_run"]["id"] = serde_json::json!(123456790_u64);

        let first = GitHubCiVerifier
            .verify(VerificationInput {
                bounty_id,
                submission: submission.clone(),
                expected_artifact_digest: None,
                rubric: None,
                evidence: Some(github_ci_evidence()),
            })
            .await
            .unwrap();
        let second = GitHubCiVerifier
            .verify(VerificationInput {
                bounty_id,
                submission,
                expected_artifact_digest: None,
                rubric: None,
                evidence: Some(replayed),
            })
            .await
            .unwrap();

        assert_eq!(first.decision, VerificationDecision::Accepted);
        assert_eq!(second.decision, VerificationDecision::Accepted);
        assert_ne!(first.signed_payload_hash, second.signed_payload_hash);
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

    fn github_ci_evidence() -> Value {
        serde_json::json!({
            "repository": "agent-bounties/agent-bounties",
            "pull_request_url": "https://github.com/agent-bounties/agent-bounties/pull/42",
            "pull_request": {
                "author_login": "solver-agent",
                "merged": true,
                "merged_by_login": "maintainer",
                "reviews": [
                    {
                        "author_login": "maintainer",
                        "state": "APPROVED"
                    }
                ]
            },
            "commit_sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "check_run": {
                "id": 123456789_u64,
                "name": "full-check",
                "status": "completed",
                "conclusion": "success",
                "head_sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "html_url": "https://github.com/agent-bounties/agent-bounties/actions/runs/123456789",
                "repository": {
                    "full_name": "agent-bounties/agent-bounties"
                }
            }
        })
    }
}
