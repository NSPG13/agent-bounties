//! An isolated liveness server for a new deployment before dependency admission.
//! This module never constructs AppState, connects to a store or starts a worker.
use axum::{
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use std::env;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BootMode {
    Normal,
    RecoveryHealthOnly,
    RecoveryReadOnly,
}

impl BootMode {
    pub(crate) fn from_env() -> anyhow::Result<Self> {
        match env::var("API_BOOT_MODE") {
            Ok(value) => Self::parse(Some(&value)),
            Err(env::VarError::NotPresent) => Self::parse(None),
            Err(env::VarError::NotUnicode(_)) => anyhow::bail!("API_BOOT_MODE is invalid"),
        }
    }

    fn parse(value: Option<&str>) -> anyhow::Result<Self> {
        match value {
            None | Some("normal") => Ok(Self::Normal),
            Some("recovery-health-only") => Ok(Self::RecoveryHealthOnly),
            Some("recovery-read-only") => Ok(Self::RecoveryReadOnly),
            _ => anyhow::bail!(
                "API_BOOT_MODE must be normal, recovery-health-only or recovery-read-only"
            ),
        }
    }
}

fn contained(mut response: Response) -> Response {
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        "x-agent-bounties-boot-mode",
        HeaderValue::from_static("recovery-health-only"),
    );
    headers.insert(
        "x-agent-bounties-x402-relay",
        HeaderValue::from_static("disabled"),
    );
    response
}

async fn health() -> Response {
    contained(crate::health_response(&crate::deployment_revision()).into_response())
}

async fn unavailable() -> Response {
    contained(
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "schema": "agentizable/recovery-boot-v1",
                "ready": false,
                "status": "unavailable",
                "reason": "recovery_health_only"
            })),
        )
            .into_response(),
    )
}

fn router() -> Router {
    Router::new()
        .route("/health", get(health).fallback(unavailable))
        .fallback(unavailable)
}

pub(crate) async fn serve() -> anyhow::Result<()> {
    anyhow::ensure!(
        env::args_os().len() == 1,
        "Recovery health-only mode does not accept command arguments"
    );
    let bind_addr = crate::service_bind_addr(
        env::var("API_BIND_ADDR").ok().as_deref(),
        env::var("PORT").ok().as_deref(),
        "127.0.0.1:8080",
    );
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    axum::serve(listener, router()).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boot_mode_is_explicit_and_rejects_misspellings() {
        assert_eq!(BootMode::parse(None).unwrap(), BootMode::Normal);
        assert_eq!(BootMode::parse(Some("normal")).unwrap(), BootMode::Normal);
        assert_eq!(
            BootMode::parse(Some("recovery-health-only")).unwrap(),
            BootMode::RecoveryHealthOnly
        );
        assert_eq!(
            BootMode::parse(Some("recovery-read-only")).unwrap(),
            BootMode::RecoveryReadOnly
        );
        for value in [
            "", "true", "false", "recovery", "NORMAL", " normal", "normal ",
        ] {
            assert!(BootMode::parse(Some(value)).is_err());
        }
    }
}
