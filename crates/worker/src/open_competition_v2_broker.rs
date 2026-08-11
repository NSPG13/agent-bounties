use anyhow::{anyhow, Context};
use chain_base::{
    fetch_base_contract_logs, fetch_safe_block_identity, fetch_transaction_receipt,
    open_competition_v2_broker_refund_digest, plan_open_competition_v2_broker_payment,
    plan_open_competition_v2_proof, rpc_logs_to_evm_logs, BaseContractLogQuery, BaseRpcUrlConfig,
    BaseTransactionRelayer, ChainBaseError, EvmLog, OpenCompetitionV2BrokerPaymentAuthorization,
    OpenCompetitionV2Event, OpenCompetitionV2EventKind, OpenCompetitionV2ProofSystem,
};
use chrono::{Duration as ChronoDuration, Utc};
use db::{
    OpenCompetitionV2ProofJob, OpenCompetitionV2ProofJobState, OpenCompetitionV2ProofJobUpdate,
    PostgresStore,
};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha3::{Digest, Keccak256};
use std::time::Duration;
use uuid::Uuid;

pub const OPEN_COMPETITION_V2_PROVER_URL_ENV: &str = "OPEN_COMPETITION_V2_PROVER_URL";
pub const OPEN_COMPETITION_V2_PROVER_API_KEY_ENV: &str = "OPEN_COMPETITION_V2_PROVER_API_KEY";
pub const OPEN_COMPETITION_V2_RELAYER_PRIVATE_KEY_ENV: &str = "X402_RELAYER_PRIVATE_KEY";

#[derive(Clone)]
pub struct OpenCompetitionV2BrokerConfig {
    pub prover_url: Url,
    pub prover_api_key: Option<String>,
    pub request_timeout_seconds: u64,
    pub lease_seconds: u32,
    pub refund_window_seconds: i64,
    client: reqwest::Client,
}

impl std::fmt::Debug for OpenCompetitionV2BrokerConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OpenCompetitionV2BrokerConfig")
            .field("prover_url", &self.prover_url)
            .field("prover_api_key_configured", &self.prover_api_key.is_some())
            .field("request_timeout_seconds", &self.request_timeout_seconds)
            .field("lease_seconds", &self.lease_seconds)
            .field("refund_window_seconds", &self.refund_window_seconds)
            .finish()
    }
}

impl OpenCompetitionV2BrokerConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    pub fn from_lookup<F>(lookup: F) -> anyhow::Result<Self>
    where
        F: Fn(&str) -> Option<String>,
    {
        let prover_url = lookup(OPEN_COMPETITION_V2_PROVER_URL_ENV)
            .context("OPEN_COMPETITION_V2_PROVER_URL is required")?
            .parse::<Url>()
            .context("OPEN_COMPETITION_V2_PROVER_URL is invalid")?;
        let is_loopback = prover_url
            .host_str()
            .is_some_and(|host| matches!(host, "127.0.0.1" | "localhost" | "::1"));
        if prover_url.scheme() != "https" && !(prover_url.scheme() == "http" && is_loopback) {
            return Err(anyhow!(
                "OPEN_COMPETITION_V2_PROVER_URL must use HTTPS, except loopback development"
            ));
        }
        if prover_url.username() != "" || prover_url.password().is_some() {
            return Err(anyhow!(
                "OPEN_COMPETITION_V2_PROVER_URL must not contain credentials"
            ));
        }
        let request_timeout_seconds =
            positive_u64(&lookup, "OPEN_COMPETITION_V2_PROVER_TIMEOUT_SECONDS", 120)?.min(600);
        let lease_seconds = u32::try_from(positive_u64(
            &lookup,
            "OPEN_COMPETITION_V2_BROKER_LEASE_SECONDS",
            request_timeout_seconds.saturating_add(30),
        )?)
        .context("OPEN_COMPETITION_V2_BROKER_LEASE_SECONDS exceeds u32")?;
        if u64::from(lease_seconds) <= request_timeout_seconds {
            return Err(anyhow!(
                "OPEN_COMPETITION_V2_BROKER_LEASE_SECONDS must exceed the prover timeout"
            ));
        }
        let refund_window_seconds = i64::try_from(positive_u64(
            &lookup,
            "OPEN_COMPETITION_V2_REFUND_WINDOW_SECONDS",
            1_800,
        )?)
        .context("OPEN_COMPETITION_V2_REFUND_WINDOW_SECONDS exceeds i64")?;
        if refund_window_seconds > 1_800 {
            return Err(anyhow!(
                "OPEN_COMPETITION_V2_REFUND_WINDOW_SECONDS cannot exceed 1800"
            ));
        }
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(request_timeout_seconds))
            .build()
            .context("failed to build V2 prover client")?;
        Ok(Self {
            prover_url,
            prover_api_key: lookup(OPEN_COMPETITION_V2_PROVER_API_KEY_ENV)
                .filter(|value| !value.trim().is_empty()),
            request_timeout_seconds,
            lease_seconds,
            refund_window_seconds,
            client,
        })
    }
}

#[derive(Clone)]
pub struct OpenCompetitionV2BrokerChainConfig {
    pub rpc_urls: BaseRpcUrlConfig,
    pub relayer: BaseTransactionRelayer,
    pub max_gas: u64,
    pub max_fee_per_gas_wei: u128,
}

impl std::fmt::Debug for OpenCompetitionV2BrokerChainConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OpenCompetitionV2BrokerChainConfig")
            .field("relayer", &self.relayer.address())
            .field("max_gas", &self.max_gas)
            .field("max_fee_per_gas_wei", &self.max_fee_per_gas_wei)
            .finish_non_exhaustive()
    }
}

