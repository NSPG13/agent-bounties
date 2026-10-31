//! Explicit wallet authentication commands. Wallet signing stays in the user's wallet.
use anyhow::{bail, Context, Result};
use clap::Subcommand;
use serde_json::{json, Value};
use std::{fs, io::Write, path::PathBuf};

#[derive(Subcommand)]
pub enum AuthCommand {
    /// Print a short-lived SIWE challenge for your wallet to review and sign.
    Challenge {
        #[arg(long)]
        address: String,
        #[arg(long, default_value_t = 8453)]
        chain_id: u64,
        #[arg(long, default_value = "https://api.agentizable.ai")]
        api_base_url: String,
    },
    /// Exchange a signed challenge JSON file and write its bearer token to a new private file.
    Session {
        #[arg(long)]
        request_file: PathBuf,
        #[arg(long)]
        token_file: PathBuf,
        #[arg(long, default_value = "https://api.agentizable.ai")]
        api_base_url: String,
    },
    /// Read the principal for AGENT_BOUNTIES_SESSION_TOKEN.
    Status {
        #[arg(long, default_value = "https://api.agentizable.ai")]
        api_base_url: String,
    },
    /// Revoke AGENT_BOUNTIES_SESSION_TOKEN.
    Revoke {
        #[arg(long, default_value = "https://api.agentizable.ai")]
        api_base_url: String,
    },
}
pub async fn run(command: AuthCommand) -> Result<()> {
    match command {
        AuthCommand::Challenge {
            address,
            chain_id,
            api_base_url,
        } => {
            if !matches!(chain_id, 8453 | 84532) {
                bail!("chain-id must be 8453 or 84532");
            }
            let value = request(
                &api_base_url,
                "/v1/auth/wallet/challenge",
                Some(json!({"address":address,"chain_id":chain_id})),
                false,
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&value)?);
        }
        AuthCommand::Session {
            request_file,
            token_file,
            api_base_url,
        } => {
            if fs::metadata(&request_file)?.len() > 20 * 1024 {
                bail!("authentication request file exceeds 20 KiB");
            }
            let body: Value = serde_json::from_slice(&fs::read(request_file)?)
                .context("request file must contain challenge_id, chain_id and signature JSON")?;
            // Reserve the private destination before redeeming a single-use nonce.
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options
                .open(&token_file)
                .context("token-file must be a new private file")?;
            let value =
                match request(&api_base_url, "/v1/auth/wallet/session", Some(body), false).await {
                    Ok(value) => value,
                    Err(error) => {
                        drop(file);
                        let _ = fs::remove_file(&token_file);
                        return Err(error);
                    }
                };
            let token = value["token"]
                .as_str()
                .filter(|s| valid_token(s))
                .context("invalid session response")?;
            writeln!(file, "{token}")?;
            file.sync_all()?;
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &json!({"token_file":token_file,"expires_at":value["expires_at"],"principal":value["principal"]})
                )?
            );
        }
        AuthCommand::Status { api_base_url } => println!(
            "{}",
            serde_json::to_string_pretty(
                &request(&api_base_url, "/v1/auth/session", None, true).await?
            )?
        ),
        AuthCommand::Revoke { api_base_url } => {
            request(
                &api_base_url,
                "/v1/auth/session/revoke",
                Some(json!({})),
                true,
            )
            .await?;
            println!("Session revoked.");
        }
    }
    Ok(())
}
fn valid_token(token: &str) -> bool {
    token.strip_prefix("abws_").is_some_and(|s| {
        s.len() == 64
            && s.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}
async fn request(
    base: &str,
    path: &str,
    body: Option<Value>,
    authenticated: bool,
) -> Result<Value> {
    let origin = reqwest::Url::parse(base)?;
    if origin.path() != "/"
        || origin.query().is_some()
        || origin.fragment().is_some()
        || origin.password().is_some()
        || !origin.username().is_empty()
        || !(origin.scheme() == "https"
            || (origin.scheme() == "http"
                && matches!(origin.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))))
    {
        bail!("API URL must be an HTTPS origin or local development origin");
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(45))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let mut request = if let Some(body) = body {
        client
            .post(format!("{}{}", base.trim_end_matches('/'), path))
            .json(&body)
    } else {
        client.get(format!("{}{}", base.trim_end_matches('/'), path))
    };
    request = request.header("x-agent-bounties-interface", "cli");
    if authenticated {
        let token = std::env::var("AGENT_BOUNTIES_SESSION_TOKEN")
            .context("set AGENT_BOUNTIES_SESSION_TOKEN from your private session file")?;
        if !valid_token(&token) {
            bail!("AGENT_BOUNTIES_SESSION_TOKEN is invalid");
        }
        let expected = std::env::var("AGENT_BOUNTIES_SESSION_ORIGIN")
            .unwrap_or_else(|_| "https://api.agentizable.ai".into());
        if reqwest::Url::parse(&expected)?.origin() != origin.origin() {
            bail!(
                "session origin mismatch; set AGENT_BOUNTIES_SESSION_ORIGIN to the origin that issued the session"
            );
        }
        request = request.bearer_auth(token);
    }
    let response = request.send().await?;
    let status = response.status();
    if !status.is_success() {
        bail!(
            "authentication request failed with HTTP {}",
            status.as_u16()
        );
    }
    response
        .json()
        .await
        .context("invalid authentication response JSON")
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn token_validation_rejects_headers_and_wrong_lengths() {
        assert!(valid_token(&format!("abws_{}", "a".repeat(64))));
        assert!(!valid_token("abws_\r\nAuthorization: injected"));
        assert!(!valid_token("abws_short"));
    }
}
