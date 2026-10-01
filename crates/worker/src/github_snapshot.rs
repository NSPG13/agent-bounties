//! Bounded, credential-free immutable GitHub staging shared by preparation and execution.
use crate::{
    snapshot_directory, stage_regression_input, DirectorySnapshot, RegressionInputKind,
    StagedRegressionInput,
};
use anyhow::{anyhow, bail, Context, Result};
use reqwest::{redirect::Policy, Url};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::{io::AsyncWriteExt, sync::Semaphore};

const MAX_COMPRESSED: u64 = 256 * 1024 * 1024;
const MAX_EXPANDED: u64 = 512 * 1024 * 1024;
const MAX_ENTRIES: usize = 100_000;
static DOWNLOAD_SLOT: Semaphore = Semaphore::const_new(1);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GithubSnapshotReference {
    pub repository: String,
    pub commit: String,
    pub subdirectory: String,
}

impl GithubSnapshotReference {
    pub fn from_commit_url(reference: &str, subdirectory: &str) -> Result<Self> {
        let url = Url::parse(reference)
            .context("artifact_reference_invalid: expected an exact public GitHub commit URL")?;
        let parts: Vec<_> = url.path().trim_start_matches('/').split('/').collect();
        let component = |part: &str| {
            !part.is_empty()
                && part.len() <= 100
                && part != "."
                && part != ".."
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        };
        if url.scheme() != "https"
            || url.host_str() != Some("github.com")
            || url.port().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || parts.len() != 4
            || !component(parts[0])
            || !component(parts[1])
            || parts[2] != "commit"
            || parts[3].len() != 40
            || !parts[3].bytes().all(|b| b.is_ascii_hexdigit())
            || !(subdirectory == "." || safe_path(subdirectory))
        {
            bail!("artifact_reference_invalid: use an exact public GitHub commit and a normalized source_subdirectory");
        }
        Ok(Self {
            repository: format!("{}/{}", parts[0], parts[1]),
            commit: parts[3].to_ascii_lowercase(),
            subdirectory: subdirectory.into(),
        })
    }
}

fn safe_path(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 4096
        && !value.starts_with('/')
        && !value.contains('\\')
        && !value.chars().any(char::is_control)
        && value
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != ".." && !part.contains(':'))
}

