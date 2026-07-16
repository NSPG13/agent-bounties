use anyhow::{anyhow, Context};
use async_trait::async_trait;
use chain_base::{
    base_network_descriptor, keccak256_canonical_json, sha256_canonical_json, sha256_utf8,
    AutonomousVerificationJob,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{io::AsyncReadExt, process::Command, task, time::Instant};
use verifier_sdk::{
    RegressionSandboxExecution, RegressionSandboxExecutor, RegressionSandboxExecutorError,
    RegressionSandboxIsolation, RegressionSandboxPolicy, RegressionVerificationOutcome,
    RegressionVerificationScope, RegressionVerificationTask, SandboxedRegressionVerifier,
};

pub const REGRESSION_SANDBOX_ENGINE: &str = "sandboxed_regression_v1";
pub const REGRESSION_SANDBOX_STAGING_ROOT_ENV: &str = "REGRESSION_SANDBOX_STAGING_ROOT";
pub const REGRESSION_SANDBOX_DOCKER_BINARY_ENV: &str = "REGRESSION_SANDBOX_DOCKER_BINARY";
const DIRECTORY_DIGEST_DOMAIN: &[u8] = b"agent-bounties/directory-v1\0";
static RUN_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegressionSandboxRunRequest {
    pub job: AutonomousVerificationJob,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegressionInputKind {
    Source,
    Benchmark,
}

impl RegressionInputKind {
    pub fn parse(value: &str) -> anyhow::Result<Self> {
        match value {
            "source" => Ok(Self::Source),
            "benchmark" => Ok(Self::Benchmark),
            _ => Err(anyhow!("input kind must be source or benchmark")),
        }
    }

    fn directory_name(self) -> &'static str {
        match self {
            Self::Source => "sources",
            Self::Benchmark => "benchmarks",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectorySnapshot {
    pub digest: String,
    pub file_count: u32,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StagedRegressionInput {
    pub kind: RegressionInputKind,
    pub path: PathBuf,
    pub snapshot: DirectorySnapshot,
}

#[derive(Debug, Clone)]
pub struct PreparedRegressionRun {
    pub policy: RegressionSandboxPolicy,
    pub task: RegressionVerificationTask,
    pub source_path: PathBuf,
    pub benchmark_path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct DockerCliRegressionExecutor {
    docker_binary: PathBuf,
    source_path: PathBuf,
    benchmark_path: PathBuf,
}

impl DockerCliRegressionExecutor {
    pub fn new(
        docker_binary: impl Into<PathBuf>,
        source_path: impl AsRef<Path>,
        benchmark_path: impl AsRef<Path>,
    ) -> anyhow::Result<Self> {
        let source_path = canonical_sandbox_root(source_path.as_ref(), "source")?;
        let benchmark_path = canonical_sandbox_root(benchmark_path.as_ref(), "benchmark")?;
        if source_path == benchmark_path
            || source_path.starts_with(&benchmark_path)
            || benchmark_path.starts_with(&source_path)
        {
            return Err(anyhow!(
                "source and benchmark roots must be distinct and non-nested"
            ));
        }
        Ok(Self {
            docker_binary: docker_binary.into(),
            source_path,
            benchmark_path,
        })
    }

    fn docker_arguments(&self, policy: &RegressionSandboxPolicy, name: &str) -> Vec<String> {
        let cpu_limit = format!(
            "{}.{:03}",
            policy.cpu_millis / 1_000,
            policy.cpu_millis % 1_000
        );
        vec![
            "run".to_string(),
            "--pull".to_string(),
            "never".to_string(),
            "--name".to_string(),
            name.to_string(),
            "--platform".to_string(),
            policy.platform.clone(),
            "--network".to_string(),
            "none".to_string(),
            "--ipc".to_string(),
            "none".to_string(),
            "--read-only".to_string(),
            "--cap-drop".to_string(),
            "ALL".to_string(),
            "--security-opt".to_string(),
            "no-new-privileges=true".to_string(),
            "--user".to_string(),
            "65532:65532".to_string(),
            "--pids-limit".to_string(),
            policy.pids_limit.to_string(),
            "--cpus".to_string(),
            cpu_limit,
            "--memory".to_string(),
            policy.memory_bytes.to_string(),
            "--memory-swap".to_string(),
            policy.memory_bytes.to_string(),
            "--stop-timeout".to_string(),
            "1".to_string(),
            "--log-driver".to_string(),
            "none".to_string(),
            "--tmpfs".to_string(),
            format!("/tmp:rw,nosuid,nodev,size={}", policy.tmpfs_bytes),
            "--mount".to_string(),
            format!(
                "type=bind,src={},dst=/workspace,readonly",
                docker_mount_source(&self.source_path)
            ),
            "--mount".to_string(),
            format!(
                "type=bind,src={},dst=/benchmark,readonly",
                docker_mount_source(&self.benchmark_path)
            ),
            "--workdir".to_string(),
            policy.workdir.clone(),
            "--env".to_string(),
            "HOME=/tmp/home".to_string(),
            "--env".to_string(),
            "LANG=C.UTF-8".to_string(),
            "--env".to_string(),
            "LC_ALL=C.UTF-8".to_string(),
            "--env".to_string(),
            "TZ=UTC".to_string(),
            "--env".to_string(),
            "SOURCE_DATE_EPOCH=0".to_string(),
            "--env".to_string(),
            format!("AGENT_BOUNTIES_TEST_SEED={}", policy.test_seed),
            "--".to_string(),
            policy.image.clone(),
        ]
        .into_iter()
        .chain(policy.command.iter().cloned())
        .collect()
    }

    async fn remove_container(&self, name: &str) {
        let _ = Command::new(&self.docker_binary)
            .args(["rm", "--force", name])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;
    }
}

#[async_trait]
impl RegressionSandboxExecutor for DockerCliRegressionExecutor {
    async fn execute(
        &self,
        policy: &RegressionSandboxPolicy,
        source_digest: &str,
    ) -> Result<RegressionSandboxExecution, RegressionSandboxExecutorError> {
        let source_path = self.source_path.clone();
        let source_max_bytes = policy.max_source_bytes;
        let source_max_files = policy.max_source_files;
        let source_snapshot = task::spawn_blocking(move || {
            snapshot_directory(&source_path, source_max_bytes, source_max_files)
        })
        .await
        .map_err(|_| RegressionSandboxExecutorError::FailedClosed)?
        .map_err(|_| RegressionSandboxExecutorError::InputUnavailable)?;
        let benchmark_path = self.benchmark_path.clone();
        let benchmark_max_bytes = policy.max_benchmark_bytes;
        let benchmark_max_files = policy.max_benchmark_files;
        let benchmark_snapshot = task::spawn_blocking(move || {
            snapshot_directory(&benchmark_path, benchmark_max_bytes, benchmark_max_files)
        })
        .await
        .map_err(|_| RegressionSandboxExecutorError::FailedClosed)?
        .map_err(|_| RegressionSandboxExecutorError::InputUnavailable)?;
        if source_snapshot.digest != source_digest
            || benchmark_snapshot.digest != policy.benchmark_digest
        {
            return Err(RegressionSandboxExecutorError::InputUnavailable);
        }

        let name = format!(
            "agent-bounties-regression-{}-{}",
            std::process::id(),
            RUN_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        );
        let mut child = Command::new(&self.docker_binary)
            .args(self.docker_arguments(policy, &name))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|_| RegressionSandboxExecutorError::RuntimeUnavailable)?;
        let Some(stdout) = child.stdout.take() else {
            self.remove_container(&name).await;
            return Err(RegressionSandboxExecutorError::FailedClosed);
        };
        let Some(stderr) = child.stderr.take() else {
            self.remove_container(&name).await;
            return Err(RegressionSandboxExecutorError::FailedClosed);
        };
        let output_exceeded = Arc::new(AtomicBool::new(false));
        let total_output_bytes = Arc::new(AtomicU64::new(0));
        let stdout_task = tokio::spawn(hash_stream(
            stdout,
            policy.max_output_bytes,
            Arc::clone(&total_output_bytes),
            Arc::clone(&output_exceeded),
        ));
        let stderr_task = tokio::spawn(hash_stream(
            stderr,
            policy.max_output_bytes,
            Arc::clone(&total_output_bytes),
            Arc::clone(&output_exceeded),
        ));

        enum Completion {
            Status(std::process::ExitStatus),
            TimedOut,
            OutputLimitExceeded,
        }
        let deadline = Instant::now() + Duration::from_secs(policy.timeout_seconds);
        let completion = loop {
            if output_exceeded.load(Ordering::Acquire) {
                let _ = child.kill().await;
                let _ = child.wait().await;
                break Completion::OutputLimitExceeded;
            }
            if Instant::now() >= deadline {
                let _ = child.kill().await;
                let _ = child.wait().await;
                break Completion::TimedOut;
            }
            match child.try_wait() {
                Ok(Some(status)) => break Completion::Status(status),
                Ok(None) => tokio::time::sleep(Duration::from_millis(10)).await,
                Err(_) => {
                    let _ = child.kill().await;
                    let _ = child.wait().await;
                    self.remove_container(&name).await;
                    return Err(RegressionSandboxExecutorError::FailedClosed);
                }
            }
        };
        self.remove_container(&name).await;
        let stdout = stdout_task
            .await
            .map_err(|_| RegressionSandboxExecutorError::FailedClosed)?
            .map_err(|_| RegressionSandboxExecutorError::FailedClosed)?;
        let stderr = stderr_task
            .await
            .map_err(|_| RegressionSandboxExecutorError::FailedClosed)?
            .map_err(|_| RegressionSandboxExecutorError::FailedClosed)?;

        match completion {
            Completion::TimedOut => return Err(RegressionSandboxExecutorError::TimedOut),
            Completion::OutputLimitExceeded => {
                return Err(RegressionSandboxExecutorError::OutputLimitExceeded)
            }
            Completion::Status(status) => {
                let code = status
                    .code()
                    .ok_or(RegressionSandboxExecutorError::ResourceLimitExceeded)?;
                if matches!(code, 125..=127) {
                    return Err(RegressionSandboxExecutorError::FailedClosed);
                }
                if code >= 128 {
                    return Err(RegressionSandboxExecutorError::ResourceLimitExceeded);
                }
                Ok(RegressionSandboxExecution {
                    exit_code: code,
                    stdout_sha256: stdout.digest,
                    stderr_sha256: stderr.digest,
                    stdout_bytes: stdout.bytes,
                    stderr_bytes: stderr.bytes,
                    isolation: RegressionSandboxIsolation::default(),
                })
            }
        }
    }
}

#[derive(Debug)]
struct StreamDigest {
    digest: String,
    bytes: u64,
}

async fn hash_stream(
    mut stream: impl tokio::io::AsyncRead + Unpin,
    limit: u64,
    total_bytes: Arc<AtomicU64>,
    exceeded: Arc<AtomicBool>,
) -> std::io::Result<StreamDigest> {
    let mut hasher = Sha256::new();
    let mut bytes = 0u64;
    let mut buffer = [0u8; 8 * 1024];
    loop {
        let read = stream.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        bytes = bytes.saturating_add(read as u64);
        let previous_total = total_bytes.fetch_add(read as u64, Ordering::AcqRel);
        if previous_total
            .checked_add(read as u64)
            .is_none_or(|total| total > limit)
        {
            exceeded.store(true, Ordering::Release);
        }
    }
    Ok(StreamDigest {
        digest: format!("sha256:{}", hex::encode(hasher.finalize())),
        bytes,
    })
}

pub fn prepare_regression_run(
    job: AutonomousVerificationJob,
    observed_at_unix: u64,
    staging_root: &Path,
) -> anyhow::Result<PreparedRegressionRun> {
    let network =
        base_network_descriptor(&job.network).context("verification job network is unsupported")?;
    let evidence_network = base_network_descriptor(&job.submission_evidence.network)
        .context("submission evidence network is unsupported")?;
    if network.chain_id != evidence_network.chain_id {
        return Err(anyhow!("verification job and evidence networks differ"));
    }
    let canonical_network = if network.chain_id == 8_453 {
        "base-mainnet"
    } else if network.chain_id == 84_532 {
        "base-sepolia"
    } else {
        return Err(anyhow!(
            "regression verification supports only Base networks"
        ));
    };
    if job.verification_mode != "signed_quorum" || job.verifier_module.is_some() {
        return Err(anyhow!(
            "sandboxed regression verification requires signed_quorum without a verifier module"
        ));
    }
    if job.threshold < 2
        || job.eligible_verifiers.len() < usize::from(job.threshold)
        || job.verification_expires_at <= observed_at_unix
    {
        return Err(anyhow!(
            "sandboxed regression verification requires a live quorum threshold of at least two"
        ));
    }
    let mut distinct_verifiers = HashSet::new();
    for verifier in &job.eligible_verifiers {
        validate_evm_address("eligible verifier", verifier)?;
        if !distinct_verifiers.insert(verifier.to_ascii_lowercase()) {
            return Err(anyhow!("eligible regression verifiers must be distinct"));
        }
    }

    validate_terms_hashes(&job)?;
    let evidence = &job.submission_evidence;
    if !evidence
        .bounty_contract
        .eq_ignore_ascii_case(&job.bounty_contract)
        || !evidence.bounty_id.eq_ignore_ascii_case(&job.bounty_id)
        || evidence.round != job.round
        || !evidence
            .solver_wallet
            .eq_ignore_ascii_case(&job.solver_wallet)
        || !evidence
            .artifact_hash
            .eq_ignore_ascii_case(&sha256_utf8(&evidence.artifact_reference))
        || !evidence
            .evidence_hash
            .eq_ignore_ascii_case(&sha256_canonical_json(&evidence.evidence)?)
    {
        return Err(anyhow!(
            "verification job scope or published submission preimages do not match"
        ));
    }

    let verification_policy = job
        .terms
        .document
        .verification_policy
        .as_object()
        .ok_or_else(|| anyhow!("verification policy must be an object"))?;
    if verification_policy
        .get("mechanism")
        .and_then(serde_json::Value::as_str)
        != Some("signed_quorum")
        || verification_policy
            .get("engine")
            .and_then(serde_json::Value::as_str)
            != Some(REGRESSION_SANDBOX_ENGINE)
        || verification_policy
            .get("threshold")
            .and_then(serde_json::Value::as_u64)
            != Some(u64::from(job.threshold))
    {
        return Err(anyhow!(
            "verification policy does not commit sandboxed_regression_v1 signed quorum"
        ));
    }
    let policy_verifiers = verification_policy
        .get("verifiers")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| anyhow!("verification policy verifiers are unavailable"))?;
    if policy_verifiers.len() != job.eligible_verifiers.len()
        || policy_verifiers
            .iter()
            .zip(&job.eligible_verifiers)
            .any(|(committed, indexed)| {
                !committed
                    .as_str()
                    .is_some_and(|value| value.eq_ignore_ascii_case(indexed))
            })
    {
        return Err(anyhow!(
            "indexed eligible verifiers differ from the immutable policy"
        ));
    }

    let benchmark = job
        .terms
        .document
        .benchmark
        .as_object()
        .ok_or_else(|| anyhow!("regression benchmark must be an object"))?;
    if benchmark.get("engine").and_then(serde_json::Value::as_str)
        != Some(REGRESSION_SANDBOX_ENGINE)
    {
        return Err(anyhow!("benchmark does not commit sandboxed_regression_v1"));
    }
    let policy: RegressionSandboxPolicy = serde_json::from_value(
        benchmark
            .get("runner_manifest")
            .cloned()
            .ok_or_else(|| anyhow!("benchmark runner_manifest is unavailable"))?,
    )
    .context("benchmark runner_manifest is invalid")?;
    policy.validate()?;
    let source_digest = evidence
        .evidence
        .get("source_snapshot_digest")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| anyhow!("submission evidence source_snapshot_digest is unavailable"))?
        .to_string();

    validate_bytes32("bounty_id", &job.bounty_id)?;
    validate_evm_address("bounty_contract", &job.bounty_contract)?;
    validate_evm_address("solver_wallet", &job.solver_wallet)?;
    let task = RegressionVerificationTask {
        scope: RegressionVerificationScope {
            network: canonical_network.to_string(),
            bounty_id: job.bounty_id,
            bounty_contract: job.bounty_contract.to_ascii_lowercase(),
            round: job.round,
            solver_wallet: job.solver_wallet.to_ascii_lowercase(),
            submission_hash: evidence.artifact_hash.to_ascii_lowercase(),
            evidence_hash: evidence.evidence_hash.to_ascii_lowercase(),
            terms_hash: job.terms.terms_hash.to_ascii_lowercase(),
            committed_policy_hash: job.terms.policy_hash.to_ascii_lowercase(),
            verification_expires_at: job.verification_expires_at,
        },
        source_digest: source_digest.clone(),
    };
    task.validate()?;
    Ok(PreparedRegressionRun {
        source_path: staged_input_path(staging_root, RegressionInputKind::Source, &source_digest)?,
        benchmark_path: staged_input_path(
            staging_root,
            RegressionInputKind::Benchmark,
            &policy.benchmark_digest,
        )?,
        policy,
        task,
    })
}

fn validate_terms_hashes(job: &AutonomousVerificationJob) -> anyhow::Result<()> {
    let document = serde_json::to_value(&job.terms.document)?;
    let acceptance = serde_json::to_value(&job.terms.document.acceptance_criteria)?;
    for (field, actual, expected) in [
        (
            "terms_hash",
            job.terms.terms_hash.as_str(),
            keccak256_canonical_json(&document)?,
        ),
        (
            "policy_hash",
            job.terms.policy_hash.as_str(),
            keccak256_canonical_json(&job.terms.document.verification_policy)?,
        ),
        (
            "acceptance_criteria_hash",
            job.terms.acceptance_criteria_hash.as_str(),
            keccak256_canonical_json(&acceptance)?,
        ),
        (
            "benchmark_hash",
            job.terms.benchmark_hash.as_str(),
            keccak256_canonical_json(&job.terms.document.benchmark)?,
        ),
        (
            "evidence_schema_hash",
            job.terms.evidence_schema_hash.as_str(),
            keccak256_canonical_json(&job.terms.document.evidence_schema)?,
        ),
    ] {
        validate_bytes32(field, actual)?;
        if !actual.eq_ignore_ascii_case(&expected) {
            return Err(anyhow!("{field} does not match its immutable preimage"));
        }
    }
    let contract_network = job
        .terms
        .document
        .contract_terms
        .get("network")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| anyhow!("contract terms network is unavailable"))?;
    if base_network_descriptor(contract_network)?.chain_id
        != base_network_descriptor(&job.network)?.chain_id
    {
        return Err(anyhow!("contract terms network differs from the job"));
    }
    Ok(())
}

pub async fn run_regression_sandbox_request(
    request: RegressionSandboxRunRequest,
    staging_root: &Path,
    docker_binary: impl Into<PathBuf>,
) -> anyhow::Result<RegressionVerificationOutcome> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before the Unix epoch")?
        .as_secs();
    let prepared = prepare_regression_run(request.job, now, staging_root)?;
    let executor = DockerCliRegressionExecutor::new(
        docker_binary,
        &prepared.source_path,
        &prepared.benchmark_path,
    )?;
    SandboxedRegressionVerifier {
        executor,
        policy: prepared.policy,
    }
    .verify(prepared.task)
    .await
    .context("sandboxed regression verification failed closed")
}

pub fn stage_regression_input(
    input: &Path,
    staging_root: &Path,
    kind: RegressionInputKind,
    max_bytes: u64,
    max_files: u32,
) -> anyhow::Result<StagedRegressionInput> {
    let source_snapshot = snapshot_directory(input, max_bytes, max_files)?;
    fs::create_dir_all(staging_root).context("failed to create regression staging root")?;
    let staging_root = canonical_sandbox_root(staging_root, "staging")?;
    let target = staged_input_path(&staging_root, kind, &source_snapshot.digest)?;
    if target.exists() {
        let existing = snapshot_directory(&target, max_bytes, max_files)?;
        if existing != source_snapshot {
            return Err(anyhow!(
                "content-addressed staging target does not match its digest"
            ));
        }
        return Ok(StagedRegressionInput {
            kind,
            path: target,
            snapshot: existing,
        });
    }

    let parent = target
        .parent()
        .ok_or_else(|| anyhow!("staging target has no parent"))?;
    fs::create_dir_all(parent).context("failed to create content-addressed staging namespace")?;
    let temporary = staging_root.join(".tmp").join(format!(
        "{}-{}",
        std::process::id(),
        RUN_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    if let Some(parent) = temporary.parent() {
        fs::create_dir_all(parent).context("failed to create temporary staging namespace")?;
    }
    fs::create_dir(&temporary).context("failed to create temporary staging directory")?;
    let copy_result = copy_directory_tree(input, &temporary).and_then(|()| {
        let copied = snapshot_directory(&temporary, max_bytes, max_files)?;
        if copied != source_snapshot {
            return Err(anyhow!("sandbox input changed while it was being staged"));
        }
        make_tree_contents_read_only(&temporary)?;
        Ok(copied)
    });
    let copied = match copy_result {
        Ok(copied) => copied,
        Err(error) => {
            remove_tree_best_effort(&temporary);
            return Err(error);
        }
    };
    match fs::rename(&temporary, &target) {
        Ok(()) => {
            if let Err(error) = make_directory_read_only(&target) {
                remove_tree_best_effort(&target);
                return Err(error).context("failed to secure staged regression input");
            }
        }
        Err(_) if target.exists() => {
            remove_tree_best_effort(&temporary);
            let existing = snapshot_directory(&target, max_bytes, max_files)?;
            if existing != source_snapshot {
                return Err(anyhow!(
                    "concurrent content-addressed staging target is inconsistent"
                ));
            }
        }
        Err(error) => {
            remove_tree_best_effort(&temporary);
            return Err(error).context("failed to publish staged regression input");
        }
    }
    Ok(StagedRegressionInput {
        kind,
        path: target,
        snapshot: copied,
    })
}

fn staged_input_path(
    staging_root: &Path,
    kind: RegressionInputKind,
    digest: &str,
) -> anyhow::Result<PathBuf> {
    let digest = digest
        .strip_prefix("sha256:")
        .filter(|value| {
            value.len() == 64
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        })
        .ok_or_else(|| anyhow!("staged input digest must be sha256:<64 lowercase hex>"))?;
    Ok(staging_root
        .join(kind.directory_name())
        .join("sha256")
        .join(digest))
}

pub fn snapshot_directory(
    root: &Path,
    max_bytes: u64,
    max_files: u32,
) -> anyhow::Result<DirectorySnapshot> {
    if max_bytes == 0 || max_files == 0 {
        return Err(anyhow!("directory snapshot limits must be positive"));
    }
    let root = canonical_sandbox_root(root, "snapshot")?;
    let files = enumerate_regular_files(&root, max_bytes, max_files)?;
    let mut hasher = Sha256::new();
    hasher.update(DIRECTORY_DIGEST_DOMAIN);
    for file in &files {
        let path_bytes = file.relative.as_bytes();
        hasher.update((path_bytes.len() as u64).to_be_bytes());
        hasher.update(path_bytes);
        hasher.update(file.size.to_be_bytes());
        let mut input = fs::File::open(&file.path).context("failed to open sandbox input file")?;
        std::io::copy(&mut input, &mut HashWriter(&mut hasher))
            .context("failed to hash sandbox input file")?;
    }
    Ok(DirectorySnapshot {
        digest: format!("sha256:{}", hex::encode(hasher.finalize())),
        file_count: files.len() as u32,
        total_bytes: files.iter().map(|file| file.size).sum(),
    })
}

struct RegularFile {
    relative: String,
    path: PathBuf,
    size: u64,
}

fn enumerate_regular_files(
    root: &Path,
    max_bytes: u64,
    max_files: u32,
) -> anyhow::Result<Vec<RegularFile>> {
    let mut pending = vec![root.to_path_buf()];
    let mut files = Vec::new();
    let mut total_bytes = 0u64;
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).context("failed to enumerate sandbox input")? {
            let entry = entry.context("failed to read sandbox directory entry")?;
            let path = entry.path();
            let metadata =
                fs::symlink_metadata(&path).context("failed to inspect sandbox directory entry")?;
            let file_type = metadata.file_type();
            if file_type.is_symlink() {
                return Err(anyhow!("sandbox inputs cannot contain symbolic links"));
            }
            if file_type.is_dir() {
                pending.push(path);
                continue;
            }
            if !file_type.is_file() {
                return Err(anyhow!("sandbox inputs cannot contain special files"));
            }
            let relative = path
                .strip_prefix(root)
                .context("sandbox entry escaped its root")?
                .to_str()
                .ok_or_else(|| anyhow!("sandbox input paths must be UTF-8"))?
                .replace('\\', "/");
            if relative.is_empty()
                || relative.split('/').any(|part| {
                    part.is_empty() || part == "." || part == ".." || part.contains('\0')
                })
            {
                return Err(anyhow!("sandbox input path is invalid"));
            }
            total_bytes = total_bytes
                .checked_add(metadata.len())
                .ok_or_else(|| anyhow!("sandbox input size overflow"))?;
            if total_bytes > max_bytes || files.len() >= max_files as usize {
                return Err(anyhow!("sandbox input exceeds its committed size limits"));
            }
            files.push(RegularFile {
                relative,
                path,
                size: metadata.len(),
            });
        }
    }
    files.sort_by(|left, right| left.relative.cmp(&right.relative));
    Ok(files)
}

fn copy_directory_tree(source: &Path, destination: &Path) -> anyhow::Result<()> {
    let source = canonical_sandbox_root(source, "source")?;
    let mut pending = vec![source.clone()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).context("failed to enumerate staged input")? {
            let entry = entry.context("failed to read staged input entry")?;
            let path = entry.path();
            let metadata =
                fs::symlink_metadata(&path).context("failed to inspect staged input entry")?;
            if metadata.file_type().is_symlink() {
                return Err(anyhow!("sandbox inputs cannot contain symbolic links"));
            }
            let relative = path
                .strip_prefix(&source)
                .context("staged input escaped its root")?;
            let target = destination.join(relative);
            if metadata.is_dir() {
                fs::create_dir(&target).context("failed to create staged input directory")?;
                pending.push(path);
            } else if metadata.is_file() {
                fs::copy(&path, &target).context("failed to copy staged input file")?;
            } else {
                return Err(anyhow!("sandbox inputs cannot contain special files"));
            }
        }
    }
    Ok(())
}

