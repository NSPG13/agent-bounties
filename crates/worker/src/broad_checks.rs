//! Exact-input execution. Docker is only enabled explicitly on a dedicated runner.
use crate::{
    artifact_store::ArtifactStore,
    regression_sandbox::{snapshot_directory, DockerCliRegressionExecutor},
};
use broad_verification::{Manifest, Plan, Purpose, Report, Rule, Status, MAX_FILES, MAX_UPLOAD};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Duration,
};
use uuid::Uuid;
use verifier_sdk::{
    RegressionSandboxExecutor, RegressionSandboxPolicy, REGRESSION_SANDBOX_POLICY_VERSION,
};

struct Staging(PathBuf);
impl Drop for Staging {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn stage(root: &Path, files: &BTreeMap<String, Vec<u8>>) -> anyhow::Result<()> {
    std::fs::create_dir(root)?;
    for (name, bytes) in files {
        anyhow::ensure!(broad_verification::safe_path(name), "unsafe_path");
        let path = root.join(name);
        std::fs::create_dir_all(path.parent().expect("child path"))?;
        std::fs::write(path, bytes)?;
    }
    Ok(())
}
async fn published_test(
    rule: &Rule,
    files: &BTreeMap<String, Vec<u8>>,
    docker: &str,
) -> anyhow::Result<verifier_sdk::RegressionSandboxExecution> {
    let Rule::PublishedTestsV1 {
        image,
        benchmark_sha256,
        benchmark_files,
        command,
        timeout_seconds,
        memory_bytes,
    } = rule
    else {
        anyhow::bail!("not_published_tests");
    };
    anyhow::ensure!(
        broad_verification::value_digest(benchmark_files)? == *benchmark_sha256,
        "benchmark_changed"
    );
    let staging =
        Staging(std::env::temp_dir().join(format!("broad-verification-{}", Uuid::new_v4())));
    std::fs::create_dir(&staging.0)?;
    let source = staging.0.join("source");
    let benchmark = staging.0.join("benchmark");
    stage(&source, files)?;
    stage(
        &benchmark,
        &benchmark_files
            .iter()
            .map(|(k, v)| (k.clone(), v.as_bytes().to_vec()))
            .collect(),
    )?;
    let snapshot = snapshot_directory(&source, MAX_UPLOAD as u64, MAX_FILES as u32)?;
    let tests = snapshot_directory(&benchmark, 262144, MAX_FILES as u32)?;
    let policy = RegressionSandboxPolicy {
        schema_version: REGRESSION_SANDBOX_POLICY_VERSION.into(),
        image: image.clone(),
        command: command.clone(),
        workdir: "/workspace".into(),
        benchmark_digest: tests.digest,
        timeout_seconds: *timeout_seconds as u64,
        cpu_millis: 1000,
        memory_bytes: *memory_bytes,
        pids_limit: 64,
        max_output_bytes: 1024 * 1024,
        tmpfs_bytes: 64 * 1024 * 1024,
        max_source_bytes: MAX_UPLOAD as u64,
        max_source_files: MAX_FILES as u32,
        max_benchmark_bytes: 262144,
        max_benchmark_files: MAX_FILES as u32,
        platform: "linux/amd64".into(),
        test_seed: 0,
    };
    policy
        .validate()
        .map_err(|_| anyhow::anyhow!("unsupported_runner_policy"))?;
    DockerCliRegressionExecutor::new(docker, &source, &benchmark)?
        .execute(&policy, &snapshot.digest)
        .await
        .map_err(|_| anyhow::anyhow!("bounded_runner_unavailable"))
}
/// No caller-supplied receipt enters this path. All outcomes come from actual bytes.
pub async fn execute(
    plan: &Plan,
    manifest: &Manifest,
    files: &BTreeMap<String, Vec<u8>>,
    docker: Option<&str>,
) -> anyhow::Result<Report> {
    let mut report = broad_verification::run(plan, manifest, files)?;
    let cheap_failed = report.checks.iter().zip(&plan.checks).any(|(r, c)| {
        c.purpose == Purpose::Required
            && !matches!(c.rule, Rule::PublishedTestsV1 { .. })
            && r.status != Status::Pass
    });
    for (result, check) in report.checks.iter_mut().zip(&plan.checks) {
        if !matches!(check.rule, Rule::PublishedTestsV1 { .. }) || result.status == Status::Blocked
        {
            continue;
        }
        if cheap_failed {
            result.status = Status::Blocked;
            result.reason = "required_precheck_did_not_pass".into();
            continue;
        }
        let Some(docker) = docker else {
            continue;
        };
        match published_test(&check.rule, files, docker).await {
            Ok(execution) => {
                result.status = if execution.exit_code == 0 {
                    Status::Pass
                } else {
                    Status::Fail
                };
                result.reason = format!("published_command_exit_{}", execution.exit_code);
                result.evidence = Some(serde_json::to_value(execution)?);
            }
            Err(_) => {
                result.status = Status::CheckUnavailable;
                result.reason = "bounded_runner_unavailable_no_verdict".into();
            }
        }
    }
    report.required_checks_passed = manifest.verify(files).is_ok()
        && report
            .checks
            .iter()
            .filter(|c| c.purpose == Purpose::Required)
            .all(|c| c.status == Status::Pass);
    Ok(report)
}
pub async fn poll(
    store: &db::PostgresStore,
    objects: &ArtifactStore,
    docker: Option<&str>,
) -> anyhow::Result<bool> {
    let Some(job) = store.take_verification_job(docker.is_some()).await? else {
        return Ok(false);
    };
    let result = async {
        let files = objects.get(job.artifact.id, &job.artifact.manifest).await?;
        let report = execute(&job.plan, &job.artifact.manifest, &files, docker).await?;
        store
            .finish_verification_run(job.id, &job.owner, job.lease, &report)
            .await?;
        anyhow::Ok(())
    }
    .await;
    if result.is_err() {
        store
            .unavailable_verification_run(job.id, job.lease)
            .await?;
    }
    Ok(true)
}
pub async fn run(store: db::PostgresStore, objects: ArtifactStore, docker: Option<String>) {
    loop {
        let _ = poll(&store, &objects, docker.as_deref()).await;
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use broad_verification::{Check, Criterion, VERSION};
    fn plan(rule: Rule) -> Plan {
        Plan {
            schema_version: VERSION.into(),
            time_windows: None,
            criteria: vec![Criterion {
                id: "tests".into(),
                text: "Published tests pass".into(),
                review: Some("Check what the tests do not cover".into()),
            }],
            checks: vec![Check {
                id: "published".into(),
                criteria: vec!["tests".into()],
                purpose: Purpose::Required,
                rule,
            }],
        }
    }
    #[tokio::test]
    #[ignore = "requires local Docker and BROAD_CHECKS_TEST_IMAGE pinned Python image"]
    async fn actual_tests_run_on_read_only_exact_bytes_without_network() {
        let image = std::env::var("BROAD_CHECKS_TEST_IMAGE").unwrap();
        let benchmark_files=BTreeMap::from([("test.py".into(),"import pathlib, socket\nassert pathlib.Path('/workspace/answer.txt').read_text() == '42'\nassert pathlib.Path('/benchmark/test.py').exists()\ntry:\n pathlib.Path('/workspace/written').write_text('bad')\n raise AssertionError('source is writable')\nexcept OSError: pass\nassert not [i for i in pathlib.Path('/sys/class/net').iterdir() if i.name != 'lo']\n".into())]);
        let rule = Rule::PublishedTestsV1 {
            image,
            benchmark_sha256: broad_verification::value_digest(&benchmark_files).unwrap(),
            benchmark_files,
            command: vec!["python".into(), "/benchmark/test.py".into()],
            timeout_seconds: 20,
            memory_bytes: 128 * 1024 * 1024,
        };
        let plan = plan(rule);
        let files = BTreeMap::from([("answer.txt".into(), b"42".to_vec())]);
        let manifest = Manifest::from_files(&files).unwrap();
        let report = execute(&plan, &manifest, &files, Some("docker"))
            .await
            .unwrap();
        assert_eq!(
            report.checks[0].status,
            Status::Pass,
            "{}",
            report.checks[0].reason
        );
        assert!(!report.payment_authorized);
        let bad = BTreeMap::from([("answer.txt".into(), b"43".to_vec())]);
        let manifest = Manifest::from_files(&bad).unwrap();
        let report = execute(&plan, &manifest, &bad, Some("docker"))
            .await
            .unwrap();
        assert_eq!(report.checks[0].status, Status::Fail);
        let report = execute(&plan, &manifest, &bad, None).await.unwrap();
        assert_eq!(report.checks[0].status, Status::CheckUnavailable);
    }
}
