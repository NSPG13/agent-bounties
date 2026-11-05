//! Compatibility schemas for retired platform-hosted generation.
//! No provider client, credential loading, or generation runtime is retained.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use utoipa::ToSchema;

pub const HOSTED_GENERATION_REMOVED: &str = "hosted_generation_removed";
pub const OWN_AI_NEXT_ACTION: &str = "Use your own AI account to prepare or analyze the draft, then review it at https://agentizable.ai/post.html. Agentizable does not run a hosted model.";
const MAX_INPUT_CHARS: usize = 12_000;

fn default_objective_task_limit() -> u8 {
    5
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub struct CloudAgentReadiness {
    pub schema_version: String,
    pub available: bool,
    pub status: String,
    pub error_code: String,
    pub next_action: String,
    pub execution: String,
    pub provider: String,
    pub protocol: Option<String>,
    pub model: Option<String>,
    pub reasoning_effort: String,
    pub public_drafts: bool,
    pub local_fallback: bool,
    pub max_input_chars: usize,
    pub max_output_tokens: u32,
    pub max_daily_drafts: u32,
    pub missing_configuration: Vec<String>,
    pub capabilities: Vec<String>,
    pub authority: String,
    pub evidence_boundary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub struct CloudBountyDraftRequest {
    pub objective: String,
    #[serde(default)]
    pub context: Option<String>,
    #[serde(default)]
    pub constraints: Vec<String>,
    #[serde(default)]
    pub source_url: Option<String>,
    #[serde(default)]
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ToSchema)]
pub struct CloudBountyDraft {
    pub schema_version: String,
    pub provider: String,
    pub model: String,
    pub title: String,
    pub goal: String,
    pub acceptance_criteria: Vec<String>,
    pub benchmark: Value,
    pub evidence_schema: Value,
    pub questions: Vec<String>,
    pub risk_flags: Vec<String>,
    pub source_url: Option<String>,
    pub next_action: String,
    pub evidence_boundary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ToSchema)]
pub struct CloudObjectivePlanRequest {
    pub objective: String,
    #[serde(default)]
    pub context: Option<String>,
    #[serde(default)]
    pub constraints: Vec<String>,
    #[serde(default = "default_objective_task_limit")]
    pub max_tasks: u8,
    #[serde(default)]
    pub solver_budget_usdc: Option<String>,
    #[serde(default)]
    pub source_url: Option<String>,
    #[serde(default)]
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub struct CloudObjectiveVerifierDraft {
    pub kind: String,
    pub command: Option<String>,
    pub endpoint: Option<String>,
    pub expected_status: Option<u16>,
    pub expected_output_contains: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ToSchema)]
pub struct CloudObjectiveTask {
    pub task_id: String,
    pub title: String,
    pub goal: String,
    pub depends_on: Vec<String>,
    pub acceptance_criteria: Vec<String>,
    pub verifier: CloudObjectiveVerifierDraft,
    pub evidence_schema: Value,
    pub effort_weight: u16,
    pub suggested_solver_reward_usdc: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub struct CloudObjectiveExecutionPolicy {
    pub digital_only: bool,
    pub dependency_model: String,
    pub maximum_tasks: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub struct CloudObjectiveVerificationPolicy {
    pub committed_before_claim: bool,
    pub allowed_verifier_kinds: Vec<String>,
    pub model_authority: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub struct CloudObjectiveSettlementPolicy {
    pub protocol: String,
    pub network: String,
    pub asset: String,
    pub funded_before_claim: bool,
    pub payout_evidence: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ToSchema)]
pub struct CloudObjectivePlan {
    pub schema_version: String,
    pub provider: String,
    pub model: String,
    pub title: String,
    pub objective: String,
    pub success_definition: String,
    pub tasks: Vec<CloudObjectiveTask>,
    pub parallel_layers: Vec<Vec<String>>,
    pub solver_budget_usdc: Option<String>,
    pub execution_policy: CloudObjectiveExecutionPolicy,
    pub verification_policy: CloudObjectiveVerificationPolicy,
    pub settlement_policy: CloudObjectiveSettlementPolicy,
    pub questions: Vec<String>,
    pub risk_flags: Vec<String>,
    pub source_url: Option<String>,
    pub next_action: String,
    pub evidence_boundary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ToSchema)]
pub struct CloudBountyAnalysisRequest {
    pub terms_hash: String,
    pub title: String,
    pub goal: String,
    pub acceptance_criteria: Vec<String>,
    pub benchmark: Value,
    pub evidence_schema: Value,
    pub verification_policy: Value,
    pub reward: Value,
    pub bond: Value,
    pub deadline: Option<String>,
    pub payment_status: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub struct CloudBountyAnalysisReference {
    pub field: String,
    pub rationale: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ToSchema)]
pub struct CloudBountyAnalysis {
    pub schema_version: String,
    pub provider: String,
    pub model: String,
    pub terms_hash: String,
    pub required_skills: Vec<String>,
    pub hard_requirements: Vec<String>,
    pub deliverable_checklist: Vec<String>,
    pub evidence_checklist: Vec<String>,
    pub reward: Value,
    pub bond: Value,
    pub deadline: Option<String>,
    pub payment_status: Value,
    pub verification_risks: Vec<String>,
    pub ambiguous_requirements: Vec<String>,
    pub missing_information: Vec<String>,
    pub source_field_references: Vec<CloudBountyAnalysisReference>,
    pub confidence: f32,
    pub next_action: String,
    pub evidence_boundary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub struct CloudUnfundedBountyRequest {
    pub title: String,
    pub goal: String,
    pub acceptance_criteria: Vec<String>,
    #[serde(default)]
    pub source_url: Option<String>,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ToSchema)]
pub struct CloudDemoSolution {
    pub schema_version: String,
    pub provider: String,
    pub model: String,
    pub agent_name: String,
    pub completion_status: String,
    pub summary: String,
    pub deliverable_markdown: String,
    pub evidence: Value,
    pub limitations: Vec<String>,
    pub payment_due_usdc: String,
    pub evidence_boundary: String,
}

#[derive(Debug, Error)]
pub enum CloudAgentError {
    #[error("invalid unfunded bounty request: {0}")]
    InvalidRequest(String),
}

#[derive(Clone, Default)]
pub struct CloudAgentService;

impl CloudAgentService {
    // Kept as a compatibility constructor. Legacy environment flags and model
    // credentials cannot re-enable the removed service.
    pub fn from_env() -> Result<Self, CloudAgentError> {
        Ok(Self)
    }