impl OpenCompetitionV2BrokerChainConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        let private_key = std::env::var(OPEN_COMPETITION_V2_RELAYER_PRIVATE_KEY_ENV)
            .context("X402_RELAYER_PRIVATE_KEY is required for the V2 broker")?;
        let relayer = BaseTransactionRelayer::from_private_key(&private_key)
            .context("X402_RELAYER_PRIVATE_KEY is invalid")?;
        let max_gas = env_positive_u64("OPEN_COMPETITION_V2_RELAYER_MAX_GAS", 8_000_000)?;
        let max_fee_per_gas_wei = env_positive_u128(
            "OPEN_COMPETITION_V2_RELAYER_MAX_FEE_PER_GAS_WEI",
            10_000_000_000,
        )?;
        Ok(Self {
            rpc_urls: BaseRpcUrlConfig::from_env(),
            relayer,
            max_gas,
            max_fee_per_gas_wei,
        })
    }

    pub fn rpc(&self, network: &str) -> anyhow::Result<(u64, String, String)> {
        let (descriptor, rpc_url) = self.rpc_urls.resolve(network)?;
        Ok((
            descriptor.chain_id,
            rpc_url,
            descriptor.native_usdc_token_address,
        ))
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct ProverRequest<'a> {
    schema_version: &'static str,
    idempotency_key: &'a str,
    proof_job_id: Uuid,
    proof_system: &'a str,
    program_input: &'a Value,
    expected_public_values: &'a str,
    proof_sla_deadline: i64,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ProverStatus {
    Pending,
    Proved,
    Failed,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProverResponse {
    status: ProverStatus,
    provider_job_id: String,
    proof: Option<String>,
    public_values: Option<String>,
    failure_code: Option<String>,
    failure_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct OpenCompetitionV2BrokerPollReport {
    pub leased_job_id: Option<Uuid>,
    pub previous_state: Option<OpenCompetitionV2ProofJobState>,
    pub current_state: Option<OpenCompetitionV2ProofJobState>,
    pub action: String,
}

pub async fn poll_open_competition_v2_broker_once(
    store: &PostgresStore,
    config: &OpenCompetitionV2BrokerConfig,
    chain: &OpenCompetitionV2BrokerChainConfig,
) -> anyhow::Result<OpenCompetitionV2BrokerPollReport> {
    let lease_token = Uuid::new_v4();
    let Some(mut job) = store
        .lease_next_open_competition_v2_proof_job(lease_token, config.lease_seconds)
        .await?
    else {
        return Ok(OpenCompetitionV2BrokerPollReport {
            leased_job_id: None,
            previous_state: None,
            current_state: None,
            action: "idle".to_string(),
        });
    };
    let previous_state = job.state;
    let result = match job.state {
        OpenCompetitionV2ProofJobState::Paid | OpenCompetitionV2ProofJobState::Proving => {
            process_prover_job(store, config, &mut job).await
        }
        OpenCompetitionV2ProofJobState::Relaying => {
            process_relay_job(store, config, chain, &mut job).await
        }
        OpenCompetitionV2ProofJobState::RefundDue => {
            process_refund_job(store, chain, &mut job).await
        }
        state => Err(anyhow!("leased unsupported proof job state {state:?}")),
    };
    let release_result = store
        .release_open_competition_v2_proof_job_lease(job.id, lease_token)
        .await;
    let (current_state, action) = result?;
    if !release_result? {
        return Err(anyhow!("proof job lease ownership was lost"));
    }
    Ok(OpenCompetitionV2BrokerPollReport {
        leased_job_id: Some(job.id),
        previous_state: Some(previous_state),
        current_state: Some(current_state),
        action,
    })
}

async fn process_prover_job(
    store: &PostgresStore,
    config: &OpenCompetitionV2BrokerConfig,
    job: &mut OpenCompetitionV2ProofJob,
) -> anyhow::Result<(OpenCompetitionV2ProofJobState, String)> {
    if Utc::now() >= job.proof_sla_deadline {
        *job = mark_refund_due(
            store,
            config,
            job,
            "proof_sla_expired",
            "Proof SLA expired before a proof was delivered.",
        )
        .await?;
        return Ok((job.state, "refund_due".to_string()));
    }
    if job.state == OpenCompetitionV2ProofJobState::Paid {
        *job = store
            .transition_open_competition_v2_proof_job(
                job.id,
                OpenCompetitionV2ProofJobState::Paid,
                OpenCompetitionV2ProofJobState::Proving,
                &OpenCompetitionV2ProofJobUpdate::default(),
            )
            .await?;
    }
    let request = ProverRequest {
        schema_version: "agent-bounties/open-competition-v2-prover-request-v1",
        idempotency_key: &job.idempotency_key,
        proof_job_id: job.id,
        proof_system: &job.proof_system,
        program_input: &job.program_input,
        expected_public_values: &job.expected_public_values,
        proof_sla_deadline: job.proof_sla_deadline.timestamp(),
    };
    let mut request_builder = config
        .client
        .post(config.prover_url.clone())
        .header("idempotency-key", &job.idempotency_key)
        .json(&request);
    if let Some(api_key) = &config.prover_api_key {
        request_builder = request_builder.bearer_auth(api_key);
    }
    let response = request_builder.send().await;
    let response = match response {
        Ok(response) if response.status().is_success() => response,
        Ok(response)
            if response.status().is_server_error() || response.status().as_u16() == 429 =>
        {
            return Ok((job.state, "provider_retry".to_string()));
        }
        Ok(response) => {
            *job = mark_refund_due(
                store,
                config,
                job,
                "proof_provider_rejected",
                &format!(
                    "Proof provider returned HTTP {}.",
                    response.status().as_u16()
                ),
            )
            .await?;
            return Ok((job.state, "refund_due".to_string()));
        }
        Err(_) => return Ok((job.state, "provider_retry".to_string())),
    };
    let response = response
        .json::<ProverResponse>()
        .await
        .context("proof provider returned an invalid response")?;
    if response.provider_job_id.trim().is_empty() {
        return Err(anyhow!("proof provider omitted provider_job_id"));
    }
    match response.status {
        ProverStatus::Pending => {
            *job = store
                .transition_open_competition_v2_proof_job(
                    job.id,
                    OpenCompetitionV2ProofJobState::Proving,
                    OpenCompetitionV2ProofJobState::Proving,
                    &OpenCompetitionV2ProofJobUpdate {
                        proof_provider_job_id: Some(response.provider_job_id),
                        ..Default::default()
                    },
                )
                .await?;
            Ok((job.state, "provider_pending".to_string()))
        }
        ProverStatus::Failed => {
            *job = mark_refund_due(
                store,
                config,
                job,
                response
                    .failure_code
                    .as_deref()
                    .unwrap_or("proof_provider_failed"),
                response
                    .failure_message
                    .as_deref()
                    .unwrap_or("Proof provider failed without a reason."),
            )
            .await?;
            Ok((job.state, "refund_due".to_string()))
        }
        ProverStatus::Proved => {
            let proof = bounded_hex(
                response
                    .proof
                    .as_deref()
                    .context("proved response omitted proof")?,
                1,
                4 * 1024 * 1024,
            )?;
            if !proof_has_expected_selector(&job.proof_system, &proof) {
                *job = mark_refund_due(
                    store,
                    config,
                    job,
                    "proof_system_selector_mismatch",
                    "Proof bytes did not carry the quote-bound canonical SP1 verifier selector.",
                )
                .await?;
                return Ok((job.state, "refund_due".to_string()));
            }
            let public_values = bounded_hex(
                response
                    .public_values
                    .as_deref()
                    .context("proved response omitted public_values")?,
                640,
                640,
            )?;
            let public_values_hex = format!("0x{}", hex::encode(&public_values));
            if !public_values_hex.eq_ignore_ascii_case(&job.expected_public_values) {
                *job = mark_refund_due(
                    store,
                    config,
                    job,
                    "proof_journal_mismatch",
                    "Proof provider journal did not equal the quote-bound expected journal.",
                )
                .await?;
                return Ok((job.state, "refund_due".to_string()));
            }
            *job = store
                .transition_open_competition_v2_proof_job(
                    job.id,
                    OpenCompetitionV2ProofJobState::Proving,
                    OpenCompetitionV2ProofJobState::Proved,
                    &OpenCompetitionV2ProofJobUpdate {
                        proof_hash: Some(keccak_hex(&proof)),
                        public_values_hash: Some(keccak_hex(&public_values)),
                        proof: Some(format!("0x{}", hex::encode(proof))),
                        public_values: Some(public_values_hex),
                        proof_provider_job_id: Some(response.provider_job_id),
                        ..Default::default()
                    },
                )
                .await?;
            Ok((job.state, "proof_delivered".to_string()))
        }
    }
}

async fn process_relay_job(
    store: &PostgresStore,
    config: &OpenCompetitionV2BrokerConfig,
    chain: &OpenCompetitionV2BrokerChainConfig,
    job: &mut OpenCompetitionV2ProofJob,
) -> anyhow::Result<(OpenCompetitionV2ProofJobState, String)> {
    let events = store
        .list_open_competition_v2_events_for_contract(&job.network, &job.competition_contract)
        .await?;
    if let Some((next, update, action)) = reconcile_competition_events(job, &events)? {
        *job = store
            .transition_open_competition_v2_proof_job(job.id, job.state, next, &update)
            .await?;
        return Ok((job.state, action));
    }

    let (chain_id, rpc_url, _) = chain.rpc(&job.network)?;
    if let Some(tx_hash) = job.relay_tx_hash.as_deref() {
        let safe = fetch_safe_block_identity(&rpc_url, 71).await?;
        let receipt = fetch_transaction_receipt(&rpc_url, tx_hash, 72)
            .await?
            .result;
        if let Some(receipt) = receipt {
            let block_number = receipt.block_number()?;
            if receipt.succeeded()? == Some(false)
                && block_number.is_some_and(|block| block <= safe.number)
            {
                *job = mark_refund_due(
                    store,
                    config,
                    job,
                    "relay_transaction_reverted",
                    "The hosted proof relay reverted before a qualifying entry was indexed.",
                )
                .await?;
                return Ok((job.state, "refund_due".to_string()));
            }
        } else if job.solver_authorization_deadline.is_some_and(|deadline| {
            u64::try_from(Utc::now().timestamp()).unwrap_or(u64::MAX) >= deadline
        }) {
            *job = mark_refund_due(
                store,
                config,
                job,
                "relay_authorization_expired_unmined",
                "The relay remained unmined until its authorization could no longer execute successfully.",
            )
            .await?;
            return Ok((job.state, "refund_due".to_string()));
        }
        return Ok((job.state, "awaiting_safe_entry".to_string()));
    }

    let authorization_deadline = job
        .solver_authorization_deadline
        .context("relaying proof job omitted solver authorization deadline")?;
    if u64::try_from(Utc::now().timestamp()).unwrap_or(u64::MAX) >= authorization_deadline {
        *job = mark_refund_due(
            store,
            config,
            job,
            "relay_authorization_expired",
            "The scoped solver authorization expired before a qualifying entry was confirmed.",
        )
        .await?;
        return Ok((job.state, "refund_due".to_string()));
    }
    let prepared_relay = (|| -> anyhow::Result<_> {
        let public_values = bounded_hex(
            job.public_values
                .as_deref()
                .context("relaying proof job omitted public values")?,
            640,
            640,
        )?;
        let proof = bounded_hex(
            job.proof
                .as_deref()
                .context("relaying proof job omitted proof")?,
            1,
            4 * 1024 * 1024,
        )?;
        let signature = bounded_hex(
            job.solver_signature
                .as_deref()
                .context("relaying proof job omitted solver signature")?,
            1,
            16 * 1024,
        )?;
        let proof_system = parse_proof_system(&job.proof_system)?;
        let plan = plan_open_competition_v2_proof(
            &job.network,
            &job.competition_contract,
            &job.solver,
            job.solver_nonce
                .parse::<u128>()
                .context("solver nonce is invalid")?,
            proof_system,
            &public_values,
            &proof,
            authorization_deadline,
            Some(&signature),
        )?;
        plan.relay_call_after_signature
            .context("proof planner did not create a relay call")
    })();
    let mut intent = match prepared_relay {
        Ok(intent) => intent,
        Err(_) => {
            *job = mark_refund_due(
                store,
                config,
                job,
                "relay_preparation_failed",
                "The hosted broker could not construct the exact authorized proof relay.",
            )
            .await?;
            return Ok((job.state, "refund_due".to_string()));
        }
    };
    intent.from = Some(chain.relayer.address());
    let transaction = match chain
        .relayer
        .simulate_and_broadcast(
            &rpc_url,
            chain_id,
            &intent,
            chain.max_gas,
            chain.max_fee_per_gas_wei,
        )
        .await
    {
        Ok(transaction) => transaction,
        Err(error) => match relay_failure_disposition(&error) {
            RelayFailureDisposition::Retry => {
                return Ok((job.state, "relay_retry".to_string()));
            }
            RelayFailureDisposition::Refund { code, message } => {
                *job = mark_refund_due(store, config, job, code, message).await?;
                return Ok((job.state, "refund_due".to_string()));
            }
        },
    };
    *job = store
        .transition_open_competition_v2_proof_job(
            job.id,
            OpenCompetitionV2ProofJobState::Relaying,
            OpenCompetitionV2ProofJobState::Relaying,
            &OpenCompetitionV2ProofJobUpdate {
                relay_tx_hash: Some(transaction.tx_hash),
                ..Default::default()
            },
        )
        .await?;
    Ok((job.state, "relay_broadcast".to_string()))
}

#[derive(Debug, PartialEq, Eq)]
enum RelayFailureDisposition {
    Retry,
    Refund {
        code: &'static str,
        message: &'static str,
    },
}

fn relay_failure_disposition(error: &ChainBaseError) -> RelayFailureDisposition {
    match error {
        ChainBaseError::RelayerProvider(_) => RelayFailureDisposition::Retry,
        ChainBaseError::RelayerSimulation(_) => RelayFailureDisposition::Refund {
            code: "relay_simulation_failed",
            message: "The exact authorized proof relay was rejected during contract simulation.",
        },
        ChainBaseError::RelayerInsufficientBalance { .. } => RelayFailureDisposition::Refund {
            code: "relay_gas_unavailable",
            message: "The hosted relayer could not fund the bounded transaction cost.",
        },
        ChainBaseError::RelayerGasLimitExceeded { .. }
        | ChainBaseError::RelayerFeeCapExceeded { .. } => RelayFailureDisposition::Refund {
            code: "relay_policy_cap_exceeded",
            message: "The proof relay exceeded its precommitted gas or fee cap.",
        },
        ChainBaseError::RelayerChainMismatch { .. } => RelayFailureDisposition::Refund {
            code: "relay_chain_mismatch",
            message: "The hosted relayer was connected to the wrong chain.",
        },
        _ => RelayFailureDisposition::Refund {
            code: "relay_validation_failed",
            message: "The exact authorized proof relay failed deterministic validation.",
        },
    }
}

fn reconcile_competition_events(
    job: &OpenCompetitionV2ProofJob,
    events: &[OpenCompetitionV2Event],
) -> anyhow::Result<
    Option<(
        OpenCompetitionV2ProofJobState,
        OpenCompetitionV2ProofJobUpdate,
        String,
    )>,
> {
    let public_values = bounded_hex(&job.expected_public_values, 640, 640)?;
    let submission_hash = format!("0x{}", hex::encode(&public_values[6 * 32..7 * 32]));
    let matching_entry = events.iter().find(|event| {
        event.kind == OpenCompetitionV2EventKind::EntryQualified
            && json_address(&event.data, "solver")
                .is_some_and(|solver| solver.eq_ignore_ascii_case(&job.solver))
            && json_decimal(&event.data, "solver_nonce").as_deref()
                == Some(job.solver_nonce.as_str())
            && json_address(&event.data, "submission_hash")
                .is_some_and(|hash| hash.eq_ignore_ascii_case(&submission_hash))
    });
    let settled = events
        .iter()
        .find(|event| event.kind == OpenCompetitionV2EventKind::CompetitionSettled);
    if let Some(settled) = settled {
        if let Some(entry) = matching_entry {
            let winning_sequence = json_decimal(&settled.data, "winning_sequence");
            let entry_sequence = json_decimal(&entry.data, "sequence");
            if winning_sequence == entry_sequence
                && json_address(&settled.data, "solver")
                    .is_some_and(|solver| solver.eq_ignore_ascii_case(&job.solver))
            {
                return Ok(Some((
                    OpenCompetitionV2ProofJobState::Confirmed,
                    OpenCompetitionV2ProofJobUpdate {
                        relay_tx_hash: Some(entry.tx_hash.clone()),
                        settlement_event_id: Some(settled.id),
                        ..Default::default()
                    },
                    "settlement_confirmed".to_string(),
                )));
            }
        }
        return Ok(Some((
            OpenCompetitionV2ProofJobState::LostCompetition,
            OpenCompetitionV2ProofJobUpdate {
                relay_tx_hash: matching_entry.map(|entry| entry.tx_hash.clone()),
                failure_code: Some("competition_lost".to_string()),
                failure_message: Some(
                    "A different qualifying entry won the immutable competition rules.".to_string(),
                ),
                ..Default::default()
            },
            "competition_lost".to_string(),
        )));
    }
    if events
        .iter()
        .any(|event| event.kind == OpenCompetitionV2EventKind::CompetitionCancelled)
        && matching_entry.is_none()
    {
        return Ok(Some((
            OpenCompetitionV2ProofJobState::RefundDue,
            OpenCompetitionV2ProofJobUpdate {
                refund_due_at: Some(Utc::now() + ChronoDuration::minutes(30)),
                failure_code: Some("competition_cancelled_before_entry".to_string()),
                failure_message: Some(
                    "The competition cancelled before the hosted relay produced a qualifying entry."
                        .to_string(),
                ),
                ..Default::default()
            },
            "refund_due".to_string(),
        )));
    }
    Ok(None)
}

async fn process_refund_job(
    store: &PostgresStore,
    chain: &OpenCompetitionV2BrokerChainConfig,
    job: &mut OpenCompetitionV2ProofJob,
) -> anyhow::Result<(OpenCompetitionV2ProofJobState, String)> {
    let payer = job
        .payer
        .as_deref()
        .context("refundable proof job omitted payer")?;
    let amount = job
        .maximum_charge
        .parse::<u64>()
        .context("proof job maximum charge is invalid")?;
    let (chain_id, rpc_url, settlement_token) = chain.rpc(&job.network)?;
    let refund_nonce = refund_nonce(job);
    if let Some((tx_hash, block_number, evidence)) = find_canonical_refund(
        &rpc_url,
        &settlement_token,
        &chain.relayer.address(),
        payer,
        amount,
        &refund_nonce,
        job,
    )
    .await?
    {
        *job = store
            .transition_open_competition_v2_proof_job(
                job.id,
                OpenCompetitionV2ProofJobState::RefundDue,
                OpenCompetitionV2ProofJobState::Refunded,
                &OpenCompetitionV2ProofJobUpdate {
                    refund_tx_hash: Some(tx_hash),
                    refund_block_number: Some(block_number),
                    refund_evidence: Some(evidence),
                    ..Default::default()
                },
            )
            .await?;
        return Ok((job.state, "refund_confirmed".to_string()));
    }
    if job.refund_tx_hash.is_some() {
        return Ok((job.state, "awaiting_safe_refund".to_string()));
    }

    let now = u64::try_from(Utc::now().timestamp()).context("system clock is before epoch")?;
    let valid_before = now.checked_add(600).context("refund validity overflow")?;
    let digest = open_competition_v2_broker_refund_digest(
        &job.network,
        &settlement_token,
        &chain.relayer.address(),
        payer,
        amount,
        valid_before,
        &refund_nonce,
    )?;
    let signature = bounded_hex(&chain.relayer.sign_digest(&digest)?, 65, 65)?;
    let authorization = OpenCompetitionV2BrokerPaymentAuthorization {
        payer: chain.relayer.address(),
        recipient: payer.to_string(),
        amount,
        valid_before,
        nonce: refund_nonce,
        v: signature[64],
        r: format!("0x{}", hex::encode(&signature[..32])),
        s: format!("0x{}", hex::encode(&signature[32..64])),
    };
    let intent = plan_open_competition_v2_broker_payment(
        &job.network,
        &settlement_token,
        &chain.relayer.address(),
        &authorization,
    )?;
    let transaction = chain
        .relayer
        .simulate_and_broadcast(
            &rpc_url,
            chain_id,
            &intent,
            chain.max_gas,
            chain.max_fee_per_gas_wei,
        )
        .await?;
    *job = store
        .transition_open_competition_v2_proof_job(
            job.id,
            OpenCompetitionV2ProofJobState::RefundDue,
            OpenCompetitionV2ProofJobState::RefundDue,
            &OpenCompetitionV2ProofJobUpdate {
                refund_tx_hash: Some(transaction.tx_hash),
                ..Default::default()
            },
        )
        .await?;
    Ok((job.state, "refund_broadcast".to_string()))
}

#[allow(clippy::too_many_arguments)]
async fn find_canonical_refund(
    rpc_url: &str,
    settlement_token: &str,
    broker: &str,
    payer: &str,
    amount: u64,
    refund_nonce: &str,
    job: &OpenCompetitionV2ProofJob,
) -> anyhow::Result<Option<(String, u64, Value)>> {
    let safe = fetch_safe_block_identity(rpc_url, 81).await?;
    if let Some(tx_hash) = job.refund_tx_hash.as_deref() {
        return canonical_refund_from_transaction(
            rpc_url,
            tx_hash,
            settlement_token,
            broker,
            payer,
            amount,
            refund_nonce,
            &safe,
        )
        .await;
    }

    let from_block = job
        .payment_block_number
        .unwrap_or(safe.number)
        .min(safe.number);
    let query = BaseContractLogQuery::new(
        settlement_token,
        from_block,
        Some(safe.number),
        vec![
            event_topic("Transfer(address,address,uint256)"),
            event_topic("AuthorizationUsed(address,bytes32)"),
        ],
    )?;
    let logs = rpc_logs_to_evm_logs(fetch_base_contract_logs(rpc_url, &query, 82).await?.result)?;
    let authorization_topic = event_topic("AuthorizationUsed(address,bytes32)");
    let broker_topic = address_topic(broker)?;
    let nonce_topic = normalize_word(refund_nonce)?;
    let tx_hash = logs.iter().find_map(|log| {
        (log.topics.len() == 3
            && log.topics[0].eq_ignore_ascii_case(&authorization_topic)
            && log.topics[1].eq_ignore_ascii_case(&broker_topic)
            && log.topics[2].eq_ignore_ascii_case(&nonce_topic))
        .then(|| log.tx_hash.clone())
    });
    let Some(tx_hash) = tx_hash else {
        return Ok(None);
    };
    canonical_refund_from_transaction(
        rpc_url,
        &tx_hash,
        settlement_token,
        broker,
        payer,
        amount,
        refund_nonce,
        &safe,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn canonical_refund_from_transaction(
    rpc_url: &str,
    tx_hash: &str,
    settlement_token: &str,
    broker: &str,
    payer: &str,
    amount: u64,
    refund_nonce: &str,
    safe: &chain_base::BaseBlockIdentity,
) -> anyhow::Result<Option<(String, u64, Value)>> {
    let Some(receipt) = fetch_transaction_receipt(rpc_url, tx_hash, 83)
        .await?
        .result
    else {
        return Ok(None);
    };
    let Some(block_number) = receipt.block_number()? else {
        return Ok(None);
    };
    if block_number > safe.number || receipt.succeeded()? != Some(true) {
        return Ok(None);
    }
    let logs = receipt.logs_to_evm_logs()?;
    let has_transfer = logs
        .iter()
        .any(|log| exact_usdc_transfer(log, settlement_token, broker, payer, amount, tx_hash));
    let authorization_topic = event_topic("AuthorizationUsed(address,bytes32)");
    let broker_topic = address_topic(broker)?;
    let nonce_topic = normalize_word(refund_nonce)?;
    let has_authorization = logs.iter().any(|log| {
        log.address.eq_ignore_ascii_case(settlement_token)
            && log.topics.len() == 3
            && log.topics[0].eq_ignore_ascii_case(&authorization_topic)
            && log.topics[1].eq_ignore_ascii_case(&broker_topic)
            && log.topics[2].eq_ignore_ascii_case(&nonce_topic)
    });
    if !has_transfer || !has_authorization {
        return Err(anyhow!(
            "refund transaction did not contain the exact authorization and transfer"
        ));
    }
    Ok(Some((
        tx_hash.to_ascii_lowercase(),
        block_number,
        serde_json::json!({
            "schema_version": "agent-bounties/open-competition-v2-proof-refund-evidence-v1",
            "asset": settlement_token,
            "payer": broker,
            "recipient": payer,
            "amount": amount.to_string(),
            "authorization_nonce": refund_nonce,
            "transaction_hash": tx_hash,
            "block_number": block_number,
            "block_hash": receipt.block_hash,
            "safe_block_number": safe.number,
            "safe_block_hash": safe.hash,
        }),
    )))
}

async fn mark_refund_due(
    store: &PostgresStore,
    config: &OpenCompetitionV2BrokerConfig,
    job: &OpenCompetitionV2ProofJob,
    code: &str,
    message: &str,
) -> anyhow::Result<OpenCompetitionV2ProofJob> {
    Ok(store
        .transition_open_competition_v2_proof_job(
            job.id,
            job.state,
            OpenCompetitionV2ProofJobState::RefundDue,
            &OpenCompetitionV2ProofJobUpdate {
                refund_due_at: Some(
                    Utc::now() + ChronoDuration::seconds(config.refund_window_seconds),
                ),
                failure_code: Some(code.chars().take(80).collect()),
                failure_message: Some(message.chars().take(300).collect()),
                ..Default::default()
            },
        )
        .await?)
}

fn bounded_hex(value: &str, minimum: usize, maximum: usize) -> anyhow::Result<Vec<u8>> {
    let raw = value
        .strip_prefix("0x")
        .context("proof values must be 0x-prefixed")?;
    if raw.len() % 2 != 0 || raw.len() / 2 < minimum || raw.len() / 2 > maximum {
        return Err(anyhow!("proof value length is outside the allowed range"));
    }
    hex::decode(raw).context("proof values contain non-hex data")
}

fn keccak_hex(value: &[u8]) -> String {
    format!("0x{}", hex::encode(Keccak256::digest(value)))
}

fn parse_proof_system(value: &str) -> anyhow::Result<OpenCompetitionV2ProofSystem> {
    match value {
        "groth16" | "sp1-groth16" => Ok(OpenCompetitionV2ProofSystem::Groth16),
        "plonk" | "sp1-plonk" => Ok(OpenCompetitionV2ProofSystem::Plonk),
        _ => Err(anyhow!("unsupported stored proof system")),
    }
}

fn proof_has_expected_selector(proof_system: &str, proof: &[u8]) -> bool {
    let expected = match proof_system {
        "groth16" | "sp1-groth16" => [0x43, 0x88, 0xa2, 0x1c],
        "plonk" | "sp1-plonk" => [0x5a, 0x09, 0x3a, 0x2f],
        _ => return false,
    };
    proof.starts_with(&expected)
}

fn refund_nonce(job: &OpenCompetitionV2ProofJob) -> String {
    keccak_hex(
        format!(
            "agent-bounties/open-competition-v2-proof-refund-v1/{}/{}",
            job.network, job.id
        )
        .as_bytes(),
    )
}

fn json_address<'a>(value: &'a Value, field: &str) -> Option<&'a str> {
    value.get(field).and_then(Value::as_str)
}

fn json_decimal(value: &Value, field: &str) -> Option<String> {
    value.get(field).and_then(|value| {
        value
            .as_str()
            .map(str::to_string)
            .or_else(|| value.as_u64().map(|value| value.to_string()))
    })
}

fn event_topic(signature: &str) -> String {
    keccak_hex(signature.as_bytes())
}

fn address_topic(address: &str) -> anyhow::Result<String> {
    let raw = address
        .strip_prefix("0x")
        .or_else(|| address.strip_prefix("0X"))
        .context("address must be 0x-prefixed")?;
    if raw.len() != 40 || !raw.bytes().all(|value| value.is_ascii_hexdigit()) {
        return Err(anyhow!("address is not 20-byte hex"));
    }
    Ok(format!("0x{}{}", "0".repeat(24), raw.to_ascii_lowercase()))
}

fn normalize_word(value: &str) -> anyhow::Result<String> {
    let raw = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .context("word must be 0x-prefixed")?;
    if raw.len() != 64 || !raw.bytes().all(|value| value.is_ascii_hexdigit()) {
        return Err(anyhow!("word is not 32-byte hex"));
    }
    Ok(format!("0x{}", raw.to_ascii_lowercase()))
}

fn exact_usdc_transfer(
    log: &EvmLog,
    token: &str,
    payer: &str,
    recipient: &str,
    amount: u64,
    tx_hash: &str,
) -> bool {
    if !log.address.eq_ignore_ascii_case(token)
        || !log.tx_hash.eq_ignore_ascii_case(tx_hash)
        || log.topics.len() != 3
        || !log.topics[0].eq_ignore_ascii_case(&event_topic("Transfer(address,address,uint256)"))
    {
        return false;
    }
    let Ok(payer_topic) = address_topic(payer) else {
        return false;
    };
    let Ok(recipient_topic) = address_topic(recipient) else {
        return false;
    };
    if !log.topics[1].eq_ignore_ascii_case(&payer_topic)
        || !log.topics[2].eq_ignore_ascii_case(&recipient_topic)
    {
        return false;
    }
    let Some(raw) = log.data.strip_prefix("0x") else {
        return false;
    };
    raw.len() == 64
        && raw[..48].bytes().all(|value| value == b'0')
        && u64::from_str_radix(&raw[48..], 16).ok() == Some(amount)
}

fn positive_u64<F>(lookup: &F, key: &str, default: u64) -> anyhow::Result<u64>
where
    F: Fn(&str) -> Option<String>,
{
    let value = lookup(key)
        .map(|value| {
            value
                .parse::<u64>()
                .with_context(|| format!("{key} must be an integer"))
        })
        .transpose()?
        .unwrap_or(default);
    if value == 0 {
        return Err(anyhow!("{key} must be positive"));
    }
    Ok(value)
}

fn env_positive_u64(key: &str, default: u64) -> anyhow::Result<u64> {
    std::env::var(key)
        .ok()
        .map(|value| {
            value
                .parse::<u64>()
                .with_context(|| format!("{key} must be an integer"))
        })
        .transpose()?
        .unwrap_or(default)
        .checked_sub(1)
        .and_then(|value| value.checked_add(1))
        .context(format!("{key} must be positive"))
}

fn env_positive_u128(key: &str, default: u128) -> anyhow::Result<u128> {
    let value = std::env::var(key)
        .ok()
        .map(|value| {
            value
                .parse::<u128>()
                .with_context(|| format!("{key} must be an integer"))
        })
        .transpose()?
        .unwrap_or(default);
    if value == 0 {
        return Err(anyhow!("{key} must be positive"));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn proof_job() -> OpenCompetitionV2ProofJob {
        let now = Utc::now();
        let mut public_values = vec![0_u8; 640];
        public_values[6 * 32..7 * 32].fill(0xaa);
        OpenCompetitionV2ProofJob {
            id: Uuid::nil(),
            idempotency_key: "quote-1".to_string(),
            network: "base-sepolia".to_string(),
            competition_contract: format!("0x{}", "11".repeat(20)),
            solver: format!("0x{}", "22".repeat(20)),
            solver_nonce: "7".to_string(),
            artifact_hash: format!("0x{}", "33".repeat(32)),
            program_input: serde_json::json!({}),
            expected_public_values: format!("0x{}", hex::encode(public_values)),
            requested_relay: true,
            proof_system: "groth16".to_string(),
            state: OpenCompetitionV2ProofJobState::Relaying,
            gross_prize: "1000000".to_string(),
            proof_fee_quote: "100000".to_string(),
            relay_fee_quote: "10000".to_string(),
            net_prize_if_win: "890000".to_string(),
            maximum_charge: "110000".to_string(),
            winner_mode: "first_proven".to_string(),
            competition_risk: "beta".to_string(),
            quote_expires_at: now + ChronoDuration::minutes(5),
            proof_sla_deadline: now + ChronoDuration::minutes(10),
            payer: Some(format!("0x{}", "44".repeat(20))),
            payment_authorization_nonce: Some(format!("0x{}", "55".repeat(32))),
            payment_authorization: None,
            payment_tx_hash: Some(format!("0x{}", "66".repeat(32))),
            payment_block_number: Some(100),
            payment_evidence: Some(serde_json::json!({"safe": true})),
            proof_hash: Some(format!("0x{}", "77".repeat(32))),
            public_values_hash: Some(format!("0x{}", "88".repeat(32))),
            proof: Some("0x1234".to_string()),
            public_values: None,
            proof_provider_job_id: Some("provider-1".to_string()),
            solver_authorization_deadline: Some(2_000_000_000),
            solver_signature: Some(format!("0x{}", "99".repeat(65))),
            relay_tx_hash: None,
            settlement_event_id: None,
            refund_evidence: None,
            refund_tx_hash: None,
            refund_block_number: None,
            refund_due_at: None,
            failure_code: None,
            failure_message: None,
            attempt_count: 0,
            lease_token: None,
            lease_expires_at: None,
            created_at: now,
            updated_at: now,
        }
    }

    fn event(kind: OpenCompetitionV2EventKind, data: Value, index: u64) -> OpenCompetitionV2Event {
        OpenCompetitionV2Event {
            id: Uuid::new_v4(),
            protocol_version: "agent-bounties/open-competition-v2-beta1".to_string(),
            log_key: format!("log-{index}"),
            tx_hash: format!("0x{:064x}", index + 1),
            block_number: 200,
            log_index: index,
            contract_address: format!("0x{}", "11".repeat(20)),
            bounty_id: format!("0x{}", "bb".repeat(32)),
            kind,
            data,
            occurred_at: Utc::now(),
        }
    }

    #[test]
    fn broker_config_requires_https_and_bounded_refunds() {
        let values = HashMap::from([(
            OPEN_COMPETITION_V2_PROVER_URL_ENV,
            "https://prover.example/jobs",
        )]);
        let config = OpenCompetitionV2BrokerConfig::from_lookup(|key| {
            values.get(key).map(|value| value.to_string())
        })
        .unwrap();
        assert_eq!(config.refund_window_seconds, 1_800);
        assert!(config.lease_seconds > config.request_timeout_seconds as u32);

        let insecure = HashMap::from([(
            OPEN_COMPETITION_V2_PROVER_URL_ENV,
            "http://prover.example/jobs",
        )]);
        assert!(OpenCompetitionV2BrokerConfig::from_lookup(|key| {
            insecure.get(key).map(|value| value.to_string())
        })
        .is_err());
    }

    #[test]
    fn proof_hex_parser_is_bounded_and_keccak_is_stable() {
        assert_eq!(bounded_hex("0x1234", 1, 2).unwrap(), vec![0x12, 0x34]);
        assert!(bounded_hex("1234", 1, 2).is_err());
        assert!(bounded_hex("0x12", 2, 2).is_err());
        assert_eq!(
            keccak_hex(b"agent-bounties"),
            "0xbef7892d64c4651df16fad4b6d6ed8a97654e5e6e3c3bbdde31b80c440c7b133"
        );
        assert!(proof_has_expected_selector(
            "groth16",
            &[0x43, 0x88, 0xa2, 0x1c, 1]
        ));
        assert!(proof_has_expected_selector(
            "plonk",
            &[0x5a, 0x09, 0x3a, 0x2f, 1]
        ));
        assert!(!proof_has_expected_selector(
            "groth16",
            &[0x5a, 0x09, 0x3a, 0x2f]
        ));
    }

    #[test]
    fn settlement_reconciliation_binds_solver_nonce_and_submission() {
        let job = proof_job();
        let entry = event(
            OpenCompetitionV2EventKind::EntryQualified,
            serde_json::json!({
                "sequence": 1,
                "solver": job.solver,
                "solver_nonce": 7,
                "submission_hash": format!("0x{}", "aa".repeat(32))
            }),
            0,
        );
        let settlement = event(
            OpenCompetitionV2EventKind::CompetitionSettled,
            serde_json::json!({"winning_sequence": 1, "solver": job.solver}),
            1,
        );
        let outcome = reconcile_competition_events(&job, &[entry, settlement.clone()])
            .unwrap()
            .unwrap();
        assert_eq!(outcome.0, OpenCompetitionV2ProofJobState::Confirmed);
        assert_eq!(outcome.1.settlement_event_id, Some(settlement.id));

        let wrong_nonce = event(
            OpenCompetitionV2EventKind::EntryQualified,
            serde_json::json!({
                "sequence": 1,
                "solver": job.solver,
                "solver_nonce": 8,
                "submission_hash": format!("0x{}", "aa".repeat(32))
            }),
            0,
        );
        let outcome = reconcile_competition_events(&job, &[wrong_nonce, settlement])
            .unwrap()
            .unwrap();
        assert_eq!(outcome.0, OpenCompetitionV2ProofJobState::LostCompetition);
    }

    #[test]
    fn deterministic_refund_identity_is_job_and_network_bound() {
        let job = proof_job();
        let nonce = refund_nonce(&job);
        assert_eq!(nonce, refund_nonce(&job));
        let mut another = job.clone();
        another.network = "base-mainnet".to_string();
        assert_ne!(nonce, refund_nonce(&another));
    }

    #[test]
    fn relay_failures_retry_only_transient_provider_errors() {
        assert_eq!(
            relay_failure_disposition(&ChainBaseError::RelayerProvider(
                "temporary timeout".to_string()
            )),
            RelayFailureDisposition::Retry
        );
        assert_eq!(
            relay_failure_disposition(&ChainBaseError::RelayerSimulation(
                "execution reverted".to_string()
            )),
            RelayFailureDisposition::Refund {
                code: "relay_simulation_failed",
                message:
                    "The exact authorized proof relay was rejected during contract simulation.",
            }
        );
        assert_eq!(
            relay_failure_disposition(&ChainBaseError::RelayerInsufficientBalance {
                balance: 1,
                required: 2,
            }),
            RelayFailureDisposition::Refund {
                code: "relay_gas_unavailable",
                message: "The hosted relayer could not fund the bounded transaction cost.",
            }
        );
    }

    #[test]
    fn exact_transfer_requires_every_bound_field() {
        let token = format!("0x{}", "01".repeat(20));
        let payer = format!("0x{}", "02".repeat(20));
        let recipient = format!("0x{}", "03".repeat(20));
        let tx_hash = format!("0x{}", "04".repeat(32));
        let log = EvmLog {
            address: token.clone(),
            topics: vec![
                event_topic("Transfer(address,address,uint256)"),
                address_topic(&payer).unwrap(),
                address_topic(&recipient).unwrap(),
            ],
            data: format!("0x{:064x}", 110_000_u64),
            tx_hash: tx_hash.clone(),
            block_number: 10,
            log_index: 0,
            occurred_at: None,
        };
        assert!(exact_usdc_transfer(
            &log, &token, &payer, &recipient, 110_000, &tx_hash
        ));
        assert!(!exact_usdc_transfer(
            &log, &token, &payer, &recipient, 110_001, &tx_hash
        ));
    }
}
