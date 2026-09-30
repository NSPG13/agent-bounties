use base64::{engine::general_purpose::STANDARD, Engine};
use broad_verification::{Manifest, Plan};
use clap::{Args, Subcommand};
use serde_json::{json, Value};
use std::{collections::BTreeMap, path::PathBuf};
#[derive(Debug, Args)]
pub struct HostedOptions {
    #[arg(
        long,
        default_value = "https://api.agentbounties.app",
        env = "AGENT_BOUNTIES_API_URL"
    )]
    api_url: String,
}
impl HostedOptions {
    async fn request(
        &self,
        path: &str,
        body: Option<Value>,
        key: Option<&str>,
    ) -> anyhow::Result<Value> {
        let base = reqwest::Url::parse(&self.api_url)?;
        anyhow::ensure!(
            base.username().is_empty()
                && base.password().is_none()
                && base.query().is_none()
                && base.fragment().is_none(),
            "invalid API URL"
        );
        anyhow::ensure!(
            base.scheme() == "https"
                || (base.scheme() == "http"
                    && matches!(base.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))),
            "API URL must use HTTPS, except local tests"
        );
        let token=std::env::var("AGENT_BOUNTIES_SESSION_TOKEN").map_err(|_|anyhow::anyhow!("Sign in on the first-party website, then set AGENT_BOUNTIES_SESSION_TOKEN to your wallet session. Never use a wallet private key."))?;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(180))
            .build()?;
        let url = base.join(path)?;
        let mut request = match body {
            Some(body) => client.post(url).json(&body),
            None => client.get(url),
        }
        .bearer_auth(token);
        if let Some(key) = key {
            request = request.header("Idempotency-Key", key);
        }
        let response = request.send().await?;
        anyhow::ensure!(response.status().is_success(),"Request could not finish ({}). Keep the same idempotency key when retrying; no transaction was sent.",response.status());
        Ok(response.json().await?)
    }
}
fn read_plan(path: PathBuf) -> anyhow::Result<Plan> {
    let bytes = std::fs::read(path)?;
    anyhow::ensure!(bytes.len() <= 262144, "plan exceeds 256 KiB");
    let plan: Plan = serde_json::from_value(broad_verification::parse_json(&bytes)?)?;
    broad_verification::validate_plan(&plan)?;
    Ok(plan)
}
#[derive(Debug, Subcommand)]
pub enum VerificationCommand {
    /// Print versioned check claims, limits and machine-readable plan schema.
    Catalog,
    /// Upload exact bytes privately. Retry with the same key; this does not submit on-chain.
    Upload {
        directory: PathBuf,
        #[arg(long)]
        idempotency_key: String,
        #[command(flatten)]
        hosted: HostedOptions,
    },
    /// Queue the shared runner on stored files. Uses your session, never an operator token.
    Run {
        artifact_id: uuid::Uuid,
        plan: PathBuf,
        #[command(flatten)]
        hosted: HostedOptions,
    },
    /// Read a previously queued run after a restart.
    Result {
        id: uuid::Uuid,
        #[command(flatten)]
        hosted: HostedOptions,
    },
    /// List review tasks assigned to your account or wallet.
    Tasks {
        #[command(flatten)]
        hosted: HostedOptions,
    },
    /// Validate every criterion's check or review mapping.
    Validate { plan: PathBuf },
    /// Run local file checks; published tests require the hosted bounded runner.
    Check { plan: PathBuf, directory: PathBuf },
}
fn collect(
    root: &std::path::Path,
    dir: &std::path::Path,
    files: &mut BTreeMap<String, Vec<u8>>,
    size: &mut u64,
) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        anyhow::ensure!(!kind.is_symlink(), "symlinks are not submission files");
        if kind.is_dir() {
            collect(root, &entry.path(), files, size)?;
        } else {
            anyhow::ensure!(kind.is_file(), "unsupported file kind");
            *size = size
                .checked_add(entry.metadata()?.len())
                .ok_or_else(|| anyhow::anyhow!("size overflow"))?;
            anyhow::ensure!(
                *size <= broad_verification::MAX_UPLOAD as u64
                    && files.len() < broad_verification::MAX_FILES,
                "submission exceeds limits"
            );
            let path = entry
                .path()
                .strip_prefix(root)?
                .to_string_lossy()
                .replace('\\', "/");
            anyhow::ensure!(broad_verification::safe_path(&path), "unsafe path");
            files.insert(path, std::fs::read(entry.path())?);
        }
    }
    Ok(())
}
pub async fn execute(command: VerificationCommand) -> anyhow::Result<()> {
    let output = match command {
        VerificationCommand::Catalog => broad_verification::catalog(),
        VerificationCommand::Upload {
            directory,
            idempotency_key,
            hosted,
        } => {
            let mut files = BTreeMap::new();
            collect(&directory, &directory, &mut files, &mut 0)?;
            Manifest::from_files(&files)?;
            let files: Vec<_> = files
                .into_iter()
                .map(|(path, bytes)| json!({"path":path,"base64":STANDARD.encode(bytes)}))
                .collect();
            hosted
                .request(
                    "/v1/verification/artifacts",
                    Some(json!({"files":files})),
                    Some(&idempotency_key),
                )
                .await?
        }
        VerificationCommand::Run {
            artifact_id,
            plan,
            hosted,
        } => {
            hosted
                .request(
                    "/v1/verification/runs",
                    Some(json!({"artifact_id":artifact_id,"plan":read_plan(plan)?})),
                    None,
                )
                .await?
        }
        VerificationCommand::Result { id, hosted } => {
            hosted
                .request(&format!("/v1/verification/runs/{id}"), None, None)
                .await?
        }
        VerificationCommand::Tasks { hosted } => {
            hosted
                .request("/v1/verification/review-tasks", None, None)
                .await?
        }
        VerificationCommand::Validate { plan } => {
            let plan = read_plan(plan)?;
            broad_verification::validate_plan(&plan)?;
            serde_json::json!({"valid":true,"plan_hash":broad_verification::value_digest(&plan)?,"payment_authorized":false})
        }
        VerificationCommand::Check { plan, directory } => {
            let plan = read_plan(plan)?;
            let mut files = BTreeMap::new();
            collect(&directory, &directory, &mut files, &mut 0)?;
            let manifest = Manifest::from_files(&files)?;
            serde_json::to_value(broad_verification::run(&plan, &manifest, &files)?)?
        }
    };
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}