fn make_tree_contents_read_only(root: &Path) -> anyhow::Result<()> {
    let mut directories = vec![root.to_path_buf()];
    let mut all_directories = Vec::new();
    while let Some(directory) = directories.pop() {
        all_directories.push(directory.clone());
        for entry in fs::read_dir(&directory).context("failed to secure staged input")? {
            let path = entry?.path();
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.is_dir() {
                directories.push(path);
            } else if metadata.is_file() {
                let mut permissions = metadata.permissions();
                permissions.set_readonly(true);
                fs::set_permissions(path, permissions)?;
            } else {
                return Err(anyhow!("staged input contains a special file"));
            }
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for directory in all_directories
            .into_iter()
            .rev()
            .filter(|directory| directory != root)
        {
            fs::set_permissions(directory, fs::Permissions::from_mode(0o555))?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn make_directory_read_only(path: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o555))?;
    Ok(())
}

#[cfg(not(unix))]
fn make_directory_read_only(_path: &Path) -> anyhow::Result<()> {
    Ok(())
}

fn remove_tree_best_effort(path: &Path) {
    #[cfg(unix)]
    fn clear(path: &Path) {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(metadata) = fs::symlink_metadata(path) {
            if metadata.is_dir() {
                if let Ok(entries) = fs::read_dir(path) {
                    for entry in entries.flatten() {
                        clear(&entry.path());
                    }
                }
                let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o700));
            } else if metadata.is_file() {
                let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
            }
        }
    }
    #[cfg(windows)]
    #[allow(clippy::permissions_set_readonly_false)]
    fn clear(path: &Path) {
        if let Ok(metadata) = fs::symlink_metadata(path) {
            if metadata.is_dir() {
                if let Ok(entries) = fs::read_dir(path) {
                    for entry in entries.flatten() {
                        clear(&entry.path());
                    }
                }
            }
            let mut permissions = metadata.permissions();
            permissions.set_readonly(false);
            let _ = fs::set_permissions(path, permissions);
        }
    }
    #[cfg(any(unix, windows))]
    clear(path);
    let _ = fs::remove_dir_all(path);
}

