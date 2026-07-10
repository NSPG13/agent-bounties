use anyhow::{anyhow, Context};
use app::{
    BaseEscrowReconciliation, BaseReleaseAttestation, BaseReleaseTransactionEvidence, BountyNetwork,
};
use chain_base::{
    base_network_descriptor, calldata_keccak256, decode_base_release_calldata, evm_event_topic,
    fetch_base_escrow_logs, fetch_block_number, fetch_transaction_by_hash,
    fetch_transaction_receipt, normalize_evm_address, normalize_evm_hash, rpc_logs_to_evm_logs,
    BaseEscrowEvent, BaseEscrowEventKind, BaseEscrowLogDecoder, BaseEscrowLogQuery,
    BaseNetworkDescriptor, ChainEventIndexer, EvmLog,
};
use chrono::{DateTime, Utc};
use db::{
    BaseIndexerHeartbeat, BaseLogPersistenceBatch, BaseLogPersistenceCursor,
    BaseReleaseAttestationRecord, PostgresStore,
};
use domain::{Id, Submission, VerifierResult};
use ledger::{Ledger, LedgerEntry};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use verifier_sdk::{VerificationInput, Verifier, VerifierResultType};

pub struct VerificationJob<V: Verifier> {
    pub verifier: V,
    pub input: VerificationInput,
}

impl<V: Verifier> VerificationJob<V> {
    pub async fn run(self) -> VerifierResultType<VerifierResult> {
        self.verifier.verify(self.input).await
    }
}

