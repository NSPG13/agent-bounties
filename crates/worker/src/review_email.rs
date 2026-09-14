//! Privacy-safe review notices. The durable outbox owns recipient authority,
//! payload freezing, leases, and a retry lifetime shorter than Resend's 24 hours.

use chrono::{DateTime, Utc};
use reqwest::{header, redirect::Policy, Client, Response, StatusCode, Url};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{fmt, time::Duration};

const RESEND_ENDPOINT: &str = "https://api.resend.com/emails";
const REVIEW_ORIGIN: &str = "https://agentbounties.app";
const SUBJECT: &str = "A solution is ready for your review";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const RESPONSE_LIMIT: usize = 16 * 1024;
const PAYLOAD_LIMIT: usize = 16 * 1024;

#[derive(Clone)]
pub struct ReviewEmailConfig {
    authorization: header::HeaderValue,
    sender: String,
}

impl fmt::Debug for ReviewEmailConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReviewEmailConfig")
            .field("authorization", &"[redacted]")
            .field("sender", &"[configured]")
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewEmailConfigError {
    InvalidEnabledFlag,
    MissingApiKey,
    InvalidApiKey,
    MissingSender,
    InvalidSender,
    HttpClientUnavailable,
}

impl fmt::Display for ReviewEmailConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidEnabledFlag => "VERIFIER_EMAIL_ENABLED must be true or false",
            Self::MissingApiKey => "enabled verifier email requires RESEND_API_KEY",
            Self::InvalidApiKey => "RESEND_API_KEY has an invalid format",
            Self::MissingSender => {
                "enabled verifier email requires VERIFIER_EMAIL_FROM or AUTH_EMAIL_FROM"
            }
            Self::InvalidSender => "verifier email sender has an invalid format",
            Self::HttpClientUnavailable => "verifier email HTTP client could not be initialized",
        })
    }
}

impl std::error::Error for ReviewEmailConfigError {}

impl ReviewEmailConfig {
    /// Disabled unless explicitly enabled. Configuration errors contain no values.
    pub fn from_env() -> Result<Option<Self>, ReviewEmailConfigError> {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    pub fn from_lookup(
        lookup: impl Fn(&str) -> Option<String>,
    ) -> Result<Option<Self>, ReviewEmailConfigError> {
        let enabled = lookup("VERIFIER_EMAIL_ENABLED").unwrap_or_default();
        match enabled.trim().to_ascii_lowercase().as_str() {
            "" | "false" => return Ok(None),
            "true" => {}
            _ => return Err(ReviewEmailConfigError::InvalidEnabledFlag),
        }
        let key = lookup("RESEND_API_KEY")
            .filter(|value| !value.trim().is_empty())
            .ok_or(ReviewEmailConfigError::MissingApiKey)?;
        let key = key.trim();
        if key.len() > 1024 || !key.bytes().all(|byte| byte.is_ascii_graphic()) {
            return Err(ReviewEmailConfigError::InvalidApiKey);
        }
        let mut authorization = header::HeaderValue::from_str(&format!("Bearer {key}"))
            .map_err(|_| ReviewEmailConfigError::InvalidApiKey)?;
        authorization.set_sensitive(true);
        let sender = lookup("VERIFIER_EMAIL_FROM")
            .filter(|value| !value.trim().is_empty())
            .or_else(|| lookup("AUTH_EMAIL_FROM"))
            .filter(|value| !value.trim().is_empty())
            .ok_or(ReviewEmailConfigError::MissingSender)?;
        let sender = sender.trim().to_owned();
        if !valid_sender(&sender) {
            return Err(ReviewEmailConfigError::InvalidSender);
        }
        Ok(Some(Self {
            authorization,
            sender,
        }))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewEmailInputError {
    InvalidRecipient,
    InvalidContract,
}

impl fmt::Display for ReviewEmailInputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidRecipient => "verifier email recipient has an invalid format",
            Self::InvalidContract => "verifier review contract has an invalid format",
        })
    }
}

impl std::error::Error for ReviewEmailInputError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewEmailFailureCode {
    InvalidStoredPayload,
    InvalidIdempotencyKey,
    Timeout,
    Transport,
    ProviderRateLimited,
    ProviderUnavailable,
    ProviderConcurrentRequest,
    ProviderIdempotencyConflict,
    ProviderRejected,
    ProviderRedirect,
    InvalidProviderResponse,
}