fn extract_zip(
    archive: &Path,
    destination: &Path,
    subdirectory: &str,
    max_bytes: u64,
    max_files: u32,
    max_archive_bytes: u64,
) -> Result<()> {
    let mut zip =
        zip::ZipArchive::new(fs::File::open(archive)?).context("artifact_archive_invalid")?;
    if zip.len() > MAX_ENTRIES {
        bail!("artifact_archive_entry_limit");
    }
    let mut total = 0u64;
    let mut selected_bytes = 0u64;
    let mut selected_files = 0u32;
    let mut selected_entries = 0u64;
    let mut names = HashSet::new();
    let mut root = None;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).context("artifact_archive_invalid")?;
        let path = std::str::from_utf8(entry.name_raw())
            .context("artifact_archive_non_utf8_path")?
            .trim_end_matches('/')
            .to_owned();
        if !safe_path(&path) || !names.insert(path.clone()) {
            bail!("artifact_archive_unsafe_path");
        }
        total = total
            .checked_add(entry.size())
            .ok_or_else(|| anyhow!("artifact_archive_expansion_limit"))?;
        if total > max_archive_bytes {
            bail!("artifact_archive_expansion_limit");
        }
        let (archive_root, relative) = path.split_once('/').unwrap_or((&path, ""));
        if root.as_deref().is_some_and(|r| r != archive_root) {
            bail!("artifact_archive_multiple_roots");
        }
        root = Some(archive_root.to_owned());
        let selected = if subdirectory == "." {
            relative
        } else if relative == subdirectory {
            ""
        } else {
            let Some(selected) = relative
                .strip_prefix(subdirectory)
                .and_then(|p| p.strip_prefix('/'))
            else {
                continue;
            };
            selected
        };
        if selected.is_empty() {
            continue;
        }
        selected_entries += 1;
        if selected_entries > u64::from(max_files) * 4 + 100 {
            bail!("artifact_archive_entry_limit");
        }
        if entry
            .unix_mode()
            .is_some_and(|m| !matches!(m & 0o170000, 0 | 0o100000 | 0o040000))
        {
            bail!("artifact_archive_unsafe_member");
        }
        let target = destination.join(selected);
        if entry.is_dir() {
            fs::create_dir_all(&target)?;
            continue;
        }
        selected_files = selected_files
            .checked_add(1)
            .ok_or_else(|| anyhow!("artifact_file_limit"))?;
        selected_bytes = selected_bytes
            .checked_add(entry.size())
            .ok_or_else(|| anyhow!("artifact_size_limit"))?;
        if selected_files > max_files || selected_bytes > max_bytes {
            bail!("artifact_snapshot_exceeds_committed_limits");
        }
        fs::create_dir_all(target.parent().context("artifact_archive_unsafe_path")?)?;
        let executable = entry.unix_mode().is_some_and(|m| m & 0o111 != 0);
        let declared = entry.size();
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)?;
        let count = std::io::copy(&mut (&mut entry).take(declared + 1), &mut output)
            .context("artifact_archive_invalid_bytes")?;
        output.flush()?;
        if count != declared {
            bail!("artifact_archive_size_mismatch");
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(
                &target,
                fs::Permissions::from_mode(if executable { 0o555 } else { 0o444 }),
            )?;
        }
        #[cfg(not(unix))]
        let _ = executable;
    }
    if selected_files == 0 {
        bail!("artifact_snapshot_empty");
    }
    Ok(())
}

pub struct DownloadedGithubSnapshot {
    _temporary: tempfile::TempDir,
    pub path: PathBuf,
    pub snapshot: DirectorySnapshot,
}

pub async fn download_github_snapshot(
    reference: &GithubSnapshotReference,
    max_bytes: u64,
    max_files: u32,
    kind: RegressionInputKind,
) -> Result<DownloadedGithubSnapshot> {
    if max_bytes == 0 || max_bytes > MAX_EXPANDED || max_files == 0 || max_files > 50_000 {
        bail!("artifact_limits_invalid");
    }
    // Validate even typed/internal callers before constructing an outbound request.
    let checked = GithubSnapshotReference::from_commit_url(
        &format!(
            "https://github.com/{}/commit/{}",
            reference.repository, reference.commit
        ),
        &reference.subdirectory,
    )?;
    let _permit = DOWNLOAD_SLOT.try_acquire().map_err(|_| {
        anyhow!("artifact_check_unavailable: another snapshot is being checked; retry later")
    })?;
    let temporary = tempfile::Builder::new()
        .prefix("agent-bounties-snapshot-")
        .tempdir()?;
    let archive = temporary.path().join("snapshot.zip");
    let mut file = tokio::fs::File::create(&archive).await?;
    let client = reqwest::Client::builder()
        .redirect(Policy::none())
        .timeout(Duration::from_secs(90))
        .build()?;
    let url = format!(
        "https://codeload.github.com/{}/zip/{}",
        checked.repository, checked.commit
    );
    let mut response = client
        .get(url)
        .header("User-Agent", "agent-bounties-immutable-snapshot/1")
        .send()
        .await
        .map_err(|_| anyhow!("artifact_check_unavailable: public GitHub download failed"))?;
    if !response.status().is_success() {
        bail!(
            "artifact_check_unavailable: public GitHub returned HTTP {}",
            response.status().as_u16()
        );
    }
    let max_compressed = if kind == RegressionInputKind::Benchmark {
        MAX_COMPRESSED / 2
    } else {
        MAX_COMPRESSED
    };
    let max_archive_bytes = if kind == RegressionInputKind::Benchmark {
        MAX_EXPANDED / 2
    } else {
        MAX_EXPANDED
    };
    let mut received = 0u64;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| anyhow!("artifact_check_unavailable: incomplete public GitHub download"))?
    {
        received = received
            .checked_add(chunk.len() as u64)
            .ok_or_else(|| anyhow!("artifact_download_limit"))?;
        if received > max_compressed {
            bail!("artifact_download_limit");
        }
        file.write_all(&chunk).await?;
    }
    file.flush().await?;
    drop(file);
    tokio::task::spawn_blocking(move || {
        // Keep the capacity permit through extraction, even if the caller disconnects.
        let _permit = _permit;
        let path = temporary.path().join("files");
        fs::create_dir(&path)?;
        extract_zip(
            &archive,
            &path,
            &checked.subdirectory,
            max_bytes,
            max_files,
            max_archive_bytes,
        )?;
        let snapshot = snapshot_directory(&path, max_bytes, max_files)?;
        Ok(DownloadedGithubSnapshot {
            _temporary: temporary,
            path,
            snapshot,
        })
    })
    .await
    .context("artifact_check_unavailable: snapshot task failed")?
}