pub fn submission_summary(submission: &Submission) -> String {
    format!(
        "submission={} bounty={} artifact_digest={}",
        submission.id, submission.bounty_id, submission.artifact_digest
    )
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaseLogCursor {
    pub last_block_number: Option<u64>,
    pub last_log_index: Option<u64>,
    pub last_log_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppliedBaseEvent {
    pub event_id: Id,
    pub bounty_id: Id,
    pub kind: BaseEscrowEventKind,
    pub log_key: String,
    pub ledger_entries: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaseLogFailure {
    pub block_number: u64,
    pub log_index: u64,
    pub log_key: String,
    pub reason: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BaseLogPipelineReport {
    pub starting_cursor: BaseLogCursor,
    pub ending_cursor: BaseLogCursor,
    pub decoded_events: usize,
    pub applied_events: Vec<AppliedBaseEvent>,
    pub skipped_duplicate_logs: usize,
    pub ledger_entries: Vec<LedgerEntry>,
    pub release_attestations: Vec<BaseReleaseAttestationRecord>,
    pub failures: Vec<BaseLogFailure>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseReleaseAttestationFailureEvidence {
    pub tx_hash: String,
    pub expected_chain_id: u64,
    pub escrow_contract: String,
    pub settlement_signer: String,
    pub platform_fee_wallet: Option<String>,
    pub input: Option<String>,
    pub reason: String,
}

#[derive(Debug)]
pub struct BaseEscrowLogWorker {
    decoder: BaseEscrowLogDecoder,
    indexer: ChainEventIndexer,
    cursor: BaseLogCursor,
}

impl Default for BaseEscrowLogWorker {
    fn default() -> Self {
        Self::new("usdc")
    }
}

impl BaseEscrowLogWorker {
    pub fn new(currency: impl Into<String>) -> Self {
        Self {
            decoder: BaseEscrowLogDecoder::new(currency),
            indexer: ChainEventIndexer::default(),
            cursor: BaseLogCursor::default(),
        }
    }

    pub fn from_indexed_events(
        currency: impl Into<String>,
        events: impl IntoIterator<Item = BaseEscrowEvent>,
    ) -> Result<Self, chain_base::ChainBaseError> {
        let events = events.into_iter().collect::<Vec<_>>();
        let mut decoder = BaseEscrowLogDecoder::new(currency);
        for event in &events {
            decoder.remember_event(event);
        }
        let cursor = cursor_from_events(&events);
        let indexer = ChainEventIndexer::from_events(events)?;
        Ok(Self {
            decoder,
            indexer,
            cursor,
        })
    }

    pub fn cursor(&self) -> &BaseLogCursor {
        &self.cursor
    }

    pub fn indexed_events(&self) -> &[BaseEscrowEvent] {
        self.indexer.events()
    }

    pub fn ingest_indexed_event(
        &mut self,
        event: BaseEscrowEvent,
    ) -> Result<(), chain_base::ChainBaseError> {
        self.decoder.remember_event(&event);
        if self.indexer.has_seen_log_key(&event.log_key) {
            return Ok(());
        }
        let block_number = event.block_number;
        let log_index = log_index_from_key(&event.log_key).unwrap_or(0);
        let log_key = event.log_key.clone();
        self.indexer.ingest(event)?;
        self.advance_cursor(block_number, log_index, log_key);
        Ok(())
    }

    pub fn process_logs(
        &mut self,
        logs: impl IntoIterator<Item = EvmLog>,
        network: &mut BountyNetwork,
    ) -> BaseLogPipelineReport {
        self.process_logs_with_release_attestations(logs, network, &HashMap::new())
    }

    pub fn process_logs_with_release_attestations(
        &mut self,
        logs: impl IntoIterator<Item = EvmLog>,
        network: &mut BountyNetwork,
        release_attestations: &HashMap<String, BaseReleaseTransactionEvidence>,
    ) -> BaseLogPipelineReport {
        self.process_logs_with_release_evidence(
            logs,
            network,
            release_attestations,
            &HashMap::new(),
        )
    }

    pub fn process_logs_with_release_evidence(
        &mut self,
        logs: impl IntoIterator<Item = EvmLog>,
        network: &mut BountyNetwork,
        release_attestations: &HashMap<String, BaseReleaseTransactionEvidence>,
        failed_release_attestations: &HashMap<String, BaseReleaseAttestationFailureEvidence>,
    ) -> BaseLogPipelineReport {
        let mut logs = logs.into_iter().collect::<Vec<_>>();
        logs.sort_by_key(|log| (log.block_number, log.log_index));

        let mut report = BaseLogPipelineReport {
            starting_cursor: self.cursor.clone(),
            ending_cursor: self.cursor.clone(),
            ..BaseLogPipelineReport::default()
        };

        for log in logs {
            let block_number = log.block_number;
            let log_index = log.log_index;
            let raw_log_key = format!("{}:{}", log.tx_hash, log.log_index);
            let event = match self.decoder.decode(log) {
                Ok(event) => event,
                Err(error) => {
                    report.failures.push(BaseLogFailure {
                        block_number,
                        log_index,
                        log_key: raw_log_key,
                        reason: error.to_string(),
                    });
                    break;
                }
            };
            report.decoded_events += 1;

            if self.indexer.has_seen_log_key(&event.log_key) {
                report.skipped_duplicate_logs += 1;
                self.advance_cursor(block_number, log_index, event.log_key.clone());
                report.ending_cursor = self.cursor.clone();
                continue;
            }

            if event.kind == BaseEscrowEventKind::Released {
                if let Some(failure) =
                    failed_release_attestations.get(&release_tx_key(&event.tx_hash))
                {
                    report
                        .release_attestations
                        .push(failed_release_attestation_record(&event, failure));
                    report.failures.push(BaseLogFailure {
                        block_number,
                        log_index,
                        log_key: event.log_key,
                        reason: failure.reason.clone(),
                    });
                    break;
                }
            }

            let reconciliation = match apply_event_with_optional_attestation(
                network,
                event.clone(),
                release_attestations,
            ) {
                Ok(reconciliation) => reconciliation,
                Err(error) => {
                    if event.kind == BaseEscrowEventKind::Released {
                        if let Some(evidence) =
                            release_attestations.get(&event.tx_hash.to_ascii_lowercase())
                        {
                            report.release_attestations.push(release_attestation_record(
                                &event,
                                evidence,
                                None,
                                "failed",
                                &error.to_string(),
                            ));
                        }
                    }
                    report.failures.push(BaseLogFailure {
                        block_number,
                        log_index,
                        log_key: event.log_key,
                        reason: error.to_string(),
                    });
                    break;
                }
            };
            let ledger_entries = reconciliation.ledger_entries.clone();
            if let Some(attestation) = reconciliation.release_attestation.as_ref() {
                if let Some(evidence) =
                    release_attestations.get(&event.tx_hash.to_ascii_lowercase())
                {
                    report.release_attestations.push(release_attestation_record(
                        &event,
                        evidence,
                        Some(attestation),
                        "passed",
                        &attestation.reason,
                    ));
                }
            }
            let applied = applied_event(&event, &reconciliation);

            if let Err(error) = self.indexer.ingest(event.clone()) {
                report.failures.push(BaseLogFailure {
                    block_number,
                    log_index,
                    log_key: event.log_key,
                    reason: error.to_string(),
                });
                break;
            }

            report.ledger_entries.extend(ledger_entries);
            report.applied_events.push(applied);
            self.advance_cursor(block_number, log_index, event.log_key);
            report.ending_cursor = self.cursor.clone();
        }

        report
    }

    fn advance_cursor(&mut self, block_number: u64, log_index: u64, log_key: String) {
        self.cursor.last_block_number = Some(block_number);
        self.cursor.last_log_index = Some(log_index);
        self.cursor.last_log_key = Some(log_key);
    }
}

fn apply_event_with_optional_attestation(
    network: &mut BountyNetwork,
    event: BaseEscrowEvent,
    release_attestations: &HashMap<String, BaseReleaseTransactionEvidence>,
) -> app::AppResult<BaseEscrowReconciliation> {
    if event.kind == BaseEscrowEventKind::Released {
        let evidence = release_attestations
            .get(&event.tx_hash.to_ascii_lowercase())
            .cloned()
            .ok_or_else(|| {
                app::AppError::InvalidBaseEscrowEvent(
                    "released event requires matching release transaction calldata attestation"
                        .to_string(),
                )
            })?;
        return network.apply_attested_base_release_event(event, evidence);
    }
    network.apply_base_escrow_event(event)
}

fn applied_event(
    event: &BaseEscrowEvent,
    reconciliation: &BaseEscrowReconciliation,
) -> AppliedBaseEvent {
    AppliedBaseEvent {
        event_id: event.id,
        bounty_id: event.bounty_id,
        kind: event.kind.clone(),
        log_key: event.log_key.clone(),
        ledger_entries: reconciliation.ledger_entries.len(),
    }
}

fn release_attestation_record(
    event: &BaseEscrowEvent,
    evidence: &BaseReleaseTransactionEvidence,
    attestation: Option<&BaseReleaseAttestation>,
    verdict: &str,
    reason: &str,
) -> BaseReleaseAttestationRecord {
    let recipients = attestation
        .map(|attestation| {
            serde_json::to_value(&attestation.recipients).unwrap_or_else(|_| serde_json::json!([]))
        })
        .unwrap_or_else(|| serde_json::json!([]));
    BaseReleaseAttestationRecord {
        id: Id::new_v4(),
        network: evidence.expected_chain_id.to_string(),
        tx_hash: normalize_evm_hash(&evidence.tx_hash).unwrap_or_else(|_| evidence.tx_hash.clone()),
        log_key: event.log_key.clone(),
        bounty_id: event.bounty_id,
        onchain_escrow_id: event.onchain_escrow_id.to_string(),
        calldata_hash: attestation
            .map(|attestation| attestation.calldata_hash.clone())
            .or_else(|| calldata_keccak256(&evidence.input).ok()),
        proof_hash: attestation
            .map(|attestation| attestation.proof_hash.clone())
            .or_else(|| event.proof_hash.clone()),
        recipients,
        escrow_contract: normalize_evm_address(&evidence.escrow_contract)
            .unwrap_or_else(|_| evidence.escrow_contract.clone()),
        settlement_signer: normalize_evm_address(&evidence.settlement_signer)
            .unwrap_or_else(|_| evidence.settlement_signer.clone()),
        platform_fee_wallet: evidence
            .platform_fee_wallet
            .as_ref()
            .map(|wallet| normalize_evm_address(wallet).unwrap_or_else(|_| wallet.clone())),
        verdict: verdict.to_string(),
        reason: reason.to_string(),
        created_at: Utc::now(),
    }
}

fn failed_release_attestation_record(
    event: &BaseEscrowEvent,
    failure: &BaseReleaseAttestationFailureEvidence,
) -> BaseReleaseAttestationRecord {
    let decoded = failure
        .input
        .as_deref()
        .and_then(|input| decode_base_release_calldata(input, "usdc").ok());
    let recipients = decoded
        .as_ref()
        .map(|decoded| {
            serde_json::to_value(&decoded.recipients).unwrap_or_else(|_| serde_json::json!([]))
        })
        .unwrap_or_else(|| serde_json::json!([]));
    BaseReleaseAttestationRecord {
        id: Id::new_v4(),
        network: failure.expected_chain_id.to_string(),
        tx_hash: normalize_evm_hash(&failure.tx_hash).unwrap_or_else(|_| failure.tx_hash.clone()),
        log_key: event.log_key.clone(),
        bounty_id: event.bounty_id,
        onchain_escrow_id: event.onchain_escrow_id.to_string(),
        calldata_hash: failure
            .input
            .as_deref()
            .and_then(|input| calldata_keccak256(input).ok()),
        proof_hash: decoded
            .as_ref()
            .map(|decoded| decoded.proof_hash.clone())
            .or_else(|| event.proof_hash.clone()),
        recipients,
        escrow_contract: normalize_evm_address(&failure.escrow_contract)
            .unwrap_or_else(|_| failure.escrow_contract.clone()),
        settlement_signer: normalize_evm_address(&failure.settlement_signer)
            .unwrap_or_else(|_| failure.settlement_signer.clone()),
        platform_fee_wallet: failure
            .platform_fee_wallet
            .as_ref()
            .map(|wallet| normalize_evm_address(wallet).unwrap_or_else(|_| wallet.clone())),
        verdict: "failed".to_string(),
        reason: failure.reason.clone(),
        created_at: Utc::now(),
    }
}

fn release_tx_key(tx_hash: &str) -> String {
    normalize_evm_hash(tx_hash).unwrap_or_else(|_| tx_hash.to_ascii_lowercase())
}

fn cursor_from_events(events: &[BaseEscrowEvent]) -> BaseLogCursor {
    events
        .iter()
        .filter_map(|event| log_index_from_key(&event.log_key).map(|index| (event, index)))
        .max_by_key(|(event, index)| (event.block_number, *index))
        .map(|(event, index)| BaseLogCursor {
            last_block_number: Some(event.block_number),
            last_log_index: Some(index),
            last_log_key: Some(event.log_key.clone()),
        })
        .unwrap_or_default()
}

fn log_index_from_key(log_key: &str) -> Option<u64> {
    log_key.rsplit_once(':')?.1.parse().ok()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaseIndexerConfig {
    pub network: String,
    pub rpc_url: String,
    pub escrow_contract: String,
    pub settlement_signer: String,
    pub platform_fee_wallet: Option<String>,
    pub start_block: Option<u64>,
    pub poll_seconds: u64,
    pub confirmations: u64,
    pub max_blocks_per_query: u64,
    pub request_id: u64,
}

impl BaseIndexerConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    pub fn from_lookup<F>(lookup: F) -> anyhow::Result<Self>
    where
        F: Fn(&str) -> Option<String>,
    {
        let requested_network = lookup("BASE_INDEXER_NETWORK")
            .filter(|value| nonempty(value))
            .unwrap_or_else(|| "base-sepolia".to_string());
        let descriptor = base_network_descriptor(&requested_network)?;
        let network = canonical_base_network(&descriptor);
        let escrow_contract_env = escrow_contract_env_for_network(&descriptor)?;
        let escrow_contract = lookup("BASE_INDEXER_ESCROW_CONTRACT")
            .filter(|value| nonempty(value))
            .or_else(|| lookup(escrow_contract_env).filter(|value| nonempty(value)))
            .ok_or_else(|| {
                anyhow!(
                    "set BASE_INDEXER_ESCROW_CONTRACT or {escrow_contract_env} before running the Base indexer"
                )
            })?;
        let rpc_url = lookup("BASE_INDEXER_RPC_URL")
            .filter(|value| nonempty(value))
            .or_else(|| lookup(&descriptor.rpc_url_env).filter(|value| nonempty(value)))
            .ok_or_else(|| {
                anyhow!(
                    "set BASE_INDEXER_RPC_URL or {} before running the Base indexer",
                    descriptor.rpc_url_env
                )
            })?;
        let start_block = lookup("BASE_INDEXER_START_BLOCK")
            .or_else(|| lookup("BASE_INDEXER_FROM_BLOCK"))
            .filter(|value| nonempty(value))
            .map(|value| parse_u64_env("BASE_INDEXER_START_BLOCK", &value))
            .transpose()?;
        let poll_seconds = lookup("BASE_INDEXER_POLL_SECONDS")
            .filter(|value| nonempty(value))
            .map(|value| parse_u64_env("BASE_INDEXER_POLL_SECONDS", &value))
            .transpose()?
            .unwrap_or(15);
        let confirmations = lookup("BASE_INDEXER_CONFIRMATIONS")
            .filter(|value| nonempty(value))
            .map(|value| parse_u64_env("BASE_INDEXER_CONFIRMATIONS", &value))
            .transpose()?
            .unwrap_or(2);
        let max_blocks_per_query = lookup("BASE_INDEXER_MAX_BLOCKS_PER_QUERY")
            .filter(|value| nonempty(value))
            .map(|value| parse_u64_env("BASE_INDEXER_MAX_BLOCKS_PER_QUERY", &value))
            .transpose()?
            .unwrap_or(2_000)
            .max(1);
        let request_id = lookup("BASE_INDEXER_REQUEST_ID")
            .filter(|value| nonempty(value))
            .map(|value| parse_u64_env("BASE_INDEXER_REQUEST_ID", &value))
            .transpose()?
            .unwrap_or(1);
        let settlement_signer = lookup("BASE_SETTLEMENT_SIGNER")
            .filter(|value| nonempty(value))
            .ok_or_else(|| {
                anyhow!("set BASE_SETTLEMENT_SIGNER before reconciling Base release logs")
            })?;
        let platform_fee_wallet =
            lookup("BASE_PLATFORM_FEE_WALLET").filter(|value| nonempty(value));

        let escrow_contract =
            BaseEscrowLogQuery::new(escrow_contract, start_block.unwrap_or(0), None)?
                .escrow_contract;
        let settlement_signer = normalize_evm_address(&settlement_signer)
            .map_err(|error| anyhow!("invalid BASE_SETTLEMENT_SIGNER: {error}"))?;
        let platform_fee_wallet = platform_fee_wallet
            .map(|wallet| {
                normalize_evm_address(&wallet)
                    .map_err(|error| anyhow!("invalid BASE_PLATFORM_FEE_WALLET: {error}"))
            })
            .transpose()?;

        Ok(Self {
            network,
            rpc_url,
            escrow_contract,
            settlement_signer,
            platform_fee_wallet,
            start_block,
            poll_seconds,
            confirmations,
            max_blocks_per_query,
            request_id,
        })
    }

    pub fn network_descriptor(&self) -> anyhow::Result<BaseNetworkDescriptor> {
        Ok(base_network_descriptor(&self.network)?)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaseIndexerPollReport {
    pub network: BaseNetworkDescriptor,
    pub escrow_contract: String,
    pub latest_block: u64,
    pub confirmations: u64,
    pub confirmed_to_block: Option<u64>,
    pub from_block: Option<u64>,
    pub to_block: Option<u64>,
    pub fetched_logs: usize,
    pub reconciliation: Option<BaseLogPipelineReport>,
    pub persisted_cursor_block: Option<u64>,
    pub skipped_reason: Option<String>,
}

pub const BASE_INDEXER_HEARTBEAT_SUCCESS: &str = "Success";
pub const BASE_INDEXER_HEARTBEAT_SKIPPED: &str = "Skipped";
pub const BASE_INDEXER_HEARTBEAT_FAILED: &str = "Failed";

pub async fn poll_base_indexer_once_with_heartbeat(
    store: &PostgresStore,
    config: &BaseIndexerConfig,
) -> anyhow::Result<BaseIndexerPollReport> {
    let started_at = Utc::now();
    match poll_base_indexer_once(store, config).await {
        Ok(report) => {
            let completed_at = Utc::now();
            let heartbeat =
                base_indexer_heartbeat_from_report(config, started_at, completed_at, &report);
            store.upsert_base_indexer_heartbeat(&heartbeat).await?;
            Ok(report)
        }
        Err(error) => {
            let completed_at = Utc::now();
            let error_message = error.to_string();
            let heartbeat = base_indexer_heartbeat_from_error(
                config,
                started_at,
                completed_at,
                error_message.as_str(),
            );
            if let Err(heartbeat_error) = store.upsert_base_indexer_heartbeat(&heartbeat).await {
                return Err(error).context(format!(
                    "failed to persist Base indexer failure heartbeat: {heartbeat_error}"
                ));
            }
            Err(error)
        }
    }
}

pub fn base_indexer_heartbeat_from_report(
    config: &BaseIndexerConfig,
    started_at: DateTime<Utc>,
    completed_at: DateTime<Utc>,
    report: &BaseIndexerPollReport,
) -> BaseIndexerHeartbeat {
    let failure_summary = report.reconciliation.as_ref().and_then(|reconciliation| {
        if reconciliation.failures.is_empty() {
            None
        } else {
            Some(
                reconciliation
                    .failures
                    .iter()
                    .map(|failure| {
                        format!(
                            "{}:{} {}",
                            failure.block_number, failure.log_index, failure.reason
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("; "),
            )
        }
    });
    let status = if failure_summary.is_some() {
        BASE_INDEXER_HEARTBEAT_FAILED
    } else if report.skipped_reason.is_some() {
        BASE_INDEXER_HEARTBEAT_SKIPPED
    } else {
        BASE_INDEXER_HEARTBEAT_SUCCESS
    };

    BaseIndexerHeartbeat {
        network: config.network.clone(),
        escrow_contract: config.escrow_contract.clone(),
        status: status.to_string(),
        started_at,
        completed_at: Some(completed_at),
        latest_block: Some(report.latest_block),
        confirmed_to_block: report.confirmed_to_block,
        from_block: report.from_block,
        to_block: report.to_block,
        fetched_logs: report.fetched_logs as u64,
        persisted_cursor_block: report.persisted_cursor_block,
        skipped_reason: report.skipped_reason.clone(),
        error_message: failure_summary,
        updated_at: completed_at,
    }
}

pub fn base_indexer_heartbeat_from_error(
    config: &BaseIndexerConfig,
    started_at: DateTime<Utc>,
    completed_at: DateTime<Utc>,
    error_message: &str,
) -> BaseIndexerHeartbeat {
    BaseIndexerHeartbeat {
        network: config.network.clone(),
        escrow_contract: config.escrow_contract.clone(),
        status: BASE_INDEXER_HEARTBEAT_FAILED.to_string(),
        started_at,
        completed_at: Some(completed_at),
        latest_block: None,
        confirmed_to_block: None,
        from_block: None,
        to_block: None,
        fetched_logs: 0,
        persisted_cursor_block: None,
        skipped_reason: None,
        error_message: Some(error_message.to_string()),
        updated_at: completed_at,
    }
}

pub async fn poll_base_indexer_once(
    store: &PostgresStore,
    config: &BaseIndexerConfig,
) -> anyhow::Result<BaseIndexerPollReport> {
    let descriptor = config.network_descriptor()?;
    let latest_block = fetch_block_number(&config.rpc_url, config.request_id).await?;
    let confirmed_to_block = latest_block.checked_sub(config.confirmations);
    let scan_cursor = store
        .get_base_log_cursor(&config.network, &config.escrow_contract)
        .await?;
    let mut network = hydrate_bounty_network(store).await?;
    let mut worker = hydrate_base_log_worker(store).await?;
    let from_block = next_indexer_from_block(
        scan_cursor.as_ref().map(|cursor| cursor.last_scanned_block),
        worker.cursor().last_block_number,
        config.start_block,
    )?;

    let Some(confirmed_to_block) = confirmed_to_block else {
        return Ok(BaseIndexerPollReport {
            network: descriptor,
            escrow_contract: config.escrow_contract.clone(),
            latest_block,
            confirmations: config.confirmations,
            confirmed_to_block: None,
            from_block: Some(from_block),
            to_block: None,
            fetched_logs: 0,
            reconciliation: None,
            persisted_cursor_block: scan_cursor.map(|cursor| cursor.last_scanned_block),
            skipped_reason: Some("latest block is below configured confirmations".to_string()),
        });
    };

    if confirmed_to_block < from_block {
        return Ok(BaseIndexerPollReport {
            network: descriptor,
            escrow_contract: config.escrow_contract.clone(),
            latest_block,
            confirmations: config.confirmations,
            confirmed_to_block: Some(confirmed_to_block),
            from_block: Some(from_block),
            to_block: None,
            fetched_logs: 0,
            reconciliation: None,
            persisted_cursor_block: scan_cursor.map(|cursor| cursor.last_scanned_block),
            skipped_reason: Some("no confirmed blocks are ready to scan".to_string()),
        });
    }

    let to_block = bounded_to_block(from_block, confirmed_to_block, config.max_blocks_per_query);
    let query = BaseEscrowLogQuery::new(&config.escrow_contract, from_block, Some(to_block))?;
    let response = fetch_base_escrow_logs(&config.rpc_url, &query, config.request_id + 1).await?;
    let logs = rpc_logs_to_evm_logs(response.result)?;
    let fetched_logs = logs.len();
    let release_attestation_fetch =
        fetch_release_transaction_evidence_for_logs(&config.rpc_url, &descriptor, config, &logs)
            .await?;
    let reconciliation = process_base_evm_logs_and_persist_with_release_evidence(
        store,
        &mut worker,
        &mut network,
        logs,
        &release_attestation_fetch.attestations,
        &release_attestation_fetch.failures,
        Some(BaseLogPersistenceCursor {
            network: &config.network,
            escrow_contract: &config.escrow_contract,
            last_scanned_block: to_block,
            last_log_key: None,
        }),
    )
    .await?;
    let persisted_cursor_block = if reconciliation.failures.is_empty() {
        Some(to_block)
    } else {
        scan_cursor.map(|cursor| cursor.last_scanned_block)
    };

    Ok(BaseIndexerPollReport {
        network: descriptor,
        escrow_contract: config.escrow_contract.clone(),
        latest_block,
        confirmations: config.confirmations,
        confirmed_to_block: Some(confirmed_to_block),
        from_block: Some(from_block),
        to_block: Some(to_block),
        fetched_logs,
        reconciliation: Some(reconciliation),
        persisted_cursor_block,
        skipped_reason: None,
    })
}

pub async fn hydrate_bounty_network(store: &PostgresStore) -> anyhow::Result<BountyNetwork> {
    Ok(BountyNetwork {
        agents: store
            .list_agents()
            .await?
            .into_iter()
            .map(|agent| (agent.id, agent))
            .collect(),
        contributor_contacts: store
            .list_contributor_contacts()
            .await?
            .into_iter()
            .map(|contact| (contact.id, contact))
            .collect(),
        audience_members: store
            .list_audience_members()
            .await?
            .into_iter()
            .map(|member| (member.id, member))
            .collect(),
        audience_interactions: store
            .list_audience_interactions()
            .await?
            .into_iter()
            .map(|interaction| (interaction.id, interaction))
            .collect(),
        discovery_responses: store
            .list_discovery_responses()
            .await?
            .into_iter()
            .map(|response| (response.id, response))
            .collect(),
        outreach_attempts: store
            .list_outreach_attempts()
            .await?
            .into_iter()
            .map(|attempt| (attempt.id, attempt))
            .collect(),
        capabilities: store
            .list_capabilities()
            .await?
            .into_iter()
            .map(|capability| (capability.id, capability))
            .collect(),
        help_requests: store
            .list_help_requests()
            .await?
            .into_iter()
            .map(|request| (request.id, request))
            .collect(),
        quotes: store
            .list_quotes()
            .await?
            .into_iter()
            .map(|quote| (quote.id, quote))
            .collect(),
        bounties: store
            .list_bounties()
            .await?
            .into_iter()
            .map(|bounty| (bounty.id, bounty))
            .collect(),
        funding_intents: store
            .list_funding_intents()
            .await?
            .into_iter()
            .map(|intent| (intent.id, intent))
            .collect(),
        funding_contributions: store
            .list_funding_contributions()
            .await?
            .into_iter()
            .map(|contribution| (contribution.id, contribution))
            .collect(),
        escrows: store
            .list_escrows()
            .await?
            .into_iter()
            .map(|escrow| (escrow.id, escrow))
            .collect(),
        claims: store
            .list_claims()
            .await?
            .into_iter()
            .map(|claim| (claim.id, claim))
            .collect(),
        submissions: store
            .list_submissions()
            .await?
            .into_iter()
            .map(|submission| (submission.id, submission))
            .collect(),
        verifier_results: store
            .list_verifier_results()
            .await?
            .into_iter()
            .map(|result| (result.id, result))
            .collect(),
        proofs: store
            .list_proof_records()
            .await?
            .into_iter()
            .map(|proof| (proof.id, proof))
            .collect(),
        settlements: store
            .list_settlements()
            .await?
            .into_iter()
            .map(|settlement| (settlement.id, settlement))
            .collect(),
        reputation_events: store
            .list_reputation_events()
            .await?
            .into_iter()
            .map(|event| (event.id, event))
            .collect(),
        template_signals: store
            .list_template_signals()
            .await?
            .into_iter()
            .map(|signal| (signal.id, signal))
            .collect(),
        risk_events: store
            .list_risk_events()
            .await?
            .into_iter()
            .map(|event| (event.id, event))
            .collect(),
        risk_reviews: store
            .list_risk_reviews()
            .await?
            .into_iter()
            .map(|review| (review.id, review))
            .collect(),
        payment_events: store
            .list_payment_events()
            .await?
            .into_iter()
            .map(|event| (event.id, event))
            .collect(),
        ledger: Ledger::from_entries(store.list_ledger_entries().await?)
            .context("hydrate ledger from Postgres")?,
        ..BountyNetwork::default()
    })
}

pub async fn hydrate_base_log_worker(store: &PostgresStore) -> anyhow::Result<BaseEscrowLogWorker> {
    Ok(BaseEscrowLogWorker::from_indexed_events(
        "usdc",
        store.list_base_escrow_events().await?,
    )?)
}

pub async fn process_base_evm_logs_and_persist(
    store: &PostgresStore,
    worker: &mut BaseEscrowLogWorker,
    network: &mut BountyNetwork,
    logs: Vec<EvmLog>,
) -> anyhow::Result<BaseLogPipelineReport> {
    process_base_evm_logs_and_persist_with_release_attestations(
        store,
        worker,
        network,
        logs,
        &HashMap::new(),
        None,
    )
    .await
}

pub async fn process_base_evm_logs_and_persist_with_release_attestations(
    store: &PostgresStore,
    worker: &mut BaseEscrowLogWorker,
    network: &mut BountyNetwork,
    logs: Vec<EvmLog>,
    release_attestations: &HashMap<String, BaseReleaseTransactionEvidence>,
    cursor_if_success: Option<BaseLogPersistenceCursor<'_>>,
) -> anyhow::Result<BaseLogPipelineReport> {
    process_base_evm_logs_and_persist_with_release_evidence(
        store,
        worker,
        network,
        logs,
        release_attestations,
        &HashMap::new(),
        cursor_if_success,
    )
    .await
}

pub async fn process_base_evm_logs_and_persist_with_release_evidence(
    store: &PostgresStore,
    worker: &mut BaseEscrowLogWorker,
    network: &mut BountyNetwork,
    logs: Vec<EvmLog>,
    release_attestations: &HashMap<String, BaseReleaseTransactionEvidence>,
    failed_release_attestations: &HashMap<String, BaseReleaseAttestationFailureEvidence>,
    cursor_if_success: Option<BaseLogPersistenceCursor<'_>>,
) -> anyhow::Result<BaseLogPipelineReport> {
    let report = worker.process_logs_with_release_evidence(
        logs,
        network,
        release_attestations,
        failed_release_attestations,
    );
    let applied_event_ids = report
        .applied_events
        .iter()
        .map(|event| event.event_id)
        .collect::<HashSet<_>>();
    let indexed_events = worker
        .indexed_events()
        .iter()
        .filter(|event| applied_event_ids.contains(&event.id))
        .cloned()
        .collect::<Vec<_>>();
    let bounty_ids = report
        .applied_events
        .iter()
        .map(|event| event.bounty_id)
        .collect::<HashSet<_>>();
    let bounties = bounty_ids
        .iter()
        .filter_map(|id| network.bounties.get(id).cloned())
        .collect::<Vec<_>>();
    let funding_intents = network
        .funding_intents
        .values()
        .filter(|intent| bounty_ids.contains(&intent.bounty_id))
        .cloned()
        .collect::<Vec<_>>();
    let escrows = network
        .escrows
        .values()
        .filter(|escrow| bounty_ids.contains(&escrow.bounty_id))
        .cloned()
        .collect::<Vec<_>>();
    let settlements = network
        .settlements
        .values()
        .filter(|settlement| bounty_ids.contains(&settlement.bounty_id))
        .cloned()
        .collect::<Vec<_>>();

    let cursor = if report.failures.is_empty() {
        cursor_if_success.map(|cursor| BaseLogPersistenceCursor {
            last_log_key: report.ending_cursor.last_log_key.as_deref(),
            ..cursor
        })
    } else {
        None
    };
    store
        .persist_base_log_pipeline(BaseLogPersistenceBatch {
            bounties: &bounties,
            release_attestations: &report.release_attestations,
            funding_intents: &funding_intents,
            escrows: &escrows,
            settlements: &settlements,
            ledger_entries: &report.ledger_entries,
            base_escrow_events: &indexed_events,
            cursor,
        })
        .await?;

    Ok(report)
}

#[derive(Debug, Clone, Default)]
struct ReleaseTransactionEvidenceFetchReport {
    attestations: HashMap<String, BaseReleaseTransactionEvidence>,
    failures: HashMap<String, BaseReleaseAttestationFailureEvidence>,
}

async fn fetch_release_transaction_evidence_for_logs(
    rpc_url: &str,
    descriptor: &BaseNetworkDescriptor,
    config: &BaseIndexerConfig,
    logs: &[EvmLog],
) -> anyhow::Result<ReleaseTransactionEvidenceFetchReport> {
    let release_topic = evm_event_topic("EscrowReleased(uint256,bytes32)");
    let mut tx_hashes = Vec::new();
    let mut report = ReleaseTransactionEvidenceFetchReport::default();
    for log in logs {
        if log.topics.first() != Some(&release_topic) {
            continue;
        }
        let tx_key = release_tx_key(&log.tx_hash);
        let log_address = match normalize_evm_address(&log.address) {
            Ok(address) => address,
            Err(error) => {
                report.failures.insert(
                    tx_key,
                    failed_release_fetch(
                        &log.tx_hash,
                        descriptor,
                        config,
                        None,
                        format!("invalid release log emitter address: {error}"),
                    ),
                );
                continue;
            }
        };
        if log_address != config.escrow_contract {
            report.failures.insert(
                tx_key,
                failed_release_fetch(
                    &log.tx_hash,
                    descriptor,
                    config,
                    None,
                    format!(
                        "release log emitted by {log_address}, expected configured escrow {}",
                        config.escrow_contract
                    ),
                ),
            );
            continue;
        }
        let tx_hash = match normalize_evm_hash(&log.tx_hash) {
            Ok(tx_hash) => tx_hash,
            Err(error) => {
                report.failures.insert(
                    tx_key,
                    failed_release_fetch(
                        &log.tx_hash,
                        descriptor,
                        config,
                        None,
                        format!("invalid release log transaction hash: {error}"),
                    ),
                );
                continue;
            }
        };
        if !tx_hashes.contains(&tx_hash) {
            tx_hashes.push(tx_hash);
        }
    }

    for (index, tx_hash) in tx_hashes.into_iter().enumerate() {
        let request_id = config.request_id + 10 + (index as u64 * 2);
        let receipt_response = match fetch_transaction_receipt(rpc_url, &tx_hash, request_id).await
        {
            Ok(response) => response,
            Err(error) => {
                report.failures.insert(
                    tx_hash.clone(),
                    failed_release_fetch(
                        &tx_hash,
                        descriptor,
                        config,
                        None,
                        format!("release transaction receipt fetch failed for {tx_hash}: {error}"),
                    ),
                );
                continue;
            }
        };
        let Some(receipt) = receipt_response.result else {
            report.failures.insert(
                tx_hash.clone(),
                failed_release_fetch(
                    &tx_hash,
                    descriptor,
                    config,
                    None,
                    format!("release transaction receipt not found for {tx_hash}"),
                ),
            );
            continue;
        };
        let receipt_tx_hash = match receipt.normalized_tx_hash() {
            Ok(tx_hash) => tx_hash,
            Err(error) => {
                report.failures.insert(
                    tx_hash.clone(),
                    failed_release_fetch(
                        &tx_hash,
                        descriptor,
                        config,
                        None,
                        format!("invalid release receipt transaction hash for {tx_hash}: {error}"),
                    ),
                );
                continue;
            }
        };
        if receipt_tx_hash != tx_hash {
            report.failures.insert(
                tx_hash.clone(),
                failed_release_fetch(
                    &tx_hash,
                    descriptor,
                    config,
                    None,
                    format!(
                        "release receipt hash {receipt_tx_hash} does not match requested transaction {tx_hash}"
                    ),
                ),
            );
            continue;
        }
        let receipt_logs = match receipt.logs_to_evm_logs() {
            Ok(logs) => logs,
            Err(error) => {
                report.failures.insert(
                    tx_hash.clone(),
                    failed_release_fetch(
                        &tx_hash,
                        descriptor,
                        config,
                        None,
                        format!("release receipt logs are invalid for {tx_hash}: {error}"),
                    ),
                );
                continue;
            }
        };
        let receipt_has_configured_release = receipt_logs.iter().any(|log| {
            log.tx_hash == tx_hash
                && log.topics.first() == Some(&release_topic)
                && normalize_evm_address(&log.address)
                    .map(|address| address == config.escrow_contract)
                    .unwrap_or(false)
        });
        if !receipt_has_configured_release {
            let reason = format!(
                "release receipt {tx_hash} has no EscrowReleased log emitted by configured escrow {}",
                config.escrow_contract
            );
            report.failures.insert(
                tx_hash.clone(),
                failed_release_fetch(&tx_hash, descriptor, config, None, reason),
            );
            continue;
        }

        let transaction_response =
            match fetch_transaction_by_hash(rpc_url, &tx_hash, request_id + 1).await {
                Ok(response) => response,
                Err(error) => {
                    report.failures.insert(
                        tx_hash.clone(),
                        failed_release_fetch(
                            &tx_hash,
                            descriptor,
                            config,
                            None,
                            format!("release transaction fetch failed for {tx_hash}: {error}"),
                        ),
                    );
                    continue;
                }
            };
        let Some(transaction) = transaction_response.result else {
            report.failures.insert(
                tx_hash.clone(),
                failed_release_fetch(
                    &tx_hash,
                    descriptor,
                    config,
                    None,
                    format!("release transaction not found for {tx_hash}"),
                ),
            );
            continue;
        };
        let input = Some(transaction.input.clone());
        let transaction_hash = match transaction.normalized_hash() {
            Ok(hash) => hash,
            Err(error) => {
                report.failures.insert(
                    tx_hash.clone(),
                    failed_release_fetch(
                        &tx_hash,
                        descriptor,
                        config,
                        input,
                        format!("invalid release transaction hash for {tx_hash}: {error}"),
                    ),
                );
                continue;
            }
        };
        if transaction_hash != tx_hash {
            report.failures.insert(
                tx_hash.clone(),
                failed_release_fetch(
                    &tx_hash,
                    descriptor,
                    config,
                    input,
                    format!(
                        "release transaction hash {transaction_hash} does not match receipt {tx_hash}"
                    ),
                ),
            );
            continue;
        }
        let chain_id = match transaction.chain_id() {
            Ok(Some(chain_id)) => chain_id,
            Ok(None) => {
                report.failures.insert(
                    tx_hash.clone(),
                    failed_release_fetch(
                        &tx_hash,
                        descriptor,
                        config,
                        input,
                        format!("release transaction {tx_hash} is missing chainId"),
                    ),
                );
                continue;
            }
            Err(error) => {
                report.failures.insert(
                    tx_hash.clone(),
                    failed_release_fetch(
                        &tx_hash,
                        descriptor,
                        config,
                        input,
                        format!("invalid release transaction chainId for {tx_hash}: {error}"),
                    ),
                );
                continue;
            }
        };
        let to = match transaction.normalized_to() {
            Ok(Some(to)) => to,
            Ok(None) => {
                report.failures.insert(
                    tx_hash.clone(),
                    failed_release_fetch(
                        &tx_hash,
                        descriptor,
                        config,
                        input,
                        format!("release transaction {tx_hash} has no target contract"),
                    ),
                );
                continue;
            }
            Err(error) => {
                report.failures.insert(
                    tx_hash.clone(),
                    failed_release_fetch(
                        &tx_hash,
                        descriptor,
                        config,
                        input,
                        format!("invalid release transaction target for {tx_hash}: {error}"),
                    ),
                );
                continue;
            }
        };
        let from = match transaction.normalized_from() {
            Ok(from) => from,
            Err(error) => {
                report.failures.insert(
                    tx_hash.clone(),
                    failed_release_fetch(
                        &tx_hash,
                        descriptor,
                        config,
                        input,
                        format!("invalid release transaction sender for {tx_hash}: {error}"),
                    ),
                );
                continue;
            }
        };
        let receipt_succeeded = match receipt.succeeded() {
            Ok(value) => value.unwrap_or(false),
            Err(error) => {
                report.failures.insert(
                    tx_hash.clone(),
                    failed_release_fetch(
                        &tx_hash,
                        descriptor,
                        config,
                        input,
                        format!("invalid release receipt status for {tx_hash}: {error}"),
                    ),
                );
                continue;
            }
        };

        report.attestations.insert(
            tx_hash.clone(),
            BaseReleaseTransactionEvidence {
                tx_hash,
                chain_id,
                expected_chain_id: descriptor.chain_id,
                from,
                settlement_signer: config.settlement_signer.clone(),
                to,
                escrow_contract: config.escrow_contract.clone(),
                input: transaction.input.clone(),
                receipt_succeeded,
                platform_fee_wallet: config.platform_fee_wallet.clone(),
            },
        );
    }

    Ok(report)
}

fn failed_release_fetch(
    tx_hash: &str,
    descriptor: &BaseNetworkDescriptor,
    config: &BaseIndexerConfig,
    input: Option<String>,
    reason: String,
) -> BaseReleaseAttestationFailureEvidence {
    BaseReleaseAttestationFailureEvidence {
        tx_hash: tx_hash.to_string(),
        expected_chain_id: descriptor.chain_id,
        escrow_contract: config.escrow_contract.clone(),
        settlement_signer: config.settlement_signer.clone(),
        platform_fee_wallet: config.platform_fee_wallet.clone(),
        input,
        reason,
    }
}

pub fn next_indexer_from_block(
    scan_cursor_block: Option<u64>,
    event_cursor_block: Option<u64>,
    configured_start_block: Option<u64>,
) -> anyhow::Result<u64> {
    if let Some(block) = scan_cursor_block {
        return block
            .checked_add(1)
            .ok_or_else(|| anyhow!("base indexer cursor overflowed"));
    }
    if let Some(block) = event_cursor_block {
        return Ok(block);
    }
    configured_start_block.ok_or_else(|| {
        anyhow!(
            "set BASE_INDEXER_START_BLOCK for the first run; it should be the escrow contract deployment block"
        )
    })
}

pub fn bounded_to_block(
    from_block: u64,
    confirmed_to_block: u64,
    max_blocks_per_query: u64,
) -> u64 {
    let capped_end = from_block.saturating_add(max_blocks_per_query.saturating_sub(1));
    capped_end.min(confirmed_to_block)
}

fn canonical_base_network(descriptor: &BaseNetworkDescriptor) -> String {
    match descriptor.chain_id {
        8_453 => "base-mainnet".to_string(),
        84_532 => "base-sepolia".to_string(),
        other => other.to_string(),
    }
}

fn escrow_contract_env_for_network(
    descriptor: &BaseNetworkDescriptor,
) -> anyhow::Result<&'static str> {
    match descriptor.chain_id {
        8_453 => Ok("BASE_MAINNET_ESCROW_CONTRACT"),
        84_532 => Ok("BASE_SEPOLIA_ESCROW_CONTRACT"),
        _ => Err(anyhow!("unsupported Base chain id {}", descriptor.chain_id)),
    }
}

fn parse_u64_env(name: &str, value: &str) -> anyhow::Result<u64> {
    value
        .trim()
        .parse::<u64>()
        .with_context(|| format!("{name} must be a non-negative integer"))
}

fn nonempty(value: &str) -> bool {
    !value.trim().is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use app::{
        hash_artifact, ClaimBountyRequest, PlanBaseReleaseRequest, PostBountyRequest,
        RegisterAgentRequest, SubmitResultRequest, VerifySubmissionRequest,
    };
    use chain_base::{
        evm_address_word, evm_bytes32_word, evm_event_topic, evm_uint256_word, evm_words_data,
    };
    use domain::{
        Bounty, BountyStatus, EscrowStatus, FundingMode, Money, PayoutStatus, PrivacyLevel,
        ProofRecord, VerifierKind,
    };
    use std::collections::HashMap;

    #[tokio::test]
    async fn raw_base_logs_require_release_calldata_attestation() {
        let (mut network, bounty, proof) = payable_base_bounty().await;
        let logs = raw_created_and_released_logs(&bounty, &proof);
        let mut worker = BaseEscrowLogWorker::default();

        let report = worker.process_logs(logs.clone(), &mut network);

        assert_eq!(report.failures.len(), 1);
        assert!(report.failures[0]
            .reason
            .contains("requires matching release transaction calldata attestation"));
        assert_eq!(report.decoded_events, 2);
        assert_eq!(report.applied_events.len(), 1);
        assert!(report.ledger_entries.is_empty());
        assert_eq!(worker.indexed_events().len(), 1);
        assert_eq!(worker.cursor().last_block_number, Some(10));

        let status = network.status(bounty.id).unwrap();
        assert_eq!(status.bounty.status, BountyStatus::Payable);
        assert_eq!(
            status.settlements[0].payout_intents[0].status,
            PayoutStatus::Pending
        );
        assert_eq!(network.ledger.entries().len(), 1);

        let release_attestations = release_attestations_for(&network, &bounty, &logs[1]);
        let mut attested_worker =
            BaseEscrowLogWorker::from_indexed_events("usdc", worker.indexed_events().to_vec())
                .unwrap();
        let attested = attested_worker.process_logs_with_release_attestations(
            vec![logs[1].clone()],
            &mut network,
            &release_attestations,
        );
        assert!(attested.failures.is_empty());
        assert_eq!(attested.applied_events.len(), 1);
        assert_eq!(attested.ledger_entries.len(), 1);
        assert_eq!(attested_worker.indexed_events().len(), 2);
        let status = network.status(bounty.id).unwrap();
        assert_eq!(status.bounty.status, BountyStatus::Paid);
        assert_eq!(status.escrows[0].status, EscrowStatus::Released);
        assert_eq!(network.ledger.entries().len(), 2);
    }

    #[tokio::test]
    async fn release_fetch_failure_persists_decoded_quarantine_record() {
        let (mut network, bounty, proof) = payable_base_bounty().await;
        let logs = raw_created_and_released_logs(&bounty, &proof);
        let release_attestations = release_attestations_for(&network, &bounty, &logs[1]);
        let planned = release_attestations
            .get(&release_tx_key(&logs[1].tx_hash))
            .expect("planned release evidence");
        let failures = HashMap::from([(
            release_tx_key(&logs[1].tx_hash),
            BaseReleaseAttestationFailureEvidence {
                tx_hash: logs[1].tx_hash.clone(),
                expected_chain_id: planned.expected_chain_id,
                escrow_contract: planned.escrow_contract.clone(),
                settlement_signer: planned.settlement_signer.clone(),
                platform_fee_wallet: planned.platform_fee_wallet.clone(),
                input: Some(planned.input.clone()),
                reason: "release transaction receipt not found for quarantine test".to_string(),
            },
        )]);
        let mut worker = BaseEscrowLogWorker::default();

        let report = worker.process_logs_with_release_evidence(
            logs,
            &mut network,
            &HashMap::new(),
            &failures,
        );

        assert_eq!(report.applied_events.len(), 1);
        assert_eq!(report.failures.len(), 1);
        assert_eq!(
            report.failures[0].reason,
            "release transaction receipt not found for quarantine test"
        );
        assert_eq!(report.release_attestations.len(), 1);
        let record = &report.release_attestations[0];
        assert_eq!(record.verdict, "failed");
        assert_eq!(
            record.reason,
            "release transaction receipt not found for quarantine test"
        );
        assert!(record.calldata_hash.is_some());
        let expected_proof_hash = format!("0x{}", proof.proof_hash);
        assert_eq!(
            record.proof_hash.as_deref(),
            Some(expected_proof_hash.as_str())
        );
        let recipients = record.recipients.as_array().expect("recipients array");
        assert_eq!(recipients.len(), 1);
        assert_eq!(
            recipients[0]["address"],
            serde_json::json!("0x2222222222222222222222222222222222222222")
        );
        assert_eq!(
            recipients[0]["amount"]["amount"],
            serde_json::json!(1_000_000)
        );
        assert_eq!(
            recipients[0]["amount"]["currency"],
            serde_json::json!("usdc")
        );
        let status = network.status(bounty.id).unwrap();
        assert_eq!(status.bounty.status, BountyStatus::Payable);
        assert_eq!(
            status.settlements[0].payout_intents[0].status,
            PayoutStatus::Pending
        );
    }

    #[tokio::test]
    async fn worker_can_resume_terminal_logs_after_created_event_restart() {
        let (mut network, bounty, proof) = payable_base_bounty().await;
        let logs = raw_created_and_released_logs(&bounty, &proof);
        let mut first_worker = BaseEscrowLogWorker::default();

        let first_report = first_worker.process_logs(vec![logs[0].clone()], &mut network);
        assert!(first_report.failures.is_empty());
        assert_eq!(first_report.applied_events.len(), 1);
        let persisted_events = first_worker.indexed_events().to_vec();

        let mut restarted_worker =
            BaseEscrowLogWorker::from_indexed_events("usdc", persisted_events).unwrap();
        assert_eq!(restarted_worker.cursor().last_block_number, Some(10));

        let release_attestations = release_attestations_for(&network, &bounty, &logs[1]);
        let second_report = restarted_worker.process_logs_with_release_attestations(
            vec![logs[1].clone()],
            &mut network,
            &release_attestations,
        );

        assert!(second_report.failures.is_empty());
        assert_eq!(second_report.applied_events.len(), 1);
        assert_eq!(second_report.ledger_entries.len(), 1);
        let status = network.status(bounty.id).unwrap();
        assert_eq!(status.bounty.status, BountyStatus::Paid);
    }

    #[test]
    fn terminal_log_without_create_does_not_advance_cursor() {
        let mut network = BountyNetwork::default();
        let mut worker = BaseEscrowLogWorker::default();
        let release = raw_released_log(7, &format!("0x{}", "22".repeat(32)), 11, 0);

        let report = worker.process_logs(vec![release], &mut network);

        assert_eq!(report.failures.len(), 1);
        assert_eq!(
            report.failures[0].reason,
            "terminal escrow log arrived before created log"
        );
        assert_eq!(worker.cursor(), &BaseLogCursor::default());
        assert!(worker.indexed_events().is_empty());
    }

    #[test]
    fn base_indexer_config_uses_network_specific_env_defaults() {
        let values = HashMap::from([
            ("BASE_INDEXER_NETWORK", "base-mainnet"),
            ("BASE_MAINNET_RPC_URL", "https://base.example"),
            (
                "BASE_MAINNET_ESCROW_CONTRACT",
                "0x1111111111111111111111111111111111111111",
            ),
            ("BASE_INDEXER_START_BLOCK", "123"),
            (
                "BASE_SETTLEMENT_SIGNER",
                "0x5555555555555555555555555555555555555555",
            ),
        ]);

        let config =
            BaseIndexerConfig::from_lookup(|key| values.get(key).map(|value| value.to_string()))
                .unwrap();

        assert_eq!(config.network, "base-mainnet");
        assert_eq!(config.rpc_url, "https://base.example");
        assert_eq!(
            config.escrow_contract,
            "0x1111111111111111111111111111111111111111"
        );
        assert_eq!(config.start_block, Some(123));
        assert_eq!(config.confirmations, 2);
        assert_eq!(config.max_blocks_per_query, 2_000);
    }

    #[test]
    fn base_indexer_config_ignores_blank_override_vars() {
        let values = HashMap::from([
            ("BASE_INDEXER_NETWORK", "base-sepolia"),
            ("BASE_INDEXER_RPC_URL", ""),
            ("BASE_INDEXER_ESCROW_CONTRACT", ""),
            ("BASE_SEPOLIA_RPC_URL", "https://sepolia.example"),
            (
                "BASE_SEPOLIA_ESCROW_CONTRACT",
                "0xABCDEFabcdefABCDEFabcdefABCDEFabcdefABCD",
            ),
            ("BASE_INDEXER_START_BLOCK", "123"),
            (
                "BASE_SETTLEMENT_SIGNER",
                "0x5555555555555555555555555555555555555555",
            ),
        ]);

        let config =
            BaseIndexerConfig::from_lookup(|key| values.get(key).map(|value| value.to_string()))
                .unwrap();

        assert_eq!(config.rpc_url, "https://sepolia.example");
        assert_eq!(
            config.escrow_contract,
            "0xabcdefabcdefabcdefabcdefabcdefabcdefabcd"
        );
    }

    #[test]
    fn base_indexer_requires_start_block_without_existing_cursors() {
        let err = next_indexer_from_block(None, None, None).unwrap_err();

        assert!(err.to_string().contains("BASE_INDEXER_START_BLOCK"));
        assert_eq!(next_indexer_from_block(Some(10), None, None).unwrap(), 11);
        assert_eq!(next_indexer_from_block(None, Some(10), None).unwrap(), 10);
        assert_eq!(next_indexer_from_block(None, None, Some(5)).unwrap(), 5);
    }

    #[test]
    fn base_indexer_caps_scan_ranges() {
        assert_eq!(bounded_to_block(100, 500, 50), 149);
        assert_eq!(bounded_to_block(100, 120, 50), 120);
        assert_eq!(bounded_to_block(100, 120, 0), 100);
    }

    fn test_base_indexer_config() -> BaseIndexerConfig {
        BaseIndexerConfig {
            network: "base-sepolia".to_string(),
            rpc_url: "https://base-sepolia.example".to_string(),
            escrow_contract: "0x1111111111111111111111111111111111111111".to_string(),
            settlement_signer: "0x5555555555555555555555555555555555555555".to_string(),
            platform_fee_wallet: Some("0x4444444444444444444444444444444444444444".to_string()),
            start_block: Some(1),
            poll_seconds: 15,
            confirmations: 2,
            max_blocks_per_query: 2_000,
            request_id: 1,
        }
    }

    #[test]
    fn base_indexer_heartbeat_marks_skipped_poll() {
        let config = test_base_indexer_config();
        let now = Utc::now();
        let report = BaseIndexerPollReport {
            network: base_network_descriptor("base-sepolia").unwrap(),
            escrow_contract: config.escrow_contract.clone(),
            latest_block: 10,
            confirmations: 2,
            confirmed_to_block: Some(8),
            from_block: Some(9),
            to_block: None,
            fetched_logs: 0,
            reconciliation: None,
            persisted_cursor_block: Some(8),
            skipped_reason: Some("no confirmed blocks are ready to scan".to_string()),
        };

        let heartbeat = base_indexer_heartbeat_from_report(&config, now, now, &report);

        assert_eq!(heartbeat.status, BASE_INDEXER_HEARTBEAT_SKIPPED);
        assert_eq!(
            heartbeat.skipped_reason.as_deref(),
            Some("no confirmed blocks are ready to scan")
        );
        assert_eq!(heartbeat.latest_block, Some(10));
        assert_eq!(heartbeat.persisted_cursor_block, Some(8));
    }

    #[test]
    fn base_indexer_heartbeat_marks_reconciliation_failure() {
        let config = test_base_indexer_config();
        let now = Utc::now();
        let report = BaseIndexerPollReport {
            network: base_network_descriptor("base-sepolia").unwrap(),
            escrow_contract: config.escrow_contract.clone(),
            latest_block: 10,
            confirmations: 2,
            confirmed_to_block: Some(8),
            from_block: Some(7),
            to_block: Some(8),
            fetched_logs: 1,
            reconciliation: Some(BaseLogPipelineReport {
                failures: vec![BaseLogFailure {
                    block_number: 8,
                    log_index: 0,
                    log_key: "0xabc:0".to_string(),
                    reason: "terminal event before create".to_string(),
                }],
                ..BaseLogPipelineReport::default()
            }),
            persisted_cursor_block: Some(7),
            skipped_reason: None,
        };

        let heartbeat = base_indexer_heartbeat_from_report(&config, now, now, &report);

        assert_eq!(heartbeat.status, BASE_INDEXER_HEARTBEAT_FAILED);
        assert_eq!(heartbeat.fetched_logs, 1);
        assert!(heartbeat
            .error_message
            .as_deref()
            .unwrap()
            .contains("terminal event before create"));
    }

    async fn payable_base_bounty() -> (BountyNetwork, Bounty, ProofRecord) {
        let mut network = BountyNetwork::default();
        let solver = network.register_agent(RegisterAgentRequest {
            handle: "solver".to_string(),
            payout_wallet: Some("0x2222222222222222222222222222222222222222".to_string()),
        });
        let bounty = network
            .post_funded_bounty(PostBountyRequest {
                title: "Extract data".to_string(),
                template_slug: "extract-data-to-schema".to_string(),
                amount_minor: 1_000_000,
                currency: "usdc".to_string(),
                funding_mode: FundingMode::BaseUsdcEscrow,
                privacy: PrivacyLevel::Public,
            })
            .unwrap();
        network
            .apply_base_escrow_event(chain_base::simulated_created_event(
                bounty.id,
                7,
                "0x3333333333333333333333333333333333333333",
                bounty.amount.clone(),
                bounty.terms_hash.clone().unwrap(),
            ))
            .unwrap();
        network
            .claim_bounty(ClaimBountyRequest {
                bounty_id: bounty.id,
                solver_agent_id: solver.id,
            })
            .unwrap();
        let artifact = "{\"ok\":true}";
        let submission = network
            .submit_result(SubmitResultRequest {
                bounty_id: bounty.id,
                solver_agent_id: solver.id,
                artifact_uri: "s3://worker/artifact.json".to_string(),
                artifact_body: artifact.to_string(),
            })
            .unwrap();
        let proof = network
            .verify_submission(VerifySubmissionRequest {
                bounty_id: bounty.id,
                submission_id: submission.id,
                expected_artifact_digest: hash_artifact(artifact),
                verifier_kind: Some(VerifierKind::JsonSchema),
                rubric: None,
                evidence: None,
                approved_risk_event_id: None,
            })
            .await
            .unwrap();
        (network, bounty, proof)
    }

    fn raw_created_and_released_logs(bounty: &Bounty, proof: &ProofRecord) -> Vec<EvmLog> {
        let terms_hash = format!("0x{}", bounty.terms_hash.clone().unwrap());
        let proof_hash = format!("0x{}", proof.proof_hash);
        vec![
            raw_created_log(
                7,
                bounty.id,
                "0x2222222222222222222222222222222222222222",
                "0x3333333333333333333333333333333333333333",
                bounty.amount.clone(),
                &terms_hash,
                10,
                0,
            ),
            raw_released_log(7, &proof_hash, 11, 0),
        ]
    }

    fn release_attestations_for(
        network: &BountyNetwork,
        bounty: &Bounty,
        release_log: &EvmLog,
    ) -> HashMap<String, BaseReleaseTransactionEvidence> {
        let release_plan = network
            .plan_base_release(PlanBaseReleaseRequest {
                bounty_id: bounty.id,
                escrow_contract: "0x1111111111111111111111111111111111111111".to_string(),
                platform_fee_wallet: "0x4444444444444444444444444444444444444444".to_string(),
                network: Some("base-mainnet".to_string()),
            })
            .unwrap();
        HashMap::from([(
            release_log.tx_hash.to_ascii_lowercase(),
            BaseReleaseTransactionEvidence {
                tx_hash: release_log.tx_hash.clone(),
                chain_id: release_plan.network.chain_id,
                expected_chain_id: release_plan.network.chain_id,
                from: "0x5555555555555555555555555555555555555555".to_string(),
                settlement_signer: "0x5555555555555555555555555555555555555555".to_string(),
                to: release_plan.transaction.to.clone(),
                escrow_contract: "0x1111111111111111111111111111111111111111".to_string(),
                input: release_plan.transaction.data,
                receipt_succeeded: true,
                platform_fee_wallet: Some("0x4444444444444444444444444444444444444444".to_string()),
            },
        )])
    }

    #[allow(clippy::too_many_arguments)]
    fn raw_created_log(
        escrow_id: u128,
        bounty_id: Id,
        payer: &str,
        token: &str,
        amount: Money,
        terms_hash: &str,
        block_number: u64,
        log_index: u64,
    ) -> EvmLog {
        EvmLog {
            address: "0x1111111111111111111111111111111111111111".to_string(),
            topics: vec![
                evm_event_topic("EscrowCreated(uint256,bytes32,address,address,uint256,bytes32)"),
                evm_uint256_word(escrow_id),
                evm_bytes32_word(&bounty_bytes32(bounty_id)).unwrap(),
                evm_address_word(payer).unwrap(),
            ],
            data: evm_words_data(&[
                evm_address_word(token).unwrap(),
                evm_uint256_word(amount.amount.try_into().unwrap()),
                evm_bytes32_word(terms_hash).unwrap(),
            ])
            .unwrap(),
            tx_hash: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .to_string(),
            block_number,
            log_index,
            occurred_at: None,
        }
    }

    fn raw_released_log(
        escrow_id: u128,
        proof_hash: &str,
        block_number: u64,
        log_index: u64,
    ) -> EvmLog {
        EvmLog {
            address: "0x1111111111111111111111111111111111111111".to_string(),
            topics: vec![
                evm_event_topic("EscrowReleased(uint256,bytes32)"),
                evm_uint256_word(escrow_id),
            ],
            data: evm_words_data(&[evm_bytes32_word(proof_hash).unwrap()]).unwrap(),
            tx_hash: "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                .to_string(),
            block_number,
            log_index,
            occurred_at: None,
        }
    }

    fn bounty_bytes32(bounty_id: Id) -> String {
        format!("0x{}{}", "0".repeat(32), bounty_id.simple())
    }
}