struct HashWriter<'a>(&'a mut Sha256);

impl std::io::Write for HashWriter<'_> {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.0.update(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn canonical_sandbox_root(path: &Path, kind: &str) -> anyhow::Result<PathBuf> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("failed to inspect {kind} sandbox root"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(anyhow!("{kind} sandbox root must be a real directory"));
    }
    let canonical =
        fs::canonicalize(path).with_context(|| format!("failed to resolve {kind} sandbox root"))?;
    let text = canonical
        .to_str()
        .ok_or_else(|| anyhow!("{kind} sandbox root must be UTF-8"))?;
    if text.contains(',') || text.contains('\n') || text.contains('\r') {
        return Err(anyhow!(
            "{kind} sandbox root cannot contain mount delimiters"
        ));
    }
    Ok(canonical)
}

fn docker_mount_source(path: &Path) -> String {
    let text = path.to_string_lossy();
    #[cfg(windows)]
    let text = text.strip_prefix("\\\\?\\").unwrap_or(&text);
    text.to_string()
}

fn validate_bytes32(field: &str, value: &str) -> anyhow::Result<()> {
    if value.len() != 66
        || !value.starts_with("0x")
        || !value[2..].bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(anyhow!("{field} must be a 0x-prefixed bytes32 value"));
    }
    Ok(())
}