pub async fn stage_github_snapshot(
    reference: &GithubSnapshotReference,
    staging_root: &Path,
    kind: RegressionInputKind,
    max_bytes: u64,
    max_files: u32,
) -> Result<StagedRegressionInput> {
    let downloaded = download_github_snapshot(reference, max_bytes, max_files, kind).await?;
    let staging_root = staging_root.to_owned();
    tokio::task::spawn_blocking(move || {
        let staged =
            stage_regression_input(&downloaded.path, &staging_root, kind, max_bytes, max_files);
        drop(downloaded);
        staged
    })
    .await?
}

/// No cache is used: every preparation inspects its exact immutable bytes.
pub async fn preflight_regression_artifact(
    benchmark: &serde_json::Value,
    artifact_reference: &str,
    evidence: &mut serde_json::Value,
) -> Result<Option<serde_json::Value>> {
    if benchmark["engine"] != "sandboxed_regression_v1" {
        return Ok(None);
    }
    let profile = verifier_sdk::approved_regression_profile(benchmark)?;
    let policy: verifier_sdk::RegressionSandboxPolicy =
        serde_json::from_value(profile.runner_manifest.clone())?;
    let object = evidence
        .as_object_mut()
        .ok_or_else(|| anyhow!("artifact_evidence_invalid: expected an object"))?;
    let subdirectory = match object.get("source_subdirectory") {
        None => ".",
        Some(value) => value.as_str().ok_or_else(|| {
            anyhow!("artifact_evidence_invalid: source_subdirectory must be text")
        })?,
    };
    let reference =
        GithubSnapshotReference::from_commit_url(artifact_reference.trim(), subdirectory)?;
    for key in ["commit", "commit_sha"] {
        if object.get(key).is_some_and(|value| {
            !value
                .as_str()
                .is_some_and(|value| value.eq_ignore_ascii_case(&reference.commit))
        }) {
            bail!("artifact_evidence_invalid: supplied commit differs from artifact_reference");
        }
    }
    if object.get("repository").is_some_and(|value| {
        !value
            .as_str()
            .is_some_and(|value| value.eq_ignore_ascii_case(&reference.repository))
    }) {
        bail!("artifact_evidence_invalid: supplied repository differs from artifact_reference");
    }
    let downloaded = download_github_snapshot(
        &reference,
        policy.max_source_bytes,
        policy.max_source_files,
        RegressionInputKind::Source,
    )
    .await?;
    bind_snapshot_evidence(object, &downloaded.snapshot)?;
    Ok(Some(
        serde_json::json!({"artifact_reference":artifact_reference.trim(),"repository":reference.repository,"commit":reference.commit,"source_subdirectory":reference.subdirectory,"source_snapshot_digest":downloaded.snapshot.digest,"file_count":downloaded.snapshot.file_count,"total_bytes":downloaded.snapshot.total_bytes,"profile_id":profile.id,"profile_registry_digest":verifier_sdk::regression_profile_registry_digest(),"claim":"Exact source bytes were inspected. This is not a test result, acceptance or payment."}),
    ))
}

