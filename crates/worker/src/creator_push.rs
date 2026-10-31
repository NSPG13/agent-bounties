//! Encrypted Web Push, sent only to an authenticated account's opted-in device.
use base64::Engine;
use db::{PostgresStore, PushDelivery};
use serde_json::json;
use web_push::{ContentEncoding, SubscriptionInfo, VapidSignatureBuilder, WebPushMessageBuilder};

pub struct CreatorPushSender {
    private_key: String,
    public_key: String,
    subject: String,
    client: reqwest::Client,
}
impl CreatorPushSender {
    pub fn from_env() -> anyhow::Result<Option<Self>> {
        Self::from_lookup(|key| std::env::var(key).ok())
    }
    fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> anyhow::Result<Option<Self>> {
        match lookup("CREATOR_PUSH_ENABLED").as_deref() {
            None | Some("") | Some("false") => return Ok(None),
            Some("true") => {}
            _ => anyhow::bail!("Invalid CREATOR_PUSH_ENABLED"),
        };
        let private_key = lookup("CREATOR_PUSH_VAPID_PRIVATE_KEY")
            .ok_or_else(|| anyhow::anyhow!("Push signing key missing"))?;
        let builder = VapidSignatureBuilder::from_base64_no_sub(&private_key)
            .map_err(|_| anyhow::anyhow!("Invalid push signing key"))?;
        let public_key =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(builder.get_public_key());
        let subject =
            lookup("CREATOR_PUSH_VAPID_SUBJECT").unwrap_or_else(|| "https://agentizable.ai".into());
        if !matches!(
            subject.as_str(),
            "https://agentizable.ai" | "https://agentbounties.app"
        ) {
            anyhow::bail!("Push identity must be the first-party origin");
        }
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .connect_timeout(std::time::Duration::from_secs(3))
            .timeout(std::time::Duration::from_secs(8))
            .build()?;
        Ok(Some(Self {
            private_key,
            public_key,
            subject,
            client,
        }))
    }
    pub fn public_key(&self) -> &str {
        &self.public_key
    }
    /// The account endpoint reserves a single attempt first. This cannot create
    /// bounty evidence, retry an ambiguous send, or select an arbitrary recipient.
    pub async fn send_delivery_test(
        &self,
        store: &PostgresStore,
        delivery: &db::PushTestDelivery,
    ) -> anyhow::Result<&'static str> {
        let payload = test_payload(delivery.id, chrono::Utc::now());
        let prepared = self.encrypt(
            &delivery.endpoint,
            &delivery.p256dh,
            &delivery.auth,
            &payload,
            60,
        );
        let Ok((headers, body)) = prepared else {
            store
                .finish_push_delivery_test(delivery, "failed", false)
                .await?;
            return Ok("failed");
        };
        if !store.push_delivery_test_is_current(delivery).await? {
            store
                .finish_push_delivery_test(delivery, "cancelled", false)
                .await?;
            return Ok("cancelled");
        }
        let response = self
            .client
            .post(&delivery.endpoint)
            .headers(headers)
            .body(body)
            .send()
            .await;
        let code = response.as_ref().map(|r| r.status().as_u16()).ok();
        let status = match code {
            Some(200..=299) => "accepted",
            Some(400..=499) => "failed",
            _ => "unknown",
        };
        store
            .finish_push_delivery_test(delivery, status, matches!(code, Some(404 | 410)))
            .await?;
        Ok(status)
    }
    pub async fn poll(&self, store: &PostgresStore, network: &str) -> anyhow::Result<usize> {
        self.poll_inner(store, network, None).await
    }
    pub async fn poll_creator_open(
        &self,
        store: &PostgresStore,
        network: &str,
        rpc: &str,
    ) -> anyhow::Result<usize> {
        self.poll_inner(store, network, Some(rpc)).await
    }
    async fn poll_inner(
        &self,
        store: &PostgresStore,
        network: &str,
        rpc: Option<&str>,
    ) -> anyhow::Result<usize> {
        store.enqueue_creator_push(network).await?;
        store.enqueue_review_deadline_push(network).await?;
        let mut accepted = 0;
        // Each channel gets a bounded batch so reminder backlog cannot starve
        // a newly submitted solution's first notification.
        for reminder in [false, true] {
            for _ in 0..5 {
                let delivery = if reminder {
                    store
                        .claim_review_deadline_push_for_protocol(network, rpc.is_some())
                        .await?
                } else {
                    store
                        .claim_creator_push_for_protocol(network, rpc.is_some())
                        .await?
                };
                let Some(delivery) = delivery else {
                    break;
                };
                let prepared = self.prepare(&delivery);
                let Ok((headers, body)) = prepared else {
                    store
                        .finish_creator_push(&delivery, false, false, false)
                        .await?;
                    continue;
                };
                if let Some(rpc) = rpc {
                    if !crate::creator_open_notifications::source_current(
                        store,
                        network,
                        &delivery.bounty_contract,
                        rpc,
                    )
                    .await?
                    {
                        continue;
                    }
                }
                if !store.creator_push_is_current(&delivery).await? {
                    store
                        .finish_creator_push(&delivery, false, false, false)
                        .await?;
                    continue;
                }
                let response = self
                    .client
                    .post(&delivery.endpoint)
                    .headers(headers)
                    .body(body)
                    .send()
                    .await;
                // No provider response body, subscription URL or cryptographic key
                // is logged or stored as an error. A 2xx means provider acceptance.
                let status = response.as_ref().map(|r| r.status().as_u16()).ok();
                let sent = status.is_some_and(|s| (200..300).contains(&s));
                let gone = matches!(status, Some(404 | 410));
                let retry = status.is_none() || matches!(status, Some(408 | 429 | 500..=599));
                store
                    .finish_creator_push(&delivery, sent, retry, gone)
                    .await?;
                accepted += usize::from(sent);
            }
        }
        Ok(accepted)
    }
    fn prepare(&self, d: &PushDelivery) -> Result<(reqwest::header::HeaderMap, Vec<u8>), ()> {
        if !db::valid_push_subscription(&d.endpoint, &d.p256dh, &d.auth)
            || d.bounty_contract.len() != 42
            || !d.bounty_contract.starts_with("0x")
            || !d.bounty_contract[2..]
                .bytes()
                .all(|b| b.is_ascii_hexdigit())
            || !matches!(
                d.kind.as_str(),
                "claim" | "submission" | "creator_open_submission"
            )
            || !matches!(
                d.stage.as_str(),
                "initial" | "remaining_24h" | "remaining_6h" | "remaining_1h" | "overdue"
            )
            || (d.stage != "initial" && (d.kind == "claim" || d.deadline.is_none()))
            || d.network != "base-mainnet"
            || d.entry <= 0
        {
            return Err(());
        }
        let now = chrono::Utc::now();
        let remaining = d
            .deadline
            .map(|time| (time - now).num_seconds())
            .unwrap_or(3600);
        let (start, end) = match d.stage.as_str() {
            "remaining_24h" => (86400, 21600),
            "remaining_6h" => (21600, 3600),
            "remaining_1h" => (3600, 0),
            "overdue" => (0, -86400),
            _ => (i64::MAX, 0),
        };
        if remaining > start || remaining <= end {
            return Err(());
        }
        let title = if d.stage == "overdue" {
            "A review deadline was missed"
        } else if d.stage != "initial" {
            "Your review deadline is approaching"
        } else if d.kind == "claim" {
            "Work has started on your bounty"
        } else {
            "A solution is ready for your review"
        };
        let date = d
            .deadline
            .map(|time| time.format("%Y-%m-%d %H:%M:%S UTC").to_string());
        let url = if d.kind == "creator_open_submission" {
            format!(
                "/creator-open.html?network=base-mainnet&bounty={}&entry={}",
                d.bounty_contract, d.entry
            )
        } else {
            format!(
                "/participate.html?bountyContract={}&network=base-mainnet&role=verifier",
                d.bounty_contract
            )
        };
        let ttl = (remaining.saturating_sub(end)).min(3600) as u32;
        // Older installed workers must drop deadline notices rather than label
        // an overdue reminder as a new, actionable submission.
        let schema = if d.stage == "initial" {
            "agent-bounties/creator-push-v1"
        } else {
            "agent-bounties/creator-push-v2"
        };
        let body = serde_json::to_vec(&json!({"schema":schema,"id":d.id,
            "kind":d.kind,"stage":d.stage,"title":title,"deadline":date,"url":url,
            "expires_at":now.timestamp()+i64::from(ttl),
            "body":"Open Agentizable to check the latest status and deadline."}))
        .map_err(|_| ())?;
        self.encrypt(&d.endpoint, &d.p256dh, &d.auth, &body, ttl)
    }
    fn encrypt(
        &self,
        endpoint: &str,
        p256dh: &str,
        auth: &str,
        body: &[u8],
        ttl: u32,
    ) -> Result<(reqwest::header::HeaderMap, Vec<u8>), ()> {
        if !db::valid_push_subscription(endpoint, p256dh, auth) {
            return Err(());
        }
        let subscription = SubscriptionInfo::new(endpoint, p256dh, auth);
        let mut signature =
            VapidSignatureBuilder::from_base64(&self.private_key, &subscription).map_err(|_| ())?;
        signature.add_claim("sub", self.subject.clone());
        let mut builder = WebPushMessageBuilder::new(&subscription);
        builder.set_payload(ContentEncoding::Aes128Gcm, body);
        builder.set_vapid_signature(signature.build().map_err(|_| ())?);
        builder.set_ttl(ttl);
        let request =
            web_push::request_builder::build_request::<Vec<u8>>(builder.build().map_err(|_| ())?);
        let (parts, body) = request.into_parts();
        let mut headers = reqwest::header::HeaderMap::new();
        for (key, value) in &parts.headers {
            let name =
                reqwest::header::HeaderName::from_bytes(key.as_str().as_bytes()).map_err(|_| ())?;
            let mut value =
                reqwest::header::HeaderValue::from_bytes(value.as_bytes()).map_err(|_| ())?;
            value.set_sensitive(true);
            headers.insert(name, value);
        }
        Ok((headers, body))
    }
}

