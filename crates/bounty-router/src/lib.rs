use domain::{Capability, CapabilityClass, FundingMode, HelpRequest, PrivacyLevel, VerifierKind};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub enum RouterAction {
    UseTemplate,
    RequestQuotes,
    PostBounty,
    RequestVerification,
    SolveDirectly,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RouteDecision {
    pub action: RouterAction,
    pub template_slug: Option<String>,
    pub capability_class: CapabilityClass,
    pub verifier_kind: VerifierKind,
    pub funding_mode: FundingMode,
    pub confidence: f32,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct BountyRouter {
    pub quote_threshold_minor: i64,
}

impl Default for BountyRouter {
    fn default() -> Self {
        Self {
            quote_threshold_minor: 500,
        }
    }
}

impl BountyRouter {
    pub fn route_blocked_goal(
        &self,
        request: &HelpRequest,
        capabilities: &[Capability],
    ) -> RouteDecision {
        let class = classify_goal(&request.goal, &request.context);
        let template = template_for_class(&class);
        let verifier = verifier_for_class(&class);
        let matching_capabilities = capabilities
            .iter()
            .filter(|capability| capability.class == class)
            .count();

        let action = if request.required_confidence >= 0.95 {
            RouterAction::RequestVerification
        } else if matching_capabilities == 0 {
            RouterAction::PostBounty
        } else if request.budget.amount >= self.quote_threshold_minor {
            RouterAction::RequestQuotes
        } else {
            RouterAction::UseTemplate
        };

        let funding_mode = match request.privacy {
            PrivacyLevel::Public | PrivacyLevel::RedactedPublicProof => FundingMode::BaseUsdcEscrow,
            PrivacyLevel::Private => FundingMode::StripeFiatLedger,
        };

        RouteDecision {
            action,
            template_slug: Some(template.to_string()),
            capability_class: class,
            verifier_kind: verifier,
            funding_mode,
            confidence: if matching_capabilities > 0 {
                0.86
            } else {
                0.62
            },
            reason: format!(
                "matched {matching_capabilities} capabilities and selected template {template}"
            ),
        }
    }
}

pub fn classify_goal(goal: &str, context: &str) -> CapabilityClass {
    let text = format!("{} {}", goal.to_lowercase(), context.to_lowercase());
    let tokens = text
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();

    if tokens.contains(&"ci") || text.contains("test failure") || text.contains("check failed") {
        CapabilityClass::Ci
    } else if text.contains("extract") || text.contains("schema") || text.contains("pdf") {
        CapabilityClass::Extraction
    } else if text.contains("browser") || text.contains("website") || text.contains("login") {
        CapabilityClass::BrowserWorkflow
    } else if text.contains("verify") || text.contains("confirm") || text.contains("evidence") {
        CapabilityClass::Verification
    } else if text.contains("docs") || text.contains("documentation") || text.contains("readme") {
        CapabilityClass::Documentation
    } else if text.contains("research") || text.contains("sources") || text.contains("compare") {
        CapabilityClass::Research
    } else {
        CapabilityClass::Coding
    }
}

pub fn template_for_class(class: &CapabilityClass) -> &'static str {
    match class {
        CapabilityClass::Coding => "small-code-change",
        CapabilityClass::Research => "primary-source-research",
        CapabilityClass::Extraction => "extract-data-to-schema",
        CapabilityClass::Verification => "independent-claim-verification",
        CapabilityClass::Documentation => "write-docs-for-area",
        CapabilityClass::Ci => "fix-ci-failure",
        CapabilityClass::BrowserWorkflow => "run-browser-workflow",
    }
}

pub fn verifier_for_class(class: &CapabilityClass) -> VerifierKind {
    match class {
        CapabilityClass::Coding | CapabilityClass::Ci => VerifierKind::GitHubCi,
        CapabilityClass::Extraction => VerifierKind::JsonSchema,
        CapabilityClass::Verification | CapabilityClass::Research => VerifierKind::Manual,
        CapabilityClass::Documentation => VerifierKind::AiJudgeFilter,
        CapabilityClass::BrowserWorkflow => VerifierKind::DockerCommand,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::{Agent, Money};

    #[test]
    fn routes_ci_failure_to_ci_template() {
        let requester = Agent::new("requester");
        let solver = Agent::new("solver");
        let request = HelpRequest::new(
            requester.id,
            "CI failed after dependency update",
            "GitHub check failed",
            Money::new(1_000, "usdc").unwrap(),
            PrivacyLevel::Public,
        );
        let capability = Capability {
            id: uuid::Uuid::new_v4(),
            agent_id: solver.id,
            class: CapabilityClass::Ci,
            template_slugs: vec!["fix-ci-failure".to_string()],
            min_price: Money::new(100, "usdc").unwrap(),
            max_price: Money::new(5_000, "usdc").unwrap(),
            latency_seconds: 600,
            supported_verifiers: vec![VerifierKind::GitHubCi],
        };

        let decision = BountyRouter::default().route_blocked_goal(&request, &[capability]);
        assert_eq!(decision.capability_class, CapabilityClass::Ci);
        assert_eq!(decision.template_slug.as_deref(), Some("fix-ci-failure"));
        assert_eq!(decision.verifier_kind, VerifierKind::GitHubCi);
        assert_eq!(decision.action, RouterAction::RequestQuotes);
    }
}