fn validate_evm_address(field: &str, value: &str) -> anyhow::Result<()> {
    if value.len() != 42
        || !value.starts_with("0x")
        || !value[2..].bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(anyhow!("{field} must be an EVM address"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chain_base::{build_autonomous_bounty_terms_record, BASE_MAINNET_USDC_TOKEN_ADDRESS};
    use chrono::{TimeZone, Utc};
    use domain::{AutonomousBountyTermsDocument, AutonomousSubmissionEvidenceRecord};
    use serde_json::json;

    #[test]
    fn directory_snapshot_is_stable_and_content_addressed() {
        let root = temp_directory("snapshot");
        fs::create_dir_all(root.join("nested")).unwrap();
        fs::write(root.join("b.txt"), b"two").unwrap();
        fs::write(root.join("nested").join("a.txt"), b"one").unwrap();

        let first = snapshot_directory(&root, 1_024, 10).unwrap();
        let second = snapshot_directory(&root, 1_024, 10).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.file_count, 2);

        fs::write(root.join("nested").join("a.txt"), b"changed").unwrap();
        let changed = snapshot_directory(&root, 1_024, 10).unwrap();
        assert_ne!(first.digest, changed.digest);
        remove_tree_best_effort(&root);
    }

    #[test]
    fn staged_inputs_are_digest_derived_and_immutable() {
        let input = temp_directory("stage-input");
        let staging = temp_directory("stage-root");
        fs::create_dir(input.join("nested")).unwrap();
        fs::write(input.join("result.txt"), b"expected").unwrap();
        fs::write(input.join("nested").join("details.txt"), b"stable").unwrap();

        let staged =
            stage_regression_input(&input, &staging, RegressionInputKind::Source, 1_024, 10)
                .unwrap();
        let canonical_staging = fs::canonicalize(&staging).unwrap();
        assert!(staged
            .path
            .starts_with(canonical_staging.join("sources").join("sha256")));
        assert_eq!(
            snapshot_directory(&staged.path, 1_024, 10).unwrap(),
            staged.snapshot
        );
        assert!(fs::metadata(staged.path.join("result.txt"))
            .unwrap()
            .permissions()
            .readonly());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&staged.path).unwrap().permissions().mode() & 0o222,
                0
            );
        }

        remove_tree_best_effort(&input);
        remove_tree_best_effort(&staging);
        assert!(!staging.exists());
    }

    #[test]
    fn docker_arguments_enforce_the_isolation_contract_and_end_options() {
        let source = temp_directory("source");
        let benchmark = temp_directory("benchmark");
        let executor = DockerCliRegressionExecutor::new("docker", &source, &benchmark).unwrap();
        let policy = regression_policy(
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        );
        let args = executor.docker_arguments(&policy, "agent-bounties-test");
        let joined = args.join(" ");

        for required in [
            "--network none",
            "--ipc none",
            "--read-only",
            "--cap-drop ALL",
            "no-new-privileges=true",
            "--user 65532:65532",
            "dst=/workspace,readonly",
            "dst=/benchmark,readonly",
            "--pull never",
            "--log-driver none",
        ] {
            assert!(joined.contains(required), "missing {required}: {joined}");
        }
        assert!(!joined.contains("docker.sock"));
        assert!(!joined.contains("--privileged"));
        let separator = args.iter().position(|argument| argument == "--").unwrap();
        assert_eq!(args[separator + 1], policy.image);
        assert_eq!(&args[separator + 2..], policy.command);
        remove_tree_best_effort(&source);
        remove_tree_best_effort(&benchmark);
    }

    #[test]
    fn autonomous_job_adapter_binds_every_payment_relevant_scope() {
        let staging = temp_directory("adapter");
        let job = verification_job();
        let prepared = prepare_regression_run(job.clone(), 1_800_000_000, &staging).unwrap();
        assert_eq!(prepared.task.scope.bounty_id, job.bounty_id);
        assert_eq!(
            prepared.task.scope.committed_policy_hash,
            job.terms.policy_hash
        );
        assert_eq!(prepared.task.source_digest, source_digest());

        type JobMutation = Box<dyn Fn(&mut AutonomousVerificationJob)>;
        let mut mutations: Vec<JobMutation> = vec![
            Box::new(|job| job.submission_evidence.network = "base-sepolia".to_string()),
            Box::new(|job| job.submission_evidence.bounty_contract = address('9')),
            Box::new(|job| job.submission_evidence.bounty_id = bytes32('9')),
            Box::new(|job| job.submission_evidence.round += 1),
            Box::new(|job| job.submission_evidence.solver_wallet = address('9')),
            Box::new(|job| job.submission_evidence.artifact_hash = bytes32('9')),
            Box::new(|job| job.submission_evidence.evidence_hash = bytes32('9')),
            Box::new(|job| job.terms.policy_hash = bytes32('9')),
            Box::new(|job| job.verification_expires_at = 1_800_000_000),
        ];
        for mutate in mutations.drain(..) {
            let mut invalid = job.clone();
            mutate(&mut invalid);
            assert!(prepare_regression_run(invalid, 1_800_000_000, &staging).is_err());
        }
        remove_tree_best_effort(&staging);
    }

    #[test]
    fn autonomous_job_adapter_rejects_weak_or_wrong_verification_policy() {
        let staging = temp_directory("policy-adapter");
        let mut threshold_one = verification_job();
        threshold_one.threshold = 1;
        assert!(prepare_regression_run(threshold_one, 1_800_000_000, &staging).is_err());

        let mut duplicate = verification_job();
        duplicate.eligible_verifiers[1] = duplicate.eligible_verifiers[0].clone();
        assert!(prepare_regression_run(duplicate, 1_800_000_000, &staging).is_err());

        let mut wrong_engine = verification_job();
        wrong_engine.terms.document.verification_policy["engine"] = json!("github_ci");
        assert!(prepare_regression_run(wrong_engine, 1_800_000_000, &staging).is_err());

        let mut missing_source = verification_job();
        missing_source
            .submission_evidence
            .evidence
            .as_object_mut()
            .unwrap()
            .remove("source_snapshot_digest");
        missing_source.submission_evidence.evidence_hash =
            sha256_canonical_json(&missing_source.submission_evidence.evidence).unwrap();
        assert!(prepare_regression_run(missing_source, 1_800_000_000, &staging).is_err());
        remove_tree_best_effort(&staging);
    }

    #[tokio::test]
    #[ignore = "requires a local Docker daemon and the pinned Alpine image"]
    async fn docker_rehearsal_passes_fails_and_produces_no_infrastructure_verdicts() {
        let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("regression");
        let staging = temp_directory("docker-rehearsal");
        let benchmark = stage_regression_input(
            &fixture_root.join("benchmark"),
            &staging,
            RegressionInputKind::Benchmark,
            1_024 * 1_024,
            100,
        )
        .unwrap();
        let good = stage_regression_input(
            &fixture_root.join("source"),
            &staging,
            RegressionInputKind::Source,
            1_024 * 1_024,
            100,
        )
        .unwrap();
        let bad = stage_regression_input(
            &fixture_root.join("source-bad"),
            &staging,
            RegressionInputKind::Source,
            1_024 * 1_024,
            100,
        )
        .unwrap();
        let observed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let mut policy = regression_policy(&benchmark.snapshot.digest);
        policy.image = pinned_alpine_image();
        policy.command = vec![
            "cmp".to_string(),
            "/workspace/result.txt".to_string(),
            "/benchmark/expected.txt".to_string(),
        ];

        let passed = run_regression_sandbox_request(
            RegressionSandboxRunRequest {
                job: verification_job_with(observed, &good.snapshot.digest, policy.clone()),
            },
            &staging,
            "docker",
        )
        .await
        .unwrap();
        assert_eq!(passed.verdict, verifier_sdk::RegressionVerdict::Passed);

        let failed = run_regression_sandbox_request(
            RegressionSandboxRunRequest {
                job: verification_job_with(observed, &bad.snapshot.digest, policy.clone()),
            },
            &staging,
            "docker",
        )
        .await
        .unwrap();
        assert_eq!(failed.verdict, verifier_sdk::RegressionVerdict::Failed);

        let mut timeout_policy = policy.clone();
        timeout_policy.command = vec!["sleep".to_string(), "2".to_string()];
        timeout_policy.timeout_seconds = 1;
        let timeout_error = run_regression_sandbox_request(
            RegressionSandboxRunRequest {
                job: verification_job_with(observed, &good.snapshot.digest, timeout_policy),
            },
            &staging,
            "docker",
        )
        .await
        .unwrap_err();
        assert!(format!("{timeout_error:#}").contains("no verdict"));

        let mut output_policy = policy;
        output_policy.command = vec!["yes".to_string()];
        output_policy.max_output_bytes = 1_024;
        let output_error = run_regression_sandbox_request(
            RegressionSandboxRunRequest {
                job: verification_job_with(observed, &good.snapshot.digest, output_policy),
            },
            &staging,
            "docker",
        )
        .await
        .unwrap_err();
        assert!(format!("{output_error:#}").contains("no verdict"));
        remove_tree_best_effort(&staging);
    }

    #[test]
    fn nested_roots_and_size_overruns_are_rejected() {
        let source = temp_directory("nested-source");
        let benchmark = source.join("benchmark");
        fs::create_dir_all(&benchmark).unwrap();
        assert!(DockerCliRegressionExecutor::new("docker", &source, &benchmark).is_err());
        fs::write(source.join("large.bin"), [0u8; 16]).unwrap();
        assert!(snapshot_directory(&source, 8, 10).is_err());
        remove_tree_best_effort(&source);
    }

    #[cfg(unix)]
    #[test]
    fn symbolic_links_are_rejected() {
        use std::os::unix::fs::symlink;
        let root = temp_directory("symlink");
        fs::write(root.join("target"), b"target").unwrap();
        symlink(root.join("target"), root.join("link")).unwrap();
        assert!(snapshot_directory(&root, 1_024, 10).is_err());
        remove_tree_best_effort(&root);
    }

    fn verification_job() -> AutonomousVerificationJob {
        verification_job_with(
            1_800_000_000,
            &source_digest(),
            regression_policy(&benchmark_digest()),
        )
    }

    fn verification_job_with(
        observed: u64,
        source_digest: &str,
        manifest: RegressionSandboxPolicy,
    ) -> AutonomousVerificationJob {
        let document = AutonomousBountyTermsDocument {
            schema_version: "agent-bounties/terms-v1".to_string(),
            contract_terms: json!({
                "protocol_version": "agent-bounties/autonomous-v1",
                "creator_wallet": address('4'),
                "network": "base-mainnet",
                "settlement_token": BASE_MAINNET_USDC_TOKEN_ADDRESS,
                "solver_reward": {"amount": 1_900_000, "currency": "usdc"},
                "verifier_reward": {"amount": 100_000, "currency": "usdc"},
                "claim_bond": {"amount": 100_000, "currency": "usdc"},
                "initial_funding": {"amount": 2_000_000, "currency": "usdc"},
                "funding_deadline": observed + 86_400,
                "claim_window_seconds": 3_600,
                "verification_window_seconds": 1_800,
                "creation_nonce": bytes32('1'),
            }),
            title: "Run immutable regression tests".to_string(),
            goal: "Verify a submitted source snapshot".to_string(),
            acceptance_criteria: vec!["committed tests pass".to_string()],
            benchmark: json!({
                "engine": REGRESSION_SANDBOX_ENGINE,
                "runner_manifest": manifest,
            }),
            evidence_schema: json!({
                "type": "object",
                "required": ["source_snapshot_digest"]
            }),
            verification_policy: json!({
                "mechanism": "signed_quorum",
                "engine": REGRESSION_SANDBOX_ENGINE,
                "verifiers": [address('5'), address('6')],
                "threshold": 2
            }),
            source_url: None,
            discovery_source: None,
            agent_eligibility: None,
            claim_coordination: None,
        };
        let created_at = Utc.timestamp_opt(observed as i64, 0).unwrap();
        let terms = build_autonomous_bounty_terms_record(&address('4'), document, created_at)
            .expect("valid terms fixture");
        let evidence = json!({"source_snapshot_digest": source_digest});
        let artifact_reference = "https://example.com/source.tar".to_string();
        let submission_evidence = AutonomousSubmissionEvidenceRecord {
            network: "base-mainnet".to_string(),
            bounty_contract: address('2'),
            bounty_id: bytes32('a'),
            round: 1,
            solver_wallet: address('3'),
            artifact_hash: sha256_utf8(&artifact_reference),
            artifact_reference,
            evidence_hash: sha256_canonical_json(&evidence).unwrap(),
            evidence,
            created_at,
        };
        AutonomousVerificationJob {
            job_id: format!("base-mainnet:{}:1", address('2')),
            network: "base-mainnet".to_string(),
            bounty_id: bytes32('a'),
            bounty_contract: address('2'),
            round: 1,
            solver_wallet: address('3'),
            verification_mode: "signed_quorum".to_string(),
            verifier_module: None,
            eligible_verifiers: vec![address('5'), address('6')],
            threshold: 2,
            verifier_reward: "100000".to_string(),
            current_solver_payout: "1900000".to_string(),
            verification_expires_at: observed + 1_800,
            terms,
            submission_evidence,
            required_action: "run committed regression verifier".to_string(),
            payout_boundary: "only BountySettled proves payment".to_string(),
        }
    }

    fn regression_policy(benchmark_digest: &str) -> RegressionSandboxPolicy {
        RegressionSandboxPolicy {
            schema_version: verifier_sdk::REGRESSION_SANDBOX_POLICY_VERSION.to_string(),
            image: format!("docker.io/library/alpine@sha256:{}", "b".repeat(64)),
            command: vec!["true".to_string()],
            workdir: "/workspace".to_string(),
            benchmark_digest: benchmark_digest.to_string(),
            timeout_seconds: 30,
            cpu_millis: 500,
            memory_bytes: 128 * 1024 * 1024,
            pids_limit: 32,
            max_output_bytes: 64 * 1024,
            tmpfs_bytes: 64 * 1024 * 1024,
            max_source_bytes: 1024 * 1024,
            max_source_files: 100,
            max_benchmark_bytes: 1024 * 1024,
            max_benchmark_files: 100,
            platform: "linux/amd64".to_string(),
            test_seed: 7,
        }
    }

    fn source_digest() -> String {
        format!("sha256:{}", "c".repeat(64))
    }

    fn benchmark_digest() -> String {
        format!("sha256:{}", "d".repeat(64))
    }

    fn pinned_alpine_image() -> String {
        "docker.io/library/alpine@sha256:48b0309ca019d89d40f670aa1bc06e426dc0931948452e8491e3d65087abc07d"
            .to_string()
    }

    fn bytes32(character: char) -> String {
        format!("0x{}", character.to_string().repeat(64))
    }

    fn address(character: char) -> String {
        format!("0x{}", character.to_string().repeat(40))
    }

    fn temp_directory(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "agent-bounties-{label}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }
}