impl ReviewEmailFailureCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidStoredPayload => "invalid_stored_payload",
            Self::InvalidIdempotencyKey => "invalid_idempotency_key",
            Self::Timeout => "provider_timeout",
            Self::Transport => "provider_transport",
            Self::ProviderRateLimited => "provider_rate_limited",
            Self::ProviderUnavailable => "provider_unavailable",
            Self::ProviderConcurrentRequest => "provider_concurrent_request",
            Self::ProviderIdempotencyConflict => "provider_idempotency_conflict",
            Self::ProviderRejected => "provider_rejected",
            Self::ProviderRedirect => "provider_redirect_rejected",
            Self::InvalidProviderResponse => "invalid_provider_response",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewEmailOutcome {
    /// Resend accepted the request. This is not evidence of mailbox delivery.
    Accepted {
        provider_id: String,
    },
    Retryable {
        code: ReviewEmailFailureCode,
        retry_after_seconds: Option<u64>,
    },
    PermanentFailure {
        code: ReviewEmailFailureCode,
    },
}

pub struct ReviewEmailSender {
    config: ReviewEmailConfig,
    client: Client,
    endpoint: Url,
    timeout: Duration,
}

impl fmt::Debug for ReviewEmailSender {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ReviewEmailSender { provider: Resend, credentials: [redacted] }")
    }
}

impl ReviewEmailSender {
    pub fn new(config: ReviewEmailConfig) -> Result<Self, ReviewEmailConfigError> {
        Self::build(config, RESEND_ENDPOINT, REQUEST_TIMEOUT)
    }

    fn build(
        config: ReviewEmailConfig,
        endpoint: &str,
        timeout: Duration,
    ) -> Result<Self, ReviewEmailConfigError> {
        let endpoint =
            Url::parse(endpoint).map_err(|_| ReviewEmailConfigError::HttpClientUnavailable)?;
        let client = Client::builder()
            .redirect(Policy::none())
            .no_proxy()
            .connect_timeout(Duration::from_secs(3).min(timeout))
            .timeout(timeout)
            .build()
            .map_err(|_| ReviewEmailConfigError::HttpClientUnavailable)?;
        Ok(Self {
            config,
            client,
            endpoint,
            timeout,
        })
    }

    /// The caller must use a verified, opted-in contact and a canonically verified
    /// contract. Syntax validation here does not establish either authority.
    /// Persist this complete payload before sending, then reuse it unchanged.
    pub fn prepare_payload(
        &self,
        recipient_email: &str,
        bounty_contract: &str,
        review_deadline: Option<DateTime<Utc>>,
    ) -> Result<Value, ReviewEmailInputError> {
        if !valid_mailbox(recipient_email) {
            return Err(ReviewEmailInputError::InvalidRecipient);
        }
        if bounty_contract.len() != 42
            || !bounty_contract.starts_with("0x")
            || !bounty_contract[2..]
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || bounty_contract[2..].bytes().all(|byte| byte == b'0')
        {
            return Err(ReviewEmailInputError::InvalidContract);
        }
        let link = format!(
            "{REVIEW_ORIGIN}/participate.html?bountyContract={}&network=base-mainnet&role=verifier",
            bounty_contract.to_ascii_lowercase()
        );
        let deadline = match review_deadline {
            Some(deadline) => format!(
                "Review deadline: {}.",
                deadline.format("%Y-%m-%d %H:%M:%S UTC")
            ),
            None => "No review deadline is set.".to_owned(),
        };
        let instruction = "Check the current submission and review deadline on the site before deciding whether to sign a verdict. This email does not extend the review window.";
        let reason = "You received this because a wallet linked to your account is designated to review this bounty.";
        let settings = format!("{REVIEW_ORIGIN}/#account");
        Ok(json!({
            "from": self.config.sender,
            "to": [recipient_email],
            "subject": SUBJECT,
            "text": format!("{SUBJECT}.\n\nReview the solution: {link}\n\n{deadline}\n\n{instruction}\n\n{reason}\nManage review emails: {settings}"),
            "html": format!(
                "<p>{}.</p><p><a href=\"{}\">Review the solution</a></p><p>{}</p><p>{}</p><p>{}</p><p><a href=\"{}\">Manage review emails</a></p>",
                escape_html(SUBJECT), escape_html(&link), escape_html(&deadline), escape_html(instruction), escape_html(reason), escape_html(&settings)
            ),
        }))
    }