fn bind_snapshot_evidence(
    object: &mut serde_json::Map<String, serde_json::Value>,
    snapshot: &DirectorySnapshot,
) -> Result<()> {
    if object
        .get("source_snapshot_digest")
        .is_some_and(|digest| digest.as_str() != Some(&snapshot.digest))
    {
        bail!("artifact_digest_mismatch: supplied source_snapshot_digest does not match the exact commit and files; no signing payload prepared");
    }
    object.insert(
        "source_snapshot_digest".into(),
        serde_json::json!(snapshot.digest),
    );
    Ok(())
}

/// Safe public errors exclude host paths, provider responses and private evidence.
pub fn public_preflight_error(error: &anyhow::Error) -> (&'static str, &'static str, bool) {
    let message = error.to_string();
    if message.starts_with("artifact_digest_mismatch:") {
        ("artifact_digest_mismatch", "The supplied file hash does not match the exact commit and folder. Check the files before signing.", false)
    } else if message.starts_with("artifact_reference_invalid:")
        || message.starts_with("artifact_evidence_invalid:")
    {
        (
            "artifact_reference_invalid",
            "Use a full public GitHub commit URL and a valid source_subdirectory.",
            false,
        )
    } else if message.starts_with("artifact_archive_")
        || message.starts_with("artifact_snapshot_")
        || message.starts_with("artifact_download_limit")
    {
        (
            "artifact_snapshot_invalid",
            "The source archive is empty, unsafe, damaged or exceeds the published limits.",
            false,
        )
    } else {
        ("artifact_check_unavailable", "Files could not be checked. No signing payload or verdict was created. Try again when verification is available.", true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zip::write::SimpleFileOptions;
    fn archive(root: &Path, names: &[(&str, &[u8])]) -> PathBuf {
        let path = root.join("input.zip");
        let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
        for (name, data) in names {
            zip.start_file(*name, SimpleFileOptions::default()).unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap();
        path
    }
    #[test]
    fn exact_public_commit_only() {
        let url = format!("https://github.com/owner/repo/commit/{}", "a".repeat(40));
        assert!(GithubSnapshotReference::from_commit_url(&url, ".").is_ok());
        for value in [
            url.replace("github.com", "github.com.evil.test"),
            format!("{url}?x=y"),
            format!("{url}/"),
            url.replace("/commit/", "/tree/"),
            url.replace("https://", "https://user@"),
            url.replace(&"a".repeat(40), "main"),
        ] {
            assert!(GithubSnapshotReference::from_commit_url(&value, ".").is_err());
        }
        for path in ["../escape", "/absolute", "a//b", "a/./b", "a\\b", "a/b/"] {
            assert!(GithubSnapshotReference::from_commit_url(&url, path).is_err());
        }
    }
    #[test]
    fn selected_bytes_use_the_worker_directory_digest_and_limits() {
        let temp = tempfile::tempdir().unwrap();
        let output = temp.path().join("files");
        fs::create_dir(&output).unwrap();
        let zip = archive(
            temp.path(),
            &[
                ("root/other/data", b"unselected"),
                ("root/work/result.txt", b"exact bytes"),
            ],
        );
        extract_zip(&zip, &output, "work", 11, 1, MAX_EXPANDED).unwrap();
        let digest = snapshot_directory(&output, 11, 1).unwrap();
        assert_eq!(digest.file_count, 1);
        assert_eq!(digest.total_bytes, 11);
        let source = temp.path().join("independent");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("result.txt"), b"exact bytes").unwrap();
        assert_eq!(digest, snapshot_directory(&source, 11, 1).unwrap());
        fs::write(source.join("result.txt"), b"wrong bytes").unwrap();
        assert_ne!(digest, snapshot_directory(&source, 11, 1).unwrap());
        let output2 = temp.path().join("limited");
        fs::create_dir(&output2).unwrap();
        assert!(extract_zip(&zip, &output2, "work", 10, 1, MAX_EXPANDED).is_err());
    }
    #[test]
    fn symlinks_and_unselected_expansion_are_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let output = temp.path().join("files");
        fs::create_dir(&output).unwrap();
        let archive_path = temp.path().join("links.zip");
        let mut zip = zip::ZipWriter::new(fs::File::create(&archive_path).unwrap());
        zip.add_symlink("root/link", "../escape", SimpleFileOptions::default())
            .unwrap();
        zip.finish().unwrap();
        assert!(extract_zip(&archive_path, &output, ".", 100, 10, MAX_EXPANDED).is_err());
        let zip = archive(
            temp.path(),
            &[("root/unselected", b"large file"), ("root/work/a", b"a")],
        );
        assert!(extract_zip(&zip, &output, "work", 100, 10, 8).is_err());
    }

    #[tokio::test]
    async fn mismatched_commit_evidence_fails_before_network_or_signature() {
        let profile = &verifier_sdk::regression_profile_registry()
            .unwrap()
            .profiles[0];
        let benchmark = serde_json::json!({"engine":"sandboxed_regression_v1","source":profile.sources[0],"runner_manifest":profile.runner_manifest});
        let mut evidence = serde_json::json!({"commit_sha":"b".repeat(40)});
        let error = preflight_regression_artifact(
            &benchmark,
            &format!("https://github.com/owner/repo/commit/{}", "a".repeat(40)),
            &mut evidence,
        )
        .await
        .unwrap_err();
        assert!(error.to_string().starts_with("artifact_evidence_invalid:"));
        assert_eq!(evidence, serde_json::json!({"commit_sha":"b".repeat(40)}));
    }

    #[test]
    fn only_exact_inspected_bytes_can_be_bound_to_evidence() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("result.txt"), b"original").unwrap();
        let original = snapshot_directory(temp.path(), 100, 1).unwrap();
        let mut evidence = serde_json::json!({"notes":"unchanged"});
        bind_snapshot_evidence(evidence.as_object_mut().unwrap(), &original).unwrap();
        assert_eq!(evidence["source_snapshot_digest"], original.digest);
        assert_eq!(evidence["notes"], "unchanged");
        let bound = evidence.clone();
        bind_snapshot_evidence(evidence.as_object_mut().unwrap(), &original).unwrap();
        assert_eq!(evidence, bound);
        fs::write(temp.path().join("result.txt"), b"modified").unwrap();
        let modified = snapshot_directory(temp.path(), 100, 1).unwrap();
        let error =
            bind_snapshot_evidence(evidence.as_object_mut().unwrap(), &modified).unwrap_err();
        assert_eq!(public_preflight_error(&error).0, "artifact_digest_mismatch");
        assert_eq!(evidence, bound);
        for invalid in [
            serde_json::Value::Null,
            serde_json::json!(123),
            serde_json::json!("sha256:wrong"),
        ] {
            let mut evidence = serde_json::json!({"source_snapshot_digest":invalid});
            let before = evidence.clone();
            assert!(bind_snapshot_evidence(evidence.as_object_mut().unwrap(), &original).is_err());
            assert_eq!(evidence, before);
        }
    }

    #[test]
    fn unsafe_archive_paths_and_multiple_roots_fail_closed() {
        for names in [
            vec![("root/../escape", &b"x"[..])],
            vec![("root/a", &b"x"[..]), ("other/b", &b"y"[..])],
            vec![("root/a\\b", &b"x"[..])],
        ] {
            let temp = tempfile::tempdir().unwrap();
            let output = temp.path().join("files");
            fs::create_dir(&output).unwrap();
            let zip = archive(temp.path(), &names);
            assert!(extract_zip(&zip, &output, ".", 100, 10, MAX_EXPANDED).is_err());
        }
    }
}
