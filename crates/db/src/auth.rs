//! Durable authentication and authorization state. No raw bearer token or IP is stored.
use crate::PostgresStore;
use chrono::{DateTime, Utc};
use sqlx::Row;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum AuthStoreError {
    #[error("authentication state unavailable")]
    Unavailable(#[from] sqlx::Error),
    #[error("invalid or expired authentication challenge")]
    InvalidChallenge,
    #[error("resource owner or idempotency key conflicts")]
    Conflict,
    #[error("quota exceeded")]
    QuotaExceeded,
    #[error("invalid authentication input")]
    InvalidInput,
}
#[derive(Debug, Clone)]
pub struct WalletChallenge {
    pub id: Uuid,
    pub domain: String,
    pub chain_id: u64,
    pub address: String,
    pub message: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}
#[derive(Clone)]
pub struct WalletSession {
    pub challenge: Option<WalletChallenge>,
    pub contract_signature: Option<String>,
    pub chain_id: u64,
    pub address: String,
    pub expires_at: DateTime<Utc>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuotaReservation {
    pub is_new: bool,
}
fn bounded(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}
fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
impl PostgresStore {
    pub async fn create_wallet_challenge(
        &self,
        value: &WalletChallenge,
    ) -> Result<(), AuthStoreError> {
        sqlx::query("INSERT INTO security_wallet_challenges(id,domain,chain_id,address,message,created_at,expires_at) VALUES($1,$2,$3,$4,$5,$6,$7)")
            .bind(value.id).bind(&value.domain).bind(value.chain_id as i64).bind(&value.address).bind(&value.message).bind(value.created_at).bind(value.expires_at).execute(&self.pool).await?;
        Ok(())
    }
    /// Reserve one bounded signature attempt before any RPC work. Expired/used nonces fail closed.
    pub async fn wallet_challenge_attempt(
        &self,
        id: Uuid,
    ) -> Result<WalletChallenge, AuthStoreError> {
        let row = sqlx::query("UPDATE security_wallet_challenges SET attempts=attempts+1 WHERE id=$1 AND consumed_at IS NULL AND expires_at>now() AND attempts<5 RETURNING id,domain,chain_id,address,message,created_at,expires_at")
            .bind(id).fetch_optional(&self.pool).await?.ok_or(AuthStoreError::InvalidChallenge)?;
        Ok(WalletChallenge {
            id: row.try_get("id")?,
            domain: row.try_get("domain")?,
            chain_id: row.try_get::<i64, _>("chain_id")? as u64,
            address: row.try_get("address")?,
            message: row.try_get("message")?,
            created_at: row.try_get("created_at")?,
            expires_at: row.try_get("expires_at")?,
        })
    }
    /// Caller verifies the exact stored message; the DB atomically consumes its nonce and derives session scope from it.
    pub async fn consume_wallet_challenge_and_create_session(
        &self,
        challenge_id: Uuid,
        token_hash: &str,
        contract_signature: Option<&str>,
    ) -> Result<WalletSession, AuthStoreError> {
        if !digest(token_hash) {
            return Err(AuthStoreError::InvalidInput);
        }
        let mut tx = self.pool.begin().await?;
        let row=sqlx::query("UPDATE security_wallet_challenges SET consumed_at=now() WHERE id=$1 AND consumed_at IS NULL AND expires_at>now() RETURNING chain_id,address,created_at")
            .bind(challenge_id).fetch_optional(&mut *tx).await?.ok_or(AuthStoreError::InvalidChallenge)?;
        let chain_id: i64 = row.try_get("chain_id")?;
        let address: String = row.try_get("address")?;
        let issued: DateTime<Utc> = row.try_get("created_at")?;
        let expiry:DateTime<Utc>=sqlx::query_scalar("INSERT INTO security_wallet_sessions(token_hash,challenge_id,chain_id,address,created_at,expires_at,contract_signature) VALUES($1,$2,$3,$4,now(),LEAST(now()+interval '15 minutes',$5+interval '15 minutes'),$6) RETURNING expires_at")
            .bind(token_hash).bind(challenge_id).bind(chain_id).bind(&address).bind(issued).bind(contract_signature).fetch_one(&mut *tx).await?;
        tx.commit().await?;
        Ok(WalletSession {
            chain_id: chain_id as u64,
            address,
            expires_at: expiry,
            challenge: None,
            contract_signature: contract_signature.map(str::to_string),
        })
    }
    pub async fn find_auth_session(
        &self,
        token_hash: &str,
    ) -> Result<Option<WalletSession>, AuthStoreError> {
        if !digest(token_hash) {
            return Ok(None);
        }
        let row=sqlx::query("SELECT s.chain_id,s.address,s.expires_at,s.contract_signature,c.id,c.domain,c.message,c.created_at,c.expires_at AS challenge_expires_at FROM security_wallet_sessions s JOIN security_wallet_challenges c ON c.id=s.challenge_id WHERE s.token_hash=$1 AND s.revoked_at IS NULL AND s.expires_at>now()")
            .bind(token_hash).fetch_optional(&self.pool).await?;
        let Some(row) = row else { return Ok(None) };
        let chain_id = row.try_get::<i64, _>("chain_id")? as u64;
        let address: String = row.try_get("address")?;
        let signature: Option<String> = row.try_get("contract_signature")?;
        let challenge = if signature.is_some() {
            Some(WalletChallenge {
                id: row.try_get("id")?,
                domain: row.try_get("domain")?,
                chain_id,
                address: address.clone(),
                message: row.try_get("message")?,
                created_at: row.try_get("created_at")?,
                expires_at: row.try_get("challenge_expires_at")?,
            })
        } else {
            None
        };
        Ok(Some(WalletSession {
            chain_id,
            address,
            expires_at: row.try_get("expires_at")?,
            challenge,
            contract_signature: signature,
        }))
    }
    pub async fn revoke_auth_session(&self, token_hash: &str) -> Result<(), AuthStoreError> {
        sqlx::query("UPDATE security_wallet_sessions SET revoked_at=now() WHERE token_hash=$1 AND revoked_at IS NULL").bind(token_hash).execute(&self.pool).await?;
        Ok(())
    }
    pub async fn claim_resource_ownership(
        &self,
        kind: &str,
        id: &str,
        principal: &str,
    ) -> Result<(), AuthStoreError> {
        let mut tx = self.pool.begin().await?;
        claim_resource_ownership_in_transaction(&mut tx, kind, id, principal).await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn resource_is_authorized(
        &self,
        kind: &str,
        id: &str,
        principal: &str,
        permission: &str,
    ) -> Result<bool, AuthStoreError> {
        if !matches!(permission, "read" | "write") {
            return Ok(false);
        }
        Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM security_resource_owners o WHERE o.resource_kind=$1 AND o.resource_id=$2 AND (o.principal_key=$3 OR EXISTS(SELECT 1 FROM security_resource_grants g WHERE g.resource_kind=o.resource_kind AND g.resource_id=o.resource_id AND g.grantee_key=$3 AND g.permission=$4 AND g.expires_at>now())))")
            .bind(kind).bind(id).bind(principal).bind(permission).fetch_one(&self.pool).await?)
    }
    pub async fn grant_resource_access(
        &self,
        kind: &str,
        id: &str,
        owner: &str,
        grantee: &str,
        permission: &str,
        expires: DateTime<Utc>,
    ) -> Result<(), AuthStoreError> {
        if !matches!(permission, "read" | "write") || !bounded(grantee, 200) {
            return Err(AuthStoreError::InvalidInput);
        }
        let count=sqlx::query("INSERT INTO security_resource_grants(resource_kind,resource_id,grantee_key,permission,expires_at) SELECT resource_kind,resource_id,$4,$5,$6 FROM security_resource_owners WHERE resource_kind=$1 AND resource_id=$2 AND principal_key=$3 AND $6>now() ON CONFLICT(resource_kind,resource_id,grantee_key,permission) DO UPDATE SET expires_at=EXCLUDED.expires_at")
            .bind(kind).bind(id).bind(owner).bind(grantee).bind(permission).bind(expires).execute(&self.pool).await?.rows_affected();
        if count == 1 {
            Ok(())
        } else {
            Err(AuthStoreError::Conflict)
        }
    }
    pub async fn revoke_resource_access(
        &self,
        kind: &str,
        id: &str,
        owner: &str,
        grantee: &str,
    ) -> Result<(), AuthStoreError> {
        let owned: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM security_resource_owners WHERE resource_kind=$1 AND resource_id=$2 AND principal_key=$3)")
            .bind(kind).bind(id).bind(owner).fetch_one(&self.pool).await?;
        if !owned {
            return Err(AuthStoreError::Conflict);
        }
        sqlx::query("DELETE FROM security_resource_grants WHERE resource_kind=$1 AND resource_id=$2 AND grantee_key=$3")
            .bind(kind).bind(id).bind(grantee).execute(&self.pool).await?;
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    pub async fn consume_security_quota_once(
        &self,
        principal: &str,
        action: &str,
        key_hash: &str,
        payload_hash: &str,
        limit: i64,
        window_seconds: i64,
    ) -> Result<QuotaReservation, AuthStoreError> {
        let mut tx = self.pool.begin().await?;
        let result = consume_security_quota_in_transaction(
            &mut tx,
            principal,
            action,
            key_hash,
            payload_hash,
            limit,
            window_seconds,
        )
        .await?;
        tx.commit().await?;
        Ok(result)
    }
    /// Bounded housekeeping; leaves a consumed challenge while its session is retained.
    pub async fn prune_security_auth_state(&self) -> Result<(), AuthStoreError> {
        sqlx::query("DELETE FROM security_wallet_sessions WHERE token_hash IN (SELECT token_hash FROM security_wallet_sessions WHERE expires_at<now()-interval '1 hour' LIMIT 1000)").execute(&self.pool).await?;
        sqlx::query("DELETE FROM security_wallet_challenges WHERE id IN (SELECT c.id FROM security_wallet_challenges c WHERE c.expires_at<now()-interval '1 hour' AND NOT EXISTS(SELECT 1 FROM security_wallet_sessions s WHERE s.challenge_id=c.id) LIMIT 1000)").execute(&self.pool).await?;
        sqlx::query("DELETE FROM security_quota_reservations WHERE ctid IN (SELECT ctid FROM security_quota_reservations WHERE expires_at<=now() LIMIT 1000)").execute(&self.pool).await?;
        Ok(())
    }
}

/// Compose quota reservation with its resource mutation in the caller's transaction.
#[allow(clippy::too_many_arguments)]
pub async fn consume_security_quota_in_transaction(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    principal: &str,
    action: &str,
    key_hash: &str,
    payload_hash: &str,
    limit: i64,
    window_seconds: i64,
) -> Result<QuotaReservation, AuthStoreError> {
    if !bounded(principal, 200)
        || !bounded(action, 40)
        || !digest(key_hash)
        || !digest(payload_hash)
        || limit <= 0
        || !(1..=86400).contains(&window_seconds)
    {
        return Err(AuthStoreError::InvalidInput);
    }
    // Serialize every quota bucket across both API and MCP instances.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 4270265038))")
        .bind(format!("{principal}\0{action}").replace('\0', ":"))
        .execute(&mut **tx)
        .await?;
    sqlx::query("DELETE FROM security_quota_reservations WHERE principal_key=$1 AND action=$2 AND expires_at<=now()")
            .bind(principal).bind(action).execute(&mut **tx).await?;
    let existing:Option<String>=sqlx::query_scalar("SELECT payload_hash FROM security_quota_reservations WHERE principal_key=$1 AND action=$2 AND idempotency_hash=$3")
            .bind(principal).bind(action).bind(key_hash).fetch_optional(&mut **tx).await?;
    if let Some(existing) = existing {
        if existing != payload_hash {
            return Err(AuthStoreError::Conflict);
        }
        return Ok(QuotaReservation { is_new: false });
    }
    let used:i64=sqlx::query_scalar("SELECT count(*) FROM security_quota_reservations WHERE principal_key=$1 AND action=$2 AND created_at>now()-make_interval(secs=>$3::double precision)")
            .bind(principal).bind(action).bind(window_seconds as f64).fetch_one(&mut **tx).await?;
    if used >= limit {
        return Err(AuthStoreError::QuotaExceeded);
    }
    sqlx::query("INSERT INTO security_quota_reservations(principal_key,action,idempotency_hash,payload_hash,expires_at) VALUES($1,$2,$3,$4,now()+make_interval(secs=>$5::double precision))")
            .bind(principal).bind(action).bind(key_hash).bind(payload_hash).bind(if matches!(action, "nonce_ip" | "nonce_wallet" | "auth_exchange_ip" | "auth_read_ip" | "auth_revoke_ip" | "managed_read" | "managed_accept" | "managed_checkout" | "managed_quote_issue") { 300.0 } else {604800.0}).execute(&mut **tx).await?;
    Ok(QuotaReservation { is_new: true })
}

pub async fn claim_resource_ownership_in_transaction(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    kind: &str,
    id: &str,
    principal: &str,
) -> Result<(), AuthStoreError> {
    if !bounded(kind, 40) || !bounded(id, 200) || !bounded(principal, 200) {
        return Err(AuthStoreError::InvalidInput);
    }
    let row:Option<String>=sqlx::query_scalar("INSERT INTO security_resource_owners(resource_kind,resource_id,principal_key) VALUES($1,$2,$3) ON CONFLICT(resource_kind,resource_id) DO UPDATE SET principal_key=security_resource_owners.principal_key WHERE security_resource_owners.principal_key=EXCLUDED.principal_key RETURNING principal_key")
            .bind(kind).bind(id).bind(principal).fetch_optional(&mut **tx).await?;
    row.ok_or(AuthStoreError::Conflict).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use sha2::{Digest, Sha256};
    fn hash(value: &str) -> String {
        hex::encode(Sha256::digest(value.as_bytes()))
    }
    #[tokio::test]
    #[ignore = "requires isolated SECURITY_AUTH_TEST_DATABASE_URL on localhost"]
    async fn durable_nonce_session_owner_and_quota_boundaries() {
        let url =
            std::env::var("SECURITY_AUTH_TEST_DATABASE_URL").expect("isolated test DB required");
        crate::test_support::assert_isolated_security_database(&url);
        let store = PostgresStore::connect(&url).await.unwrap();
        store.migrate().await.unwrap();
        let id = Uuid::new_v4();
        let now = Utc::now();
        let challenge = WalletChallenge {
            id,
            domain: "localhost".into(),
            chain_id: 8453,
            address: "0x1111111111111111111111111111111111111111".into(),
            message: "synthetic local test".into(),
            created_at: now,
            expires_at: now + Duration::minutes(5),
        };
        store.create_wallet_challenge(&challenge).await.unwrap();
        for _ in 0..5 {
            assert!(store.wallet_challenge_attempt(id).await.is_ok());
        }
        assert!(matches!(
            store.wallet_challenge_attempt(id).await,
            Err(AuthStoreError::InvalidChallenge)
        ));
        let a = store.clone();
        let b = store.clone();
        let token = hash(&format!("local synthetic token {id}"));
        let token2 = hash(&format!("local synthetic different token {id}"));
        let (a, b) = tokio::join!(
            a.consume_wallet_challenge_and_create_session(id, &token, None),
            b.consume_wallet_challenge_and_create_session(id, &token2, None)
        );
        assert_eq!(
            usize::from(a.is_ok()) + usize::from(b.is_ok()),
            1,
            "exactly one nonce exchange commits"
        );
        let token = if a.is_ok() { token } else { token2 };
        let session = store.find_auth_session(&token).await.unwrap().unwrap();
        assert_eq!(session.address, challenge.address);
        assert!(session.expires_at <= now + Duration::minutes(15));
        store.revoke_auth_session(&token).await.unwrap();
        assert!(store.find_auth_session(&token).await.unwrap().is_none());
        let expired = WalletChallenge {
            id: Uuid::new_v4(),
            created_at: now - Duration::minutes(10),
            expires_at: now - Duration::minutes(5),
            ..challenge
        };
        store.create_wallet_challenge(&expired).await.unwrap();
        assert!(store.wallet_challenge_attempt(expired.id).await.is_err());

        let resource = Uuid::new_v4().to_string();
        store
            .claim_resource_ownership("bounty", &resource, "account:owner")
            .await
            .unwrap();
        assert!(store
            .claim_resource_ownership("bounty", &resource, "wallet:8453:other")
            .await
            .is_err());
        assert!(!store
            .resource_is_authorized("bounty", &resource, "wallet:8453:other", "read")
            .await
            .unwrap());
        store
            .grant_resource_access(
                "bounty",
                &resource,
                "account:owner",
                "wallet:8453:other",
                "read",
                now + Duration::minutes(1),
            )
            .await
            .unwrap();
        assert!(store
            .resource_is_authorized("bounty", &resource, "wallet:8453:other", "read")
            .await
            .unwrap());
        assert!(!store
            .resource_is_authorized("bounty", &resource, "wallet:8453:other", "write")
            .await
            .unwrap());
        assert!(!store
            .resource_is_authorized("bounty", "unowned-legacy", "account:owner", "read")
            .await
            .unwrap());
        assert!(store
            .grant_resource_access(
                "bounty",
                &resource,
                "wallet:8453:other",
                "account:attacker",
                "write",
                now + Duration::minutes(1)
            )
            .await
            .is_err());

        let principal = format!("account:{}", Uuid::new_v4());
        let key = hash("same request");
        let payload = hash("same payload");
        assert!(
            store
                .consume_security_quota_once(&principal, "registration", &key, &payload, 5, 86400)
                .await
                .unwrap()
                .is_new
        );
        assert!(
            !store
                .consume_security_quota_once(&principal, "registration", &key, &payload, 5, 86400)
                .await
                .unwrap()
                .is_new
        );
        assert!(matches!(
            store
                .consume_security_quota_once(
                    &principal,
                    "registration",
                    &key,
                    &hash("changed payload"),
                    5,
                    86400
                )
                .await,
            Err(AuthStoreError::Conflict)
        ));
        for i in 1..5 {
            store
                .consume_security_quota_once(
                    &principal,
                    "registration",
                    &hash(&format!("request{i}")),
                    &payload,
                    5,
                    86400,
                )
                .await
                .unwrap();
        }
        assert!(matches!(
            store
                .consume_security_quota_once(
                    &principal,
                    "registration",
                    &hash("sixth"),
                    &payload,
                    5,
                    86400
                )
                .await,
            Err(AuthStoreError::QuotaExceeded)
        ));
        let rollback_principal = format!("account:{}", Uuid::new_v4());
        let rollback_resource = Uuid::new_v4().to_string();
        let mut tx = store.pool.begin().await.unwrap();
        consume_security_quota_in_transaction(
            &mut tx,
            &rollback_principal,
            "registration",
            &key,
            &payload,
            5,
            86400,
        )
        .await
        .unwrap();
        claim_resource_ownership_in_transaction(
            &mut tx,
            "agent",
            &rollback_resource,
            &rollback_principal,
        )
        .await
        .unwrap();
        tx.rollback().await.unwrap();
        assert!(!store
            .resource_is_authorized("agent", &rollback_resource, &rollback_principal, "read")
            .await
            .unwrap());
        assert!(
            store
                .consume_security_quota_once(
                    &rollback_principal,
                    "registration",
                    &key,
                    &payload,
                    5,
                    86400
                )
                .await
                .unwrap()
                .is_new
        );
    }
}
