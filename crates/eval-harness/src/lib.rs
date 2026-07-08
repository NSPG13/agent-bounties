use bounty_router::BountyRouter;
use chrono::Utc;
use domain::{
    Agent, Capability, CapabilityClass, FundingMode, HelpRequest, Money, PaymentRail, PrivacyLevel,
    RiskAction, RiskSurface, Submission, VerificationDecision, VerifierKind,
};
use risk::{
    BountyRiskInput, HelpRequestRiskInput, RiskAssessment, RiskPolicy, SubmissionRiskInput,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use utoipa::ToSchema;
use uuid::Uuid;
use verifier_sdk::{verify_with_builtin, VerificationInput};

#[derive(Debug, Error)]
pub enum EvalError {
    #[error("invalid fixture: {0}")]
    InvalidFixture(String),
    #[error("verifier fixture failed: {0}")]
    Verifier(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct HelpRequestFixture {
    pub name: String,
    pub goal: String,
    pub context: String,
    pub budget_minor: i64,
    pub currency: String,
    pub privacy: PrivacyLevel,
    pub expected_template: String,
    pub expected_capability_class: CapabilityClass,
    pub expected_verifier: VerifierKind,
    pub expected_funding_mode: FundingMode,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EvalCaseResult {
    pub name: String,
    pub passed: bool,
    pub score: f32,
    pub failures: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EvalSuiteResult {
    pub suite: String,
    pub score: f32,
    pub passed: bool,
    pub cases: Vec<EvalCaseResult>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub enum EvalLoopKind {
    Router,
    Template,
    Verifier,
    Proof,
    Abuse,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LoopCandidateResult {
    pub name: String,
    pub score: f32,
    pub accepted: bool,
    pub failures: Vec<String>,
    pub source_suite: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LoopRunResult {
    pub loop_name: String,
    pub kind: EvalLoopKind,
    pub baseline: LoopCandidate,
    pub gate_threshold: f32,
    pub candidates: Vec<LoopCandidateResult>,
    pub accepted_candidate: Option<String>,
    pub score_delta: f32,
    pub passed: bool,
    pub source_suites: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LoopSuiteResult {
    pub suite: String,
    pub passed: bool,
    pub accepted_count: usize,
    pub best_score: f32,
    pub loops: Vec<LoopRunResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AbuseCaseFixture {
    pub name: String,
    pub surface: RiskSurface,
    pub expected_action: RiskAction,
    pub expected_reason: String,
}

#[derive(Default)]
pub struct BountyBench {
    router: BountyRouter,
}

impl BountyBench {
    pub fn run(&self, fixtures: &[HelpRequestFixture]) -> Result<EvalSuiteResult, EvalError> {
        if fixtures.is_empty() {
            return Err(EvalError::InvalidFixture(
                "at least one fixture is required".to_string(),
            ));
        }

        let requester = Agent::new("eval-requester");
        let capabilities = seeded_capabilities();

        let cases = fixtures
            .iter()
            .map(|fixture| {
                let request = HelpRequest::new(
                    requester.id,
                    fixture.goal.clone(),
                    fixture.context.clone(),
                    Money::new(fixture.budget_minor, fixture.currency.clone()).unwrap(),
                    fixture.privacy.clone(),
                );

                let decision = self.router.route_blocked_goal(&request, &capabilities);
                let mut failures = Vec::new();

                if decision.template_slug.as_deref() != Some(fixture.expected_template.as_str()) {
                    failures.push(format!(
                        "template expected {}, got {:?}",
                        fixture.expected_template, decision.template_slug
                    ));
                }
                if decision.capability_class != fixture.expected_capability_class {
                    failures.push(format!(
                        "capability expected {:?}, got {:?}",
                        fixture.expected_capability_class, decision.capability_class
                    ));
                }
                if decision.verifier_kind != fixture.expected_verifier {
                    failures.push(format!(
                        "verifier expected {:?}, got {:?}",
                        fixture.expected_verifier, decision.verifier_kind
                    ));
                }
                if decision.funding_mode != fixture.expected_funding_mode {
                    failures.push(format!(
                        "funding expected {:?}, got {:?}",
                        fixture.expected_funding_mode, decision.funding_mode
                    ));
                }

                let score = 1.0 - (failures.len() as f32 / 4.0);
                EvalCaseResult {
                    name: fixture.name.clone(),
                    passed: failures.is_empty(),
                    score,
                    failures,
                }
            })
            .collect::<Vec<_>>();

        let score = cases.iter().map(|case| case.score).sum::<f32>() / cases.len() as f32;

        Ok(EvalSuiteResult {
            suite: "BountyBench/router-v0".to_string(),
            score,
            passed: score >= 0.95,
            cases,
        })
    }
}

pub fn bundled_fixtures() -> Vec<HelpRequestFixture> {
    serde_json::from_str(include_str!("../fixtures/help_requests.json"))
        .expect("bundled fixtures must be valid")
}

#[derive(Default)]
pub struct AbuseBench {
    policy: RiskPolicy,
}

impl AbuseBench {
    pub fn run(&self, fixtures: &[AbuseCaseFixture]) -> Result<EvalSuiteResult, EvalError> {
        if fixtures.is_empty() {
            return Err(EvalError::InvalidFixture(
                "at least one abuse fixture is required".to_string(),
            ));
        }

        let cases = fixtures
            .iter()
            .map(|fixture| {
                let assessment = self.assess_fixture(fixture);
                let mut failures = Vec::new();
                if assessment.surface != fixture.surface {
                    failures.push(format!(
                        "surface expected {:?}, got {:?}",
                        fixture.surface, assessment.surface
                    ));
                }
                if assessment.action != fixture.expected_action {
                    failures.push(format!(
                        "action expected {:?}, got {:?}",
                        fixture.expected_action, assessment.action
                    ));
                }
                if !fixture.expected_reason.is_empty() {
                    let reasons = assessment.reasons.join(" ").to_ascii_lowercase();
                    if !reasons.contains(&fixture.expected_reason.to_ascii_lowercase()) {
                        failures.push(format!(
                            "reason expected to contain {}, got {:?}",
                            fixture.expected_reason, assessment.reasons
                        ));
                    }
                }

                let score = 1.0 - (failures.len() as f32 / 3.0);
                EvalCaseResult {
                    name: fixture.name.clone(),
                    passed: failures.is_empty(),
                    score,
                    failures,
                }
            })
            .collect::<Vec<_>>();

        let score = cases.iter().map(|case| case.score).sum::<f32>() / cases.len() as f32;
        Ok(EvalSuiteResult {
            suite: "AbuseBench/risk-v0".to_string(),
            score,
            passed: score >= 0.95,
            cases,
        })
    }

    fn assess_fixture(&self, fixture: &AbuseCaseFixture) -> RiskAssessment {
        match fixture.name.as_str() {
            "non_claim_owner_submission" => {
                let claimed = Uuid::new_v4();
                self.policy.evaluate_submission(&SubmissionRiskInput {
                    bounty_id: Uuid::new_v4(),
                    solver_agent_id: Uuid::new_v4(),
                    claimed_solver_agent_id: Some(claimed),
                    artifact_uri: "s3://bucket/artifact.json".to_string(),
                    artifact_body: "{}".to_string(),
                })
            }
            "high_value_base_bounty" => self.policy.evaluate_bounty(&BountyRiskInput {
                title: "Fix deterministic payout reconciliation failure".to_string(),
                template_slug: "fix-ci-failure".to_string(),
                amount: Money::new(25_000_000, "usdc").unwrap(),
                funding_mode: FundingMode::BaseUsdcEscrow,
                privacy: PrivacyLevel::Public,
            }),
            "unsafe_credential_request" => {
                self.policy.evaluate_help_request(&HelpRequestRiskInput {
                    goal: "Extract credential material from logs".to_string(),
                    context: "Find credential tokens and password dump entries".to_string(),
                    budget: Money::new(1_000_000, "usdc").unwrap(),
                    privacy: PrivacyLevel::Public,
                })
            }
            "normal_extraction_bounty" => self.policy.evaluate_bounty(&BountyRiskInput {
                title: "Extract invoice fields into JSON".to_string(),
                template_slug: "extract-data-to-schema".to_string(),
                amount: Money::new(1_000_000, "usdc").unwrap(),
                funding_mode: FundingMode::BaseUsdcEscrow,
                privacy: PrivacyLevel::Public,
            }),
            _ if fixture.surface == RiskSurface::Payout => {
                self.policy.evaluate_payout(&risk::PayoutRiskInput {
                    bounty_id: Uuid::new_v4(),
                    rail: PaymentRail::BaseUsdc,
                    amount: Money::new(1_000_000, "usdc").unwrap(),
                })
            }
            _ => RiskAssessment::allow(fixture.surface),
        }
    }
}

pub fn bundled_abuse_fixtures() -> Vec<AbuseCaseFixture> {
    serde_json::from_str(include_str!("../fixtures/abuse_cases.json"))
        .expect("bundled abuse fixtures must be valid")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub enum JudgeSurface {
    BountyClarity,
    AcceptanceCriteria,
    SpamFraudRisk,
    ProofPageUsefulness,
    SubmissionQuality,
    TemplateFit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub enum JudgeDecision {
    Pass,
    NeedsRevision,
    NeedsReview,
    Reject,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct JudgeInput {
    pub surface: JudgeSurface,
    pub title: String,
    pub body: String,
    pub template_slug: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct JudgeAssessment {
    pub surface: JudgeSurface,
    pub decision: JudgeDecision,
    pub score: u16,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct JudgeFixture {
    pub name: String,
    pub input: JudgeInput,
    pub expected_decision: JudgeDecision,
    pub expected_reason: String,
}

#[derive(Default)]
pub struct ProductJudge;

impl ProductJudge {
    pub fn assess(&self, input: &JudgeInput) -> JudgeAssessment {
        match input.surface {
            JudgeSurface::BountyClarity => assess_bounty_clarity(input),
            JudgeSurface::AcceptanceCriteria => assess_acceptance_criteria(input),
            JudgeSurface::SpamFraudRisk => assess_spam_fraud_risk(input),
            JudgeSurface::ProofPageUsefulness => assess_proof_page_usefulness(input),
            JudgeSurface::SubmissionQuality => assess_submission_quality(input),
            JudgeSurface::TemplateFit => assess_template_fit(input),
        }
    }
}

#[derive(Default)]
pub struct JudgeBench {
    judge: ProductJudge,
}

impl JudgeBench {
    pub fn run(&self, fixtures: &[JudgeFixture]) -> Result<EvalSuiteResult, EvalError> {
        if fixtures.is_empty() {
            return Err(EvalError::InvalidFixture(
                "at least one judge fixture is required".to_string(),
            ));
        }

        let cases = fixtures
            .iter()
            .map(|fixture| {
                let assessment = self.judge.assess(&fixture.input);
                let mut failures = Vec::new();
                if assessment.surface != fixture.input.surface {
                    failures.push(format!(
                        "surface expected {:?}, got {:?}",
                        fixture.input.surface, assessment.surface
                    ));
                }
                if assessment.decision != fixture.expected_decision {
                    failures.push(format!(
                        "decision expected {:?}, got {:?}",
                        fixture.expected_decision, assessment.decision
                    ));
                }
                if !fixture.expected_reason.is_empty() {
                    let reasons = assessment.reasons.join(" ").to_ascii_lowercase();
                    if !reasons.contains(&fixture.expected_reason.to_ascii_lowercase()) {
                        failures.push(format!(
                            "reason expected to contain {}, got {:?}",
                            fixture.expected_reason, assessment.reasons
                        ));
                    }
                }

                let score = 1.0 - (failures.len() as f32 / 3.0);
                EvalCaseResult {
                    name: fixture.name.clone(),
                    passed: failures.is_empty(),
                    score,
                    failures,
                }
            })
            .collect::<Vec<_>>();

        let score = cases.iter().map(|case| case.score).sum::<f32>() / cases.len() as f32;
        Ok(EvalSuiteResult {
            suite: "JudgeBench/product-quality-v0".to_string(),
            score,
            passed: score >= 0.95,
            cases,
        })
    }
}

pub fn bundled_judge_fixtures() -> Vec<JudgeFixture> {
    serde_json::from_str(include_str!("../fixtures/judge_cases.json"))
        .expect("bundled judge fixtures must be valid")
}

fn assess_bounty_clarity(input: &JudgeInput) -> JudgeAssessment {
    let text = joined_text(input);
    if word_count(&text) < 10 || contains_any(&text, &["do something", "whatever", "help me"]) {
        return assessment(
            input.surface,
            JudgeDecision::NeedsRevision,
            45,
            "unclear goal or task scope",
        );
    }
    if !contains_any(
        &text,
        &[
            "fix", "extract", "verify", "write", "run", "produce", "ship",
        ],
    ) {
        return assessment(
            input.surface,
            JudgeDecision::NeedsRevision,
            60,
            "goal lacks an explicit action",
        );
    }
    assessment(input.surface, JudgeDecision::Pass, 92, "clear bounty scope")
}

fn assess_acceptance_criteria(input: &JudgeInput) -> JudgeAssessment {
    let text = joined_text(input);
    let measurable_terms = [
        "test",
        "schema",
        "proof",
        "link",
        "screenshot",
        "status",
        "ci",
        "json",
        "digest",
        "reviewer",
    ];
    if word_count(&text) < 12 || !contains_any(&text, &measurable_terms) {
        return assessment(
            input.surface,
            JudgeDecision::NeedsRevision,
            50,
            "missing measurable acceptance criteria",
        );
    }
    assessment(
        input.surface,
        JudgeDecision::Pass,
        90,
        "acceptance criteria are measurable",
    )
}

fn assess_spam_fraud_risk(input: &JudgeInput) -> JudgeAssessment {
    let text = joined_text(input);
    if contains_any(
        &text,
        &[
            "credential",
            "password",
            "private key",
            "seed phrase",
            "phishing",
            "fake review",
            "bypass kyc",
        ],
    ) {
        return assessment(
            input.surface,
            JudgeDecision::Reject,
            5,
            "fraud or credential risk",
        );
    }
    if contains_any(&text, &["bulk accounts", "scrape private", "spam"]) {
        return assessment(
            input.surface,
            JudgeDecision::NeedsReview,
            40,
            "potential abuse risk requires review",
        );
    }
    assessment(input.surface, JudgeDecision::Pass, 95, "no abuse signal")
}

fn assess_proof_page_usefulness(input: &JudgeInput) -> JudgeAssessment {
    let text = joined_text(input);
    let required = ["proof", "verifier", "bounty"];
    let missing = required
        .iter()
        .filter(|term| !text.contains(*term))
        .copied()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return assessment(
            input.surface,
            JudgeDecision::NeedsRevision,
            55,
            format!("proof page missing {}", missing.join(",")),
        );
    }
    if !contains_any(&text, &["artifact", "template", "settlement", "status"]) {
        return assessment(
            input.surface,
            JudgeDecision::NeedsRevision,
            65,
            "proof page lacks reusable context",
        );
    }
    assessment(
        input.surface,
        JudgeDecision::Pass,
        91,
        "proof page is useful",
    )
}

fn assess_submission_quality(input: &JudgeInput) -> JudgeAssessment {
    let text = joined_text(input);
    if word_count(&text) < 6 || matches!(text.trim(), "done" | "fixed" | "ok") {
        return assessment(
            input.surface,
            JudgeDecision::NeedsRevision,
            35,
            "submission is too low effort",
        );
    }
    if !contains_any(
        &text,
        &[
            "artifact",
            "pull request",
            "tests pass",
            "digest",
            "output",
            "result",
        ],
    ) {
        return assessment(
            input.surface,
            JudgeDecision::NeedsRevision,
            58,
            "submission missing artifact evidence",
        );
    }
    assessment(
        input.surface,
        JudgeDecision::Pass,
        90,
        "submission has evidence",
    )
}

fn assess_template_fit(input: &JudgeInput) -> JudgeAssessment {
    let Some(template_slug) = &input.template_slug else {
        return assessment(
            input.surface,
            JudgeDecision::NeedsRevision,
            30,
            "template is required",
        );
    };
    let text = joined_text(input);
    let matches = match template_slug.as_str() {
        "fix-ci-failure" => contains_any(&text, &["ci", "test", "build", "workflow"]),
        "small-code-change" => contains_any(&text, &["code", "patch", "bug", "feature"]),
        "extract-data-to-schema" => contains_any(&text, &["extract", "json", "schema", "csv"]),
        "independent-claim-verification" => {
            contains_any(&text, &["verify", "claim", "evidence", "source"])
        }
        "write-docs-for-area" => contains_any(&text, &["docs", "documentation", "readme"]),
        "run-browser-workflow" => contains_any(&text, &["browser", "screenshot", "page", "form"]),
        _ => false,
    };
    if !matches {
        return assessment(
            input.surface,
            JudgeDecision::NeedsRevision,
            50,
            "template does not fit request",
        );
    }
    assessment(
        input.surface,
        JudgeDecision::Pass,
        93,
        "template fits request",
    )
}

fn assessment(
    surface: JudgeSurface,
    decision: JudgeDecision,
    score: u16,
    reason: impl Into<String>,
) -> JudgeAssessment {
    JudgeAssessment {
        surface,
        decision,
        score,
        reasons: vec![reason.into()],
    }
}

fn joined_text(input: &JudgeInput) -> String {
    format!("{} {}", input.title, input.body).to_ascii_lowercase()
}

fn contains_any(text: &str, terms: &[&str]) -> bool {
    terms.iter().any(|term| text.contains(term))
}

fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

fn seeded_capabilities() -> Vec<Capability> {
    let solver = Uuid::new_v4();
    let currency = "usdc";
    vec![
        capability(
            solver,
            CapabilityClass::Coding,
            "small-code-change",
            VerifierKind::GitHubCi,
            currency,
        ),
        capability(
            solver,
            CapabilityClass::Research,
            "primary-source-research",
            VerifierKind::Manual,
            currency,
        ),
        capability(
            solver,
            CapabilityClass::Extraction,
            "extract-data-to-schema",
            VerifierKind::JsonSchema,
            currency,
        ),
        capability(
            solver,
            CapabilityClass::Verification,
            "independent-claim-verification",
            VerifierKind::Manual,
            currency,
        ),
        capability(
            solver,
            CapabilityClass::Documentation,
            "write-docs-for-area",
            VerifierKind::AiJudgeFilter,
            currency,
        ),
        capability(
            solver,
            CapabilityClass::Ci,
            "fix-ci-failure",
            VerifierKind::GitHubCi,
            currency,
        ),
        capability(
            solver,
            CapabilityClass::BrowserWorkflow,
            "run-browser-workflow",
            VerifierKind::DockerCommand,
            currency,
        ),
    ]
}

fn capability(
    agent_id: Uuid,
    class: CapabilityClass,
    template: &str,
    verifier: VerifierKind,
    currency: &str,
) -> Capability {
    Capability {
        id: Uuid::new_v4(),
        agent_id,
        class,
        template_slugs: vec![template.to_string()],
        min_price: Money::new(100, currency).unwrap(),
        max_price: Money::new(100_000, currency).unwrap(),
        latency_seconds: 600,
        supported_verifiers: vec![verifier],
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LoopCandidate {
    pub name: String,
    pub score: f32,
}

pub fn keep_if_improved(baseline: &LoopCandidate, candidate: &LoopCandidate) -> bool {
    candidate.score > baseline.score
}

#[derive(Default)]
pub struct RouterLoop;

impl RouterLoop {
    pub fn run(&self) -> Result<LoopRunResult, EvalError> {
        let suite = BountyBench::default().run(&bundled_fixtures())?;
        Ok(loop_result(
            "RouterLoop",
            EvalLoopKind::Router,
            "router-ci-floor",
            0.94,
            0.95,
            vec![candidate_from_suite("current-router", &suite)],
        ))
    }
}

#[derive(Default)]
pub struct TemplateLoop;

impl TemplateLoop {
    pub fn run(&self) -> Result<LoopRunResult, EvalError> {
        let suite = judge_surface_suite(JudgeSurface::TemplateFit, "TemplateLoop/template-fit-v0")?;
        Ok(loop_result(
            "TemplateLoop",
            EvalLoopKind::Template,
            "template-fit-floor",
            0.89,
            0.90,
            vec![candidate_from_suite("current-template-fit-filter", &suite)],
        ))
    }
}

#[derive(Default)]
pub struct ProofLoop;

impl ProofLoop {
    pub fn run(&self) -> Result<LoopRunResult, EvalError> {
        let suite =
            judge_surface_suite(JudgeSurface::ProofPageUsefulness, "ProofLoop/proof-page-v0")?;
        Ok(loop_result(
            "ProofLoop",
            EvalLoopKind::Proof,
            "proof-page-floor",
            0.89,
            0.90,
            vec![candidate_from_suite("current-proof-page-filter", &suite)],
        ))
    }
}

#[derive(Default)]
pub struct AbuseLoop;

impl AbuseLoop {
    pub fn run(&self) -> Result<LoopRunResult, EvalError> {
        let suite = AbuseBench::default().run(&bundled_abuse_fixtures())?;
        Ok(loop_result(
            "AbuseLoop",
            EvalLoopKind::Abuse,
            "abuse-safety-floor",
            0.94,
            0.95,
            vec![candidate_from_suite("current-risk-policy", &suite)],
        ))
    }
}

#[derive(Default)]
pub struct VerifierLoop;

impl VerifierLoop {
    pub async fn run(&self) -> Result<LoopRunResult, EvalError> {
        let suite = verifier_suite().await?;
        Ok(loop_result(
            "VerifierLoop",
            EvalLoopKind::Verifier,
            "verifier-corpus-floor",
            0.94,
            0.95,
            vec![candidate_from_suite("current-verifier-sdk", &suite)],
        ))
    }
}

pub async fn run_eval_loops() -> Result<LoopSuiteResult, EvalError> {
    let loops = vec![
        RouterLoop.run()?,
        TemplateLoop.run()?,
        VerifierLoop.run().await?,
        ProofLoop.run()?,
        AbuseLoop.run()?,
    ];
    let accepted_count = loops
        .iter()
        .filter(|loop_result| loop_result.accepted_candidate.is_some())
        .count();
    let best_score = loops
        .iter()
        .flat_map(|loop_result| {
            loop_result
                .candidates
                .iter()
                .map(|candidate| candidate.score)
        })
        .fold(0.0_f32, f32::max);
    let passed = loops.iter().all(|loop_result| loop_result.passed);
    Ok(LoopSuiteResult {
        suite: "EvalLoops/all-v0".to_string(),
        passed,
        accepted_count,
        best_score,
        loops,
    })
}

fn judge_surface_suite(
    surface: JudgeSurface,
    suite_name: &str,
) -> Result<EvalSuiteResult, EvalError> {
    let fixtures = bundled_judge_fixtures()
        .into_iter()
        .filter(|fixture| fixture.input.surface == surface)
        .collect::<Vec<_>>();
    let mut result = JudgeBench::default().run(&fixtures)?;
    result.suite = suite_name.to_string();
    Ok(result)
}

fn candidate_from_suite(name: &str, suite: &EvalSuiteResult) -> LoopCandidateResult {
    LoopCandidateResult {
        name: name.to_string(),
        score: suite.score,
        accepted: false,
        failures: suite_failures(suite),
        source_suite: suite.suite.clone(),
    }
}

fn loop_result(
    loop_name: &str,
    kind: EvalLoopKind,
    baseline_name: &str,
    baseline_score: f32,
    gate_threshold: f32,
    candidates: Vec<LoopCandidateResult>,
) -> LoopRunResult {
    let baseline = LoopCandidate {
        name: baseline_name.to_string(),
        score: baseline_score,
    };
    let best_index = candidates
        .iter()
        .enumerate()
        .max_by(|(_, left), (_, right)| left.score.total_cmp(&right.score))
        .map(|(index, _)| index);
    let mut candidates = candidates;
    let accepted_candidate = best_index.and_then(|index| {
        let candidate = &candidates[index];
        let accepted = keep_if_improved(
            &baseline,
            &LoopCandidate {
                name: candidate.name.clone(),
                score: candidate.score,
            },
        ) && candidate.score >= gate_threshold
            && candidate.failures.is_empty();
        candidates[index].accepted = accepted;
        accepted.then(|| candidates[index].name.clone())
    });
    let best_score = best_index
        .map(|index| candidates[index].score)
        .unwrap_or_default();
    let source_suites = candidates
        .iter()
        .map(|candidate| candidate.source_suite.clone())
        .collect::<Vec<_>>();
    LoopRunResult {
        loop_name: loop_name.to_string(),
        kind,
        baseline,
        gate_threshold,
        candidates,
        accepted_candidate,
        score_delta: best_score - baseline_score,
        passed: best_score >= gate_threshold,
        source_suites,
    }
}

fn suite_failures(suite: &EvalSuiteResult) -> Vec<String> {
    suite
        .cases
        .iter()
        .flat_map(|case| {
            case.failures
                .iter()
                .map(|failure| format!("{}: {failure}", case.name))
        })
        .collect()
}

async fn verifier_suite() -> Result<EvalSuiteResult, EvalError> {
    let cases = vec![
        verifier_case(
            "json_schema_accepts_matching_digest",
            VerifierKind::JsonSchema,
            "abc123abc123abc123",
            Some("abc123abc123abc123".to_string()),
            None,
            None,
            VerificationDecision::Accepted,
        )
        .await?,
        verifier_case(
            "json_schema_rejects_digest_mismatch",
            VerifierKind::JsonSchema,
            "abc123abc123abc123",
            Some("mismatch".to_string()),
            None,
            None,
            VerificationDecision::Rejected,
        )
        .await?,
        verifier_case(
            "github_ci_accepts_success",
            VerifierKind::GitHubCi,
            "abc123abc123abc123",
            None,
            None,
            Some(github_ci_evidence(
                "success",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            )),
            VerificationDecision::Accepted,
        )
        .await?,
        verifier_case(
            "github_ci_rejects_failure",
            VerifierKind::GitHubCi,
            "abc123abc123abc123",
            None,
            None,
            Some(github_ci_evidence(
                "failure",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            )),
            VerificationDecision::Rejected,
        )
        .await?,
        verifier_case(
            "docker_accepts_zero_exit_and_digest",
            VerifierKind::DockerCommand,
            "abc123abc123abc123",
            Some("abc123abc123abc123".to_string()),
            None,
            Some(serde_json::json!({ "exit_code": 0 })),
            VerificationDecision::Accepted,
        )
        .await?,
        verifier_case(
            "docker_rejects_nonzero_exit",
            VerifierKind::DockerCommand,
            "abc123abc123abc123",
            Some("abc123abc123abc123".to_string()),
            None,
            Some(serde_json::json!({ "exit_code": 1 })),
            VerificationDecision::Rejected,
        )
        .await?,
        verifier_case(
            "http_callback_accepts_signed_acceptance",
            VerifierKind::HttpCallback,
            "abc123abc123abc123",
            None,
            None,
            Some(serde_json::json!({
                "status_code": 200,
                "decision": "accepted",
                "signature_valid": true
            })),
            VerificationDecision::Accepted,
        )
        .await?,
        verifier_case(
            "http_callback_rejects_unsigned_acceptance",
            VerifierKind::HttpCallback,
            "abc123abc123abc123",
            None,
            None,
            Some(serde_json::json!({
                "status_code": 200,
                "decision": "accepted",
                "signature_valid": false
            })),
            VerificationDecision::Rejected,
        )
        .await?,
        verifier_case(
            "manual_verifier_needs_review",
            VerifierKind::Manual,
            "abc123abc123abc123",
            None,
            None,
            None,
            VerificationDecision::NeedsReview,
        )
        .await?,
        verifier_case(
            "ai_judge_filter_never_accepts_payment",
            VerifierKind::AiJudgeFilter,
            "abc123abc123abc123",
            None,
            Some("Review artifact quality and request operator review.".to_string()),
            None,
            VerificationDecision::NeedsReview,
        )
        .await?,
    ];
    let score = cases.iter().map(|case| case.score).sum::<f32>() / cases.len() as f32;
    Ok(EvalSuiteResult {
        suite: "VerifierLoop/verifier-sdk-v0".to_string(),
        score,
        passed: score >= 0.95,
        cases,
    })
}

async fn verifier_case(
    name: &str,
    kind: VerifierKind,
    artifact_digest: &str,
    expected_artifact_digest: Option<String>,
    rubric: Option<String>,
    evidence: Option<serde_json::Value>,
    expected_decision: VerificationDecision,
) -> Result<EvalCaseResult, EvalError> {
    let bounty_id = Uuid::new_v4();
    let submission = Submission {
        id: Uuid::new_v4(),
        bounty_id,
        solver_agent_id: Uuid::new_v4(),
        artifact_digest: artifact_digest.to_string(),
        artifact_uri: if kind == VerifierKind::GitHubCi {
            "https://github.com/agent-bounties/agent-bounties/pull/42".to_string()
        } else {
            format!("s3://eval-harness/{name}.json")
        },
        submitted_at: Utc::now(),
    };
    let result = verify_with_builtin(
        kind.clone(),
        VerificationInput {
            bounty_id,
            submission,
            expected_artifact_digest,
            rubric,
            evidence,
        },
        None,
    )
    .await
    .map_err(|error| EvalError::Verifier(format!("{name}: {error}")))?;
    let mut failures = Vec::new();
    if result.kind != kind {
        failures.push(format!("kind expected {kind:?}, got {:?}", result.kind));
    }
    if result.decision != expected_decision {
        failures.push(format!(
            "decision expected {expected_decision:?}, got {:?}",
            result.decision
        ));
    }
    if kind == VerifierKind::AiJudgeFilter && result.decision == VerificationDecision::Accepted {
        failures.push("AI judge filter must not authorize settlement".to_string());
    }
    Ok(EvalCaseResult {
        name: name.to_string(),
        passed: failures.is_empty(),
        score: if failures.is_empty() { 1.0 } else { 0.0 },
        failures,
    })
}

fn github_ci_evidence(conclusion: &str, head_sha: &str) -> serde_json::Value {
    serde_json::json!({
        "repository": "agent-bounties/agent-bounties",
        "pull_request_url": "https://github.com/agent-bounties/agent-bounties/pull/42",
        "commit_sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "check_run": {
            "id": 123456789_u64,
            "name": "full-check",
            "status": "completed",
            "conclusion": conclusion,
            "head_sha": head_sha,
            "html_url": "https://github.com/agent-bounties/agent-bounties/actions/runs/123456789",
            "repository": {
                "full_name": "agent-bounties/agent-bounties"
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_bountybench_passes() {
        let result = BountyBench::default().run(&bundled_fixtures()).unwrap();
        assert!(result.passed, "{result:#?}");
    }

    #[test]
    fn bundled_abusebench_passes() {
        let result = AbuseBench::default()
            .run(&bundled_abuse_fixtures())
            .unwrap();
        assert!(result.passed, "{result:#?}");
    }

    #[test]
    fn bundled_judgebench_passes() {
        let result = JudgeBench::default()
            .run(&bundled_judge_fixtures())
            .unwrap();
        assert!(result.passed, "{result:#?}");
    }

    #[test]
    fn product_judge_never_returns_payment_decision() {
        let fixture = bundled_judge_fixtures()
            .into_iter()
            .find(|fixture| fixture.name == "credential_request_rejected")
            .unwrap();
        let assessment = ProductJudge.assess(&fixture.input);

        assert_eq!(assessment.decision, JudgeDecision::Reject);
        assert!(assessment
            .reasons
            .join(" ")
            .to_ascii_lowercase()
            .contains("credential"));
    }

    #[test]
    fn loop_keeps_only_improvements() {
        assert!(keep_if_improved(
            &LoopCandidate {
                name: "baseline".to_string(),
                score: 0.7
            },
            &LoopCandidate {
                name: "candidate".to_string(),
                score: 0.71
            }
        ));
        assert!(!keep_if_improved(
            &LoopCandidate {
                name: "baseline".to_string(),
                score: 0.7
            },
            &LoopCandidate {
                name: "candidate".to_string(),
                score: 0.69
            }
        ));
    }

    #[tokio::test]
    async fn eval_loops_accept_current_candidates() {
        let result = run_eval_loops().await.unwrap();

        assert!(result.passed, "{result:#?}");
        assert_eq!(result.loops.len(), 5);
        assert_eq!(result.accepted_count, 5);
        assert!(result
            .loops
            .iter()
            .any(|loop_result| loop_result.loop_name == "RouterLoop"));
        assert!(result
            .loops
            .iter()
            .any(|loop_result| loop_result.loop_name == "VerifierLoop"));
    }

    #[tokio::test]
    async fn verifier_loop_keeps_ai_judges_out_of_settlement() {
        let result = VerifierLoop.run().await.unwrap();
        let verifier_candidate = result
            .candidates
            .iter()
            .find(|candidate| candidate.name == "current-verifier-sdk")
            .unwrap();

        assert!(result.passed, "{result:#?}");
        assert!(verifier_candidate.failures.is_empty());
        assert_eq!(
            result.accepted_candidate.as_deref(),
            Some("current-verifier-sdk")
        );
    }
}