    pub fn public_drafts(&self) -> bool {
        false
    }

    pub fn readiness(&self) -> CloudAgentReadiness {
        CloudAgentReadiness {
            schema_version: "agent-bounties/cloud-agent-readiness-v1".into(),
            available: false,
            status: "removed".into(),
            error_code: HOSTED_GENERATION_REMOVED.into(),
            next_action: OWN_AI_NEXT_ACTION.into(),
            execution: "removed".into(),
            provider: "none".into(),
            protocol: None,
            model: None,
            reasoning_effort: "none".into(),
            public_drafts: false,
            local_fallback: false,
            max_input_chars: 0,
            max_output_tokens: 0,
            max_daily_drafts: 0,
            missing_configuration: vec![],
            capabilities: vec![],
            authority: "none".into(),
            evidence_boundary: "Hosted model generation has been removed. Use your own AI account. A draft cannot sign, fund, claim, verify, settle, or prove payment.".into(),
        }
    }
}

/// Validate a community request without generating a solution or contacting a provider.
pub fn validate_unfunded_bounty_request(
    request: &CloudUnfundedBountyRequest,
) -> Result<(), CloudAgentError> {
    let invalid = |message: &str| Err(CloudAgentError::InvalidRequest(message.into()));
    if request.title.trim().is_empty() || request.title.chars().count() > 200 {
        return invalid("title must contain 1 to 200 characters");
    }
    if request.goal.trim().is_empty() || request.goal.chars().count() > MAX_INPUT_CHARS {
        return invalid("goal must contain 1 to 12000 characters");
    }
    if request.acceptance_criteria.is_empty()
        || request.acceptance_criteria.len() > 20
        || request
            .acceptance_criteria
            .iter()
            .any(|item| item.trim().is_empty() || item.chars().count() > 1000)
    {
        return invalid(
            "acceptance_criteria must contain 1 to 20 non-empty items of at most 1000 characters",
        );
    }
    if request
        .source_url
        .as_deref()
        .is_some_and(|url| !url.starts_with("https://") || url.chars().count() > 2048)
    {
        return invalid("source_url must be a bounded HTTPS URL");
    }
    if request.idempotency_key.is_empty()
        || request.idempotency_key.len() > 128
        || !request
            .idempotency_key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':'))
    {
        return invalid(
            "idempotency_key must use 1-128 ASCII letters, digits, colon, dash, or underscore",
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> CloudUnfundedBountyRequest {
        CloudUnfundedBountyRequest {
            title: "Review a draft".into(),
            goal: "Provide one review.".into(),
            acceptance_criteria: vec!["Include an actionable review.".into()],
            source_url: None,
            idempotency_key: "review:1".into(),
        }
    }
    #[test]
    fn retired_readiness_cannot_offer_generation_or_credential_setup() {
        let service = CloudAgentService::from_env().unwrap();
        let readiness = service.readiness();
        assert!(!service.public_drafts());
        assert!(!readiness.available && !readiness.local_fallback && !readiness.public_drafts);
        assert_eq!(readiness.error_code, HOSTED_GENERATION_REMOVED);
        assert_eq!(readiness.execution, "removed");
        assert_eq!(readiness.authority, "none");
        assert!(readiness.model.is_none() && readiness.protocol.is_none());
        assert!(readiness.capabilities.is_empty() && readiness.missing_configuration.is_empty());
        assert_eq!(
            (
                readiness.max_input_chars,
                readiness.max_output_tokens,
                readiness.max_daily_drafts
            ),
            (0, 0, 0)
        );
        assert!(readiness.next_action.contains("your own AI"));
    }
    #[test]
    fn unfunded_validation_is_pure_and_bounded() {
        validate_unfunded_bounty_request(&request()).unwrap();
        let mut oversized = request();
        oversized.goal = "x".repeat(12_001);
        assert!(validate_unfunded_bounty_request(&oversized).is_err());
        let mut empty = request();
        empty.acceptance_criteria.clear();
        assert!(validate_unfunded_bounty_request(&empty).is_err());
        let mut invalid_key = request();
        invalid_key.idempotency_key = "invalid key".into();
        assert!(validate_unfunded_bounty_request(&invalid_key).is_err());
    }
}
