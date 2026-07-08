use app::{BaseEscrowReconciliation, BountyNetwork};
use chain_base::{
    BaseEscrowEvent, BaseEscrowEventKind, BaseEscrowLogDecoder, ChainEventIndexer, EvmLog,
};
use domain::{Id, Submission, VerifierResult};
use ledger::LedgerEntry;
use serde::{Deserialize, Serialize};
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
    pub failures: Vec<BaseLogFailure>,
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

            let reconciliation = match network.apply_base_escrow_event(event.clone()) {
                Ok(reconciliation) => reconciliation,
                Err(error) => {
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

#[cfg(test)]
mod tests {
    use super::*;
    use app::{
        hash_artifact, ClaimBountyRequest, PostBountyRequest, RegisterAgentRequest,
        SubmitResultRequest, VerifySubmissionRequest,
    };
    use chain_base::{
        evm_address_word, evm_bytes32_word, evm_event_topic, evm_uint256_word, evm_words_data,
    };
    use domain::{
        Bounty, BountyStatus, EscrowStatus, FundingMode, Money, PayoutStatus, PrivacyLevel,
        ProofRecord, VerifierKind,
    };

    #[tokio::test]
    async fn raw_base_logs_mark_payable_bounty_paid_once() {
        let (mut network, bounty, proof) = payable_base_bounty().await;
        let logs = raw_created_and_released_logs(&bounty, &proof);
        let mut worker = BaseEscrowLogWorker::default();

        let report = worker.process_logs(logs.clone(), &mut network);

        assert!(report.failures.is_empty());
        assert_eq!(report.decoded_events, 2);
        assert_eq!(report.applied_events.len(), 2);
        assert_eq!(report.ledger_entries.len(), 1);
        assert_eq!(worker.indexed_events().len(), 2);
        assert_eq!(worker.cursor().last_block_number, Some(11));
        assert_eq!(worker.cursor().last_log_index, Some(0));

        let status = network.status(bounty.id).unwrap();
        assert_eq!(status.bounty.status, BountyStatus::Paid);
        assert_eq!(status.escrows[0].status, EscrowStatus::Released);
        assert_eq!(
            status.settlements[0].payout_intents[0].status,
            PayoutStatus::Paid
        );
        assert_eq!(network.ledger.entries().len(), 2);

        let replay = worker.process_logs(logs, &mut network);
        assert!(replay.failures.is_empty());
        assert_eq!(replay.applied_events.len(), 0);
        assert_eq!(replay.skipped_duplicate_logs, 2);
        assert!(replay.ledger_entries.is_empty());
        assert_eq!(network.ledger.entries().len(), 2);
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

        let second_report = restarted_worker.process_logs(vec![logs[1].clone()], &mut network);

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

    async fn payable_base_bounty() -> (BountyNetwork, Bounty, ProofRecord) {
        let mut network = BountyNetwork::default();
        let solver = network.register_agent(RegisterAgentRequest {
            handle: "solver".to_string(),
            payout_wallet: Some("0xsolver".to_string()),
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
