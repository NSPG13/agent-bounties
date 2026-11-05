//! S3/R2 evidence storage. Credentials never enter artifacts, reports or errors.
use anyhow::{anyhow, Result};
use broad_verification::{digest, safe_path, Manifest, MAX_EXPANDED, MAX_FILES, MAX_UPLOAD};
use object_store::{aws::AmazonS3Builder, path::Path, Certificate, ClientOptions, ObjectStore};
use std::{
    collections::BTreeMap,
    io::{Cursor, Read},
    sync::Arc,
};
use uuid::Uuid;

#[derive(Clone)]
pub struct ArtifactStore {
    store: Arc<dyn ObjectStore>,
}
impl ArtifactStore {
    pub fn from_env() -> Result<Self> {
        let required = |name: &str| {
            std::env::var(name)
                .ok()
                .filter(|v| !v.is_empty())
                .ok_or_else(|| anyhow!("artifact_storage_not_configured"))
        };
        let endpoint = required("ARTIFACT_S3_ENDPOINT")?;
        if !endpoint.starts_with("https://") {
            return Err(anyhow!("artifact_storage_requires_https"));
        }
        let mut options = ClientOptions::new();
        // Private S3 installations and the local integration fixture may use
        // their own CA. Certificate and hostname validation remain enabled.
        if let Ok(pem) = std::env::var("ARTIFACT_S3_CA_CERTIFICATE") {
            if pem.len() > 65_536 {
                return Err(anyhow!("artifact_storage_configuration_invalid"));
            }
            options = options.with_root_certificate(
                Certificate::from_pem(pem.as_bytes())
                    .map_err(|_| anyhow!("artifact_storage_configuration_invalid"))?,
            );
        }
        let store = AmazonS3Builder::new()
            .with_client_options(options)
            .with_endpoint(endpoint)
            .with_region(std::env::var("ARTIFACT_S3_REGION").unwrap_or_else(|_| "auto".into()))
            .with_bucket_name(required("ARTIFACT_S3_BUCKET")?)
            .with_access_key_id(required("ARTIFACT_S3_ACCESS_KEY_ID")?)
            .with_secret_access_key(required("ARTIFACT_S3_SECRET_ACCESS_KEY")?)
            .build()
            .map_err(|_| anyhow!("artifact_storage_configuration_invalid"))?;
        Ok(Self {
            store: Arc::new(store),
        })
    }
    fn key(id: Uuid, hash: &str) -> Path {
        Path::from(format!(
            "submissions/{id}/{}",
            hash.trim_start_matches("sha256:")
        ))
    }
    pub async fn put(
        &self,
        id: Uuid,
        manifest: &Manifest,
        files: &BTreeMap<String, Vec<u8>>,
    ) -> Result<()> {
        manifest.verify(files)?;
        for file in &manifest.files {
            let path = Self::key(id, &file.sha256);
            self.store
                .put(&path, files[&file.path].clone().into())
                .await
                .map_err(|_| anyhow!("artifact_write_unavailable"))?;
            let bytes = self.get_file(id, &file.sha256, file.size).await?;
            if digest(&bytes) != file.sha256 {
                return Err(anyhow!("artifact_readback_mismatch"));
            }
        }
        Ok(())
    }
    pub async fn get_file(&self, id: Uuid, hash: &str, size: u64) -> Result<Vec<u8>> {
        if !broad_verification::valid_hash(hash) || size > MAX_UPLOAD as u64 {
            return Err(anyhow!("invalid_artifact_reference"));
        }
        let result = self
            .store
            .get(&Self::key(id, hash))
            .await
            .map_err(|_| anyhow!("artifact_read_unavailable"))?;
        if result.meta.size != size {
            return Err(anyhow!("artifact_size_mismatch"));
        }
        let bytes = result
            .bytes()
            .await
            .map_err(|_| anyhow!("artifact_read_unavailable"))?;
        if bytes.len() as u64 != size || digest(&bytes) != hash {
            return Err(anyhow!("artifact_digest_mismatch"));
        }
        Ok(bytes.to_vec())
    }
    pub async fn get(&self, id: Uuid, manifest: &Manifest) -> Result<BTreeMap<String, Vec<u8>>> {
        manifest.validate()?;
        let mut files = BTreeMap::new();
        for file in &manifest.files {
            files.insert(
                file.path.clone(),
                self.get_file(id, &file.sha256, file.size).await?,
            );
        }
        Ok(files)
    }
    pub async fn delete(&self, id: Uuid, manifest: &Manifest) -> Result<()> {
        let mut deleted = std::collections::BTreeSet::new();
        for file in &manifest.files {
            if !deleted.insert(&file.sha256) {
                continue;
            }
            match self.store.delete(&Self::key(id, &file.sha256)).await {
                Ok(()) | Err(object_store::Error::NotFound { .. }) => (),
                Err(_) => return Err(anyhow!("artifact_delete_unavailable")),
            }
        }
        Ok(())
    }
    #[cfg(test)]
    pub(crate) fn memory() -> Self {
        Self {
            store: Arc::new(object_store::memory::InMemory::new()),
        }
    }
}