fn test_payload(id: uuid::Uuid, now: chrono::DateTime<chrono::Utc>) -> Vec<u8> {
    // A dedicated schema makes older workers discard the test. The browser
    // supplies all displayed copy and always links to notification preferences.
    serde_json::to_vec(&json!({"schema":"agent-bounties/push-test-v1","id":id,
        "expires_at":now.timestamp()+60}))
    .expect("fixed push test payload")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sender() -> CreatorPushSender {
        CreatorPushSender::from_lookup(|name| match name {
            "CREATOR_PUSH_ENABLED" => Some("true".into()),
            "CREATOR_PUSH_VAPID_PRIVATE_KEY" => {
                Some(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([7; 32]))
            }
            _ => None,
        })
        .unwrap()
        .unwrap()
    }
    #[test]
    fn migration_push_subject_accepts_only_the_two_owned_origins() {
        for (subject, accepted) in [
            ("https://agentizable.ai", true),
            ("https://agentbounties.app", true),
            ("https://agentizable.ai.attacker.example", false),
            ("http://agentizable.ai", false),
        ] {
            let result = CreatorPushSender::from_lookup(|name| match name {
                "CREATOR_PUSH_ENABLED" => Some("true".into()),
                "CREATOR_PUSH_VAPID_PRIVATE_KEY" => {
                    Some(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([7; 32]))
                }
                "CREATOR_PUSH_VAPID_SUBJECT" => Some(subject.into()),
                _ => None,
            });
            assert_eq!(result.is_ok(), accepted);
        }
    }

    #[test]
    fn encrypted_payload_is_bounded_and_provider_urls_are_strict() {
        let sender = sender();
        let mut delivery = PushDelivery {
            id: uuid::Uuid::new_v4(),
            lease: uuid::Uuid::new_v4(),
            subscription_id: uuid::Uuid::new_v4(),
            endpoint: "https://fcm.googleapis.com/wp/fixture-only".into(),
            p256dh: sender.public_key.clone(),
            auth: base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([3; 16]),
            bounty_contract: format!("0x{}", "1".repeat(40)),
            kind: "claim".into(),
            stage: "initial".into(),
            network: "base-mainnet".into(),
            entry: 1,
            subscription_updated_at: chrono::Utc::now(),
            deadline: Some(chrono::Utc::now() + chrono::Duration::minutes(20)),
        };
        let (headers, body) = sender.prepare(&delivery).unwrap();
        assert!(body.len() < 4096);
        assert!(!String::from_utf8_lossy(&body).contains("bountyContract"));
        assert_eq!(headers.get("content-encoding").unwrap(), "aes128gcm");
        assert!(headers.get("authorization").unwrap().is_sensitive());
        for endpoint in [
            "https://127.0.0.1/wp/a",
            "https://fcm.googleapis.com.evil.example/wp/a",
            "https://user@fcm.googleapis.com/wp/a",
            "http://fcm.googleapis.com/wp/a",
            "https://fcm.googleapis.com:8443/wp/a",
            "https://fcm.googleapis.com/unrelated",
        ] {
            delivery.endpoint = endpoint.into();
            assert!(sender.prepare(&delivery).is_err(), "{endpoint}");
        }
    }
    #[test]
    fn deadline_push_expires_at_the_end_of_its_stage_and_rejects_stale_reminders() {
        let sender = sender();
        let mut delivery = PushDelivery {
            id: uuid::Uuid::new_v4(),
            lease: uuid::Uuid::new_v4(),
            subscription_id: uuid::Uuid::new_v4(),
            endpoint: "https://fcm.googleapis.com/wp/fixture-only".into(),
            p256dh: sender.public_key.clone(),
            auth: base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([3; 16]),
            bounty_contract: format!("0x{}", "1".repeat(40)),
            kind: "submission".into(),
            stage: "remaining_1h".into(),
            network: "base-mainnet".into(),
            entry: 1,
            subscription_updated_at: chrono::Utc::now(),
            deadline: None,
        };
        for (stage, remaining, valid, max_ttl) in [
            ("remaining_24h", 90000, false, 0),
            ("remaining_24h", 21700, true, 100),
            ("remaining_24h", 21000, false, 0),
            ("remaining_6h", 3700, true, 100),
            ("remaining_6h", 3500, false, 0),
            ("remaining_1h", 100, true, 100),
            ("remaining_1h", -1, false, 0),
            ("overdue", 100, false, 0),
            ("overdue", -86300, true, 100),
            ("overdue", -86500, false, 0),
        ] {
            delivery.stage = stage.into();
            delivery.deadline = Some(chrono::Utc::now() + chrono::Duration::seconds(remaining));
            let prepared = sender.prepare(&delivery);
            assert_eq!(prepared.is_ok(), valid, "{stage}: {remaining}");
            if let Ok((headers, _)) = prepared {
                let ttl: u32 = headers
                    .get("ttl")
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .parse()
                    .unwrap();
                assert!(ttl > 0 && ttl <= max_ttl);
            }
        }
        delivery.stage = "remaining_1h".into();
        delivery.deadline = Some(chrono::Utc::now() + chrono::Duration::minutes(20));
        delivery.kind = "claim".into();
        assert!(sender.prepare(&delivery).is_err());
        delivery.kind = "submission".into();
        delivery.deadline = None;
        assert!(sender.prepare(&delivery).is_err());
    }

    #[test]
    fn delivery_defaults_off_and_never_discloses_bad_key() {
        assert!(CreatorPushSender::from_lookup(|_| None).unwrap().is_none());
        let result = CreatorPushSender::from_lookup(|name| {
            Some(
                if name == "CREATOR_PUSH_ENABLED" {
                    "true"
                } else {
                    "private-key-fixture-invalid"
                }
                .into(),
            )
        });
        let error = result.err().unwrap().to_string();
        assert!(!error.contains("private-key-fixture"));
    }

    #[test]
    fn delivery_test_is_explicit_short_lived_and_encrypted() {
        let sender = sender();
        let now = chrono::Utc::now();
        let payload = test_payload(uuid::Uuid::new_v4(), now);
        let value: serde_json::Value = serde_json::from_slice(&payload).unwrap();
        assert_eq!(value["schema"], "agent-bounties/push-test-v1");
        assert_eq!(value["expires_at"], now.timestamp() + 60);
        assert!(value.get("bounty_contract").is_none());
        assert!(value.get("url").is_none());
        let auth = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([3; 16]);
        let (headers, body) = sender
            .encrypt(
                "https://fcm.googleapis.com/wp/fixture-only",
                &sender.public_key,
                &auth,
                &payload,
                60,
            )
            .unwrap();
        assert_eq!(headers["ttl"], "60");
        assert!(!String::from_utf8_lossy(&body).contains("push-test"));
        assert!(sender
            .encrypt(
                "https://127.0.0.1/wp/test",
                &sender.public_key,
                &auth,
                &payload,
                60
            )
            .is_err());
    }
}
