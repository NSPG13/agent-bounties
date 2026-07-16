use async_trait::async_trait;
use chrono::Utc;
use domain::{
    AutomaticVerificationPolicy, Id, Submission, VerificationDecision, VerificationMechanism,
    VerifierKind, VerifierResult,
};
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractAttestationEnvelope {
    pub chain_id: u64,
    pub contract_address: String,
    pub bounty_id: Id,
    pub round: u64,
    pub verifier_wallet: String,
    pub submission_hash: String,
    pub evidence_hash: String,
    pub policy_hash: String,
    pub decision: VerificationDecision,
    pub response_hash: String,
    pub deadline_unix: u64,
    pub signature: String,
}

#[derive(Debug, Clone)]
pub struct ContractAttestationScope {
    pub chain_id: u64,
    pub contract_address: String,
    pub bounty_id: Id,
    pub round: u64,
    pub submission_hash: String,
    pub evidence_hash: String,
    pub policy_hash: String,
    pub verifier_set_hash: String,
    pub allowed_verifiers: Vec<String>,
    pub observed_at_unix: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttestationEnvelopeQuorum {
    pub decision: VerificationDecision,
    pub verifier_wallets: Vec<String>,
    pub response_hashes: Vec<String>,
}

/// Validates the deterministic envelope scope before relay. EIP-712/ERC-1271
/// signature validity and settlement authority remain contract responsibilities.
pub fn validate_contract_attestation_envelopes(
    policy: &AutomaticVerificationPolicy,
    scope: &ContractAttestationScope,
    attestations: &[ContractAttestationEnvelope],
) -> VerifierResultType<AttestationEnvelopeQuorum> {
    policy
        .validate()
        .map_err(|error| VerifierError::InvalidInput(error.to_string()))?;
    if policy.mechanism == VerificationMechanism::DeterministicModule {
        return Err(VerifierError::InvalidInput(
            "deterministic module policies do not accept signed quorum envelopes".to_string(),
        ));
    }
    if attestations.len() != usize::from(policy.threshold) {
        return Err(VerifierError::InvalidInput(
            "attestation count does not equal the committed threshold".to_string(),
        ));
    }
    if scope.allowed_verifiers.len() != usize::from(policy.verifier_count)
        || policy.verifier_set_hash.as_deref() != Some(scope.verifier_set_hash.as_str())
        || !is_evm_address(&scope.contract_address)
        || !is_bytes32_hash(&scope.submission_hash)
        || !is_bytes32_hash(&scope.evidence_hash)
        || !is_bytes32_hash(&scope.policy_hash)
    {
        return Err(VerifierError::InvalidInput(
            "attestation scope does not match the committed contract policy".to_string(),
        ));
    }
    let expected_decision = attestations
        .first()
        .map(|attestation| attestation.decision.clone())
        .ok_or_else(|| VerifierError::InvalidInput("attestation quorum is empty".to_string()))?;
    if expected_decision == VerificationDecision::NeedsReview {
        return Err(VerifierError::InvalidInput(
            "needs-review attestations cannot settle a bounty".to_string(),
        ));
    }

    let allowed_verifiers = scope
        .allowed_verifiers
        .iter()
        .map(|value| value.to_ascii_lowercase())
        .collect::<Vec<_>>();
    let mut verifier_wallets = Vec::with_capacity(attestations.len());
    let mut response_hashes = Vec::with_capacity(attestations.len());
    for attestation in attestations {
        let verifier = attestation.verifier_wallet.to_ascii_lowercase();
        if attestation.chain_id != scope.chain_id
            || !attestation
                .contract_address
                .eq_ignore_ascii_case(&scope.contract_address)
            || attestation.bounty_id != scope.bounty_id
            || attestation.round != scope.round
            || !attestation
                .submission_hash
                .eq_ignore_ascii_case(&scope.submission_hash)
            || !attestation
                .evidence_hash
                .eq_ignore_ascii_case(&scope.evidence_hash)
            || !attestation
                .policy_hash
                .eq_ignore_ascii_case(&scope.policy_hash)
            || attestation.decision != expected_decision
            || attestation.deadline_unix < scope.observed_at_unix
            || !is_evm_address(&attestation.verifier_wallet)
            || !is_bytes32_hash(&attestation.response_hash)
            || !is_hex_bytes(&attestation.signature)
        {
            return Err(VerifierError::InvalidInput(
                "attestation is expired, malformed, or bound to a different bounty scope"
                    .to_string(),
            ));
        }
        if !allowed_verifiers.contains(&verifier) {
            return Err(VerifierError::InvalidInput(
                "attestation signer is outside the committed verifier set".to_string(),
            ));
        }
        if verifier_wallets.contains(&verifier) {
            return Err(VerifierError::InvalidInput(
                "duplicate verifier cannot satisfy quorum".to_string(),
            ));
        }
        verifier_wallets.push(verifier);
        response_hashes.push(attestation.response_hash.to_ascii_lowercase());
    }

    Ok(AttestationEnvelopeQuorum {
        decision: expected_decision,
        verifier_wallets,
        response_hashes,
    })
}

fn is_bytes32_hash(value: &str) -> bool {
    value.len() == 66
        && value.starts_with("0x")
        && value[2..].bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn is_evm_address(value: &str) -> bool {
    value.len() == 42
        && value.starts_with("0x")
        && value[2..].bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn is_hex_bytes(value: &str) -> bool {
    value.len() > 2
        && value.len().is_multiple_of(2)
        && value.starts_with("0x")
        && value[2..].bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[async_trait]
pub trait Verifier: Send + Sync {
    fn kind(&self) -> VerifierKind;
    async fn verify(&self, input: VerificationInput) -> VerifierResultType<VerifierResult>;
}

pub const REGRESSION_SANDBOX_POLICY_VERSION: &str = "agent-bounties/regression-sandbox-v1";
pub const REGRESSION_SANDBOX_RECEIPT_VERSION: &str = "agent-bounties/regression-sandbox-receipt-v1";
const MIB: u64 = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegressionSandboxPolicy {
    pub schema_version: String,
    pub image: String,
    pub command: Vec<String>,
    pub workdir: String,
    pub benchmark_digest: String,
    pub timeout_seconds: u64,
    pub cpu_millis: u32,
    pub memory_bytes: u64,
    pub pids_limit: u32,
    pub max_output_bytes: u64,
    pub tmpfs_bytes: u64,
    pub max_source_bytes: u64,
    pub max_source_files: u32,
    pub max_benchmark_bytes: u64,
    pub max_benchmark_files: u32,
    pub platform: String,
    pub test_seed: u64,
}

impl RegressionSandboxPolicy {
    pub fn validate(&self) -> VerifierResultType<()> {
        if self.schema_version != REGRESSION_SANDBOX_POLICY_VERSION {
            return Err(VerifierError::InvalidInput(
                "unsupported regression sandbox policy version".to_string(),
            ));
        }
        validate_pinned_image(&self.image)?;
        validate_sha256_digest("benchmark_digest", &self.benchmark_digest)?;
        if self.workdir != "/workspace" {
            return Err(VerifierError::InvalidInput(
                "regression sandbox workdir must be /workspace".to_string(),
            ));
        }
        if self.command.is_empty() || self.command.len() > 64 {
            return Err(VerifierError::InvalidInput(
                "regression command must contain 1 to 64 argv entries".to_string(),
            ));
        }
        let mut command_bytes = 0usize;
        for argument in &self.command {
            if argument.is_empty()
                || argument.len() > 4_096
                || argument.contains('\0')
                || argument.contains('\n')
                || argument.contains('\r')
            {
                return Err(VerifierError::InvalidInput(
                    "regression command contains an empty, oversized, or control-bearing argv entry"
                        .to_string(),
                ));
            }
            command_bytes = command_bytes.saturating_add(argument.len());
        }
        if command_bytes > 16_384 {
            return Err(VerifierError::InvalidInput(
                "regression command exceeds the argv byte limit".to_string(),
            ));
        }
        let executable = self.command[0]
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        if matches!(
            executable.as_str(),
            "sh" | "bash" | "dash" | "zsh" | "cmd" | "cmd.exe" | "powershell" | "pwsh"
        ) {
            return Err(VerifierError::InvalidInput(
                "regression sandbox v1 requires direct argv and forbids shell entrypoints"
                    .to_string(),
            ));
        }
        if !(1..=900).contains(&self.timeout_seconds)
            || !(100..=4_000).contains(&self.cpu_millis)
            || !(64 * MIB..=4 * 1024 * MIB).contains(&self.memory_bytes)
            || !(16..=512).contains(&self.pids_limit)
            || !(1_024..=16 * MIB).contains(&self.max_output_bytes)
            || !(64 * MIB..=4 * 1024 * MIB).contains(&self.tmpfs_bytes)
            || self.tmpfs_bytes > self.memory_bytes
            || !(1..=2 * 1024 * MIB).contains(&self.max_source_bytes)
            || !(1..=100_000).contains(&self.max_source_files)
            || !(1..=512 * MIB).contains(&self.max_benchmark_bytes)
            || !(1..=50_000).contains(&self.max_benchmark_files)
        {
            return Err(VerifierError::InvalidInput(
                "regression sandbox resource limits are outside protocol bounds".to_string(),
            ));
        }
        if !matches!(self.platform.as_str(), "linux/amd64" | "linux/arm64") {
            return Err(VerifierError::InvalidInput(
                "regression sandbox platform must be linux/amd64 or linux/arm64".to_string(),
            ));
        }
        Ok(())
    }

    pub fn runner_manifest_hash(&self) -> VerifierResultType<String> {
        self.validate()?;
        canonical_sha256_digest(self)
    }

    pub fn command_hash(&self) -> VerifierResultType<String> {
        self.validate()?;
        canonical_sha256_digest(&self.command)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegressionSandboxExecution {
    pub exit_code: i32,
    pub stdout_sha256: String,
    pub stderr_sha256: String,
    pub stdout_bytes: u64,
    pub stderr_bytes: u64,
    pub isolation: RegressionSandboxIsolation,
}

impl RegressionSandboxExecution {
    fn validate(&self, policy: &RegressionSandboxPolicy) -> VerifierResultType<()> {
        validate_sha256_digest("stdout_sha256", &self.stdout_sha256)?;
        validate_sha256_digest("stderr_sha256", &self.stderr_sha256)?;
        if !(0..=124).contains(&self.exit_code)
            || self
                .stdout_bytes
                .checked_add(self.stderr_bytes)
                .is_none_or(|bytes| bytes > policy.max_output_bytes)
        {
            return Err(VerifierError::Failed(
                "sandbox reported a runtime-reserved exit or output above the committed limit; no verdict was produced"
                    .to_string(),
            ));
        }
        self.isolation.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegressionSandboxIsolation {
    pub runtime: String,
    pub network: String,
    pub rootfs_read_only: bool,
    pub source_read_only: bool,
    pub benchmark_read_only: bool,
    pub capabilities: String,
    pub no_new_privileges: bool,
    pub user: String,
}

impl Default for RegressionSandboxIsolation {
    fn default() -> Self {
        Self {
            runtime: "docker".to_string(),
            network: "none".to_string(),
            rootfs_read_only: true,
            source_read_only: true,
            benchmark_read_only: true,
            capabilities: "none".to_string(),
            no_new_privileges: true,
            user: "65532:65532".to_string(),
        }
    }
}

impl RegressionSandboxIsolation {
    fn validate(&self) -> VerifierResultType<()> {
        if self.runtime != "docker"
            || self.network != "none"
            || !self.rootfs_read_only
            || !self.source_read_only
            || !self.benchmark_read_only
            || self.capabilities != "none"
            || !self.no_new_privileges
            || self.user != "65532:65532"
        {
            return Err(VerifierError::Failed(
                "sandbox isolation evidence does not satisfy regression-sandbox-v1; no verdict was produced"
                    .to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegressionVerificationScope {
    pub network: String,
    pub bounty_id: String,
    pub bounty_contract: String,
    pub round: u64,
    pub solver_wallet: String,
    pub submission_hash: String,
    pub evidence_hash: String,
    pub terms_hash: String,
    pub committed_policy_hash: String,
    pub verification_expires_at: u64,
}

impl RegressionVerificationScope {
    pub fn validate(&self) -> VerifierResultType<()> {
        if self.network.is_empty()
            || self.network.len() > 64
            || self
                .network
                .bytes()
                .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control())
        {
            return Err(VerifierError::InvalidInput(
                "regression verification network is invalid".to_string(),
            ));
        }
        for (field, value) in [
            ("bounty_id", &self.bounty_id),
            ("submission_hash", &self.submission_hash),
            ("evidence_hash", &self.evidence_hash),
            ("terms_hash", &self.terms_hash),
            ("committed_policy_hash", &self.committed_policy_hash),
        ] {
            if !is_bytes32_hash(value) {
                return Err(VerifierError::InvalidInput(format!(
                    "regression verification {field} must be a 0x-prefixed bytes32 value"
                )));
            }
        }
        if !is_evm_address(&self.bounty_contract) || !is_evm_address(&self.solver_wallet) {
            return Err(VerifierError::InvalidInput(
                "regression verification contract and solver must be EVM addresses".to_string(),
            ));
        }
        if self.round == 0 || self.verification_expires_at == 0 {
            return Err(VerifierError::InvalidInput(
                "regression verification round and expiry must be positive".to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegressionVerificationTask {
    pub scope: RegressionVerificationScope,
    pub source_digest: String,
}

impl RegressionVerificationTask {
    pub fn validate(&self) -> VerifierResultType<()> {
        self.scope.validate()?;
        validate_sha256_digest("source_digest", &self.source_digest)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegressionVerdict {
    Passed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegressionSandboxReceipt {
    pub schema_version: String,
    pub scope: RegressionVerificationScope,
    pub runner_manifest_hash: String,
    pub command_hash: String,
    pub source_digest: String,
    pub benchmark_digest: String,
    pub image: String,
    pub execution: RegressionSandboxExecution,
}

#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum RegressionSandboxExecutorError {
    #[error("sandbox runtime is unavailable")]
    RuntimeUnavailable,
    #[error("sandbox input is unavailable or does not match its committed digest")]
    InputUnavailable,
    #[error("sandbox exceeded its committed timeout")]
    TimedOut,
    #[error("sandbox exceeded its committed output limit")]
    OutputLimitExceeded,
    #[error("sandbox was killed by a resource or runtime limit")]
    ResourceLimitExceeded,
    #[error("sandbox execution failed closed")]
    FailedClosed,
}

#[async_trait]
pub trait RegressionSandboxExecutor: Send + Sync {
    async fn execute(
        &self,
        policy: &RegressionSandboxPolicy,
        source_digest: &str,
    ) -> Result<RegressionSandboxExecution, RegressionSandboxExecutorError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegressionVerificationOutcome {
    pub verdict: RegressionVerdict,
    pub receipt: RegressionSandboxReceipt,
    pub response_hash: String,
}

pub fn validate_regression_outcome(
    policy: &RegressionSandboxPolicy,
    task: &RegressionVerificationTask,
    outcome: &RegressionVerificationOutcome,
) -> VerifierResultType<()> {
    policy.validate()?;
    task.validate()?;
    outcome.receipt.execution.validate(policy)?;
    if outcome.receipt.schema_version != REGRESSION_SANDBOX_RECEIPT_VERSION
        || outcome.receipt.scope != task.scope
        || outcome.receipt.runner_manifest_hash != policy.runner_manifest_hash()?
        || outcome.receipt.command_hash != policy.command_hash()?
        || outcome.receipt.source_digest != task.source_digest
        || outcome.receipt.benchmark_digest != policy.benchmark_digest
        || outcome.receipt.image != policy.image
    {
        return Err(VerifierError::Failed(
            "regression receipt differs from the immutable task or runner policy; no attestation is allowed"
                .to_string(),
        ));
    }
    let expected_verdict = if outcome.receipt.execution.exit_code == 0 {
        RegressionVerdict::Passed
    } else {
        RegressionVerdict::Failed
    };
    if outcome.verdict != expected_verdict {
        return Err(VerifierError::Failed(
            "regression verdict differs from the completed process exit; no attestation is allowed"
                .to_string(),
        ));
    }
    #[derive(Serialize)]
    struct ResponsePreimage<'a> {
        schema_version: &'static str,
        verdict: RegressionVerdict,
        receipt: &'a RegressionSandboxReceipt,
    }
    let expected_response_hash = canonical_sha256_bytes32(&ResponsePreimage {
        schema_version: REGRESSION_SANDBOX_RECEIPT_VERSION,
        verdict: outcome.verdict,
        receipt: &outcome.receipt,
    })?;
    if !outcome
        .response_hash
        .eq_ignore_ascii_case(&expected_response_hash)
    {
        return Err(VerifierError::Failed(
            "regression response hash does not commit the exact receipt; no attestation is allowed"
                .to_string(),
        ));
    }
    Ok(())
}

pub struct SandboxedRegressionVerifier<E> {
    pub executor: E,
    pub policy: RegressionSandboxPolicy,
}

impl<E: RegressionSandboxExecutor> SandboxedRegressionVerifier<E> {
    pub async fn verify(
        &self,
        task: RegressionVerificationTask,
    ) -> VerifierResultType<RegressionVerificationOutcome> {
        self.policy.validate()?;
        task.validate()?;

        let execution = self
            .executor
            .execute(&self.policy, &task.source_digest)
            .await
            .map_err(|error| VerifierError::Failed(format!("{error}; no verdict was produced")))?;
        execution.validate(&self.policy)?;
        let receipt = RegressionSandboxReceipt {
            schema_version: REGRESSION_SANDBOX_RECEIPT_VERSION.to_string(),
            scope: task.scope.clone(),
            runner_manifest_hash: self.policy.runner_manifest_hash()?,
            command_hash: self.policy.command_hash()?,
            source_digest: task.source_digest.clone(),
            benchmark_digest: self.policy.benchmark_digest.clone(),
            image: self.policy.image.clone(),
            execution,
        };
        let verdict = if receipt.execution.exit_code == 0 {
            RegressionVerdict::Passed
        } else {
            RegressionVerdict::Failed
        };
        let mut outcome = RegressionVerificationOutcome {
            verdict,
            receipt,
            response_hash: String::new(),
        };
        #[derive(Serialize)]
        struct ResponsePreimage<'a> {
            schema_version: &'static str,
            verdict: RegressionVerdict,
            receipt: &'a RegressionSandboxReceipt,
        }
        outcome.response_hash = canonical_sha256_bytes32(&ResponsePreimage {
            schema_version: REGRESSION_SANDBOX_RECEIPT_VERSION,
            verdict: outcome.verdict,
            receipt: &outcome.receipt,
        })?;
        validate_regression_outcome(&self.policy, &task, &outcome)?;
        Ok(outcome)
    }
}

fn validate_pinned_image(image: &str) -> VerifierResultType<()> {
    if image.starts_with('-') || image.matches('@').count() != 1 {
        return Err(VerifierError::InvalidInput(
            "regression image must be a non-option OCI reference pinned by one sha256 digest"
                .to_string(),
        ));
    }
    let Some((name, digest)) = image.split_once("@sha256:") else {
        return Err(VerifierError::InvalidInput(
            "regression image must be pinned by sha256 digest".to_string(),
        ));
    };
    if name.is_empty()
        || name.bytes().any(|byte| {
            !matches!(
                byte,
                b'a'..=b'z'
                    | b'0'..=b'9'
                    | b'.'
                    | b'/'
                    | b':'
                    | b'_'
                    | b'-'
            )
        })
        || name.contains("..")
        || digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(VerifierError::InvalidInput(
            "regression image has an invalid immutable digest".to_string(),
        ));
    }
    Ok(())
}

fn validate_sha256_digest(field: &str, value: &str) -> VerifierResultType<()> {
    let Some(digest) = value.strip_prefix("sha256:") else {
        return Err(VerifierError::InvalidInput(format!(
            "{field} must use sha256:<64 lowercase hex>"
        )));
    };
    if digest.len() != 64
        || !digest.bytes().all(|byte| byte.is_ascii_hexdigit())
        || digest.bytes().any(|byte| byte.is_ascii_uppercase())
    {
        return Err(VerifierError::InvalidInput(format!(
            "{field} must use sha256:<64 lowercase hex>"
        )));
    }
    Ok(())
}

fn canonical_sha256_digest<T: Serialize>(value: &T) -> VerifierResultType<String> {
    let encoded = serde_json::to_vec(value).map_err(|_| {
        VerifierError::Failed("failed to encode canonical verifier evidence".to_string())
    })?;
    let mut hasher = Sha256::new();
    hasher.update(encoded);
    Ok(format!("sha256:{}", hex::encode(hasher.finalize())))
}

fn canonical_sha256_bytes32<T: Serialize>(value: &T) -> VerifierResultType<String> {
    let encoded = serde_json::to_vec(value).map_err(|_| {
        VerifierError::Failed("failed to encode canonical verifier evidence".to_string())
    })?;
    Ok(format!("0x{}", hex::encode(Sha256::digest(encoded))))
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
        let (summary, payload) = match input.evidence.as_ref() {
            None => (
                "caller-supplied GitHub CI JSON has no authenticated provenance and cannot authorize acceptance or rejection; structured evidence is also missing".to_string(),
                None,
            ),
            Some(evidence) => match GitHubCiEvidence::from_value(evidence, &input.submission) {
                Err(reason) => (
                    format!(
                        "caller-supplied GitHub CI JSON has no authenticated provenance and cannot authorize acceptance or rejection; advisory parse failed: {reason}"
                    ),
                    Some(evidence.to_string()),
                ),
                Ok(parsed) => {
                    let ownership_reason = parsed.validate_ownership(&input.submission).err();
                    let acceptance_reason = parsed.automatic_acceptance_review_reason();
                    let advisory = ownership_reason
                        .or(acceptance_reason)
                        .unwrap_or_else(|| "the untrusted fields are internally consistent".to_string());
                    let pr = parsed
                        .pull_request_number()
                        .map(|number| format!("PR #{number}"))
                        .unwrap_or_else(|| "repository evidence".to_string());
                    (
                        format!(
                            "caller-supplied GitHub CI JSON has no authenticated provenance and cannot authorize acceptance or rejection; parsed {} {} commit {} check {}#{}; advisory: {}",
                            parsed.repository,
                            pr,
                            short_sha(&parsed.commit_sha),
                            parsed.check_name,
                            parsed.check_run_id,
                            advisory
                        ),
                        Some(parsed.canonical_payload()),
                    )
                }
            },
        };
        Ok(make_result_with_payload(ResultSeed {
            bounty_id: input.bounty_id,
            submission_id: input.submission.id,
            verifier_agent_id: None,
            kind: VerifierKind::GitHubCi,
            decision: VerificationDecision::NeedsReview,
            summary,
            confidence: 0.0,
            payload: payload.as_deref(),
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
        Ok(make_result(
            input.bounty_id,
            input.submission.id,
            None,
            VerifierKind::DockerCommand,
            VerificationDecision::NeedsReview,
            "self-reported Docker exit evidence cannot authorize acceptance; run the committed policy through SandboxedRegressionVerifier",
            0.0,
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
    async fn github_ci_verifier_needs_authenticated_success_evidence() {
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
        assert_eq!(result.decision, VerificationDecision::NeedsReview);
        assert!(result.summary.contains("authenticated provenance"));
        assert_eq!(result.signed_payload_hash.len(), 64);
    }

    #[tokio::test]
    async fn github_ci_verifier_does_not_trust_mismatched_commit_json() {
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

        assert_eq!(result.decision, VerificationDecision::NeedsReview);
        assert!(result.summary.contains("authenticated provenance"));
    }

    #[tokio::test]
    async fn github_ci_verifier_does_not_trust_failure_json() {
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

        assert_eq!(result.decision, VerificationDecision::NeedsReview);
        assert!(result.summary.contains("authenticated provenance"));
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
        assert!(result.summary.contains("authenticated provenance"));
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
        assert!(result.summary.contains("authenticated provenance"));
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
        assert!(result.summary.contains("authenticated provenance"));
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
        assert!(result.summary.contains("authenticated provenance"));
    }

    #[tokio::test]
    async fn github_ci_verifier_does_not_trust_replayed_pr_json() {
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

        assert_eq!(result.decision, VerificationDecision::NeedsReview);
        assert!(result.summary.contains("authenticated provenance"));
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

        assert_eq!(first.decision, VerificationDecision::NeedsReview);
        assert_eq!(second.decision, VerificationDecision::NeedsReview);
        assert_ne!(first.signed_payload_hash, second.signed_payload_hash);
    }

    #[tokio::test]
    async fn self_reported_docker_exit_code_cannot_authorize_acceptance() {
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
        assert_eq!(result.decision, VerificationDecision::NeedsReview);
        assert!(result.summary.contains("self-reported"));
    }

    #[test]
    fn regression_policy_requires_immutable_direct_execution() {
        let policy = regression_policy();
        policy.validate().expect("valid regression policy");
        assert_eq!(
            policy.runner_manifest_hash().unwrap(),
            policy.runner_manifest_hash().unwrap()
        );
        assert_eq!(
            policy.command_hash().unwrap(),
            policy.command_hash().unwrap()
        );

        let mut mutable_image = policy.clone();
        mutable_image.image = "ghcr.io/agent-bounties/rust-verifier:latest".to_string();
        assert!(mutable_image.validate().is_err());

        let mut option_injection = policy.clone();
        option_injection.image = format!("--volume@sha256:{}", "b".repeat(64));
        assert!(option_injection.validate().is_err());

        let mut uppercase = policy.clone();
        uppercase.image = format!("GHCR.IO/owner/image@sha256:{}", "b".repeat(64));
        assert!(uppercase.validate().is_err());

        let mut shell = policy;
        shell.command = vec!["sh".to_string(), "-c".to_string(), "cargo test".to_string()];
        assert!(shell.validate().is_err());
    }

    #[tokio::test]
    async fn sandboxed_regression_accepts_only_runner_execution_and_hashes_receipt() {
        let verifier = SandboxedRegressionVerifier {
            executor: StubRegressionExecutor::completed(0),
            policy: regression_policy(),
        };
        let task = regression_task();

        let first = verifier.verify(task.clone()).await.unwrap();
        let second = verifier.verify(task).await.unwrap();

        assert_eq!(first.verdict, RegressionVerdict::Passed);
        assert_eq!(first.receipt.execution.exit_code, 0);
        assert_eq!(first.response_hash, second.response_hash);
        assert!(first.response_hash.starts_with("0x"));
        assert_eq!(first.response_hash.len(), 66);
        assert_eq!(first.receipt.scope, regression_scope());
    }

    #[tokio::test]
    async fn regression_candidate_validation_rejects_scope_verdict_and_hash_mutation() {
        let policy = regression_policy();
        let task = regression_task();
        let outcome = SandboxedRegressionVerifier {
            executor: StubRegressionExecutor::completed(0),
            policy: policy.clone(),
        }
        .verify(task.clone())
        .await
        .unwrap();
        validate_regression_outcome(&policy, &task, &outcome).unwrap();

        let mut changed_scope = outcome.clone();
        changed_scope.receipt.scope.round += 1;
        assert!(validate_regression_outcome(&policy, &task, &changed_scope).is_err());

        let mut changed_verdict = outcome.clone();
        changed_verdict.verdict = RegressionVerdict::Failed;
        assert!(validate_regression_outcome(&policy, &task, &changed_verdict).is_err());

        let mut changed_hash = outcome;
        changed_hash.response_hash = hash("a");
        assert!(validate_regression_outcome(&policy, &task, &changed_hash).is_err());
    }

    #[tokio::test]
    async fn sandboxed_regression_emits_failure_only_for_completed_nonzero_exit() {
        let failed = SandboxedRegressionVerifier {
            executor: StubRegressionExecutor::completed(1),
            policy: regression_policy(),
        }
        .verify(regression_task())
        .await
        .unwrap();
        assert_eq!(failed.verdict, RegressionVerdict::Failed);

        let timed_out = SandboxedRegressionVerifier {
            executor: StubRegressionExecutor {
                outcome: Err(RegressionSandboxExecutorError::TimedOut),
            },
            policy: regression_policy(),
        }
        .verify(regression_task())
        .await
        .unwrap_err();
        assert!(timed_out.to_string().contains("no verdict was produced"));

        let reserved_exit = SandboxedRegressionVerifier {
            executor: StubRegressionExecutor::completed(137),
            policy: regression_policy(),
        }
        .verify(regression_task())
        .await
        .unwrap_err();
        assert!(reserved_exit
            .to_string()
            .contains("no verdict was produced"));

        let mut combined_output = StubRegressionExecutor::completed(0);
        let execution = combined_output.outcome.as_mut().unwrap();
        execution.stdout_bytes = 700;
        execution.stderr_bytes = 700;
        let mut bounded_policy = regression_policy();
        bounded_policy.max_output_bytes = 1_024;
        let bounded_output = SandboxedRegressionVerifier {
            executor: combined_output,
            policy: bounded_policy,
        }
        .verify(regression_task())
        .await
        .unwrap_err();
        assert!(bounded_output
            .to_string()
            .contains("no verdict was produced"));
    }

    #[tokio::test]
    async fn sandbox_runner_failure_produces_no_verdict() {
        let verifier = SandboxedRegressionVerifier {
            executor: StubRegressionExecutor {
                outcome: Err(RegressionSandboxExecutorError::RuntimeUnavailable),
            },
            policy: regression_policy(),
        };
        let error = verifier.verify(regression_task()).await.unwrap_err();

        assert!(error.to_string().contains("no verdict was produced"));
    }

    #[tokio::test]
    async fn sandboxed_regression_rejects_malformed_payment_scope_before_execution() {
        let verifier = SandboxedRegressionVerifier {
            executor: StubRegressionExecutor::completed(0),
            policy: regression_policy(),
        };
        let mut task = regression_task();
        task.scope.submission_hash = hash("f");
        task.scope.bounty_contract = "not-an-address".to_string();
        assert!(verifier.verify(task).await.is_err());
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

    #[test]
    fn ai_quorum_envelopes_must_match_exact_contract_scope() {
        let bounty_id = Uuid::new_v4();
        let policy = ai_quorum_policy();
        let scope = attestation_scope(bounty_id);
        let attestations = vec![
            attestation(bounty_id, &scope.allowed_verifiers[0], "6"),
            attestation(bounty_id, &scope.allowed_verifiers[1], "7"),
        ];

        let quorum = validate_contract_attestation_envelopes(&policy, &scope, &attestations)
            .expect("valid quorum envelope");

        assert_eq!(quorum.decision, VerificationDecision::Accepted);
        assert_eq!(quorum.verifier_wallets.len(), 2);
    }

    #[test]
    fn duplicate_or_cross_bounty_attestations_cannot_satisfy_quorum() {
        let bounty_id = Uuid::new_v4();
        let policy = ai_quorum_policy();
        let scope = attestation_scope(bounty_id);
        let duplicate = attestation(bounty_id, &scope.allowed_verifiers[0], "6");
        let error = validate_contract_attestation_envelopes(
            &policy,
            &scope,
            &[duplicate.clone(), duplicate],
        )
        .unwrap_err();
        assert!(error.to_string().contains("duplicate verifier"));

        let mut wrong_bounty = attestation(Uuid::new_v4(), &scope.allowed_verifiers[1], "7");
        wrong_bounty.bounty_id = Uuid::new_v4();
        let error = validate_contract_attestation_envelopes(
            &policy,
            &scope,
            &[
                attestation(bounty_id, &scope.allowed_verifiers[0], "6"),
                wrong_bounty,
            ],
        )
        .unwrap_err();
        assert!(error.to_string().contains("different bounty scope"));
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

    #[derive(Clone)]
    struct StubRegressionExecutor {
        outcome: Result<RegressionSandboxExecution, RegressionSandboxExecutorError>,
    }

    impl StubRegressionExecutor {
        fn completed(exit_code: i32) -> Self {
            Self {
                outcome: Ok(RegressionSandboxExecution {
                    exit_code,
                    stdout_sha256: sha256_digest('d'),
                    stderr_sha256: sha256_digest('e'),
                    stdout_bytes: 12,
                    stderr_bytes: 0,
                    isolation: RegressionSandboxIsolation::default(),
                }),
            }
        }
    }

    #[async_trait]
    impl RegressionSandboxExecutor for StubRegressionExecutor {
        async fn execute(
            &self,
            _policy: &RegressionSandboxPolicy,
            _source_digest: &str,
        ) -> Result<RegressionSandboxExecution, RegressionSandboxExecutorError> {
            self.outcome.clone()
        }
    }

    fn regression_policy() -> RegressionSandboxPolicy {
        RegressionSandboxPolicy {
            schema_version: REGRESSION_SANDBOX_POLICY_VERSION.to_string(),
            image: format!(
                "ghcr.io/agent-bounties/rust-verifier@sha256:{}",
                "b".repeat(64)
            ),
            command: vec![
                "cargo".to_string(),
                "test".to_string(),
                "--locked".to_string(),
                "--target-dir".to_string(),
                "/tmp/target".to_string(),
            ],
            workdir: "/workspace".to_string(),
            benchmark_digest: sha256_digest('a'),
            timeout_seconds: 120,
            cpu_millis: 1_000,
            memory_bytes: 512 * MIB,
            pids_limit: 128,
            max_output_bytes: MIB,
            tmpfs_bytes: 256 * MIB,
            max_source_bytes: 512 * MIB,
            max_source_files: 50_000,
            max_benchmark_bytes: 64 * MIB,
            max_benchmark_files: 10_000,
            platform: "linux/amd64".to_string(),
            test_seed: 1,
        }
    }

    fn regression_task() -> RegressionVerificationTask {
        RegressionVerificationTask {
            scope: regression_scope(),
            source_digest: sha256_digest('c'),
        }
    }

    fn regression_scope() -> RegressionVerificationScope {
        RegressionVerificationScope {
            network: "base-mainnet".to_string(),
            bounty_id: hash("a"),
            bounty_contract: address("2"),
            round: 3,
            solver_wallet: address("3"),
            submission_hash: hash("4"),
            evidence_hash: hash("5"),
            terms_hash: hash("6"),
            committed_policy_hash: hash("7"),
            verification_expires_at: 2_000_000_000,
        }
    }

    fn sha256_digest(character: char) -> String {
        format!("sha256:{}", character.to_string().repeat(64))
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

    fn ai_quorum_policy() -> AutomaticVerificationPolicy {
        AutomaticVerificationPolicy {
            protocol_version: domain::AUTONOMOUS_BOUNTY_PROTOCOL_VERSION.to_string(),
            mechanism: VerificationMechanism::AiJudgeQuorum,
            engine: domain::VerificationEngine::AiJudge,
            terms_hash: hash("1"),
            policy_hash: hash("2"),
            acceptance_criteria_hash: hash("3"),
            benchmark_hash: hash("a"),
            evidence_schema_hash: hash("4"),
            verifier_set_hash: Some(hash("5")),
            verifier_count: 3,
            threshold: 2,
            max_automatic_payout: domain::Money::new(1_000_000, "usdc").unwrap(),
            ai_judge: Some(domain::AiJudgePolicyCommitment {
                provider: "provider".to_string(),
                model: "judge".to_string(),
                model_version: "2026-07-10".to_string(),
                system_prompt_hash: hash("8"),
                rubric_hash: hash("9"),
                benchmark_hash: hash("a"),
                decoding_parameters_hash: hash("b"),
            }),
        }
    }

    fn attestation_scope(bounty_id: Id) -> ContractAttestationScope {
        ContractAttestationScope {
            chain_id: 8453,
            contract_address: address("1"),
            bounty_id,
            round: 1,
            submission_hash: hash("c"),
            evidence_hash: hash("d"),
            policy_hash: hash("2"),
            verifier_set_hash: hash("5"),
            allowed_verifiers: vec![address("2"), address("3"), address("4")],
            observed_at_unix: 100,
        }
    }

    fn attestation(
        bounty_id: Id,
        verifier_wallet: &str,
        response_character: &str,
    ) -> ContractAttestationEnvelope {
        ContractAttestationEnvelope {
            chain_id: 8453,
            contract_address: address("1"),
            bounty_id,
            round: 1,
            verifier_wallet: verifier_wallet.to_string(),
            submission_hash: hash("c"),
            evidence_hash: hash("d"),
            policy_hash: hash("2"),
            decision: VerificationDecision::Accepted,
            response_hash: hash(response_character),
            deadline_unix: 200,
            signature: "0x0102".to_string(),
        }
    }

    fn hash(character: &str) -> String {
        format!("0x{}", character.repeat(64))
    }

    fn address(character: &str) -> String {
        format!("0x{}", character.repeat(40))
    }
}