/// Inspect ZIP members without extracting them onto the host. Archives remain exact
/// stored source bytes; derived files are never silently substituted for them.
pub fn validate_archive(path: &str, bytes: &[u8]) -> Result<u64> {
    if !path.to_ascii_lowercase().ends_with(".zip") && !bytes.starts_with(b"PK\x03\x04") {
        return Ok(bytes.len() as u64);
    }
    let mut archive =
        zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| anyhow!("invalid_zip"))?;
    if archive.len() > MAX_FILES {
        return Err(anyhow!("archive_file_limit"));
    }
    let mut header_reader = Cursor::new(bytes);
    let mut names = std::collections::BTreeSet::new();
    let mut expanded = 0u64;
    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|_| anyhow!("unsupported_or_encrypted_zip"))?;
        crate::zip_headers::require_matching_name(
            &mut header_reader,
            file.header_start(),
            file.name_raw(),
        )
        .map_err(|_| anyhow!("unsafe_archive_headers"))?;
        let name = file.name().trim_end_matches('/');
        if !safe_path(name)
            || !names.insert(name.to_owned())
            || file
                .unix_mode()
                .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err(anyhow!("unsafe_archive_member"));
        }
        expanded = expanded
            .checked_add(file.size())
            .ok_or_else(|| anyhow!("archive_expansion_limit"))?;
        if expanded > MAX_EXPANDED as u64 {
            return Err(anyhow!("archive_expansion_limit"));
        }
        if !file.is_dir() {
            let declared = file.size();
            let actual = std::io::copy(&mut (&mut file).take(declared + 1), &mut std::io::sink())
                .map_err(|_| anyhow!("invalid_archive_member"))?;
            if actual != declared {
                return Err(anyhow!("archive_size_mismatch"));
            }
        }
    }
    Ok(expanded)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Explicit deployment check for the configured private bucket. This only
    /// touches its own random artifact ID; it never lists or cleans the bucket.
    #[tokio::test]
    #[ignore = "requires approved bucket credentials and ARTIFACT_STORAGE_SMOKE=1"]
    async fn configured_private_storage_round_trip() -> Result<()> {
        anyhow::ensure!(
            std::env::var("ARTIFACT_STORAGE_SMOKE").as_deref() == Ok("1"),
            "storage_smoke_requires_explicit_opt_in"
        );
        let store = ArtifactStore::from_env()?;
        let id = Uuid::new_v4();
        let files = BTreeMap::from([
            ("probe.bin".into(), vec![0, 255, 37, 10]),
            (
                "probe.txt".into(),
                b"Disposable Agentizable storage connectivity check.\n".to_vec(),
            ),
        ]);
        let manifest = Manifest::from_files(&files)?;
        let result: Result<()> = async {
            store.put(id, &manifest, &files).await?;
            // Reconstruct the client to exercise persisted bytes rather than
            // any state retained by the uploading client.
            let reader = ArtifactStore::from_env()?;
            anyhow::ensure!(reader.get(id, &manifest).await? == files, "probe_mismatch");
            anyhow::ensure!(
                reader.get(Uuid::new_v4(), &manifest).await.is_err(),
                "probe_crossed_artifact_ids"
            );
            Ok(())
        }
        .await;
        // Clean up even when a partial upload or read fails. Never remove
        // another artifact; surface failures without exposing credential data.
        let cleanup = store.delete(id, &manifest).await;
        if cleanup.is_err() {
            return Err(anyhow!("probe_cleanup_failed: artifact {id}"));
        }
        result?;
        for file in &manifest.files {
            anyhow::ensure!(
                matches!(
                    store.store.get(&ArtifactStore::key(id, &file.sha256)).await,
                    Err(object_store::Error::NotFound { .. })
                ),
                "probe_cleanup_not_confirmed: artifact {id}"
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn exact_bytes_survive_upload_and_cannot_cross_artifact_ids() {
        let store = ArtifactStore::memory();
        let id = Uuid::new_v4();
        let files = BTreeMap::from([
            ("design.bin".into(), vec![0, 255, 37]),
            ("notes.txt".into(), b"source notes".to_vec()),
        ]);
        let manifest = Manifest::from_files(&files).unwrap();
        store.put(id, &manifest, &files).await.unwrap();
        assert_eq!(store.get(id, &manifest).await.unwrap(), files);
        assert!(store.get(Uuid::new_v4(), &manifest).await.is_err());
        store.delete(id, &manifest).await.unwrap();
        assert!(store.get(id, &manifest).await.is_err());
    }
    #[test]
    fn retained_upload_rejects_contradictory_zip_names() {
        use std::io::Write;
        for method in [
            zip::CompressionMethod::Stored,
            zip::CompressionMethod::Deflated,
        ] {
            let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
            writer
                .start_file(
                    "report.pdf",
                    zip::write::SimpleFileOptions::default().compression_method(method),
                )
                .unwrap();
            writer.write_all(b"exact retained payload").unwrap();
            let mut bytes = writer.finish().unwrap().into_inner();
            assert_eq!(validate_archive("work.zip", &bytes).unwrap(), 22);
            let offset = {
                let mut archive = zip::ZipArchive::new(Cursor::new(&bytes)).unwrap();
                let offset = archive.by_index(0).unwrap().header_start() as usize;
                offset
            };
            // Only the local name changes; central metadata and payload stay valid.
            bytes[offset + 30..offset + 40].copy_from_slice(b"worker.sh!");
            let mut archive = zip::ZipArchive::new(Cursor::new(&bytes)).unwrap();
            assert_eq!(archive.by_index(0).unwrap().name_raw(), b"report.pdf");
            assert_eq!(
                validate_archive("work.zip", &bytes)
                    .unwrap_err()
                    .to_string(),
                "unsafe_archive_headers"
            );
        }
    }

    #[test]
    fn unsafe_archive_paths_rejected_without_extraction() {
        use std::io::Write;
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        zip.start_file("../escape", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"bad").unwrap();
        let bytes = zip.finish().unwrap().into_inner();
        assert!(validate_archive("work.zip", &bytes).is_err());
    }
}