    /// Perform exactly one bounded request. Only pass a payload returned by
    /// prepare_payload and frozen by the durable outbox. In particular, never
    /// regenerate a retry with today's sender configuration or a new key.
    pub async fn send(&self, idempotency_key: &str, frozen_payload: &Value) -> ReviewEmailOutcome {
        if idempotency_key.is_empty()
            || idempotency_key.len() > 256
            || !idempotency_key.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'/' | b':')
            })
        {
            return permanent(ReviewEmailFailureCode::InvalidIdempotencyKey);
        }
        let Ok(payload) = serde_json::from_value::<ProviderPayload>(frozen_payload.clone()) else {
            return permanent(ReviewEmailFailureCode::InvalidStoredPayload);
        };
        let Ok(body) = serde_json::to_vec(frozen_payload) else {
            return permanent(ReviewEmailFailureCode::InvalidStoredPayload);
        };
        if body.len() > PAYLOAD_LIMIT
            || !valid_sender(&payload.from)
            || !valid_mailbox(&payload.to[0])
            || payload.subject != SUBJECT
            || payload.text.is_empty()
            || payload.html.is_empty()
        {
            return permanent(ReviewEmailFailureCode::InvalidStoredPayload);
        }
        match tokio::time::timeout(self.timeout, self.send_once(idempotency_key, body)).await {
            Ok(outcome) => outcome,
            Err(_) => retryable(ReviewEmailFailureCode::Timeout, None),
        }
    }

    async fn send_once(&self, idempotency_key: &str, body: Vec<u8>) -> ReviewEmailOutcome {
        let response = self
            .client
            .post(self.endpoint.clone())
            .header(header::AUTHORIZATION, self.config.authorization.clone())
            .header(header::CONTENT_TYPE, "application/json")
            .header("Idempotency-Key", idempotency_key)
            .body(body)
            .send()
            .await;
        let response = match response {
            Ok(response) => response,
            Err(error) => {
                // reqwest errors can include request URLs; never retain or log them.
                return retryable(
                    if error.is_timeout() {
                        ReviewEmailFailureCode::Timeout
                    } else {
                        ReviewEmailFailureCode::Transport
                    },
                    None,
                );
            }
        };
        let status = response.status();
        let retry_after = response
            .headers()
            .get(header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
            .map(|seconds| seconds.clamp(1, 3600));
        if status == StatusCode::TOO_MANY_REQUESTS {
            return retryable(ReviewEmailFailureCode::ProviderRateLimited, retry_after);
        }
        if status.is_server_error() {
            return retryable(ReviewEmailFailureCode::ProviderUnavailable, retry_after);
        }
        if status.is_redirection() {
            return permanent(ReviewEmailFailureCode::ProviderRedirect);
        }
        if status == StatusCode::CONFLICT {
            // Inspect only the bounded classification; never expose provider
            // messages, which may echo recipient addresses or request content.
            let body = match bounded_response(response).await {
                Ok(body) => body,
                Err(code) => return retryable(code, retry_after),
            };
            let name = serde_json::from_slice::<ProviderError>(&body)
                .ok()
                .map(|error| error.name);
            if name.as_deref() == Some("invalid_idempotent_request") {
                return permanent(ReviewEmailFailureCode::ProviderIdempotencyConflict);
            }
            return retryable(
                ReviewEmailFailureCode::ProviderConcurrentRequest,
                retry_after,
            );
        }
        if !status.is_success() {
            return permanent(ReviewEmailFailureCode::ProviderRejected);
        }
        let body = match bounded_response(response).await {
            Ok(body) => body,
            Err(code) => return retryable(code, None),
        };
        let provider_id = serde_json::from_slice::<ProviderAccepted>(&body)
            .ok()
            .and_then(|accepted| uuid::Uuid::parse_str(&accepted.id).ok());
        match provider_id {
            Some(id) if !id.is_nil() => ReviewEmailOutcome::Accepted {
                provider_id: id.to_string(),
            },
            // The provider might have accepted the email before a truncated or
            // malformed response. Only the same frozen request/key may retry.
            _ => retryable(ReviewEmailFailureCode::InvalidProviderResponse, None),
        }
    }

    #[cfg(test)]
    fn for_test(
        config: ReviewEmailConfig,
        endpoint: &str,
        timeout: Duration,
    ) -> Result<Self, ReviewEmailConfigError> {
        let url = Url::parse(endpoint).expect("test endpoint is a URL");
        assert_eq!(url.scheme(), "http");
        assert!(matches!(url.host_str(), Some("127.0.0.1" | "[::1]")));
        Self::build(config, endpoint, timeout)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderPayload {
    from: String,
    to: [String; 1],
    subject: String,
    text: String,
    html: String,
}

#[derive(Deserialize)]
struct ProviderAccepted {
    id: String,
}

#[derive(Deserialize)]
struct ProviderError {
    name: String,
}

async fn bounded_response(mut response: Response) -> Result<Vec<u8>, ReviewEmailFailureCode> {
    if response
        .content_length()
        .is_some_and(|length| length > RESPONSE_LIMIT as u64)
    {
        return Err(ReviewEmailFailureCode::InvalidProviderResponse);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|error| {
        if error.is_timeout() {
            ReviewEmailFailureCode::Timeout
        } else {
            ReviewEmailFailureCode::Transport
        }
    })? {
        if bytes.len().saturating_add(chunk.len()) > RESPONSE_LIMIT {
            return Err(ReviewEmailFailureCode::InvalidProviderResponse);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn retryable(code: ReviewEmailFailureCode, retry_after_seconds: Option<u64>) -> ReviewEmailOutcome {
    ReviewEmailOutcome::Retryable {
        code,
        retry_after_seconds,
    }
}

fn permanent(code: ReviewEmailFailureCode) -> ReviewEmailOutcome {
    ReviewEmailOutcome::PermanentFailure { code }
}

fn valid_sender(sender: &str) -> bool {
    if sender.len() > 320 || sender.chars().any(char::is_control) {
        return false;
    }
    match sender.split_once('<') {
        Some((name, address)) => {
            !name.trim().is_empty()
                && !name.contains(['>', ',', ';'])
                && address.strip_suffix('>').is_some_and(valid_mailbox)
        }
        None => valid_mailbox(sender),
    }
}

fn valid_mailbox(address: &str) -> bool {
    if address.len() > 254 || !address.is_ascii() {
        return false;
    }
    let Some((local, domain)) = address.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && local.len() <= 64
        && !local.starts_with('.')
        && !local.ends_with('.')
        && !local.contains("..")
        && local
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b".!#$%&'*+-/=?^_`{|}~".contains(&byte))
        && domain.contains('.')
        && domain.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        task::JoinHandle,
    };

    const CONTRACT: &str = "0xabcdefabcdefabcdefabcdefabcdefabcdefabcd";
    const RECIPIENT: &str = "verified-person+review@example.test";
    const KEY: &str = "re_fixture_secret_never_log";
    const IDEMPOTENCY_KEY: &str = "verifier-review/1076dbe3-306e-4243-bdf1-4aee17394c7e";
    const ACCEPTED: &str = r#"{"id":"49a3999c-0ce1-4ea6-ab68-afcd6dc2e794"}"#;

    fn config_with_sender(sender: &str) -> ReviewEmailConfig {
        ReviewEmailConfig::from_lookup(|key| match key {
            "VERIFIER_EMAIL_ENABLED" => Some("true".to_owned()),
            "RESEND_API_KEY" => Some(KEY.to_owned()),
            "VERIFIER_EMAIL_FROM" => Some(sender.to_owned()),
            _ => None,
        })
        .unwrap()
        .unwrap()
    }

    fn config() -> ReviewEmailConfig {
        config_with_sender("Agent Bounties <review@example.test>")
    }

    fn response(status: u16, headers: &str, body: &str) -> String {
        format!("HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n{body}", body.len())
    }

    async fn mock(responses: Vec<(String, Duration)>) -> (String, JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/emails", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let mut requests = Vec::new();
            for (response, delay) in responses {
                let (mut stream, _) =
                    tokio::time::timeout(Duration::from_secs(3), listener.accept())
                        .await
                        .unwrap()
                        .unwrap();
                let mut request = Vec::new();
                let mut buf = [0u8; 2048];
                loop {
                    let read = tokio::time::timeout(Duration::from_secs(3), stream.read(&mut buf))
                        .await
                        .unwrap()
                        .unwrap();
                    assert!(read > 0, "request ended before complete payload");
                    request.extend_from_slice(&buf[..read]);
                    assert!(request.len() < 32 * 1024, "fixture request exceeded bound");
                    if let Some(end) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&request[..end]).to_ascii_lowercase();
                        let length: usize = headers
                            .lines()
                            .find_map(|line| {
                                line.strip_prefix("content-length:")
                                    .map(|value| value.trim().parse().unwrap())
                            })
                            .unwrap();
                        if request.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                requests.push(String::from_utf8(request).unwrap());
                tokio::time::sleep(delay).await;
                let _ = stream.write_all(response.as_bytes()).await;
            }
            requests
        });
        (endpoint, task)
    }

    fn sender(endpoint: &str) -> ReviewEmailSender {
        ReviewEmailSender::for_test(config(), endpoint, Duration::from_secs(1)).unwrap()
    }

    fn payload(sender: &ReviewEmailSender) -> Value {
        sender.prepare_payload(RECIPIENT, CONTRACT, None).unwrap()
    }

    fn assert_redacted(value: &impl fmt::Debug) {
        let rendered = format!("{value:?}");
        for secret in [
            KEY,
            RECIPIENT,
            "private-title",
            "private-artifact",
            "review@example.test",
        ] {
            assert!(
                !rendered.contains(secret),
                "debug output disclosed fixture content"
            );
        }
    }

    #[test]
    fn configuration_is_opt_in_and_requires_valid_redacted_credentials() {
        assert!(ReviewEmailConfig::from_lookup(|_| None).unwrap().is_none());
        assert!(ReviewEmailConfig::from_lookup(|key| {
            if key == "RESEND_API_KEY" {
                Some(KEY.to_owned())
            } else {
                None
            }
        })
        .unwrap()
        .is_none());
        let cases = [
            (
                "perhaps",
                Some(KEY),
                Some("review@example.test"),
                ReviewEmailConfigError::InvalidEnabledFlag,
            ),
            (
                "true",
                None,
                Some("review@example.test"),
                ReviewEmailConfigError::MissingApiKey,
            ),
            (
                "true",
                Some("fixture\r\nsecret"),
                Some("review@example.test"),
                ReviewEmailConfigError::InvalidApiKey,
            ),
            (
                "true",
                Some(KEY),
                None,
                ReviewEmailConfigError::MissingSender,
            ),
            (
                "true",
                Some(KEY),
                Some("review@example.test\nBcc: private@example.test"),
                ReviewEmailConfigError::InvalidSender,
            ),
            (
                "true",
                Some(KEY),
                Some("review@example.test,other@example.test"),
                ReviewEmailConfigError::InvalidSender,
            ),
        ];
        for (flag, key, from, expected) in cases {
            let error = ReviewEmailConfig::from_lookup(|name| match name {
                "VERIFIER_EMAIL_ENABLED" => Some(flag.to_owned()),
                "RESEND_API_KEY" => key.map(str::to_owned),
                "VERIFIER_EMAIL_FROM" => from.map(str::to_owned),
                _ => None,
            })
            .unwrap_err();
            assert_eq!(error, expected);
            assert_redacted(&error);
            assert!(!error.to_string().contains(KEY));
        }
        let fallback = ReviewEmailConfig::from_lookup(|key| match key {
            "VERIFIER_EMAIL_ENABLED" => Some("true".to_owned()),
            "RESEND_API_KEY" => Some(KEY.to_owned()),
            "AUTH_EMAIL_FROM" => Some("fallback@example.test".to_owned()),
            _ => None,
        })
        .unwrap()
        .unwrap();
        assert_eq!(fallback.sender, "fallback@example.test");
        assert_redacted(&config());
        assert_redacted(&config().authorization);
    }

    #[test]
    fn message_has_only_generic_content_a_first_party_review_link_and_optional_utc_deadline() {
        let sender = ReviewEmailSender::new(config()).unwrap();
        assert_eq!(sender.endpoint.as_str(), RESEND_ENDPOINT);
        let no_deadline = payload(&sender);
        let expected_link = format!("https://agentbounties.app/participate.html?bountyContract={CONTRACT}&network=base-mainnet&role=verifier");
        assert_eq!(no_deadline.as_object().unwrap().len(), 5);
        assert_eq!(no_deadline["subject"], SUBJECT);
        assert_eq!(no_deadline["to"], json!([RECIPIENT]));
        assert!(no_deadline["text"]
            .as_str()
            .unwrap()
            .contains(&expected_link));
        assert!(no_deadline["text"]
            .as_str()
            .unwrap()
            .contains("No review deadline is set."));
        let html = no_deadline["html"].as_str().unwrap();
        assert!(html.contains(&expected_link.replace('&', "&amp;")));
        assert!(!html.contains(RECIPIENT));
        assert!(!html.contains("&network="));
        assert_eq!(escape_html("<&\"'>"), "&lt;&amp;&quot;&#39;&gt;");
        let deadline = DateTime::parse_from_rfc3339("2026-09-15T01:02:03-06:00")
            .unwrap()
            .with_timezone(&Utc);
        let with_deadline = sender
            .prepare_payload(RECIPIENT, CONTRACT, Some(deadline))
            .unwrap();
        assert!(with_deadline["text"]
            .as_str()
            .unwrap()
            .contains("Review deadline: 2026-09-15 07:02:03 UTC."));
        assert!(!with_deadline["text"]
            .as_str()
            .unwrap()
            .contains("No review deadline"));
        for value in [&no_deadline, &with_deadline] {
            for format in ["text", "html"] {
                let body = value[format].as_str().unwrap();
                assert!(body.contains("https://agentbounties.app/#account"));
                assert!(body.contains("a wallet linked to your account is designated"));
                assert!(body.contains("does not extend the review window"));
            }
            let serialized = value.to_string();
            for forbidden in [
                "attachments",
                "private-title",
                "private-artifact",
                KEY,
                "delivered",
                "payment",
            ] {
                assert!(!serialized.contains(forbidden));
            }
        }
        assert_redacted(&sender);
        for invalid in [
            "0x0000000000000000000000000000000000000000",
            "https://evil.test",
            "0xabc<&",
        ] {
            assert_eq!(
                sender.prepare_payload(RECIPIENT, invalid, None),
                Err(ReviewEmailInputError::InvalidContract)
            );
        }
        assert_eq!(
            sender.prepare_payload("x@example.test\r\nBcc:y@example.test", CONTRACT, None),
            Err(ReviewEmailInputError::InvalidRecipient)
        );
    }

    #[tokio::test]
    async fn accepted_is_not_delivered_and_retries_preserve_exact_body_and_key_after_config_change()
    {
        let (endpoint, task) = mock(vec![
            (
                response(503, "Retry-After: 7\r\n", "private-artifact"),
                Duration::ZERO,
            ),
            (response(200, "", ACCEPTED), Duration::ZERO),
        ])
        .await;
        let original = sender(&endpoint);
        let frozen = payload(&original);
        assert_eq!(
            original.send(IDEMPOTENCY_KEY, &frozen).await,
            retryable(ReviewEmailFailureCode::ProviderUnavailable, Some(7))
        );
        let changed = ReviewEmailSender::for_test(
            config_with_sender("New sender <new@example.test>"),
            &endpoint,
            Duration::from_secs(1),
        )
        .unwrap();
        let outcome = changed.send(IDEMPOTENCY_KEY, &frozen).await;
        assert_eq!(
            outcome,
            ReviewEmailOutcome::Accepted {
                provider_id: "49a3999c-0ce1-4ea6-ab68-afcd6dc2e794".to_owned()
            }
        );
        assert_redacted(&outcome);
        let requests = task.await.unwrap();
        assert_eq!(requests.len(), 2);
        let mut bodies = Vec::new();
        for request in requests {
            let (headers, body) = request.split_once("\r\n\r\n").unwrap();
            assert!(headers.starts_with("POST /emails HTTP/1.1"));
            assert!(headers
                .to_ascii_lowercase()
                .contains(&format!("idempotency-key: {IDEMPOTENCY_KEY}")));
            assert!(headers.contains(&format!("Bearer {KEY}")));
            assert_eq!(serde_json::from_str::<Value>(body).unwrap(), frozen);
            assert!(!body.contains("new@example.test"));
            bodies.push(body.to_owned());
        }
        assert_eq!(bodies[0], bodies[1]);
    }

    #[tokio::test]
    async fn provider_failures_are_classified_without_exposing_error_bodies() {
        let cases = [
            (
                429,
                "Retry-After: 999999\r\n",
                r#"{"name":"rate_limit_exceeded","message":"private-artifact"}"#,
                retryable(ReviewEmailFailureCode::ProviderRateLimited, Some(3600)),
            ),
            (
                500,
                "",
                "private-title",
                retryable(ReviewEmailFailureCode::ProviderUnavailable, None),
            ),
            (
                409,
                "",
                r#"{"name":"concurrent_idempotent_requests","message":"private-title"}"#,
                retryable(ReviewEmailFailureCode::ProviderConcurrentRequest, None),
            ),
            (
                409,
                "",
                r#"{"name":"invalid_idempotent_request","message":"private-artifact"}"#,
                permanent(ReviewEmailFailureCode::ProviderIdempotencyConflict),
            ),
            (
                400,
                "",
                RECIPIENT,
                permanent(ReviewEmailFailureCode::ProviderRejected),
            ),
            (
                401,
                "",
                KEY,
                permanent(ReviewEmailFailureCode::ProviderRejected),
            ),
            (
                422,
                "",
                "private-artifact",
                permanent(ReviewEmailFailureCode::ProviderRejected),
            ),
            (
                200,
                "",
                "private-artifact",
                retryable(ReviewEmailFailureCode::InvalidProviderResponse, None),
            ),
            (
                200,
                "",
                r#"{"id":"private-artifact"}"#,
                retryable(ReviewEmailFailureCode::InvalidProviderResponse, None),
            ),
        ];
        for (status, headers, body, expected) in cases {
            let (endpoint, task) =
                mock(vec![(response(status, headers, body), Duration::ZERO)]).await;
            let sender = sender(&endpoint);
            let outcome = sender.send(IDEMPOTENCY_KEY, &payload(&sender)).await;
            assert_eq!(outcome, expected);
            assert_redacted(&outcome);
            assert_eq!(task.await.unwrap().len(), 1);
        }
    }

    #[tokio::test]
    async fn timeout_and_transport_errors_are_retryable_and_do_not_retry_inside_one_attempt() {
        let (endpoint, task) = mock(vec![(
            response(200, "", ACCEPTED),
            Duration::from_millis(120),
        )])
        .await;
        let sender =
            ReviewEmailSender::for_test(config(), &endpoint, Duration::from_millis(30)).unwrap();
        let outcome = sender.send(IDEMPOTENCY_KEY, &payload(&sender)).await;
        assert_eq!(outcome, retryable(ReviewEmailFailureCode::Timeout, None));
        assert_redacted(&outcome);
        assert_eq!(task.await.unwrap().len(), 1);
        let unavailable = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/emails", unavailable.local_addr().unwrap());
        drop(unavailable);
        let sender = self::sender(&endpoint);
        let outcome = sender.send(IDEMPOTENCY_KEY, &payload(&sender)).await;
        assert_eq!(outcome, retryable(ReviewEmailFailureCode::Transport, None));
        assert_redacted(&outcome);
    }

    #[tokio::test]
    async fn redirect_never_forwards_credentials_and_invalid_payloads_never_connect() {
        let target = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let (endpoint, task) = mock(vec![(
            response(
                307,
                &format!(
                    "Location: http://{}/capture\r\n",
                    target.local_addr().unwrap()
                ),
                "",
            ),
            Duration::ZERO,
        )])
        .await;
        let sender = sender(&endpoint);
        assert_eq!(
            sender.send(IDEMPOTENCY_KEY, &payload(&sender)).await,
            permanent(ReviewEmailFailureCode::ProviderRedirect)
        );
        assert_eq!(task.await.unwrap().len(), 1);
        let mut invalid = payload(&sender);
        invalid["attachments"] = json!([{"path":"https://private-artifact.test/file"}]);
        let target_sender = ReviewEmailSender::for_test(
            config(),
            &format!("http://{}/emails", target.local_addr().unwrap()),
            Duration::from_secs(1),
        )
        .unwrap();
        assert_eq!(
            target_sender.send(IDEMPOTENCY_KEY, &invalid).await,
            permanent(ReviewEmailFailureCode::InvalidStoredPayload)
        );
        assert_eq!(
            target_sender.send("caller\r\nID", &payload(&sender)).await,
            permanent(ReviewEmailFailureCode::InvalidIdempotencyKey)
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(80), target.accept())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn oversized_and_streamed_response_bodies_are_bounded() {
        let body = "a".repeat(RESPONSE_LIMIT + 1);
        let chunked = format!("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n{body}\r\n0\r\n\r\n", body.len());
        for raw in [response(200, "", &body), chunked] {
            let (endpoint, task) = mock(vec![(raw, Duration::ZERO)]).await;
            let sender = sender(&endpoint);
            assert_eq!(
                sender.send(IDEMPOTENCY_KEY, &payload(&sender)).await,
                retryable(ReviewEmailFailureCode::InvalidProviderResponse, None)
            );
            assert_eq!(task.await.unwrap().len(), 1);
        }
    }
}
