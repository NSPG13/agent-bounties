use anyhow::{anyhow, bail, Context, Result};
use app::{
    build_live_money_readiness_report, hash_artifact, stripe_secret_key_mode_from_secret,
    AddFundingContributionRequest, BaseReleaseQueueRequest, BountyNetwork, ClaimBountyRequest,
    CreateFundingIntentRequest, CreateHelpRequestRequest, FundQuoteRequest,
    FundingIntentNextAction, FundingPartitionTargetRequest, LiveMoneyReadinessConfig,
    OpenPooledBountyRequest, PlanBaseReleaseRequest, PlanStripeTransferRequest, PostBountyRequest,
    RegisterAgentRequest, RegisterCapabilityRequest, RequestQuotesRequest, SubmitResultRequest,
    VerifySubmissionRequest,
};
use chain_base::{
    base_network_descriptor, broadcast_signed_transaction, eth_get_transaction_receipt_request,
    eth_send_raw_transaction_request, evm_address_word, evm_bytes32_word, evm_event_topic,
    evm_uint256_word, evm_words_data, fetch_base_escrow_logs, fetch_transaction_receipt,
    rpc_logs_to_evm_logs, simulated_created_event, simulated_released_event, BaseEscrowCreate,
    BaseEscrowLogDecoder, BaseEscrowLogQuery, BaseEscrowReleaseCall, BaseEscrowTxPlanner,
    BaseRpcUrlConfig, ChainEventIndexer, EscrowRecipient, EvmLog,
};
use chrono::Utc;
use clap::{Args as ClapArgs, Parser, Subcommand};
use domain::{CapabilityClass, FundingMode, Money, PaymentRail, PrivacyLevel, VerifierKind};
use eval_harness::{
    bundled_abuse_fixtures, bundled_fixtures, bundled_judge_fixtures, run_eval_loops, AbuseBench,
    BountyBench, JudgeBench,
};
use github_app::{
    bounty_check_output, claim_comment_plan, funding_comment_plan, issue_api_sync_plan,
    parse_issue_form_bounty, proof_comment_plan, GitHubClaimCommentInput,
    GitHubFundingCommentInput, GitHubIssueApiSyncInput, GitHubProofComment,
};
use payments_stripe::{
    execute_stripe_request, CheckoutTopUpRequest, ConnectAccountSnapshot, StripeEventDeduper,
    StripePlanner, StripeRequestIntent, StripeWebhookEvent, STRIPE_API_BASE_URL,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    io::{Read, Write},
    net::TcpStream,
    path::{Path, PathBuf},
    process::{Child, Command as ProcessCommand, Stdio},
    thread,
    time::Duration,
};
use uuid::Uuid;

const STATIC_FUNDING_PAGE_URL: &str = "https://nspg13.github.io/agent-bounties/funding.html";

#[derive(Parser)]
#[command(name = "agent-bounties")]
#[command(about = "Open-source agent bounty network CLI")]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug)]
struct GithubFundingCommentPlanCli {
    repository: String,
    issue_url: String,
    title: String,
    body_file: String,
    comment_body: String,
    contributor_login: Option<String>,
    comment_id: Option<String>,
    funding_api_base_url: Option<String>,
    existing_idempotency_keys: Vec<String>,
}

#[derive(Debug)]
struct GithubClaimCommentPlanCli {
    repository: String,
    issue_url: String,
    title: String,
    body_file: String,
    comment_body: String,
    contributor_login: Option<String>,
    comment_id: Option<String>,
    claim_age_minutes: Option<u64>,
    progress_signal_count: u32,
    active_claim_login: Option<String>,
}

#[derive(Debug, ClapArgs)]
struct DiscoveryReportArgs {
    #[arg(long)]
    input_fixture: String,
    #[arg(long)]
    json_out: Option<String>,
    #[arg(long)]
    markdown_out: Option<String>,
}

#[derive(Subcommand)]
enum Command {
    Demo,
    PooledFundingDemo,
    FundingRehearsalDemo,
    RealFundingReadiness {
        #[arg(long, default_value = "base-sepolia")]
        network: String,
        #[arg(long)]
        escrow_contract: Option<String>,
        #[arg(long)]
        usdc_token: Option<String>,
        #[arg(long, default_value_t = false)]
        require_live_money: bool,
    },
    Bountybench,
    Abusebench,
    Judgebench,
    EvalLoops,
    RiskPolicy,
    RiskEvents {
        #[arg(long, default_value = "http://127.0.0.1:8080")]
        api_base_url: String,
        #[arg(long)]
        action: Option<String>,
        #[arg(long)]
        surface: Option<String>,
        #[arg(long)]
        bounty_id: Option<Uuid>,
        #[arg(long)]
        agent_id: Option<Uuid>,
        #[arg(long)]
        limit: Option<usize>,
    },
    RiskApproveBounty {
        #[arg(long)]
        risk_event_id: Uuid,
        #[arg(long)]
        title: String,
        #[arg(long)]
        template_slug: String,
        #[arg(long)]
        amount_minor: i64,
        #[arg(long, default_value = "usdc")]
        currency: String,
        #[arg(long, default_value = "BaseUsdcEscrow")]
        funding_mode: String,
        #[arg(long, default_value = "Public")]
        privacy: String,
        #[arg(long)]
        operator_id: String,
        #[arg(long)]
        note: String,
        #[arg(long, default_value = "http://127.0.0.1:8080")]
        api_base_url: String,
    },
    RiskApprovePayout {
        #[arg(long)]
        risk_event_id: Uuid,
        #[arg(long)]
        operator_id: String,
        #[arg(long)]
        note: String,
        #[arg(long, default_value = "http://127.0.0.1:8080")]
        api_base_url: String,
    },
    RiskRejectEvent {
        #[arg(long)]
        risk_event_id: Uuid,
        #[arg(long)]
        operator_id: String,
        #[arg(long)]
        note: String,
        #[arg(long, default_value = "http://127.0.0.1:8080")]
        api_base_url: String,
    },
    EvalRuns {
        #[arg(long, default_value = "http://127.0.0.1:8080")]
        api_base_url: String,
    },
    AgentPaidStatus {
        #[arg(long)]
        agent_id: Uuid,
        #[arg(long, default_value = "http://127.0.0.1:8080")]
        api_base_url: String,
    },
    BasePlan {
        #[arg(long)]
        escrow_contract: String,
        #[arg(long)]
        token: String,
        #[arg(long, default_value_t = 1_000_000)]
        amount_minor: i64,
        #[arg(long, default_value = "base-sepolia")]
        network: String,
    },
    BaseDecodeDemo,
    BaseLogQuery {
        #[arg(long)]
        escrow_contract: String,
        #[arg(long)]
        from_block: u64,
        #[arg(long)]
        to_block: Option<u64>,
        #[arg(long, default_value_t = 1)]
        request_id: u64,
        #[arg(long, default_value = "base-sepolia")]
        network: String,
    },
    BaseFetchLogs {
        #[arg(long)]
        escrow_contract: String,
        #[arg(long)]
        from_block: u64,
        #[arg(long)]
        to_block: Option<u64>,
        #[arg(long, default_value_t = 1)]
        request_id: u64,
        #[arg(long, default_value = "base-sepolia")]
        network: String,
        #[arg(long)]
        rpc_url: Option<String>,
    },
    BaseBroadcastSignedTransaction {
        #[arg(long)]
        signed_transaction: String,
        #[arg(long, default_value_t = 1)]
        request_id: u64,
        #[arg(long, default_value = "base-sepolia")]
        network: String,
        #[arg(long)]
        rpc_url: Option<String>,
    },
    BaseTransactionReceipt {
        #[arg(long)]
        tx_hash: String,
        #[arg(long, default_value_t = 1)]
        request_id: u64,
        #[arg(long, default_value = "base-sepolia")]
        network: String,
        #[arg(long)]
        rpc_url: Option<String>,
    },
    BaseReleaseQueueDemo {
        #[arg(long, default_value = "0x1111111111111111111111111111111111111111")]
        escrow_contract: String,
        #[arg(long, default_value = "0x4444444444444444444444444444444444444444")]
        platform_fee_wallet: String,
    },
    BaseRefundPlan {
        #[arg(long)]
        escrow_contract: String,
        #[arg(long)]
        onchain_escrow_id: u128,
        #[arg(long)]
        reason_hash: String,
    },
    BaseDisputePlan {
        #[arg(long)]
        escrow_contract: String,
        #[arg(long)]
        onchain_escrow_id: u128,
        #[arg(long)]
        dispute_hash: String,
    },
    BaseSepoliaRunbook {
        #[arg(long)]
        settlement_signer: String,
        #[arg(long)]
        escrow_contract: String,
        #[arg(long)]
        usdc_token: String,
        #[arg(long, default_value = "0x2222222222222222222222222222222222222222")]
        payer: String,
        #[arg(long, default_value = "0x3333333333333333333333333333333333333333")]
        solver_wallet: String,
        #[arg(long, default_value = "0x4444444444444444444444444444444444444444")]
        platform_fee_wallet: String,
        #[arg(long, default_value_t = 1_000_000)]
        amount_minor: i64,
    },
    StripePlan {
        #[arg(long)]
        organization_id: Uuid,
        #[arg(long, default_value_t = 5_000)]
        amount_minor: i64,
        #[arg(long, default_value = "https://agentbounties.local")]
        platform_url: String,
    },
    StripeExecuteCheckoutTopUp {
        #[arg(long)]
        organization_id: Uuid,
        #[arg(long, default_value_t = 5_000)]
        amount_minor: i64,
        #[arg(long, default_value = "https://agentbounties.local")]
        platform_url: String,
        #[arg(long)]
        secret_key: Option<String>,
        #[arg(long)]
        api_base_url: Option<String>,
    },
    StripeExecuteConnectAccount {
        #[arg(long)]
        agent_id: Uuid,
        #[arg(long)]
        secret_key: Option<String>,
        #[arg(long)]
        api_base_url: Option<String>,
    },
    StripeExecuteRequestIntent {
        #[arg(long)]
        intent_file: String,
        #[arg(long)]
        secret_key: Option<String>,
        #[arg(long)]
        api_base_url: Option<String>,
    },
    GithubPlan {
        #[arg(long)]
        repository: String,
        #[arg(long)]
        issue_url: String,
        #[arg(long)]
        title: String,
        #[arg(long)]
        body_file: String,
    },
    GithubIssueApiSyncPlan {
        #[arg(long)]
        repository: String,
        #[arg(long)]
        issue_url: String,
        #[arg(long)]
        title: String,
        #[arg(long)]
        body_file: String,
        #[arg(long)]
        api_base_url: Option<String>,
        #[arg(long = "existing-bounty-id")]
        existing_bounty_ids: Vec<Uuid>,
        #[arg(long)]
        hosted_api_error: Option<String>,
    },
    GithubFundingCommentPlan {
        #[arg(long)]
        repository: String,
        #[arg(long)]
        issue_url: String,
        #[arg(long)]
        title: String,
        #[arg(long)]
        body_file: String,
        #[arg(long)]
        comment_body: String,
        #[arg(long)]
        contributor_login: Option<String>,
        #[arg(long)]
        comment_id: Option<String>,
        #[arg(long)]
        funding_api_base_url: Option<String>,
        #[arg(long = "existing-idempotency-key")]
        existing_idempotency_keys: Vec<String>,
    },
    GithubClaimCommentPlan {
        #[arg(long)]
        repository: String,
        #[arg(long)]
        issue_url: String,
        #[arg(long)]
        title: String,
        #[arg(long)]
        body_file: String,
        #[arg(long)]
        comment_body: String,
        #[arg(long)]
        contributor_login: Option<String>,
        #[arg(long)]
        comment_id: Option<String>,
        #[arg(long)]
        claim_age_minutes: Option<u64>,
        #[arg(long, default_value_t = 0)]
        progress_signal_count: u32,
        #[arg(long)]
        active_claim_login: Option<String>,
    },
    GithubProofCommentPlan {
        #[arg(long)]
        bounty_id: Uuid,
        #[arg(long)]
        proof_url: String,
        #[arg(long)]
        verifier_summary: String,
        #[arg(long)]
        settlement_url: Option<String>,
    },
    Discovery {
        #[arg(long, default_value = "http://127.0.0.1:8080")]
        public_base_url: String,
        #[arg(long, default_value = "http://127.0.0.1:8090")]
        mcp_base_url: String,
    },
    DiscoveryReport(DiscoveryReportArgs),
    DocsContractCheck {
        #[arg(long, default_value = ".")]
        root: String,
        #[arg(long, default_value = ".")]
        contract_root: String,
    },
    ProductionSmoke {
        #[arg(long, env = "PRODUCTION_API_BASE_URL")]
        api_base_url: String,
        #[arg(long, env = "PRODUCTION_MCP_BASE_URL")]
        mcp_base_url: String,
        #[arg(long, default_value_t = false)]
        require_eval_history: bool,
    },
    ServiceSmoke {
        #[arg(long, default_value = "http://127.0.0.1:8080")]
        api_base_url: String,
        #[arg(long, default_value = "http://127.0.0.1:8090")]
        mcp_base_url: String,
    },
    ServiceSmokeSpawn {
        #[arg(long, default_value = "http://127.0.0.1:18080")]
        api_base_url: String,
        #[arg(long, default_value = "http://127.0.0.1:18090")]
        mcp_base_url: String,
        #[arg(long)]
        database_url: Option<String>,
        #[arg(long, default_value_t = false)]
        verify_restart_persistence: bool,
    },
}

fn main() -> Result<()> {
    thread::Builder::new()
        .name("agent-bounties-cli".to_string())
        .stack_size(8 * 1024 * 1024)
        .spawn(run_cli)?
        .join()
        .map_err(|_| anyhow!("CLI thread panicked"))?
}

fn run_cli() -> Result<()> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(async_main())
}

async fn async_main() -> Result<()> {
    let args = Args::parse();
    match args.command {
        Command::Demo => demo().await,
        Command::PooledFundingDemo => pooled_funding_demo(),
        Command::FundingRehearsalDemo => funding_rehearsal_demo().await,
        Command::RealFundingReadiness {
            network,
            escrow_contract,
            usdc_token,
            require_live_money,
        } => real_funding_readiness(network, escrow_contract, usdc_token, require_live_money),
        Command::Bountybench => bountybench(),
        Command::Abusebench => abusebench(),
        Command::Judgebench => judgebench(),
        Command::EvalLoops => eval_loops().await,
        Command::RiskPolicy => risk_policy(),
        Command::RiskEvents {
            api_base_url,
            action,
            surface,
            bounty_id,
            agent_id,
            limit,
        } => risk_events(api_base_url, action, surface, bounty_id, agent_id, limit),
        Command::RiskApproveBounty {
            risk_event_id,
            title,
            template_slug,
            amount_minor,
            currency,
            funding_mode,
            privacy,
            operator_id,
            note,
            api_base_url,
        } => risk_approve_bounty(
            api_base_url,
            risk_event_id,
            title,
            template_slug,
            amount_minor,
            currency,
            funding_mode,
            privacy,
            operator_id,
            note,
        ),
        Command::RiskApprovePayout {
            risk_event_id,
            operator_id,
            note,
            api_base_url,
        } => risk_approve_payout(api_base_url, risk_event_id, operator_id, note),
        Command::RiskRejectEvent {
            risk_event_id,
            operator_id,
            note,
            api_base_url,
        } => risk_reject_event(api_base_url, risk_event_id, operator_id, note),
        Command::EvalRuns { api_base_url } => eval_runs(api_base_url),
        Command::AgentPaidStatus {
            agent_id,
            api_base_url,
        } => agent_paid_status(agent_id, api_base_url),
        Command::BasePlan {
            escrow_contract,
            token,
            amount_minor,
            network,
        } => base_plan(escrow_contract, token, amount_minor, network),
        Command::BaseDecodeDemo => base_decode_demo(),
        Command::BaseLogQuery {
            escrow_contract,
            from_block,
            to_block,
            request_id,
            network,
        } => base_log_query(escrow_contract, from_block, to_block, request_id, network),
        Command::BaseFetchLogs {
            escrow_contract,
            from_block,
            to_block,
            request_id,
            network,
            rpc_url,
        } => {
            base_fetch_logs(
                escrow_contract,
                from_block,
                to_block,
                request_id,
                network,
                rpc_url,
            )
            .await
        }
        Command::BaseBroadcastSignedTransaction {
            signed_transaction,
            request_id,
            network,
            rpc_url,
        } => {
            base_broadcast_signed_transaction(signed_transaction, request_id, network, rpc_url)
                .await
        }
        Command::BaseTransactionReceipt {
            tx_hash,
            request_id,
            network,
            rpc_url,
        } => base_transaction_receipt(tx_hash, request_id, network, rpc_url).await,
        Command::BaseReleaseQueueDemo {
            escrow_contract,
            platform_fee_wallet,
        } => base_release_queue_demo(escrow_contract, platform_fee_wallet).await,
        Command::BaseRefundPlan {
            escrow_contract,
            onchain_escrow_id,
            reason_hash,
        } => base_refund_plan(escrow_contract, onchain_escrow_id, reason_hash),
        Command::BaseDisputePlan {
            escrow_contract,
            onchain_escrow_id,
            dispute_hash,
        } => base_dispute_plan(escrow_contract, onchain_escrow_id, dispute_hash),
        Command::BaseSepoliaRunbook {
            settlement_signer,
            escrow_contract,
            usdc_token,
            payer,
            solver_wallet,
            platform_fee_wallet,
            amount_minor,
        } => base_sepolia_runbook(
            settlement_signer,
            escrow_contract,
            usdc_token,
            payer,
            solver_wallet,
            platform_fee_wallet,
            amount_minor,
        ),
        Command::StripePlan {
            organization_id,
            amount_minor,
            platform_url,
        } => stripe_plan(organization_id, amount_minor, platform_url),
        Command::StripeExecuteCheckoutTopUp {
            organization_id,
            amount_minor,
            platform_url,
            secret_key,
            api_base_url,
        } => {
            stripe_execute_checkout_top_up(
                organization_id,
                amount_minor,
                platform_url,
                secret_key,
                api_base_url,
            )
            .await
        }
        Command::StripeExecuteConnectAccount {
            agent_id,
            secret_key,
            api_base_url,
        } => stripe_execute_connect_account(agent_id, secret_key, api_base_url).await,
        Command::StripeExecuteRequestIntent {
            intent_file,
            secret_key,
            api_base_url,
        } => stripe_execute_request_intent(intent_file, secret_key, api_base_url).await,
        Command::GithubPlan {
            repository,
            issue_url,
            title,
            body_file,
        } => github_plan(repository, issue_url, title, body_file),
        Command::GithubIssueApiSyncPlan {
            repository,
            issue_url,
            title,
            body_file,
            api_base_url,
            existing_bounty_ids,
            hosted_api_error,
        } => github_issue_api_sync_plan(
            repository,
            issue_url,
            title,
            body_file,
            api_base_url,
            existing_bounty_ids,
            hosted_api_error,
        ),
        Command::GithubFundingCommentPlan {
            repository,
            issue_url,
            title,
            body_file,
            comment_body,
            contributor_login,
            comment_id,
            funding_api_base_url,
            existing_idempotency_keys,
        } => github_funding_comment_plan(GithubFundingCommentPlanCli {
            repository,
            issue_url,
            title,
            body_file,
            comment_body,
            contributor_login,
            comment_id,
            funding_api_base_url,
            existing_idempotency_keys,
        }),
        Command::GithubClaimCommentPlan {
            repository,
            issue_url,
            title,
            body_file,
            comment_body,
            contributor_login,
            comment_id,
            claim_age_minutes,
            progress_signal_count,
            active_claim_login,
        } => github_claim_comment_plan(GithubClaimCommentPlanCli {
            repository,
            issue_url,
            title,
            body_file,
            comment_body,
            contributor_login,
            comment_id,
            claim_age_minutes,
            progress_signal_count,
            active_claim_login,
        }),
        Command::GithubProofCommentPlan {
            bounty_id,
            proof_url,
            verifier_summary,
            settlement_url,
        } => github_proof_comment_plan(bounty_id, proof_url, verifier_summary, settlement_url),
        Command::Discovery {
            public_base_url,
            mcp_base_url,
        } => discovery(public_base_url, mcp_base_url),
        Command::DiscoveryReport(args) => {
            discovery_report(args.input_fixture, args.json_out, args.markdown_out)
        }
        Command::DocsContractCheck {
            root,
            contract_root,
        } => docs_contract_check(PathBuf::from(root), PathBuf::from(contract_root)),
        Command::ProductionSmoke {
            api_base_url,
            mcp_base_url,
            require_eval_history,
        } => production_smoke(api_base_url, mcp_base_url, require_eval_history).await,
        Command::ServiceSmoke {
            api_base_url,
            mcp_base_url,
        } => service_smoke(api_base_url, mcp_base_url).await,
        Command::ServiceSmokeSpawn {
            api_base_url,
            mcp_base_url,
            database_url,
            verify_restart_persistence,
        } => {
            service_smoke_spawn(
                api_base_url,
                mcp_base_url,
                database_url,
                verify_restart_persistence,
            )
            .await
        }
    }
}

async fn demo() -> Result<()> {
    let mut network = BountyNetwork::default();
    let requester = network.register_agent(RegisterAgentRequest {
        handle: "requester-agent".to_string(),
        payout_wallet: None,
    });
    let solver = network.register_agent(RegisterAgentRequest {
        handle: "solver-agent".to_string(),
        payout_wallet: Some("0xsolver".to_string()),
    });
    network.register_capability(RegisterCapabilityRequest {
        agent_id: solver.id,
        class: CapabilityClass::Extraction,
        template_slugs: vec!["extract-data-to-schema".to_string()],
        min_price_minor: 100_000,
        max_price_minor: 1_000_000,
        currency: "usdc".to_string(),
        latency_seconds: 600,
        supported_verifiers: vec![VerifierKind::JsonSchema],
    })?;

    let help = network.create_help_request(CreateHelpRequestRequest {
        requester_agent_id: requester.id,
        goal: "Extract invoice fields from this PDF into JSON schema".to_string(),
        context: "Need vendor, invoice number, date, subtotal, tax, total".to_string(),
        budget_minor: 1_000_000,
        currency: "usdc".to_string(),
        privacy: PrivacyLevel::Public,
        required_confidence: None,
    })?;
    let quote_set = network.request_quotes(RequestQuotesRequest {
        help_request_id: help.id,
    })?;
    let quote = quote_set
        .quotes
        .first()
        .expect("seeded solver capability should quote")
        .clone();
    let bounty = network.fund_quote_as_bounty(FundQuoteRequest {
        quote_id: quote.id,
        title: Some("Extract invoice fields".to_string()),
        funding_mode: Some(FundingMode::BaseUsdcEscrow),
    })?;
    let mut indexer = ChainEventIndexer::default();
    let created = simulated_created_event(
        bounty.id,
        1,
        "0x3333333333333333333333333333333333333333",
        bounty.amount.clone(),
        bounty.terms_hash.clone().expect("funded bounty has terms"),
    );
    indexer.ingest(created.clone())?;
    network.apply_base_escrow_event(created)?;

    network.claim_bounty(ClaimBountyRequest {
        bounty_id: bounty.id,
        solver_agent_id: solver.id,
    })?;
    let artifact = "{\"vendor\":\"Demo\",\"total\":100}";
    let submission = network.submit_result(SubmitResultRequest {
        bounty_id: bounty.id,
        solver_agent_id: solver.id,
        artifact_uri: "s3://local-demo/invoice.json".to_string(),
        artifact_body: artifact.to_string(),
    })?;
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
        .await?;
    let released = simulated_released_event(bounty.id, 1, proof.proof_hash.clone());
    indexer.ingest(released.clone())?;
    network.apply_base_escrow_event(released)?;
    let status = network.status(bounty.id)?;

    println!("demo_status={:?}", status.bounty.status);
    println!("template={}", status.bounty.template_slug);
    println!("quotes={}", quote_set.quotes.len());
    println!("proof={}", proof.proof_hash);
    println!("ledger_entries={}", network.ledger.entries().len());
    println!("settlements={}", status.settlements.len());
    println!("reputation_events={}", status.reputation_events.len());
    println!("template_signals={}", status.template_signals.len());
    println!("escrows={}", status.escrows.len());
    println!("indexed_chain_events={}", indexer.events().len());
    println!("solver={}", solver.handle);
    Ok(())
}

fn pooled_funding_demo() -> Result<()> {
    let mut network = BountyNetwork::default();
    let sponsor_a = network.register_agent(RegisterAgentRequest {
        handle: "sponsor-a".to_string(),
        payout_wallet: None,
    });
    let sponsor_b = network.register_agent(RegisterAgentRequest {
        handle: "sponsor-b".to_string(),
        payout_wallet: None,
    });
    let bounty = network.open_pooled_bounty(OpenPooledBountyRequest {
        bounty_id: None,
        idempotency_key: None,
        title: "Write the first agent quickstart".to_string(),
        template_slug: "write-docs-for-area".to_string(),
        target_amount_minor: 1_000_000,
        currency: "usdc".to_string(),
        funding_mode: FundingMode::Simulated,
        privacy: PrivacyLevel::Public,
        funding_targets: vec![],
    })?;
    let first = network.add_funding_contribution(AddFundingContributionRequest {
        bounty_id: bounty.id,
        contributor_agent_id: Some(sponsor_a.id),
        source_organization_id: None,
        amount_minor: 400_000,
        currency: "usdc".to_string(),
        rail: PaymentRail::Simulated,
        external_reference: Some("sponsor-a-demo".to_string()),
    })?;
    let second = network.add_funding_contribution(AddFundingContributionRequest {
        bounty_id: bounty.id,
        contributor_agent_id: Some(sponsor_b.id),
        source_organization_id: None,
        amount_minor: 600_000,
        currency: "usdc".to_string(),
        rail: PaymentRail::Simulated,
        external_reference: Some("sponsor-b-demo".to_string()),
    })?;
    let status = network.status(bounty.id)?;

    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "bounty_id": bounty.id,
            "status": format!("{:?}", status.bounty.status),
            "first_remaining_minor": first.funding_summary.remaining.amount,
            "final_applied_minor": second.funding_summary.applied.amount,
            "final_remaining_minor": second.funding_summary.remaining.amount,
            "claimable": second.funding_summary.claimable,
            "contribution_count": status.funding_contributions.len(),
            "ledger_entries": network.ledger.entries().len()
        }))?
    );
    Ok(())
}

async fn funding_rehearsal_demo() -> Result<()> {
    let mut network = BountyNetwork::default();
    let organization_id = Uuid::parse_str("00000000-0000-0000-0000-000000000f01")?;
    let solver = network.register_agent(RegisterAgentRequest {
        handle: "funding-rehearsal-solver".to_string(),
        payout_wallet: Some("0x2222222222222222222222222222222222222222".to_string()),
    });
    let platform_url = "https://agentbounties.local".to_string();

    let bounty = network.open_pooled_bounty(OpenPooledBountyRequest {
        bounty_id: None,
        idempotency_key: None,
        title: "Funding rehearsal mixed Stripe and Base bounty".to_string(),
        template_slug: "extract-data-to-schema".to_string(),
        target_amount_minor: 500,
        currency: "usd".to_string(),
        funding_mode: FundingMode::MixedRails,
        privacy: PrivacyLevel::Public,
        funding_targets: vec![
            FundingPartitionTargetRequest {
                rail: PaymentRail::StripeFiat,
                amount_minor: 500,
                currency: "usd".to_string(),
            },
            FundingPartitionTargetRequest {
                rail: PaymentRail::BaseUsdc,
                amount_minor: 1_000,
                currency: "usdc".to_string(),
            },
        ],
    })?;

    let stripe_intent = network.create_funding_intent(
        CreateFundingIntentRequest {
            bounty_id: bounty.id,
            contributor_agent_id: None,
            source_organization_id: Some(organization_id),
            amount_minor: 500,
            currency: "usd".to_string(),
            rail: PaymentRail::StripeFiat,
            external_reference: Some("funding-rehearsal-stripe-intent-500".to_string()),
            stripe_success_url: Some(format!("{platform_url}/stripe/success")),
            stripe_cancel_url: Some(format!("{platform_url}/stripe/cancel")),
            base_escrow_contract: None,
            base_payer: None,
            base_token: None,
            base_network: None,
        },
        platform_url.clone(),
    )?;
    let stripe_checkout_request = match &stripe_intent.next_action {
        FundingIntentNextAction::StripeCheckout { request } => request.clone(),
        FundingIntentNextAction::BaseEscrowFunding { .. } => {
            bail!("Stripe funding intent returned Base next action")
        }
    };
    let stripe_webhook = StripeWebhookEvent {
        id: "evt_test_funding_rehearsal_topup".to_string(),
        event_type: "checkout.session.completed".to_string(),
        payload: serde_json::json!({
            "id": "cs_test_funding_rehearsal_topup",
            "payment_status": "paid",
            "client_reference_id": organization_id.to_string(),
            "amount_total": 500,
            "currency": "usd",
            "payment_intent": "pi_test_funding_rehearsal_topup",
            "metadata": {
                "bounty_id": bounty.id.to_string(),
                "funding_intent_id": stripe_intent.intent.id.to_string(),
                "funding_intent_reference": "funding-rehearsal-stripe-intent-500",
                "purpose": "bounty_funding_intent"
            }
        }),
    };
    let stripe_credit = StripeEventDeduper::default().apply_checkout_top_up(&stripe_webhook)?;
    let stripe_reconciliation = network.apply_stripe_funding_credit(stripe_credit)?;
    let stripe_contribution = stripe_reconciliation
        .funding_report
        .as_ref()
        .map(|report| report.contribution.clone())
        .context("Stripe funding intent webhook did not reserve bounty funding")?;

    let escrow_contract = "0x1111111111111111111111111111111111111111".to_string();
    let usdc_token = "0x3333333333333333333333333333333333333333".to_string();
    let base_intent = network.create_funding_intent(
        CreateFundingIntentRequest {
            bounty_id: bounty.id,
            contributor_agent_id: None,
            source_organization_id: None,
            amount_minor: 1_000,
            currency: "usdc".to_string(),
            rail: PaymentRail::BaseUsdc,
            external_reference: Some("funding-rehearsal-base-intent-1000".to_string()),
            stripe_success_url: None,
            stripe_cancel_url: None,
            base_escrow_contract: Some(escrow_contract.clone()),
            base_payer: Some("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string()),
            base_token: Some(usdc_token.clone()),
            base_network: Some("base-sepolia".to_string()),
        },
        platform_url.clone(),
    )?;
    let base_funding_plan = match &base_intent.next_action {
        FundingIntentNextAction::BaseEscrowFunding { plan } => plan.as_ref().clone(),
        FundingIntentNextAction::StripeCheckout { .. } => {
            bail!("Base funding intent returned Stripe next action")
        }
    };
    let terms_hash = bounty
        .terms_hash
        .clone()
        .context("rehearsal bounty missing terms hash")?;
    let base_created = network.apply_base_escrow_event(simulated_created_event(
        bounty.id,
        77,
        usdc_token,
        Money::new(1_000, "usdc")?,
        terms_hash,
    ))?;

    network.claim_bounty(ClaimBountyRequest {
        bounty_id: bounty.id,
        solver_agent_id: solver.id,
    })?;
    let artifact = r#"{"funding_rehearsal":true}"#;
    let submission = network.submit_result(SubmitResultRequest {
        bounty_id: bounty.id,
        solver_agent_id: solver.id,
        artifact_uri: "memory://funding-rehearsal-artifact.json".to_string(),
        artifact_body: artifact.to_string(),
    })?;
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
        .await?;
    let base_release_plan = network.plan_base_release(PlanBaseReleaseRequest {
        bounty_id: bounty.id,
        escrow_contract,
        platform_fee_wallet: "0x5555555555555555555555555555555555555555".to_string(),
        network: Some("base-sepolia".to_string()),
    })?;
    let base_released = network.apply_base_escrow_event(simulated_released_event(
        bounty.id,
        77,
        proof.proof_hash.clone(),
    ))?;
    let stripe_connect_eligibility =
        network.apply_stripe_connect_snapshot(ConnectAccountSnapshot {
            agent_id: solver.id,
            connected_account_id: Some("acct_test_funding_rehearsal".to_string()),
            payouts_enabled: true,
            disabled_reason: None,
            currently_due: vec![],
        })?;
    let after_connect_status = network.status(bounty.id)?;
    let stripe_settlement = after_connect_status
        .settlements
        .iter()
        .find(|settlement| settlement.rail == PaymentRail::StripeFiat)
        .cloned()
        .context("missing Stripe settlement after deterministic verification")?;
    let stripe_payout_intent = stripe_settlement
        .payout_intents
        .first()
        .cloned()
        .context("missing Stripe payout intent after deterministic verification")?;
    let stripe_transfer_plan = network.plan_stripe_transfer(
        PlanStripeTransferRequest {
            payout_intent_id: stripe_payout_intent.id,
            connected_account_id: "acct_test_funding_rehearsal".to_string(),
        },
        platform_url.clone(),
    )?;
    let stripe_transfer_event = StripeWebhookEvent {
        id: "evt_test_funding_rehearsal_transfer".to_string(),
        event_type: "transfer.created".to_string(),
        payload: serde_json::json!({
            "id": "tr_test_funding_rehearsal_solver",
            "destination": "acct_test_funding_rehearsal",
            "amount": stripe_payout_intent.amount.amount,
            "currency": stripe_payout_intent.amount.currency,
            "metadata": {
                "bounty_id": stripe_settlement.bounty_id.to_string(),
                "proof_record_id": stripe_settlement.proof_record_id.to_string(),
                "settlement_id": stripe_settlement.id.to_string(),
                "payout_intent_id": stripe_payout_intent.id.to_string(),
                "agent_id": stripe_payout_intent.recipient_agent_id.to_string()
            }
        }),
    };
    let stripe_transfer_evidence =
        StripeEventDeduper::default().apply_connect_transfer(&stripe_transfer_event)?;
    let stripe_transfer_reconciliation =
        network.apply_stripe_transfer_evidence(stripe_transfer_evidence)?;
    let final_status = network.status(bounty.id)?;

    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "rehearsal": "stripe-dev-plus-base-sepolia-mixed-funding",
            "invariants": [
                "Stripe Checkout Session creation does not credit balances.",
                "Stripe fiat funding is reserved only after paid webhook reconciliation.",
                "Base USDC funding is claimable only after indexed EscrowCreated reconciliation.",
                "Deterministic digest verification creates settlement intents.",
                "Base payout is paid only after indexed EscrowReleased reconciliation.",
                "Stripe Connect eligibility does not pay a bounty.",
                "Stripe payout is paid only after transfer.created reconciliation."
            ],
            "stripe": {
                "funding_intent": stripe_intent.intent,
                "checkout_request": stripe_checkout_request,
                "webhook_event_id": stripe_webhook.id,
                "funding_reconciliation": stripe_reconciliation,
                "funding_contribution": stripe_contribution,
                "connect_eligibility": stripe_connect_eligibility,
                "transfer_plan": stripe_transfer_plan,
                "transfer_event_id": stripe_transfer_event.id,
                "transfer_reconciliation": stripe_transfer_reconciliation
            },
            "base": {
                "funding_intent": base_intent.intent,
                "funding_plan": base_funding_plan,
                "created_reconciliation": base_created,
                "release_plan": base_release_plan,
                "released_reconciliation": base_released
            },
            "operator_test_mode_next_steps": [
                "Execute the Stripe checkout_request with a sk_test key through stripe-execute-request-intent; the Checkout Session still does not credit the bounty.",
                "Complete the Stripe test Checkout or replay a signed checkout.session.completed webhook carrying bounty_id and funding_intent_id metadata.",
                "Sign and send the Base Sepolia approve and createEscrow transactions from the Base funding plan, then reconcile indexed EscrowCreated logs.",
                "After deterministic verification, sign and send the Base release plan, then reconcile the indexed EscrowReleased log.",
                "Reconcile Stripe Connect eligibility, execute the transfer_plan with a sk_test key, then reconcile the transfer.created event before treating fiat payout intents as paid."
            ],
            "proof": proof,
            "final_bounty": final_status.bounty,
            "funding_summary": final_status.funding_summary,
            "settlements": final_status.settlements,
            "ledger_entries": network.ledger.entries().len()
        }))?
    );
    Ok(())
}

fn real_funding_readiness(
    network: String,
    escrow_contract: Option<String>,
    usdc_token: Option<String>,
    require_live_money: bool,
) -> Result<()> {
    let network_descriptor = base_network_descriptor(&network)?;
    let rpc_env = network_descriptor.rpc_url_env.clone();
    let stripe_secret_key = env::var("STRIPE_SECRET_KEY").ok();
    let report = build_live_money_readiness_report(LiveMoneyReadinessConfig {
        network,
        escrow_contract,
        usdc_token,
        stripe_secret_key_mode: stripe_secret_key_mode_from_secret(stripe_secret_key.as_deref()),
        stripe_live_execution_enabled: env_flag("ENABLE_STRIPE_LIVE_EXECUTION"),
        stripe_payment_method_configuration_configured: env_nonempty(
            "STRIPE_PAYMENT_METHOD_CONFIGURATION",
        ),
        stripe_webhook_secret_configured: env_nonempty("STRIPE_WEBHOOK_SECRET"),
        allow_unsigned_stripe_webhooks: env_flag("ALLOW_UNSIGNED_STRIPE_WEBHOOKS"),
        operator_auth_configured: env_nonempty("OPERATOR_API_TOKEN"),
        base_rpc_url_configured: env_nonempty(&rpc_env),
        base_broadcast_enabled: env_flag("ENABLE_BASE_TX_BROADCAST"),
    })?;

    println!("{}", serde_json::to_string_pretty(&report)?);
    if require_live_money && !report.live_money_ready {
        bail!(
            "live money readiness failed: Stripe live mode and Base mainnet USDC are not both ready"
        );
    }
    Ok(())
}

fn env_nonempty(name: &str) -> bool {
    env::var(name).ok().is_some_and(|value| nonempty(&value))
}

fn env_flag(name: &str) -> bool {
    env::var(name)
        .ok()
        .map(|value| value.eq_ignore_ascii_case("true") || value == "1")
        .unwrap_or(false)
}

fn nonempty(value: &str) -> bool {
    !value.trim().is_empty()
}

fn bountybench() -> Result<()> {
    let result = BountyBench::default().run(&bundled_fixtures())?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

fn abusebench() -> Result<()> {
    let result = AbuseBench::default().run(&bundled_abuse_fixtures())?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

fn judgebench() -> Result<()> {
    let result = JudgeBench::default().run(&bundled_judge_fixtures())?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

async fn eval_loops() -> Result<()> {
    let result = run_eval_loops().await?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    if !result.passed {
        bail!("eval loops did not pass");
    }
    Ok(())
}

fn risk_policy() -> Result<()> {
    println!(
        "{}",
        serde_json::to_string_pretty(&risk::RiskPolicy::default().descriptor())?
    );
    Ok(())
}

fn risk_events(
    api_base_url: String,
    action: Option<String>,
    surface: Option<String>,
    bounty_id: Option<Uuid>,
    agent_id: Option<Uuid>,
    limit: Option<usize>,
) -> Result<()> {
    let api = normalize_base_url(&api_base_url);
    let mut params = Vec::new();
    push_query_param(&mut params, "action", action);
    push_query_param(&mut params, "surface", surface);
    push_query_param(&mut params, "bounty_id", bounty_id.map(|id| id.to_string()));
    push_query_param(&mut params, "agent_id", agent_id.map(|id| id.to_string()));
    push_query_param(&mut params, "limit", limit.map(|value| value.to_string()));

    let mut url = format!("{api}/v1/risk/events");
    if !params.is_empty() {
        url.push('?');
        url.push_str(&params.join("&"));
    }
    let events = get_json(&url)?;
    println!("{}", serde_json::to_string_pretty(&events)?);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn risk_approve_bounty(
    api_base_url: String,
    risk_event_id: Uuid,
    title: String,
    template_slug: String,
    amount_minor: i64,
    currency: String,
    funding_mode: String,
    privacy: String,
    operator_id: String,
    note: String,
) -> Result<()> {
    let api = normalize_base_url(&api_base_url);
    let approval = post_json(
        &format!("{api}/v1/risk/bounty-approvals"),
        serde_json::json!({
            "risk_event_id": risk_event_id,
            "title": title,
            "template_slug": template_slug,
            "amount_minor": amount_minor,
            "currency": currency,
            "funding_mode": funding_mode,
            "privacy": privacy,
            "operator_id": operator_id,
            "note": note
        }),
    )?;
    println!("{}", serde_json::to_string_pretty(&approval)?);
    Ok(())
}

fn risk_approve_payout(
    api_base_url: String,
    risk_event_id: Uuid,
    operator_id: String,
    note: String,
) -> Result<()> {
    let api = normalize_base_url(&api_base_url);
    let review = post_json(
        &format!("{api}/v1/risk/payout-approvals"),
        serde_json::json!({
            "risk_event_id": risk_event_id,
            "operator_id": operator_id,
            "note": note
        }),
    )?;
    println!("{}", serde_json::to_string_pretty(&review)?);
    Ok(())
}

fn risk_reject_event(
    api_base_url: String,
    risk_event_id: Uuid,
    operator_id: String,
    note: String,
) -> Result<()> {
    let api = normalize_base_url(&api_base_url);
    let review = post_json(
        &format!("{api}/v1/risk/events/{risk_event_id}/reject"),
        serde_json::json!({
            "risk_event_id": risk_event_id,
            "operator_id": operator_id,
            "note": note
        }),
    )?;
    println!("{}", serde_json::to_string_pretty(&review)?);
    Ok(())
}

fn eval_runs(api_base_url: String) -> Result<()> {
    let api = normalize_base_url(&api_base_url);
    let runs = get_json(&format!("{api}/v1/evals/runs"))?;
    println!("{}", serde_json::to_string_pretty(&runs)?);
    Ok(())
}

fn agent_paid_status(agent_id: Uuid, api_base_url: String) -> Result<()> {
    let api = normalize_base_url(&api_base_url);
    let status = get_json(&format!("{api}/v1/agents/{agent_id}/paid-status"))?;
    println!("{}", serde_json::to_string_pretty(&status)?);
    Ok(())
}

fn base_plan(
    escrow_contract: String,
    token: String,
    amount_minor: i64,
    network: String,
) -> Result<()> {
    let planner = BaseEscrowTxPlanner::new(escrow_contract)?;
    let bounty_id = Uuid::new_v4();
    let create = BaseEscrowCreate {
        bounty_id,
        payer: "0x2222222222222222222222222222222222222222".to_string(),
        token,
        amount: Money::new(amount_minor, "usdc")?,
        terms_hash: format!("0x{}", "11".repeat(32)),
    };
    let release = BaseEscrowReleaseCall {
        onchain_escrow_id: 1,
        proof_hash: format!("0x{}", "22".repeat(32)),
        recipients: vec![
            EscrowRecipient {
                address: "0x3333333333333333333333333333333333333333".to_string(),
                amount: Money::new(amount_minor * 90 / 100, "usdc")?,
            },
            EscrowRecipient {
                address: "0x4444444444444444444444444444444444444444".to_string(),
                amount: Money::new(amount_minor - (amount_minor * 90 / 100), "usdc")?,
            },
        ],
    };

    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "bounty_id": bounty_id,
            "network": base_network_descriptor(&network)?,
            "funding": planner.plan_funding_for_network(&network, &create)?,
            "release": planner.release(&release)?
        }))?
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn base_sepolia_runbook(
    settlement_signer: String,
    escrow_contract: String,
    usdc_token: String,
    payer: String,
    solver_wallet: String,
    platform_fee_wallet: String,
    amount_minor: i64,
) -> Result<()> {
    let planner = BaseEscrowTxPlanner::new(escrow_contract)?;
    let network = base_network_descriptor("base-sepolia")?;
    let rpc_url_env = network.rpc_url_env.clone();
    let bounty_id = Uuid::new_v4();
    let create = BaseEscrowCreate {
        bounty_id,
        payer: payer.clone(),
        token: usdc_token,
        amount: Money::new(amount_minor, "usdc")?,
        terms_hash: format!("0x{}", "11".repeat(32)),
    };
    let release = BaseEscrowReleaseCall {
        onchain_escrow_id: 1,
        proof_hash: format!("0x{}", "22".repeat(32)),
        recipients: vec![
            EscrowRecipient {
                address: solver_wallet,
                amount: Money::new(amount_minor * 90 / 100, "usdc")?,
            },
            EscrowRecipient {
                address: platform_fee_wallet,
                amount: Money::new(amount_minor - (amount_minor * 90 / 100), "usdc")?,
            },
        ],
    };
    let funding = planner.plan_funding_for_network("base-sepolia", &create)?;
    let release_tx = planner.release(&release)?;

    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "network": network,
            "working_directory": "contracts/base-escrow",
            "required_env": [
                rpc_url_env,
                "BASE_DEPLOYER_PRIVATE_KEY",
                "BASE_PAYER_PRIVATE_KEY",
                "BASE_SETTLEMENT_SIGNER_PRIVATE_KEY"
            ],
            "deploy": {
                "contract": "src/AgentBountyEscrow.sol:AgentBountyEscrow",
                "settlement_signer": settlement_signer,
                "bash": format!(
                    "forge create --rpc-url ${} --private-key $BASE_DEPLOYER_PRIVATE_KEY --verify src/AgentBountyEscrow.sol:AgentBountyEscrow --constructor-args {}",
                    rpc_url_env, settlement_signer
                ),
                "powershell": format!(
                    "forge create --rpc-url $env:{} --private-key $env:BASE_DEPLOYER_PRIVATE_KEY --verify src/AgentBountyEscrow.sol:AgentBountyEscrow --constructor-args {}",
                    rpc_url_env, settlement_signer
                )
            },
            "sample_bounty": {
                "bounty_id": bounty_id,
                "payer": payer,
                "amount_minor": amount_minor,
                "terms_hash": create.terms_hash,
                "proof_hash": release.proof_hash
            },
            "funding": {
                "network": funding.network,
                "approve": cast_send_step(&funding.approve, "BASE_PAYER_PRIVATE_KEY", &rpc_url_env),
                "create_escrow": cast_send_step(&funding.create_escrow, "BASE_PAYER_PRIVATE_KEY", &rpc_url_env)
            },
            "settlement": {
                "network": network,
                "release": cast_send_step(&release_tx, "BASE_SETTLEMENT_SIGNER_PRIVATE_KEY", &rpc_url_env),
                "note": "The platform should mark the bounty paid only after the EscrowReleased log is indexed through /v1/base/evm-logs."
            }
        }))?
    );
    Ok(())
}

fn cast_send_step(
    tx: &chain_base::EvmTransactionIntent,
    private_key_env: &str,
    rpc_url_env: &str,
) -> serde_json::Value {
    serde_json::json!({
        "function": tx.function,
        "to": tx.to,
        "expected_from": tx.from,
        "value_wei": tx.value_wei,
        "data": tx.data,
        "bash": format!(
            "cast send --rpc-url ${} --private-key ${} {} --data {}",
            rpc_url_env, private_key_env, tx.to, tx.data
        ),
        "powershell": format!(
            "cast send --rpc-url $env:{} --private-key $env:{} {} --data {}",
            rpc_url_env, private_key_env, tx.to, tx.data
        )
    })
}

fn base_decode_demo() -> Result<()> {
    let bounty_id = Uuid::from_u128(42);
    let terms_hash = format!("0x{}", "11".repeat(32));
    let proof_hash = format!("0x{}", "22".repeat(32));
    let mut decoder = BaseEscrowLogDecoder::default();

    let created = decoder.decode(EvmLog {
        address: "0x1111111111111111111111111111111111111111".to_string(),
        topics: vec![
            evm_event_topic("EscrowCreated(uint256,bytes32,address,address,uint256,bytes32)"),
            evm_uint256_word(1),
            evm_uint256_word(bounty_id.as_u128()),
            evm_address_word("0x2222222222222222222222222222222222222222")?,
        ],
        data: evm_words_data(&[
            evm_address_word("0x3333333333333333333333333333333333333333")?,
            evm_uint256_word(1_000_000),
            evm_bytes32_word(&terms_hash)?,
        ])?,
        tx_hash: format!("0x{}", "aa".repeat(32)),
        block_number: 1,
        log_index: 0,
        occurred_at: None,
    })?;
    let released = decoder.decode(EvmLog {
        address: "0x1111111111111111111111111111111111111111".to_string(),
        topics: vec![
            evm_event_topic("EscrowReleased(uint256,bytes32)"),
            evm_uint256_word(1),
        ],
        data: evm_words_data(&[evm_bytes32_word(&proof_hash)?])?,
        tx_hash: format!("0x{}", "bb".repeat(32)),
        block_number: 2,
        log_index: 0,
        occurred_at: None,
    })?;

    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "created": created,
            "released": released
        }))?
    );
    Ok(())
}

fn base_log_query(
    escrow_contract: String,
    from_block: u64,
    to_block: Option<u64>,
    request_id: u64,
    network: String,
) -> Result<()> {
    let query = BaseEscrowLogQuery::new(escrow_contract, from_block, to_block)?;
    let request = query.rpc_request(request_id);
    let network = base_network_descriptor(&network)?;
    let rpc_url_env = network.rpc_url_env.clone();
    let request_json = serde_json::to_string(&request)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "network": network,
            "request": request,
            "curl": {
                "bash": format!(
                    "curl -s -X POST \"${}\" -H 'content-type: application/json' --data '{}'",
                    rpc_url_env.as_str(),
                    request_json.replace('\'', "'\\''")
                ),
                "powershell": format!(
                    "Invoke-RestMethod -Method Post -Uri $env:{} -ContentType 'application/json' -Body '{}'",
                    rpc_url_env.as_str(),
                    request_json.replace('\'', "''")
                )
            },
            "next_step": "Submit the full JSON-RPC provider response to POST /v1/base/rpc-logs or MCP reconcile_base_rpc_logs."
        }))?
    );
    Ok(())
}

async fn base_fetch_logs(
    escrow_contract: String,
    from_block: u64,
    to_block: Option<u64>,
    request_id: u64,
    network: String,
    rpc_url: Option<String>,
) -> Result<()> {
    let query = BaseEscrowLogQuery::new(escrow_contract, from_block, to_block)?;
    let request = query.rpc_request(request_id);
    let network_descriptor = base_network_descriptor(&network)?;
    let resolved_rpc_url = resolve_base_rpc_url(&network, rpc_url)?;
    let response = fetch_base_escrow_logs(&resolved_rpc_url, &query, request_id).await?;
    let normalized_logs = rpc_logs_to_evm_logs(response.result.clone())?;

    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "network": network_descriptor,
            "request": request,
            "response": response,
            "fetched_logs": normalized_logs.len(),
            "normalized_logs": normalized_logs,
            "next_step": "Submit this response to POST /v1/base/rpc-logs, or call POST /v1/base/fetch-rpc-logs on a service configured with the same RPC URL."
        }))?
    );
    Ok(())
}

async fn base_broadcast_signed_transaction(
    signed_transaction: String,
    request_id: u64,
    network: String,
    rpc_url: Option<String>,
) -> Result<()> {
    let network_descriptor = base_network_descriptor(&network)?;
    let resolved_rpc_url = resolve_base_rpc_url(&network, rpc_url)?;
    let request = eth_send_raw_transaction_request(&signed_transaction, request_id)?;
    let response =
        broadcast_signed_transaction(&resolved_rpc_url, &signed_transaction, request_id).await?;
    let tx_hash = response.result.clone();

    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "network": network_descriptor,
            "request": request,
            "response": response,
            "tx_hash": tx_hash,
            "next_step": "Run base-transaction-receipt with this tx_hash, then reconcile emitted escrow logs through the API or MCP receipt path."
        }))?
    );
    Ok(())
}

async fn base_transaction_receipt(
    tx_hash: String,
    request_id: u64,
    network: String,
    rpc_url: Option<String>,
) -> Result<()> {
    let network_descriptor = base_network_descriptor(&network)?;
    let resolved_rpc_url = resolve_base_rpc_url(&network, rpc_url)?;
    let request = eth_get_transaction_receipt_request(&tx_hash, request_id)?;
    let normalized_tx_hash = request.params[0].clone();
    let response =
        fetch_transaction_receipt(&resolved_rpc_url, &normalized_tx_hash, request_id).await?;
    let (receipt_found, block_number, succeeded, log_count, normalized_logs) =
        if let Some(receipt) = &response.result {
            (
                true,
                receipt.block_number()?,
                receipt.succeeded()?,
                receipt.logs.len(),
                receipt.logs_to_evm_logs()?,
            )
        } else {
            (false, None, None, 0, vec![])
        };

    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "network": network_descriptor,
            "request": request,
            "response": response,
            "receipt_found": receipt_found,
            "tx_hash": normalized_tx_hash,
            "block_number": block_number,
            "succeeded": succeeded,
            "log_count": log_count,
            "normalized_logs": normalized_logs,
            "next_step": "If normalized_logs includes escrow events, submit them to POST /v1/base/evm-logs or call POST /v1/base/transaction-receipt with reconcile_logs=true on a configured service."
        }))?
    );
    Ok(())
}

fn resolve_base_rpc_url(network: &str, rpc_url: Option<String>) -> Result<String> {
    Ok(match rpc_url {
        Some(url) => url,
        None => BaseRpcUrlConfig::from_env().resolve(network)?.1,
    })
}

async fn base_release_queue_demo(
    escrow_contract: String,
    platform_fee_wallet: String,
) -> Result<()> {
    let mut network = BountyNetwork::default();
    let solver = network.register_agent(RegisterAgentRequest {
        handle: "solver-agent".to_string(),
        payout_wallet: Some("0x2222222222222222222222222222222222222222".to_string()),
    });
    let bounty = network.post_funded_bounty(PostBountyRequest {
        title: "Extract invoice fields".to_string(),
        template_slug: "extract-data-to-schema".to_string(),
        amount_minor: 1_000_000,
        currency: "usdc".to_string(),
        funding_mode: FundingMode::BaseUsdcEscrow,
        privacy: PrivacyLevel::Public,
    })?;
    let created = simulated_created_event(
        bounty.id,
        1,
        "0x3333333333333333333333333333333333333333",
        bounty.amount.clone(),
        bounty.terms_hash.clone().expect("funded bounty has terms"),
    );
    network.apply_base_escrow_event(created)?;
    network.claim_bounty(ClaimBountyRequest {
        bounty_id: bounty.id,
        solver_agent_id: solver.id,
    })?;
    let artifact = "{\"vendor\":\"Demo\",\"total\":100}";
    let submission = network.submit_result(SubmitResultRequest {
        bounty_id: bounty.id,
        solver_agent_id: solver.id,
        artifact_uri: "s3://local-demo/invoice.json".to_string(),
        artifact_body: artifact.to_string(),
    })?;
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
        .await?;
    let queue = network.list_base_release_queue(BaseReleaseQueueRequest {
        escrow_contract: Some(escrow_contract),
        platform_fee_wallet: Some(platform_fee_wallet),
        network: None,
    });

    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "proof_hash": proof.proof_hash,
            "queue": queue
        }))?
    );
    Ok(())
}

fn base_refund_plan(
    escrow_contract: String,
    onchain_escrow_id: u128,
    reason_hash: String,
) -> Result<()> {
    let transaction =
        BaseEscrowTxPlanner::new(escrow_contract)?.refund(onchain_escrow_id, &reason_hash)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "onchain_escrow_id": onchain_escrow_id,
            "reason_hash": reason_hash,
            "transaction": transaction,
            "next_step": "Sign and broadcast this transaction with the settlement signer, then reconcile the EscrowRefunded log before treating the bounty as refunded."
        }))?
    );
    Ok(())
}

fn base_dispute_plan(
    escrow_contract: String,
    onchain_escrow_id: u128,
    dispute_hash: String,
) -> Result<()> {
    let transaction = BaseEscrowTxPlanner::new(escrow_contract)?
        .mark_disputed(onchain_escrow_id, &dispute_hash)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "onchain_escrow_id": onchain_escrow_id,
            "dispute_hash": dispute_hash,
            "transaction": transaction,
            "next_step": "Sign and broadcast this transaction with the settlement signer, then reconcile the EscrowDisputed log before treating the bounty as disputed."
        }))?
    );
    Ok(())
}

fn stripe_plan(organization_id: Uuid, amount_minor: i64, platform_url: String) -> Result<()> {
    let planner = StripePlanner::new(platform_url.clone());
    let checkout = planner.checkout_top_up(&CheckoutTopUpRequest {
        organization_id,
        amount: Money::new(amount_minor, "usd")?,
        success_url: format!("{platform_url}/stripe/success"),
        cancel_url: format!("{platform_url}/stripe/cancel"),
    })?;
    let connect = planner.connect_account_v2(organization_id)?;

    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "checkout_top_up": checkout,
            "connect_account": connect
        }))?
    );
    Ok(())
}

async fn stripe_execute_checkout_top_up(
    organization_id: Uuid,
    amount_minor: i64,
    platform_url: String,
    secret_key: Option<String>,
    api_base_url: Option<String>,
) -> Result<()> {
    let planner = StripePlanner::new(platform_url.clone());
    let intent = planner.checkout_top_up(&CheckoutTopUpRequest {
        organization_id,
        amount: Money::new(amount_minor, "usd")?,
        success_url: format!("{platform_url}/stripe/success"),
        cancel_url: format!("{platform_url}/stripe/cancel"),
    })?;
    let report = execute_stripe_request(
        &intent,
        &resolve_stripe_secret(secret_key)?,
        &resolve_stripe_api_base(api_base_url),
    )
    .await?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

async fn stripe_execute_connect_account(
    agent_id: Uuid,
    secret_key: Option<String>,
    api_base_url: Option<String>,
) -> Result<()> {
    let intent = StripePlanner::new("http://127.0.0.1:8080")
        .connect_account_v2(agent_id)?
        .request;
    let report = execute_stripe_request(
        &intent,
        &resolve_stripe_secret(secret_key)?,
        &resolve_stripe_api_base(api_base_url),
    )
    .await?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

async fn stripe_execute_request_intent(
    intent_file: String,
    secret_key: Option<String>,
    api_base_url: Option<String>,
) -> Result<()> {
    let value: serde_json::Value = serde_json::from_str(&fs::read_to_string(intent_file)?)?;
    let intent = stripe_request_intent_from_value(&value)?;
    let report = execute_stripe_request(
        &intent,
        &resolve_stripe_secret(secret_key)?,
        &resolve_stripe_api_base(api_base_url),
    )
    .await?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

fn stripe_request_intent_from_value(value: &serde_json::Value) -> Result<StripeRequestIntent> {
    if let Ok(intent) = serde_json::from_value::<StripeRequestIntent>(value.clone()) {
        return Ok(intent);
    }
    for pointer in [
        "/next_action/payload/request",
        "/next_action/StripeCheckout/request",
        "/stripe/checkout_request",
    ] {
        if let Some(request) = value.pointer(pointer) {
            return serde_json::from_value(request.clone()).with_context(|| {
                format!("failed to parse StripeRequestIntent at JSON pointer {pointer}")
            });
        }
    }
    bail!("intent file must contain a StripeRequestIntent or funding-intent report")
}

fn resolve_stripe_secret(secret_key: Option<String>) -> Result<String> {
    secret_key
        .or_else(|| env::var("STRIPE_SECRET_KEY").ok())
        .filter(|secret| !secret.trim().is_empty())
        .context("STRIPE_SECRET_KEY or --secret-key is required for live Stripe execution")
}

fn resolve_stripe_api_base(api_base_url: Option<String>) -> String {
    api_base_url
        .or_else(|| env::var("STRIPE_API_BASE_URL").ok())
        .unwrap_or_else(|| STRIPE_API_BASE_URL.to_string())
}

fn github_plan(
    repository: String,
    issue_url: String,
    title: String,
    body_file: String,
) -> Result<()> {
    let body = fs::read_to_string(body_file)?;
    let parsed = parse_issue_form_bounty(&repository, &issue_url, &title, &body);
    let output = match &parsed {
        Ok(bounty) => bounty_check_output(Ok(bounty)),
        Err(error) => bounty_check_output(Err(error)),
    };

    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "parsed": parsed.ok(),
            "check": output
        }))?
    );
    Ok(())
}

fn github_issue_api_sync_plan(
    repository: String,
    issue_url: String,
    title: String,
    body_file: String,
    api_base_url: Option<String>,
    existing_bounty_ids: Vec<Uuid>,
    hosted_api_error: Option<String>,
) -> Result<()> {
    let body = fs::read_to_string(body_file)?;
    let plan = issue_api_sync_plan(GitHubIssueApiSyncInput {
        repository,
        issue_url,
        title,
        body,
        api_base_url,
        existing_bounty_ids,
        hosted_api_error,
    });
    println!("{}", serde_json::to_string_pretty(&plan)?);
    Ok(())
}

fn github_funding_comment_plan(args: GithubFundingCommentPlanCli) -> Result<()> {
    let body = fs::read_to_string(args.body_file)?;
    let plan = funding_comment_plan(GitHubFundingCommentInput {
        repository: args.repository,
        issue_url: args.issue_url,
        title: args.title,
        body,
        comment_body: args.comment_body,
        contributor_login: args.contributor_login,
        comment_id: args.comment_id,
        funding_api_base_url: args.funding_api_base_url,
        existing_idempotency_keys: args.existing_idempotency_keys,
    });
    println!("{}", serde_json::to_string_pretty(&plan)?);
    Ok(())
}

fn github_claim_comment_plan(args: GithubClaimCommentPlanCli) -> Result<()> {
    let body = fs::read_to_string(args.body_file)?;
    let plan = claim_comment_plan(GitHubClaimCommentInput {
        repository: args.repository,
        issue_url: args.issue_url,
        title: args.title,
        body,
        comment_body: args.comment_body,
        contributor_login: args.contributor_login,
        comment_id: args.comment_id,
        claim_age_minutes: args.claim_age_minutes,
        progress_signal_count: args.progress_signal_count,
        active_claim_login: args.active_claim_login,
    });
    println!("{}", serde_json::to_string_pretty(&plan)?);
    Ok(())
}

fn github_proof_comment_plan(
    bounty_id: Uuid,
    proof_url: String,
    verifier_summary: String,
    settlement_url: Option<String>,
) -> Result<()> {
    let comment = GitHubProofComment {
        bounty_id,
        proof_url,
        verifier_summary,
        settlement_url,
    };
    let plan = proof_comment_plan(comment);
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "comment": plan.comment,
            "markdown": plan.markdown,
            "fingerprint": plan.fingerprint,
            "check": plan.check
        }))?
    );
    Ok(())
}

fn discovery(public_base_url: String, mcp_base_url: String) -> Result<()> {
    let manifest = web_public::discovery_manifest(&public_base_url, &mcp_base_url);
    println!("{}", serde_json::to_string_pretty(&manifest)?);
    Ok(())
}

fn discovery_report(
    input_fixture: String,
    json_out: Option<String>,
    markdown_out: Option<String>,
) -> Result<()> {
    let report = build_discovery_report_from_path(Path::new(&input_fixture))?;
    let json = serde_json::to_string_pretty(&report)?;
    let markdown = render_discovery_report_markdown(&report);

    match json_out {
        Some(path) => write_report_file(Path::new(&path), &json)?,
        None => println!("{json}"),
    }
    if let Some(path) = markdown_out {
        write_report_file(Path::new(&path), &markdown)?;
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ContributorDiscoveryReport {
    total_records: usize,
    answered_records: usize,
    partial_answer_records: usize,
    missing_answer_records: usize,
    unique_contributors: usize,
    duplicate_contributors: Vec<String>,
    discovery_sources: Vec<ContributorDiscoveryReportBucket>,
    participation_reasons: Vec<ContributorDiscoveryReportBucket>,
    useful_labels: Vec<ContributorDiscoveryReportBucket>,
    trust_payment_signals: Vec<ContributorDiscoveryReportBucket>,
    friction_points: Vec<ContributorDiscoveryReportBucket>,
    agent_workflows: Vec<ContributorDiscoveryReportBucket>,
    records: Vec<ContributorDiscoveryReportRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ContributorDiscoveryReportBucket {
    name: String,
    count: usize,
    contributors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ContributorDiscoveryReportRecord {
    contributor: String,
    answered: bool,
    partial: bool,
    discovery_answer: Option<String>,
    participation_answer: Option<String>,
    discovery_sources: Vec<String>,
    participation_reasons: Vec<String>,
    useful_labels: Vec<String>,
    trust_payment_signals: Vec<String>,
    friction_points: Vec<String>,
    agent_workflow: Option<String>,
}

fn build_discovery_report_from_path(path: &Path) -> Result<ContributorDiscoveryReport> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("failed to read discovery report fixture {}", path.display()))?;
    build_discovery_report_from_str(&text)
}

fn build_discovery_report_from_str(text: &str) -> Result<ContributorDiscoveryReport> {
    let value: serde_json::Value =
        serde_json::from_str(text).context("discovery report fixture must be valid JSON")?;
    let raw_records = discovery_fixture_records(&value)?;
    if raw_records.is_empty() {
        bail!("discovery report fixture must contain at least one record");
    }

    let mut records = Vec::new();
    let mut contributor_counts: BTreeMap<String, usize> = BTreeMap::new();
    for (index, value) in raw_records.iter().enumerate() {
        let record = discovery_record_from_value(index, value);
        *contributor_counts
            .entry(record.contributor.clone())
            .or_default() += 1;
        records.push(record);
    }

    let duplicate_contributors = contributor_counts
        .iter()
        .filter(|(_, count)| **count > 1)
        .map(|(contributor, _)| contributor.clone())
        .collect::<Vec<_>>();
    let answered_records = records.iter().filter(|record| record.answered).count();
    let partial_answer_records = records.iter().filter(|record| record.partial).count();
    let missing_answer_records = records.iter().filter(|record| !record.answered).count();

    Ok(ContributorDiscoveryReport {
        total_records: records.len(),
        answered_records,
        partial_answer_records,
        missing_answer_records,
        unique_contributors: contributor_counts.len(),
        duplicate_contributors,
        discovery_sources: discovery_buckets(&records, |record| &record.discovery_sources),
        participation_reasons: discovery_buckets(&records, |record| &record.participation_reasons),
        useful_labels: discovery_buckets(&records, |record| &record.useful_labels),
        trust_payment_signals: discovery_buckets(&records, |record| &record.trust_payment_signals),
        friction_points: discovery_buckets(&records, |record| &record.friction_points),
        agent_workflows: discovery_buckets_option(&records, |record| {
            record.agent_workflow.as_ref()
        }),
        records,
    })
}

fn discovery_fixture_records(value: &serde_json::Value) -> Result<Vec<serde_json::Value>> {
    if let Some(records) = value.get("records").and_then(serde_json::Value::as_array) {
        return Ok(records.clone());
    }
    if let Some(records) = value.as_array() {
        return Ok(records.clone());
    }
    bail!("discovery report fixture must be an array or an object with records[]")
}

fn discovery_record_from_value(
    index: usize,
    value: &serde_json::Value,
) -> ContributorDiscoveryReportRecord {
    let body = first_string_field(value, &["body", "comment", "text"]).unwrap_or_default();
    let contributor = contributor_from_value(index, value, &body);
    let discovery_answer = first_string_field(
        value,
        &[
            "discovery_source",
            "source",
            "how_found",
            "how_did_you_find",
        ],
    )
    .or_else(|| extract_answer_after_marker(&body, &["how did you find", "how did you hear"]))
    .or_else(|| extract_found_through_answer(&body));
    let participation_answer = first_string_field(
        value,
        &[
            "participation_reason",
            "reason",
            "why_participated",
            "what_made_it_worth",
        ],
    )
    .or_else(|| {
        extract_answer_after_marker(
            &body,
            &[
                "what made this bounty",
                "what made this project",
                "worth participating",
                "why did you participate",
            ],
        )
    })
    .or_else(|| extract_because_answer(&body));
    let agent_workflow = first_string_field(
        value,
        &[
            "agent_workflow",
            "ai_agent_workflow",
            "workflow",
            "tool_prompt_link",
        ],
    )
    .or_else(|| {
        extract_answer_after_marker(
            &body,
            &[
                "if an ai agent helped",
                "what tool",
                "what prompt",
                "what workflow",
            ],
        )
    })
    .or_else(|| detect_agent_workflow(&body));

    let useful_labels = unique_sorted(
        structured_string_list(value, &["useful_labels", "labels"])
            .into_iter()
            .chain(detect_labels(&format!(
                "{} {} {}",
                body,
                discovery_answer.as_deref().unwrap_or_default(),
                participation_answer.as_deref().unwrap_or_default()
            )))
            .collect(),
    );
    let text = format!(
        "{} {} {} {}",
        body,
        discovery_answer.as_deref().unwrap_or_default(),
        participation_answer.as_deref().unwrap_or_default(),
        agent_workflow.as_deref().unwrap_or_default()
    );
    let structured_trust_payment_signals = structured_string_list(
        value,
        &[
            "trust_signal",
            "trust_signals",
            "payment_signal",
            "payment_signals",
        ],
    );
    let trust_text = format!("{} {}", text, structured_trust_payment_signals.join(" "));
    let trust_payment_signals = unique_sorted(
        structured_trust_payment_signals
            .into_iter()
            .chain(detect_trust_payment_signals(&trust_text))
            .collect(),
    );
    let structured_friction_points =
        structured_string_list(value, &["friction_point", "friction_points"]);
    let friction_text = format!("{} {}", text, structured_friction_points.join(" "));
    let friction_points = unique_sorted(
        structured_friction_points
            .into_iter()
            .chain(detect_friction_points(&friction_text))
            .collect(),
    );
    let discovery_sources = unique_sorted(
        discovery_answer
            .iter()
            .flat_map(|answer| classify_discovery_source(answer))
            .collect(),
    );
    let participation_reasons = unique_sorted(
        participation_answer
            .iter()
            .flat_map(|answer| classify_participation_reason(answer))
            .collect(),
    );
    let answered = discovery_answer.is_some() || participation_answer.is_some();
    let partial = answered && (discovery_answer.is_none() || participation_answer.is_none());

    ContributorDiscoveryReportRecord {
        contributor,
        answered,
        partial,
        discovery_answer,
        participation_answer,
        discovery_sources,
        participation_reasons,
        useful_labels,
        trust_payment_signals,
        friction_points,
        agent_workflow,
    }
}

fn contributor_from_value(index: usize, value: &serde_json::Value, body: &str) -> String {
    first_string_field(value, &["contributor", "login", "user", "author"])
        .or_else(|| {
            value
                .get("author")
                .and_then(|author| author.get("login"))
                .and_then(serde_json::Value::as_str)
                .map(ToString::to_string)
        })
        .or_else(|| {
            body.split_whitespace()
                .find(|token| token.starts_with('@') && token.len() > 1)
                .map(|token| {
                    token
                        .trim_matches(|ch: char| {
                            !(ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '@')
                        })
                        .trim_start_matches('@')
                        .to_string()
                })
        })
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| format!("unknown-{index}"))
}

fn first_string_field(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        let value = value.get(*key)?;
        match value {
            serde_json::Value::String(text) => clean_answer(text),
            serde_json::Value::Number(number) => Some(number.to_string()),
            serde_json::Value::Object(object) => object
                .get("login")
                .and_then(serde_json::Value::as_str)
                .and_then(clean_answer),
            _ => None,
        }
    })
}

fn structured_string_list(value: &serde_json::Value, keys: &[&str]) -> Vec<String> {
    let mut values = Vec::new();
    for key in keys {
        match value.get(*key) {
            Some(serde_json::Value::String(text)) => {
                values.extend(
                    split_listish(text)
                        .into_iter()
                        .filter_map(|item| clean_answer(&item)),
                );
            }
            Some(serde_json::Value::Array(items)) => {
                for item in items {
                    if let Some(text) = item.as_str().and_then(clean_answer) {
                        values.push(text);
                    }
                }
            }
            _ => {}
        }
    }
    unique_sorted(values)
}

fn extract_answer_after_marker(body: &str, markers: &[&str]) -> Option<String> {
    for line in body.lines() {
        let lower = line.to_ascii_lowercase();
        if markers.iter().any(|marker| lower.contains(marker)) {
            if let Some(answer) = line
                .split_once('?')
                .map(|(_, answer)| answer)
                .or_else(|| line.split_once(':').map(|(_, answer)| answer))
                .or_else(|| line.split_once(" - ").map(|(_, answer)| answer))
                .and_then(clean_answer)
            {
                return Some(answer);
            }
        }
    }
    None
}

fn extract_found_through_answer(body: &str) -> Option<String> {
    for marker in [
        "found agent bounties through",
        "found agent bounties manually by",
        "found agent bounties by",
        "found it through",
        "found this through",
        "found this project through",
        "found agent bounties via",
        "found it via",
        "found this via",
        "found this project via",
    ] {
        if let Some(answer) = substring_after_case_insensitive(body, marker).and_then(clean_answer)
        {
            return Some(answer);
        }
    }
    None
}

fn extract_because_answer(body: &str) -> Option<String> {
    for marker in [
        "participated because",
        "worth participating because",
        "worth participating in because",
        "worth trying because",
        "looked worth trying because",
        "joined because",
    ] {
        if let Some(answer) = substring_after_case_insensitive(body, marker).and_then(clean_answer)
        {
            return Some(answer);
        }
    }
    None
}

fn substring_after_case_insensitive<'a>(text: &'a str, marker: &str) -> Option<&'a str> {
    let lower = text.to_ascii_lowercase();
    let start = lower.find(marker)? + marker.len();
    Some(&text[start..])
}

fn clean_answer(text: &str) -> Option<String> {
    let cleaned = text
        .trim()
        .trim_start_matches(['-', ':', '=', ' '])
        .trim()
        .trim_matches(['.', ',', ';'])
        .trim();
    if cleaned.is_empty() || cleaned.eq_ignore_ascii_case("none") {
        None
    } else {
        Some(cleaned.to_string())
    }
}

fn split_listish(text: &str) -> Vec<String> {
    text.split([',', ';'])
        .flat_map(|chunk| chunk.split(" and "))
        .map(|chunk| chunk.trim().trim_matches(['`', '"', '\'']).to_string())
        .filter(|chunk| !chunk.is_empty())
        .collect()
}

fn classify_discovery_source(text: &str) -> Vec<String> {
    let lower = text.to_ascii_lowercase();
    let mut values = Vec::new();
    if contains_any(&lower, &["github", "issue", "pull request", "pr ", "repo"]) {
        values.push("github");
    }
    if contains_any(
        &lower,
        &[
            "bounty listing",
            "bounty listings",
            "twitter",
            "social",
            "x.com",
        ],
    ) {
        values.push("bounty-listing-or-social");
    }
    if contains_any(
        &lower,
        &["mcp", "llms.txt", "discovery manifest", ".well-known"],
    ) {
        values.push("machine-discovery");
    }
    if contains_any(
        &lower,
        &["proof page", "public proof", "reputation profile"],
    ) {
        values.push("proof-or-reputation-page");
    }
    if contains_any(
        &lower,
        &["codex", "claude", "chatgpt", "antigravity", "agent", "bot"],
    ) {
        values.push("ai-agent-workflow");
    }
    if contains_any(&lower, &["referral", "direct", "maintainer"]) {
        values.push("direct-referral");
    }
    if values.is_empty() {
        values.push("other");
    }
    values.into_iter().map(ToString::to_string).collect()
}

fn classify_participation_reason(text: &str) -> Vec<String> {
    let lower = text.to_ascii_lowercase();
    let mut values = Vec::new();
    if contains_any(
        &lower,
        &["usdc", "paid", "payout", "bounty", "reward", "amount"],
    ) {
        values.push("payout");
    }
    if contains_any(
        &lower,
        &[
            "clear",
            "concrete",
            "small",
            "scope",
            "acceptance criteria",
            "well scoped",
        ],
    ) {
        values.push("clear-scope");
    }
    if contains_any(
        &lower,
        &[
            "test",
            "deterministic",
            "local",
            "fixture",
            "docs-contract",
            "ci",
        ],
    ) {
        values.push("testability");
    }
    if contains_any(
        &lower,
        &[
            "escrow",
            "trust",
            "settlement",
            "payment rail",
            "operator reconciliation",
        ],
    ) {
        values.push("payment-trust");
    }
    if contains_any(&lower, &["proof", "reputation", "profile", "portfolio"]) {
        values.push("reputation-or-proof-graph");
    }
    if contains_any(&lower, &["agent", "autonomous", "ai workflow", "ai-agent"]) {
        values.push("agent-fit");
    }
    if contains_any(
        &lower,
        &["interesting", "technical", "architecture", "workflow"],
    ) {
        values.push("technical-interest");
    }
    if contains_any(&lower, &["useful", "mission", "platform", "open source"]) {
        values.push("project-mission");
    }
    if values.is_empty() {
        values.push("other");
    }
    values.into_iter().map(ToString::to_string).collect()
}

fn detect_labels(text: &str) -> Vec<String> {
    let lower = text.to_ascii_lowercase();
    [
        "bounty",
        "ai-agent-welcome",
        "good-first-agent-bounty",
        "payments",
        "distribution",
        "verifier",
        "good-first",
    ]
    .iter()
    .filter(|label| lower.contains(**label))
    .map(|label| (*label).to_string())
    .collect()
}

fn detect_trust_payment_signals(text: &str) -> Vec<String> {
    let lower = text.to_ascii_lowercase();
    let mut values = Vec::new();
    if contains_any(&lower, &["base", "usdc", "escrow"]) {
        values.push("base-usdc-escrow");
    }
    if lower.contains("stripe") {
        values.push("stripe-fiat");
    }
    if contains_any(
        &lower,
        &["deterministic", "test", "fixture", "docs-contract", "ci"],
    ) {
        values.push("deterministic-verification");
    }
    if contains_any(&lower, &["proof", "reputation", "template signal"]) {
        values.push("public-proof-graph");
    }
    if contains_any(
        &lower,
        &[
            "operator",
            "reconciliation",
            "not settlement",
            "payment boundary",
        ],
    ) {
        values.push("operator-reconciliation-boundary");
    }
    values.into_iter().map(ToString::to_string).collect()
}

fn detect_friction_points(text: &str) -> Vec<String> {
    let lower = text.to_ascii_lowercase();
    let mut values = Vec::new();
    if contains_any(
        &lower,
        &[
            "rust not installed",
            "missing rust",
            "cargo missing",
            "toolchain",
        ],
    ) {
        values.push("missing-toolchain");
    }
    if contains_any(
        &lower,
        &["stale", "rebase", "docs-contract", "contract issue"],
    ) {
        values.push("stale-docs-or-contract");
    }
    if contains_any(
        &lower,
        &[
            "unclear payout",
            "payment path",
            "settlement unclear",
            "funded, claimable",
            "claimable, and paid",
            "eligible for settlement",
        ],
    ) {
        values.push("unclear-payment-path");
    }
    if contains_any(&lower, &["review", "merge", "approval", "ci approval"]) {
        values.push("review-uncertainty");
    }
    if contains_any(&lower, &["wallet", "onboarding", "connect account"]) {
        values.push("wallet-or-onboarding");
    }
    values.into_iter().map(ToString::to_string).collect()
}

fn detect_agent_workflow(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    for tool in [
        "antigravity ai",
        "codex",
        "claude",
        "chatgpt",
        "bounty hunter",
        "mcp",
    ] {
        if lower.contains(tool) {
            return Some(tool.to_string());
        }
    }
    None
}

fn contains_any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| text.contains(needle))
}

fn discovery_buckets<F>(
    records: &[ContributorDiscoveryReportRecord],
    selector: F,
) -> Vec<ContributorDiscoveryReportBucket>
where
    F: Fn(&ContributorDiscoveryReportRecord) -> &Vec<String>,
{
    let mut buckets: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for record in records {
        for value in selector(record) {
            buckets
                .entry(value.clone())
                .or_default()
                .insert(record.contributor.clone());
        }
    }
    sorted_report_buckets(buckets)
}

fn discovery_buckets_option<F>(
    records: &[ContributorDiscoveryReportRecord],
    selector: F,
) -> Vec<ContributorDiscoveryReportBucket>
where
    F: Fn(&ContributorDiscoveryReportRecord) -> Option<&String>,
{
    let mut buckets: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for record in records {
        if let Some(value) = selector(record) {
            buckets
                .entry(value.clone())
                .or_default()
                .insert(record.contributor.clone());
        }
    }
    sorted_report_buckets(buckets)
}

fn sorted_report_buckets(
    buckets: BTreeMap<String, BTreeSet<String>>,
) -> Vec<ContributorDiscoveryReportBucket> {
    let mut values = buckets
        .into_iter()
        .map(|(name, contributors)| ContributorDiscoveryReportBucket {
            name,
            count: contributors.len(),
            contributors: contributors.into_iter().collect(),
        })
        .collect::<Vec<_>>();
    values.sort_by(|left, right| {
        right
            .count
            .cmp(&left.count)
            .then_with(|| left.name.cmp(&right.name))
    });
    values
}

fn unique_sorted(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .filter_map(|value| clean_answer(&value))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn render_discovery_report_markdown(report: &ContributorDiscoveryReport) -> String {
    format!(
        "# Contributor Discovery Report\n\n\
         - Total records: {}\n\
         - Answered records: {}\n\
         - Partial answer records: {}\n\
         - Missing answer records: {}\n\
         - Unique contributors: {}\n\
         - Duplicate contributors: {}\n\n\
         ## Discovery Sources\n{}\n\n\
         ## Participation Reasons\n{}\n\n\
         ## Useful Labels\n{}\n\n\
         ## Trust And Payment Signals\n{}\n\n\
         ## Friction Points\n{}\n\n\
         ## Agent Workflows\n{}\n",
        report.total_records,
        report.answered_records,
        report.partial_answer_records,
        report.missing_answer_records,
        report.unique_contributors,
        if report.duplicate_contributors.is_empty() {
            "none".to_string()
        } else {
            report.duplicate_contributors.join(", ")
        },
        render_bucket_markdown(&report.discovery_sources),
        render_bucket_markdown(&report.participation_reasons),
        render_bucket_markdown(&report.useful_labels),
        render_bucket_markdown(&report.trust_payment_signals),
        render_bucket_markdown(&report.friction_points),
        render_bucket_markdown(&report.agent_workflows),
    )
}

fn render_bucket_markdown(buckets: &[ContributorDiscoveryReportBucket]) -> String {
    if buckets.is_empty() {
        return "- None".to_string();
    }
    buckets
        .iter()
        .map(|bucket| {
            format!(
                "- {}: {} ({})",
                bucket.name,
                bucket.count,
                bucket.contributors.join(", ")
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn write_report_file(path: &Path, contents: &str) -> Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create output directory {}", parent.display()))?;
    }
    fs::write(path, contents).with_context(|| format!("failed to write {}", path.display()))
}

async fn production_smoke(
    api_base_url: String,
    mcp_base_url: String,
    require_eval_history: bool,
) -> Result<()> {
    let api = normalize_base_url(&api_base_url);
    let mcp = normalize_base_url(&mcp_base_url);
    let report = production_smoke_check(&api, &mcp, require_eval_history).await?;
    print_production_smoke_report(&report)
}

#[derive(Debug, Clone)]
struct ProductionSmokeReport {
    api_base_url: String,
    mcp_base_url: String,
    templates: usize,
    payment_rails: usize,
    trust_tiers: usize,
    proof_surfaces: usize,
    bounty_feed_items: usize,
    capability_feed_items: usize,
    eval_runs: usize,
    risk_events: usize,
    risk_reviews: usize,
    mcp_tools: usize,
    require_eval_history: bool,
}

async fn production_smoke_check(
    api: &str,
    mcp: &str,
    require_eval_history: bool,
) -> Result<ProductionSmokeReport> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()?;

    let api_health = production_get_text(&client, &format!("{api}/health")).await?;
    require(
        api_health.trim() == "ok",
        "API health endpoint must return ok",
    )?;
    let mcp_health = production_get_text(&client, &format!("{mcp}/health")).await?;
    require(
        mcp_health.trim() == "ok",
        "MCP health endpoint must return ok",
    )?;

    let discovery =
        production_get_json(&client, &format!("{api}/.well-known/agent-bounties.json")).await?;
    require(
        discovery
            .pointer("/open_source")
            .and_then(|value| value.as_bool())
            == Some(true),
        "discovery manifest must advertise open_source=true",
    )?;
    require(
        value_str(&discovery, "/schema")
            .map(|schema| schema.ends_with("discovery-manifest.v1.json"))
            .unwrap_or(false),
        "discovery manifest must expose the v1 schema",
    )?;
    require(
        value_str(&discovery, "/endpoints/api_base") == Some(api),
        "discovery manifest api_base must match the checked API URL",
    )?;
    require(
        value_str(&discovery, "/endpoints/mcp_tools")
            .map(|url| url.starts_with(mcp))
            .unwrap_or(false),
        "discovery manifest must point MCP tools at the checked MCP URL",
    )?;
    for endpoint in [
        "/endpoints/openapi_json",
        "/endpoints/swagger_ui",
        "/endpoints/discovery",
        "/endpoints/discovery_schema",
        "/endpoints/llms_txt",
        "/endpoints/agent_quickstart",
        "/endpoints/public_bounties",
        "/endpoints/public_bounty",
        "/endpoints/templates",
        "/endpoints/bounty_feed",
        "/endpoints/capability_feed",
        "/endpoints/eval_runs",
        "/endpoints/risk_policy",
        "/endpoints/live_money_readiness",
        "/endpoints/base_indexer_status",
        "/endpoints/risk_events",
        "/endpoints/risk_reviews",
        "/endpoints/base_escrow_events",
        "/endpoints/base_release_queue",
        "/endpoints/base_funding_plan",
        "/endpoints/risk_payout_approvals",
        "/endpoints/base_broadcast_signed_transaction",
        "/endpoints/base_transaction_receipt",
        "/endpoints/stripe_live_checkout_top_ups",
        "/endpoints/stripe_live_funding_intent_checkouts",
        "/endpoints/stripe_live_connect_accounts",
        "/endpoints/github_issue_bounty_plan",
        "/endpoints/github_claim_comment_plan",
        "/endpoints/github_proof_comment_plan",
        "/endpoints/github_proof_comment_from_proof_plan",
    ] {
        require(
            value_str(&discovery, endpoint).is_some(),
            &format!("discovery manifest missing {endpoint}"),
        )?;
    }
    let discovery_schema_url = value_str(&discovery, "/endpoints/discovery_schema")
        .context("discovery schema url missing")?;
    require(
        discovery_schema_url.starts_with(api),
        "discovery schema endpoint must be hosted by the checked API URL",
    )?;
    let discovery_schema = production_get_json(&client, discovery_schema_url).await?;
    require(
        value_str(&discovery_schema, "/$id") == value_str(&discovery, "/schema"),
        "discovery schema $id must match manifest schema id",
    )?;
    require(
        discovery_schema
            .pointer("/required")
            .and_then(|value| value.as_array())
            .map(|required| {
                required
                    .iter()
                    .any(|value| value.as_str() == Some("agent_entrypoints"))
                    && required
                        .iter()
                        .any(|value| value.as_str() == Some("payment_rails"))
                    && required
                        .iter()
                        .any(|value| value.as_str() == Some("funding_handoff"))
            })
            .unwrap_or(false),
        "discovery schema must require agent entrypoints, payment rails, and funding handoff",
    )?;
    require(
        discovery_schema
            .pointer("/properties/endpoints/required")
            .and_then(|value| value.as_array())
            .map(|required| {
                required
                    .iter()
                    .any(|value| value.as_str() == Some("github_issue_template"))
                    && required
                        .iter()
                        .any(|value| value.as_str() == Some("agent_quickstart"))
                    && required
                        .iter()
                        .any(|value| value.as_str() == Some("public_bounties"))
                    && required
                        .iter()
                        .any(|value| value.as_str() == Some("public_bounty"))
                    && required
                        .iter()
                        .any(|value| value.as_str() == Some("discovery_schema"))
                    && required
                        .iter()
                        .any(|value| value.as_str() == Some("base_escrow_events"))
                    && required
                        .iter()
                        .any(|value| value.as_str() == Some("base_indexer_status"))
                    && required
                        .iter()
                        .any(|value| value.as_str() == Some("live_money_readiness"))
                    && required
                        .iter()
                        .any(|value| value.as_str() == Some("github_claim_comment_plan"))
                    && required
                        .iter()
                        .any(|value| value.as_str() == Some("github_proof_comment_from_proof_plan"))
            })
            .unwrap_or(false),
        "discovery schema must require distribution endpoints",
    )?;

    for entrypoint in [
        "route_blocked_goal",
        "list_claimable_bounties",
        "search_capabilities",
        "claim_bounty",
        "get_paid_status",
        "plan_base_funding",
        "reconcile_base_escrow_event",
        "list_base_release_queue",
        "check_live_money_readiness",
        "check_base_indexer_status",
    ] {
        require(
            array_contains_name(&discovery, "/agent_entrypoints", entrypoint),
            &format!("discovery manifest missing agent entrypoint {entrypoint}"),
        )?;
    }

    let templates = discovery
        .pointer("/templates")
        .and_then(|value| value.as_array())
        .context("discovery manifest templates must be an array")?;
    require(
        templates.len() >= 6,
        "discovery manifest must advertise reusable bounty templates",
    )?;
    require(
        templates.iter().any(|template| {
            value_str(template, "/slug") == Some("fix-ci-failure")
                && value_str(template, "/verifier").is_some()
        }),
        "discovery manifest must include the fix-ci-failure template",
    )?;

    let payment_rails = discovery
        .pointer("/payment_rails")
        .and_then(|value| value.as_array())
        .context("discovery manifest payment_rails must be an array")?;
    for rail in [
        "Base Sepolia USDC escrow",
        "Hosted low-value Base USDC",
        "Stripe fiat ledger",
    ] {
        require(
            payment_rails
                .iter()
                .any(|value| value_str(value, "/name") == Some(rail)),
            &format!("discovery manifest missing payment rail {rail}"),
        )?;
    }
    require(
        payment_rails.iter().all(|rail| {
            rail.pointer("/funding_required_before_claim")
                .and_then(|value| value.as_bool())
                == Some(true)
        }),
        "all advertised payment rails must require funding before claim",
    )?;
    let funding_handoff = discovery
        .pointer("/funding_handoff")
        .and_then(|value| value.as_object())
        .context("discovery manifest must expose a funding_handoff object")?;
    require(
        funding_handoff.get("page").and_then(|value| value.as_str())
            == Some(STATIC_FUNDING_PAGE_URL),
        "funding handoff must point to the static public funding page",
    )?;
    require(
        funding_handoff
            .get("supported_rail")
            .and_then(|value| value.as_str())
            == Some("StripeFiat"),
        "funding handoff must advertise StripeFiat support",
    )?;
    let funding_handoff_params = funding_handoff
        .get("query_params")
        .and_then(|value| value.as_array())
        .context("funding handoff must expose query_params")?;
    for param in [
        "apiBaseUrl",
        "bountyId",
        "amountMinor",
        "currency",
        "rail",
        "source",
        "externalReference",
        "paymentPreference",
    ] {
        require(
            funding_handoff_params
                .iter()
                .any(|value| value.as_str() == Some(param)),
            &format!("funding handoff missing query parameter {param}"),
        )?;
    }
    require(
        funding_handoff
            .get("settlement_authority")
            .and_then(|value| value.as_str())
            .map(|authority| authority.contains("verified Stripe webhook"))
            .unwrap_or(false),
        "funding handoff must keep verified Stripe webhook reconciliation as settlement authority",
    )?;

    let trust_tiers = discovery
        .pointer("/trust_tiers")
        .and_then(|value| value.as_array())
        .context("discovery manifest trust_tiers must be an array")?;
    for tier in ["sandbox", "testnet", "low-value-usdc", "fiat"] {
        require(
            trust_tiers
                .iter()
                .any(|value| value_str(value, "/name") == Some(tier)),
            &format!("discovery manifest missing trust tier {tier}"),
        )?;
    }
    let proof_surfaces = discovery
        .pointer("/proof_surfaces")
        .and_then(|value| value.as_array())
        .context("discovery manifest proof_surfaces must be an array")?;
    require(
        proof_surfaces.len() >= 5,
        "discovery manifest must advertise proof/profile/template surfaces",
    )?;

    let api_llms = production_get_text(&client, &format!("{api}/llms.txt")).await?;
    for expected in [
        "/.well-known/agent-bounties.json",
        "route_blocked_goal",
        "get_paid_status",
        "Base escrow event reconciliation",
        "Risk policy",
        "Live-money readiness",
        "AI judges",
        "Stripe live execution is gated",
        "Proof-record comment planner",
    ] {
        require(
            api_llms.contains(expected),
            &format!("API llms.txt missing {expected}"),
        )?;
    }
    let mcp_llms = production_get_text(&client, &format!("{mcp}/llms.txt")).await?;
    require(
        mcp_llms.contains("MCP tools") && mcp_llms.contains("route_blocked_goal"),
        "MCP llms.txt must orient agents to MCP tools",
    )?;
    let mcp_schema = production_get_json(
        &client,
        &format!("{mcp}/schemas/discovery-manifest.v1.json"),
    )
    .await?;
    require(
        value_str(&mcp_schema, "/$id") == value_str(&discovery_schema, "/$id"),
        "MCP discovery schema endpoint must serve the same manifest schema",
    )?;

    let openapi_url =
        value_str(&discovery, "/endpoints/openapi_json").context("openapi url missing")?;
    let openapi = production_get_json(&client, openapi_url).await?;
    let paths = openapi
        .pointer("/paths")
        .and_then(|value| value.as_object())
        .context("OpenAPI must include paths")?;
    for path in [
        "/v1/route-blocked-goal",
        "/v1/bounties/feed",
        "/v1/capabilities/feed",
        "/v1/evals/runs",
        "/v1/risk/policy",
        "/v1/readiness/live-money",
        "/v1/risk/events",
        "/v1/risk/reviews",
        "/v1/risk/bounty-approvals",
        "/v1/risk/payout-approvals",
        "/v1/risk/events/{id}/reject",
        "/v1/base/escrow-events",
        "/v1/base/evm-logs",
        "/v1/base/rpc-logs",
        "/v1/base/fetch-rpc-logs",
        "/v1/base/broadcast-signed-transaction",
        "/v1/base/transaction-receipt",
        "/v1/stripe/live/checkout-top-ups",
        "/v1/stripe/live/funding-intents/{id}/checkout-session",
        "/v1/stripe/live/connect-accounts",
        "/v1/stripe/live/connect-transfers",
        "/v1/stripe/connect-transfers",
        "/v1/stripe/connect-snapshots",
        "/v1/stripe/transfer-events",
        "/v1/stripe/checkout-webhooks",
        "/v1/github/funding-comment-plan",
        "/v1/github/proof-comment-plan-from-proof",
    ] {
        require(
            paths.contains_key(path),
            &format!("OpenAPI missing production path {path}"),
        )?;
    }
    let security_schemes = openapi
        .pointer("/components/securitySchemes")
        .and_then(|value| value.as_object())
        .context("OpenAPI must include operator security schemes")?;
    require(
        security_schemes
            .get("operator_api_token")
            .and_then(|scheme| value_str(scheme, "/name"))
            == Some("x-operator-token"),
        "OpenAPI operator_api_token scheme must use x-operator-token header",
    )?;
    require(
        security_schemes
            .get("operator_bearer")
            .and_then(|scheme| value_str(scheme, "/scheme"))
            == Some("bearer"),
        "OpenAPI operator_bearer scheme must use bearer auth",
    )?;
    for path in [
        "/v1/risk/bounty-approvals",
        "/v1/risk/payout-approvals",
        "/v1/risk/events/{id}/reject",
        "/v1/base/escrow-events",
        "/v1/base/evm-logs",
        "/v1/base/rpc-logs",
        "/v1/base/fetch-rpc-logs",
        "/v1/base/broadcast-signed-transaction",
        "/v1/stripe/live/checkout-top-ups",
        "/v1/stripe/live/connect-accounts",
        "/v1/stripe/live/connect-transfers",
        "/v1/stripe/connect-snapshots",
    ] {
        let operation = paths
            .get(path)
            .and_then(|path_item| path_item.get("post"))
            .with_context(|| format!("OpenAPI missing POST operation for {path}"))?;
        let security = operation
            .get("security")
            .and_then(|value| value.as_array())
            .with_context(|| format!("OpenAPI {path} must advertise operator security"))?;
        require(
            security
                .iter()
                .any(|requirement| requirement.get("operator_api_token").is_some()),
            &format!("OpenAPI {path} missing operator_api_token security"),
        )?;
        require(
            security
                .iter()
                .any(|requirement| requirement.get("operator_bearer").is_some()),
            &format!("OpenAPI {path} missing operator_bearer security"),
        )?;
        require(
            operation.pointer("/responses/401").is_some(),
            &format!("OpenAPI {path} must document 401 operator auth responses"),
        )?;
    }
    let public_checkout_operation = paths
        .get("/v1/stripe/live/funding-intents/{id}/checkout-session")
        .and_then(|path_item| path_item.get("post"))
        .context("OpenAPI missing POST operation for public funding-intent Checkout")?;
    require(
        public_checkout_operation.get("security").is_none(),
        "OpenAPI public funding-intent Checkout must not require operator security",
    )?;
    let receipt_operation = paths
        .get("/v1/base/transaction-receipt")
        .and_then(|path_item| path_item.get("post"))
        .context("OpenAPI missing POST operation for transaction receipt")?;
    let receipt_security = receipt_operation
        .get("security")
        .and_then(|value| value.as_array())
        .context("OpenAPI transaction receipt must advertise optional operator security")?;
    require(
        receipt_security.iter().any(|requirement| {
            requirement
                .as_object()
                .is_some_and(|object| object.is_empty())
        }),
        "OpenAPI transaction receipt must allow unauthenticated receipt reads",
    )?;
    require(
        receipt_security
            .iter()
            .any(|requirement| requirement.get("operator_api_token").is_some()),
        "OpenAPI transaction receipt must advertise operator auth for log reconciliation",
    )?;
    require(
        receipt_operation.pointer("/responses/401").is_some(),
        "OpenAPI transaction receipt must document conditional 401 responses",
    )?;
    let stripe_webhook_operation = paths
        .get("/v1/stripe/checkout-webhooks")
        .and_then(|path_item| path_item.get("post"))
        .context("OpenAPI missing POST operation for Stripe checkout webhook")?;
    require(
        stripe_webhook_operation.get("security").is_none(),
        "OpenAPI Stripe checkout webhook must remain unauthenticated for Stripe delivery",
    )?;
    require(
        stripe_webhook_operation.pointer("/responses/503").is_some(),
        "OpenAPI Stripe checkout webhook must document missing verification config",
    )?;

    let risk_policy_url =
        value_str(&discovery, "/endpoints/risk_policy").context("risk policy url missing")?;
    let risk_policy = production_get_json(&client, risk_policy_url).await?;
    require(
        risk_policy.pointer("/low_value_usdc_cap_minor") == Some(&serde_json::json!(10_000_000)),
        "risk policy must expose the low-value Base USDC cap",
    )?;
    require(
        risk_policy.pointer("/ai_judges_can_authorize_payment") == Some(&serde_json::json!(false)),
        "risk policy must state that AI judges cannot authorize payment",
    )?;
    require(
        risk_policy
            .pointer("/settlement_invariants")
            .and_then(|value| value.as_array())
            .map(|rules| {
                rules.iter().any(|rule| {
                    rule.as_str()
                        .map(|text| text.contains("indexed escrow logs"))
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false),
        "risk policy must expose indexed escrow log settlement invariant",
    )?;
    let live_money_readiness_url = value_str(&discovery, "/endpoints/live_money_readiness")
        .context("live-money readiness url missing")?;
    let live_money_readiness = production_get_json(
        &client,
        &format!("{live_money_readiness_url}?network=base-mainnet"),
    )
    .await?;
    require(
        live_money_readiness.pointer("/network_chain_id") == Some(&serde_json::json!(8_453)),
        "live-money readiness must default to Base mainnet checks when requested",
    )?;
    require(
        live_money_readiness
            .pointer("/network_native_usdc_token_address")
            .and_then(|value| value.as_str())
            .map(|token| token.eq_ignore_ascii_case("0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913"))
            .unwrap_or(false),
        "live-money readiness must expose the native Base USDC address",
    )?;
    require(
        live_money_readiness
            .pointer("/live_money_ready")
            .and_then(|value| value.as_bool())
            .is_some(),
        "live-money readiness must expose a boolean live_money_ready gate",
    )?;
    let stripe_secret_key_mode = value_str(&live_money_readiness, "/stripe_secret_key_mode")
        .context("live-money readiness must expose only Stripe key mode")?;
    require(
        !stripe_secret_key_mode.starts_with("sk_") && !stripe_secret_key_mode.starts_with("rk_"),
        "live-money readiness must not expose Stripe secret material",
    )?;
    require(
        live_money_readiness
            .pointer("/stripe_payment_method_configuration_configured")
            .and_then(|value| value.as_bool())
            .is_some(),
        "live-money readiness must expose a non-secret Stripe payment-method configuration boolean",
    )?;
    require(
        live_money_readiness
            .pointer("/evidence_boundaries")
            .and_then(|value| value.as_array())
            .map(|boundaries| {
                boundaries.iter().any(|boundary| {
                    boundary
                        .as_str()
                        .map(|text| text.contains("verified checkout.session.completed webhook"))
                        .unwrap_or(false)
                }) && boundaries.iter().any(|boundary| {
                    boundary
                        .as_str()
                        .map(|text| text.contains("indexed EscrowReleased log"))
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false),
        "live-money readiness must publish Stripe and Base settlement evidence boundaries",
    )?;
    let base_indexer_status_url = value_str(&discovery, "/endpoints/base_indexer_status")
        .context("Base indexer status url missing")?;
    let base_indexer_status = production_get_json(
        &client,
        &format!("{base_indexer_status_url}?network=base-mainnet"),
    )
    .await?;
    require(
        base_indexer_status.pointer("/network_chain_id") == Some(&serde_json::json!(8_453)),
        "Base indexer status must expose Base mainnet chain id",
    )?;
    require(
        base_indexer_status
            .pointer("/indexer_ready")
            .and_then(|value| value.as_bool())
            .is_some(),
        "Base indexer status must expose an indexer_ready boolean",
    )?;
    require_base_indexer_status_contract(&base_indexer_status, "Base indexer status")?;

    let bounty_feed_url =
        value_str(&discovery, "/endpoints/bounty_feed").context("bounty feed url missing")?;
    let bounty_feed = production_get_json(&client, bounty_feed_url).await?;
    let bounty_feed_items = bounty_feed
        .as_array()
        .context("public bounty feed must be an array")?
        .len();
    let capability_feed_url = value_str(&discovery, "/endpoints/capability_feed")
        .context("capability feed url missing")?;
    let capability_feed = production_get_json(&client, capability_feed_url).await?;
    let capability_feed_items = capability_feed
        .as_array()
        .context("public capability feed must be an array")?
        .len();
    let eval_runs_url =
        value_str(&discovery, "/endpoints/eval_runs").context("eval runs url missing")?;
    let eval_runs = production_get_json(&client, eval_runs_url).await?;
    let eval_run_count = eval_runs
        .as_array()
        .context("eval run history must be an array")?
        .len();
    if require_eval_history {
        require(
            eval_run_count > 0,
            "production smoke requires at least one persisted eval run",
        )?;
    }
    let risk_events_url =
        value_str(&discovery, "/endpoints/risk_events").context("risk events url missing")?;
    let risk_events = production_get_json(&client, risk_events_url).await?;
    let risk_event_count = risk_events
        .as_array()
        .context("risk events must be an array")?
        .len();
    let risk_reviews_url =
        value_str(&discovery, "/endpoints/risk_reviews").context("risk reviews url missing")?;
    let risk_reviews = production_get_json(&client, risk_reviews_url).await?;
    let risk_review_count = risk_reviews
        .as_array()
        .context("risk reviews must be an array")?
        .len();

    let template_index_url =
        value_str(&discovery, "/endpoints/templates").context("templates url missing")?;
    let template_index = production_get_text(&client, template_index_url).await?;
    require(
        template_index.contains("Agent Bounty Templates")
            && template_index.contains("fix-ci-failure"),
        "public template index must render reusable templates",
    )?;
    let template_page =
        production_get_text(&client, &format!("{api}/public/templates/fix-ci-failure")).await?;
    require(
        template_page.contains("Fix CI Failure")
            && template_page.contains("Verifier")
            && template_page.contains(
                "https://github.com/NSPG13/agent-bounties/issues/new?template=paid-bounty.yml",
            ),
        "public template page must render verifier details and the paid-bounty issue CTA",
    )?;
    let verifier_page =
        production_get_text(&client, &format!("{api}/public/verifiers/JsonSchema")).await?;
    require(
        verifier_page.contains("JsonSchema Verifier") && verifier_page.contains("Browse templates"),
        "public verifier page must render verifier profile",
    )?;
    let public_bounties = production_get_text(&client, &format!("{api}/public/bounties")).await?;
    require(
        public_bounties.contains("Claimable Agent Bounties")
            && public_bounties.contains("Machine-readable feed")
            && public_bounties.contains("Add funding"),
        "public bounty page must point agents at the machine-readable feed",
    )?;
    let public_capabilities =
        production_get_text(&client, &format!("{api}/public/capabilities")).await?;
    require(
        public_capabilities.contains("Agent Capability Directory")
            && public_capabilities.contains("Machine-readable feed"),
        "public capability page must point agents at the machine-readable feed",
    )?;

    let mcp_discovery =
        production_get_json(&client, &format!("{mcp}/.well-known/agent-bounties.json")).await?;
    require(
        mcp_discovery.pointer("/agent_entrypoints").is_some(),
        "MCP discovery manifest must expose agent entrypoints",
    )?;
    let mcp_tools_url =
        value_str(&discovery, "/endpoints/mcp_tools").context("MCP tools url missing")?;
    let tools = production_get_json(&client, mcp_tools_url).await?;
    let tool_list = tools.as_array().context("MCP tools must be an array")?;
    require(
        tool_list.len() >= 40,
        "MCP tools should expose the full agent bounty surface",
    )?;
    for tool in tool_list {
        let name = value_str(tool, "/name").unwrap_or("<unnamed>");
        require(
            tool.pointer("/input_schema/type").is_some(),
            &format!("MCP tool {name} missing input_schema.type"),
        )?;
    }
    for expected in [
        "route_blocked_goal",
        "request_quotes",
        "post_bounty",
        "create_funding_intent",
        "claim_bounty",
        "submit_result",
        "request_verification",
        "get_paid_status",
        "plan_base_funding",
        "list_base_release_queue",
        "get_live_money_readiness",
        "get_base_indexer_status",
        "plan_stripe_connect_transfer",
        "execute_stripe_checkout_top_up",
        "execute_stripe_connect_account",
        "execute_stripe_connect_transfer",
        "list_risk_events",
        "list_risk_reviews",
        "approve_risk_bounty",
        "approve_risk_payout",
        "reject_risk_event",
        "plan_github_funding_comment",
        "plan_github_claim_comment",
        "plan_github_proof_comment_for_proof",
    ] {
        require(
            tool_list
                .iter()
                .any(|tool| value_str(tool, "/name") == Some(expected)),
            &format!("MCP tool list missing {expected}"),
        )?;
    }
    for expected in [
        "execute_stripe_checkout_top_up",
        "execute_stripe_connect_account",
        "execute_stripe_connect_transfer",
        "reconcile_stripe_connect_snapshot",
        "reconcile_stripe_transfer_event",
        "reconcile_stripe_checkout_webhook",
        "reconcile_base_escrow_event",
        "reconcile_base_evm_logs",
        "reconcile_base_rpc_logs",
        "fetch_base_rpc_logs",
        "broadcast_base_signed_transaction",
        "get_base_transaction_receipt",
        "approve_risk_bounty",
        "approve_risk_payout",
        "reject_risk_event",
    ] {
        let tool = tool_list
            .iter()
            .find(|tool| value_str(tool, "/name") == Some(expected))
            .with_context(|| format!("MCP tool list missing {expected}"))?;
        require(
            value_str(tool, "/authorization/kind") == Some("operator_api_token"),
            &format!("MCP tool {expected} missing operator auth kind"),
        )?;
        require(
            value_str(tool, "/authorization/header") == Some("x-operator-token"),
            &format!("MCP tool {expected} missing x-operator-token auth header"),
        )?;
        require(
            tool.pointer("/authorization/bearer")
                .and_then(|value| value.as_bool())
                == Some(true),
            &format!("MCP tool {expected} must advertise Bearer token support"),
        )?;
    }

    Ok(ProductionSmokeReport {
        api_base_url: api.to_string(),
        mcp_base_url: mcp.to_string(),
        templates: templates.len(),
        payment_rails: payment_rails.len(),
        trust_tiers: trust_tiers.len(),
        proof_surfaces: proof_surfaces.len(),
        bounty_feed_items,
        capability_feed_items,
        eval_runs: eval_run_count,
        risk_events: risk_event_count,
        risk_reviews: risk_review_count,
        mcp_tools: tool_list.len(),
        require_eval_history,
    })
}

fn print_production_smoke_report(report: &ProductionSmokeReport) -> Result<()> {
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "production_smoke": "ok",
            "api_base_url": report.api_base_url,
            "mcp_base_url": report.mcp_base_url,
            "templates": report.templates,
            "payment_rails": report.payment_rails,
            "trust_tiers": report.trust_tiers,
            "proof_surfaces": report.proof_surfaces,
            "bounty_feed_items": report.bounty_feed_items,
            "capability_feed_items": report.capability_feed_items,
            "eval_runs": report.eval_runs,
            "risk_events": report.risk_events,
            "risk_reviews": report.risk_reviews,
            "mcp_tools": report.mcp_tools,
            "require_eval_history": report.require_eval_history
        }))?
    );
    Ok(())
}

async fn service_smoke(api_base_url: String, mcp_base_url: String) -> Result<()> {
    let api = normalize_base_url(&api_base_url);
    let mcp = normalize_base_url(&mcp_base_url);
    let report = service_smoke_check(&api, &mcp).await?;
    print_service_smoke_report(&report)
}

#[derive(Debug, Clone)]
struct ServiceSmokeReport {
    api_base_url: String,
    mcp_base_url: String,
    bounty_id: String,
    feed_items: usize,
    mcp_tools: usize,
    pooled_bounty_id: String,
    mcp_reviewed_bounty_id: String,
    mcp_bounty_id: String,
    mcp_solver_id: String,
    mcp_final_status: String,
}

async fn service_smoke_check(api: &str, mcp: &str) -> Result<ServiceSmokeReport> {
    wait_for_health(&format!("{api}/health"))?;
    wait_for_health(&format!("{mcp}/health"))?;
    let _production_contract = production_smoke_check(api, mcp, false).await?;

    let discovery = get_json(&format!("{api}/.well-known/agent-bounties.json"))?;
    require(
        discovery.pointer("/endpoints/bounty_feed").is_some(),
        "discovery manifest must include bounty feed",
    )?;
    require(
        discovery.pointer("/endpoints/llms_txt").is_some(),
        "discovery manifest must include llms.txt",
    )?;
    require(
        discovery.pointer("/endpoints/agent_quickstart").is_some(),
        "discovery manifest must include agent quickstart",
    )?;
    require(
        discovery.pointer("/endpoints/public_bounties").is_some(),
        "discovery manifest must include public bounty pages",
    )?;
    require(
        discovery.pointer("/endpoints/public_bounty").is_some(),
        "discovery manifest must include public bounty detail route",
    )?;
    require(
        discovery.pointer("/endpoints/capability_feed").is_some(),
        "discovery manifest must include capability feed",
    )?;
    require(
        discovery.pointer("/endpoints/risk_events").is_some(),
        "discovery manifest must include risk review events endpoint",
    )?;
    require(
        discovery
            .pointer("/endpoints/live_money_readiness")
            .is_some(),
        "discovery manifest must include live-money readiness endpoint",
    )?;
    require(
        discovery
            .pointer("/endpoints/base_indexer_status")
            .is_some(),
        "discovery manifest must include Base indexer status endpoint",
    )?;
    require(
        discovery.pointer("/endpoints/risk_reviews").is_some(),
        "discovery manifest must include risk review records endpoint",
    )?;
    require(
        discovery
            .pointer("/endpoints/risk_bounty_approvals")
            .is_some(),
        "discovery manifest must include risk bounty approval endpoint",
    )?;
    require(
        discovery
            .pointer("/endpoints/risk_payout_approvals")
            .is_some(),
        "discovery manifest must include risk payout approval endpoint",
    )?;
    require(
        discovery
            .pointer("/endpoints/risk_event_rejections")
            .is_some(),
        "discovery manifest must include risk event rejection endpoint",
    )?;
    require(
        discovery.pointer("/endpoints/base_release_queue").is_some(),
        "discovery manifest must include Base release queue",
    )?;
    require(
        discovery.pointer("/endpoints/base_funding_plan").is_some(),
        "discovery manifest must include Base funding planning",
    )?;
    require(
        discovery.pointer("/endpoints/base_refund_plan").is_some(),
        "discovery manifest must include Base refund planning",
    )?;
    require(
        discovery.pointer("/endpoints/base_dispute_plan").is_some(),
        "discovery manifest must include Base dispute planning",
    )?;
    require(
        discovery.pointer("/endpoints/base_log_query").is_some(),
        "discovery manifest must include Base log query",
    )?;
    require(
        discovery.pointer("/endpoints/base_escrow_events").is_some(),
        "discovery manifest must include normalized Base escrow event reconciliation",
    )?;
    require(
        discovery.pointer("/endpoints/base_rpc_logs").is_some(),
        "discovery manifest must include Base RPC log ingestion",
    )?;
    require(
        discovery
            .pointer("/endpoints/base_fetch_rpc_logs")
            .is_some(),
        "discovery manifest must include configured Base RPC log fetching",
    )?;
    require(
        discovery
            .pointer("/endpoints/base_broadcast_signed_transaction")
            .is_some(),
        "discovery manifest must include Base signed transaction broadcast",
    )?;
    require(
        discovery
            .pointer("/endpoints/base_transaction_receipt")
            .is_some(),
        "discovery manifest must include Base transaction receipt polling",
    )?;
    require(
        discovery
            .pointer("/endpoints/stripe_live_checkout_top_ups")
            .is_some(),
        "discovery manifest must include live Stripe Checkout execution",
    )?;
    require(
        discovery
            .pointer("/endpoints/stripe_live_funding_intent_checkouts")
            .is_some(),
        "discovery manifest must include live Stripe funding-intent Checkout execution",
    )?;
    require(
        discovery
            .pointer("/endpoints/stripe_live_connect_accounts")
            .is_some(),
        "discovery manifest must include live Stripe Connect execution",
    )?;
    require(
        discovery
            .pointer("/endpoints/github_issue_bounty_plan")
            .is_some(),
        "discovery manifest must include GitHub issue bounty planning",
    )?;
    require(
        discovery
            .pointer("/endpoints/github_funding_comment_plan")
            .is_some(),
        "discovery manifest must include GitHub funding comment planning",
    )?;
    require(
        discovery
            .pointer("/endpoints/github_claim_comment_plan")
            .is_some(),
        "discovery manifest must include GitHub claim comment planning",
    )?;
    require(
        discovery
            .pointer("/endpoints/github_proof_comment_plan")
            .is_some(),
        "discovery manifest must include GitHub proof comment planning",
    )?;
    require(
        discovery
            .pointer("/endpoints/github_proof_comment_from_proof_plan")
            .is_some(),
        "discovery manifest must include proof-record GitHub proof comment planning",
    )?;
    let api_llms = get_text(&format!("{api}/llms.txt"))?;
    require(
        api_llms.contains("route_blocked_goal")
            && api_llms.contains("/.well-known/agent-bounties.json"),
        "API llms.txt must orient agents to discovery and routing",
    )?;
    require(
        api_llms.contains("docs/agent-quickstart.md"),
        "API llms.txt must point agents to the quickstart",
    )?;
    require(
        api_llms.contains("Proof-record comment planner"),
        "API llms.txt must orient agents to proof-record GitHub comments",
    )?;
    let mcp_llms = get_text(&format!("{mcp}/llms.txt"))?;
    require(
        mcp_llms.contains("MCP tools") && mcp_llms.contains("route_blocked_goal"),
        "MCP llms.txt must orient agents to MCP tools",
    )?;

    let route = post_json(
        &format!("{api}/v1/route-blocked-goal"),
        serde_json::json!({
            "goal": "Fix the failing API service smoke test",
            "context": "The HTTP route should classify this as a CI or coding task.",
            "budget_minor": 1_000_000,
            "currency": "usdc",
            "privacy": "Public"
        }),
    )?;
    require(
        route.pointer("/capability_class").is_some(),
        "route response must include capability_class",
    )?;

    let base_log_query = post_json(
        &format!("{api}/v1/base/log-query"),
        serde_json::json!({
            "escrow_contract": "0x1111111111111111111111111111111111111111",
            "from_block": 123,
            "to_block": null,
            "request_id": 7
        }),
    )?;
    require(
        value_str(&base_log_query, "/method") == Some("eth_getLogs"),
        "Base log query planner must produce eth_getLogs request",
    )?;
    require(
        value_str(&base_log_query, "/params/0/fromBlock") == Some("0x7b"),
        "Base log query planner must encode fromBlock as hex quantity",
    )?;
    let base_rpc_log_report = post_json(
        &format!("{api}/v1/base/rpc-logs"),
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 7,
            "result": []
        }),
    )?;
    require(
        base_rpc_log_report
            .pointer("/decoded_events")
            .and_then(|value| value.as_u64())
            == Some(0),
        "Base RPC log endpoint must accept an empty eth_getLogs response",
    )?;

    let smoke_id = Uuid::new_v4();
    let smoke_escrow_seed = smoke_id.as_u128() % 1_000_000_000 + 10_000;
    let requester = post_json(
        &format!("{api}/v1/agents"),
        serde_json::json!({
            "handle": format!("service-smoke-requester-{smoke_id}"),
            "payout_wallet": null
        }),
    )?;
    require(
        value_str(&requester, "/id").is_some(),
        "requester id missing",
    )?;

    let solver = post_json(
        &format!("{api}/v1/agents"),
        serde_json::json!({
            "handle": format!("service-smoke-solver-{smoke_id}"),
            "payout_wallet": "0x2222222222222222222222222222222222222222"
        }),
    )?;
    let solver_id = value_str(&solver, "/id")
        .context("solver id missing")?
        .to_string();

    let capability = post_json(
        &format!("{api}/v1/capabilities"),
        serde_json::json!({
            "agent_id": solver_id,
            "class": "Coding",
            "template_slugs": ["fix-ci-failure", "small-code-change"],
            "min_price_minor": 500_000,
            "max_price_minor": 1_000_000,
            "currency": "usdc",
            "latency_seconds": 600,
            "supported_verifiers": ["GitHubCi", "JsonSchema"]
        }),
    )?;
    require(
        value_str(&capability, "/id").is_some(),
        "registered capability id missing",
    )?;

    let capability_feed = get_json(&format!("{api}/v1/capabilities/feed"))?;
    let capability_items = capability_feed
        .as_array()
        .context("capability feed must be an array")?;
    require(
        capability_items.iter().any(|item| {
            value_str(item, "/agent_id")
                .map(|id| id == solver_id.as_str())
                .unwrap_or(false)
        }),
        "registered solver must appear in public capability feed",
    )?;

    let capability_search = post_json(
        &format!("{api}/v1/capabilities/search"),
        serde_json::json!({
            "class": "Coding",
            "template_slug": "fix-ci-failure",
            "currency": "usdc",
            "max_price_minor": 600_000
        }),
    )?;
    let capability_search_items = capability_search
        .as_array()
        .context("capability search must be an array")?;
    require(
        capability_search_items.iter().any(|item| {
            value_str(item, "/agent_id")
                .map(|id| id == solver_id.as_str())
                .unwrap_or(false)
        }),
        "registered solver must appear in filtered capability search",
    )?;

    let api_risk_policy = get_json(&format!("{api}/v1/risk/policy"))?;
    require(
        api_risk_policy.pointer("/low_value_usdc_cap_minor")
            == Some(&serde_json::json!(10_000_000)),
        "API risk policy must expose the low-value Base USDC cap",
    )?;
    require(
        api_risk_policy.pointer("/ai_judges_can_authorize_payment")
            == Some(&serde_json::json!(false)),
        "API risk policy must state that AI judges cannot authorize payment",
    )?;

    let pooled_bounty = post_json(
        &format!("{api}/v1/bounties/pooled"),
        serde_json::json!({
            "title": "Service smoke pooled bounty",
            "template_slug": "extract-data-to-schema",
            "target_amount_minor": 1_000,
            "currency": "usdc",
            "funding_mode": "Simulated",
            "privacy": "Public"
        }),
    )?;
    let pooled_bounty_id = value_str(&pooled_bounty, "/id")
        .context("pooled bounty id missing")?
        .to_string();
    let pooled_funding = post_json(
        &format!("{api}/v1/bounties/{pooled_bounty_id}/funding-contributions"),
        serde_json::json!({
            "bounty_id": pooled_bounty_id.as_str(),
            "contributor_agent_id": null,
            "source_organization_id": null,
            "amount_minor": 1_000,
            "currency": "usdc",
            "rail": "Simulated",
            "external_reference": format!("service-smoke-pooled-{smoke_id}")
        }),
    )?;
    require(
        value_str(&pooled_funding, "/bounty/status") == Some("Claimable"),
        "pooled simulated funding must make the bounty claimable at target",
    )?;
    require(
        pooled_funding
            .pointer("/contribution/funding_ledger_entry_id")
            .and_then(|value| value.as_str())
            .is_some(),
        "pooled funding contribution must link to its funding ledger entry",
    )?;

    let intent_bounty = post_json(
        &format!("{api}/v1/bounties/pooled"),
        serde_json::json!({
            "title": "Service smoke mixed funding intent bounty",
            "template_slug": "extract-data-to-schema",
            "target_amount_minor": 500,
            "currency": "usd",
            "funding_mode": "MixedRails",
            "privacy": "Public",
            "funding_targets": [
                {"rail": "StripeFiat", "amount_minor": 500, "currency": "usd"},
                {"rail": "BaseUsdc", "amount_minor": 1000, "currency": "usdc"}
            ]
        }),
    )?;
    let intent_bounty_id = value_str(&intent_bounty, "/id")
        .context("funding-intent bounty id missing")?
        .to_string();
    let funding_intent = post_json(
        &format!("{api}/v1/bounties/{intent_bounty_id}/funding-intents"),
        serde_json::json!({
            "bounty_id": intent_bounty_id.as_str(),
            "contributor_agent_id": null,
            "source_organization_id": smoke_id,
            "amount_minor": 500,
            "currency": "usd",
            "rail": "StripeFiat",
            "external_reference": format!("service-smoke-intent-{smoke_id}"),
            "stripe_success_url": null,
            "stripe_cancel_url": null,
            "base_escrow_contract": null,
            "base_payer": null,
            "base_token": null,
            "base_network": null
        }),
    )?;
    require(
        value_str(&funding_intent, "/intent/status") == Some("AwaitingEvidence"),
        "funding intent must remain pending before payment evidence",
    )?;
    require(
        value_str(&funding_intent, "/next_action/kind") == Some("StripeCheckout"),
        "Stripe funding intent must return a Checkout next action",
    )?;

    let pooled_claim = post_json(
        &format!("{api}/v1/bounties/{pooled_bounty_id}/claim"),
        serde_json::json!({
            "bounty_id": pooled_bounty_id.as_str(),
            "solver_agent_id": solver_id.as_str()
        }),
    )?;
    require(
        value_str(&pooled_claim, "/status") == Some("Claimed"),
        "pooled bounty claim must move bounty to Claimed",
    )?;
    let pooled_artifact_body = "{\"pooled_smoke\":true}";
    let pooled_submission = post_json(
        &format!("{api}/v1/bounties/{pooled_bounty_id}/submit"),
        serde_json::json!({
            "bounty_id": pooled_bounty_id.as_str(),
            "solver_agent_id": solver_id.as_str(),
            "artifact_uri": "memory://service-smoke-pooled-artifact",
            "artifact_body": pooled_artifact_body
        }),
    )?;
    let pooled_submission_id =
        value_str(&pooled_submission, "/id").context("pooled submission id missing")?;
    let pooled_proof = post_json(
        &format!("{api}/v1/bounties/{pooled_bounty_id}/verify"),
        serde_json::json!({
            "bounty_id": pooled_bounty_id.as_str(),
            "submission_id": pooled_submission_id,
            "expected_artifact_digest": hash_artifact(pooled_artifact_body),
            "verifier_kind": "JsonSchema",
            "rubric": null,
            "evidence": null,
            "approved_risk_event_id": null
        }),
    )?;
    require(
        value_str(&pooled_proof, "/proof_hash").is_some(),
        "pooled bounty verification must return a proof hash",
    )?;
    let pooled_status = get_json(&format!("{api}/v1/bounties/{pooled_bounty_id}"))?;
    let pooled_settlement_id =
        value_str(&pooled_status, "/settlements/0/id").context("pooled settlement id missing")?;
    require(
        value_str(&pooled_status, "/bounty/status") == Some("Paid"),
        "pooled simulated bounty must settle to Paid",
    )?;
    require(
        value_str(&pooled_status, "/funding_contributions/0/settlement_id")
            == Some(pooled_settlement_id),
        "pooled funding contribution must link to the settlement after verification",
    )?;

    let bounty = post_json(
        &format!("{api}/v1/bounties"),
        serde_json::json!({
            "title": "Service smoke funded bounty",
            "template_slug": "fix-ci-failure",
            "amount_minor": 1_000_000,
            "currency": "usdc",
            "funding_mode": "BaseUsdcEscrow",
            "privacy": "Public"
        }),
    )?;
    let bounty_id = value_str(&bounty, "/id")
        .context("bounty id missing")?
        .to_string();
    let bounty_terms_hash =
        value_str(&bounty, "/terms_hash").context("bounty terms hash missing")?;
    require(
        value_str(&bounty, "/status") == Some("Unfunded"),
        "newly posted Base bounty must start funding-ready",
    )?;
    let funded_bounty = post_json(
        &format!("{api}/v1/base/escrow-events"),
        base_created_event_json(&bounty_id, 1_000_000, bounty_terms_hash, smoke_escrow_seed),
    )?;
    require(
        value_str(&funded_bounty, "/bounty/status") == Some("Claimable"),
        "Base escrow event must make API bounty claimable",
    )?;

    let feed = get_json(&format!("{api}/v1/bounties/feed"))?;
    let feed_items = feed.as_array().context("bounty feed must be an array")?;
    require(
        feed_items.iter().any(|item| {
            value_str(item, "/bounty_id")
                .map(|id| id == bounty_id.as_str())
                .unwrap_or(false)
        }),
        "newly posted public bounty must appear in public feed",
    )?;
    let public_bounty_page = get_text(&format!("{api}/public/bounties/{bounty_id}"))?;
    require(
        public_bounty_page.contains("Funding State")
            && public_bounty_page.contains("application/ld+json")
            && public_bounty_page.contains("agent-bounty-public-status")
            && public_bounty_page.contains("Machine status")
            && public_bounty_page.contains(r#"data-agent-action="claim""#)
            && !public_bounty_page.contains("Add funding")
            && !public_bounty_page.contains(r#"rel="payment""#),
        "public funded bounty detail page must expose claim/status actions without unsafe funding links",
    )?;

    let mcp_discovery = get_json(&format!("{mcp}/.well-known/agent-bounties.json"))?;
    require(
        mcp_discovery.pointer("/agent_entrypoints").is_some(),
        "MCP discovery manifest must include agent entrypoints",
    )?;
    let tools = get_json(&format!("{mcp}/tools"))?;
    let tool_list = tools.as_array().context("MCP tools must be an array")?;
    for tool in tool_list {
        let name = value_str(tool, "/name").unwrap_or("<unnamed>");
        require(
            tool.pointer("/input_schema/type").is_some(),
            &format!("MCP tool {name} missing input_schema.type"),
        )?;
    }
    for expected in [
        "route_blocked_goal",
        "search_capabilities",
        "list_claimable_bounties",
        "create_funding_intent",
        "plan_base_log_query",
        "reconcile_base_escrow_event",
        "reconcile_base_rpc_logs",
        "fetch_base_rpc_logs",
        "broadcast_base_signed_transaction",
        "get_base_transaction_receipt",
        "plan_base_funding",
        "list_base_release_queue",
        "plan_base_refund",
        "plan_base_dispute",
        "get_live_money_readiness",
        "get_base_indexer_status",
        "plan_stripe_checkout_top_up",
        "plan_stripe_connect_account",
        "plan_stripe_connect_transfer",
        "execute_stripe_checkout_top_up",
        "execute_stripe_connect_account",
        "execute_stripe_connect_transfer",
        "plan_github_issue_bounty",
        "plan_github_funding_comment",
        "plan_github_claim_comment",
        "plan_github_proof_comment",
        "plan_github_proof_comment_for_proof",
        "run_eval_loops",
        "get_eval_runs",
        "get_risk_policy",
        "list_risk_events",
        "list_risk_reviews",
        "approve_risk_bounty",
        "approve_risk_payout",
        "reject_risk_event",
    ] {
        require(
            tool_list.iter().any(|tool| {
                value_str(tool, "/name")
                    .map(|name| name == expected)
                    .unwrap_or(false)
            }),
            &format!("MCP tool list missing {expected}"),
        )?;
    }

    let mcp_route = mcp_tool_post(
        mcp,
        "route_blocked_goal",
        serde_json::json!({
            "goal": "Fix a deterministic MCP lifecycle smoke failure",
            "context": "An autonomous agent is blocked and needs a paid CI-sized coding task routed.",
            "budget_minor": 1_000_000,
            "currency": "usdc",
            "privacy": "Public"
        }),
    )?;
    require(
        value_str(&mcp_route, "/capability_class").is_some(),
        "MCP route_blocked_goal must return capability_class",
    )?;

    let mcp_risk_policy = mcp_tool_get(mcp, "get_risk_policy")?;
    require(
        mcp_risk_policy.pointer("/low_value_usdc_cap_minor")
            == Some(&serde_json::json!(10_000_000)),
        "MCP get_risk_policy must expose the low-value Base USDC cap",
    )?;
    require(
        mcp_risk_policy.pointer("/ai_judges_can_authorize_payment")
            == Some(&serde_json::json!(false)),
        "MCP get_risk_policy must state that AI judges cannot authorize payment",
    )?;
    let api_live_money_readiness = get_json(&format!(
        "{api}/v1/readiness/live-money?network=base-mainnet"
    ))?;
    require(
        api_live_money_readiness.pointer("/network_chain_id") == Some(&serde_json::json!(8_453)),
        "API live-money readiness must expose Base mainnet chain id",
    )?;
    require(
        api_live_money_readiness
            .pointer("/stripe_secret_key_mode")
            .and_then(|value| value.as_str())
            .map(|mode| !mode.starts_with("sk_") && !mode.starts_with("rk_"))
            .unwrap_or(false),
        "API live-money readiness must not expose Stripe secret material",
    )?;
    require(
        api_live_money_readiness
            .pointer("/stripe_payment_method_configuration_configured")
            .and_then(|value| value.as_bool())
            .is_some(),
        "API live-money readiness must expose a non-secret Stripe payment-method configuration boolean",
    )?;
    let mcp_live_money_readiness = mcp_tool_post(
        mcp,
        "get_live_money_readiness",
        serde_json::json!({ "network": "base-mainnet" }),
    )?;
    require(
        mcp_live_money_readiness.pointer("/network_chain_id") == Some(&serde_json::json!(8_453)),
        "MCP get_live_money_readiness must expose Base mainnet chain id",
    )?;
    require(
        mcp_live_money_readiness
            .pointer("/live_money_ready")
            .and_then(|value| value.as_bool())
            .is_some(),
        "MCP get_live_money_readiness must expose live_money_ready boolean",
    )?;
    require(
        mcp_live_money_readiness
            .pointer("/stripe_payment_method_configuration_configured")
            .and_then(|value| value.as_bool())
            .is_some(),
        "MCP get_live_money_readiness must expose a non-secret Stripe payment-method configuration boolean",
    )?;
    let api_base_indexer_status = get_json(&format!(
        "{api}/v1/base/indexer-status?network=base-mainnet"
    ))?;
    require(
        api_base_indexer_status.pointer("/network_chain_id") == Some(&serde_json::json!(8_453)),
        "API Base indexer status must expose Base mainnet chain id",
    )?;
    require(
        api_base_indexer_status
            .pointer("/indexer_ready")
            .and_then(|value| value.as_bool())
            .is_some(),
        "API Base indexer status must expose indexer_ready boolean",
    )?;
    require_base_indexer_status_contract(&api_base_indexer_status, "API Base indexer status")?;
    let mcp_base_indexer_status = mcp_tool_post(
        mcp,
        "get_base_indexer_status",
        serde_json::json!({ "network": "base-mainnet" }),
    )?;
    require(
        mcp_base_indexer_status.pointer("/network_chain_id") == Some(&serde_json::json!(8_453)),
        "MCP get_base_indexer_status must expose Base mainnet chain id",
    )?;
    require_base_indexer_status_contract(&mcp_base_indexer_status, "MCP get_base_indexer_status")?;

    let mcp_risk_bounty = post_json(
        &format!("{mcp}/tools/post_bounty"),
        serde_json::json!({
            "title": "MCP service smoke review-required bounty",
            "template_slug": "fix-ci-failure",
            "amount_minor": 25_000_000,
            "currency": "usdc",
            "funding_mode": "BaseUsdcEscrow",
            "privacy": "Public"
        }),
    )?;
    require(
        value_str(&mcp_risk_bounty, "/error")
            .map(|error| error.contains("requires review"))
            .unwrap_or(false),
        "MCP post_bounty must reject over-cap Base USDC bounty for review",
    )?;
    let mcp_risk_events = mcp_tool_post(
        mcp,
        "list_risk_events",
        serde_json::json!({
            "action": "NeedsReview",
            "surface": "Bounty",
            "bounty_id": null,
            "agent_id": null,
            "limit": 10
        }),
    )?;
    require(
        mcp_risk_events
            .as_array()
            .map(|events| {
                events.iter().any(|event| {
                    value_str(event, "/action") == Some("NeedsReview")
                        && event
                            .pointer("/reasons")
                            .and_then(|reasons| reasons.as_array())
                            .map(|reasons| {
                                reasons.iter().any(|reason| {
                                    reason
                                        .as_str()
                                        .map(|text| text.contains("low-value cap"))
                                        .unwrap_or(false)
                                })
                            })
                            .unwrap_or(false)
                })
            })
            .unwrap_or(false),
        "MCP list_risk_events must expose review-required bounty events",
    )?;
    let mcp_risk_event_id = mcp_risk_events
        .as_array()
        .and_then(|events| {
            events.iter().find(|event| {
                value_str(event, "/action") == Some("NeedsReview")
                    && event
                        .pointer("/reasons")
                        .and_then(|reasons| reasons.as_array())
                        .map(|reasons| {
                            reasons.iter().any(|reason| {
                                reason
                                    .as_str()
                                    .map(|text| text.contains("low-value cap"))
                                    .unwrap_or(false)
                            })
                        })
                        .unwrap_or(false)
            })
        })
        .and_then(|event| value_str(event, "/id"))
        .context("MCP risk event id missing")?
        .to_string();
    let mcp_reviewed_bounty = mcp_tool_post(
        mcp,
        "approve_risk_bounty",
        serde_json::json!({
            "risk_event_id": mcp_risk_event_id,
            "title": "MCP service smoke review-required bounty",
            "template_slug": "fix-ci-failure",
            "amount_minor": 25_000_000,
            "currency": "usdc",
            "funding_mode": "BaseUsdcEscrow",
            "privacy": "Public",
            "operator_id": "service-smoke-operator",
            "note": "Approved review-required bounty during service smoke."
        }),
    )?;
    let mcp_reviewed_bounty_id =
        value_str(&mcp_reviewed_bounty, "/bounty/id").context("MCP reviewed bounty id missing")?;
    let mcp_reviewed_terms_hash = value_str(&mcp_reviewed_bounty, "/bounty/terms_hash")
        .context("MCP reviewed bounty terms hash missing")?;
    require(
        value_str(&mcp_reviewed_bounty, "/bounty/status") == Some("Unfunded"),
        "MCP approve_risk_bounty must create a funding-ready Base bounty",
    )?;
    require(
        value_str(&mcp_reviewed_bounty, "/review/outcome") == Some("Approved"),
        "MCP approve_risk_bounty must record an Approved review",
    )?;
    let mcp_reviewed_funding = mcp_reconcile_base_created(
        mcp,
        mcp_reviewed_bounty_id,
        25_000_000,
        mcp_reviewed_terms_hash,
        smoke_escrow_seed + 1,
    )?;
    require(
        value_str(&mcp_reviewed_funding, "/bounty/status") == Some("Claimable"),
        "MCP reconcile_base_escrow_event must make reviewed Base bounty claimable",
    )?;
    let mcp_risk_reviews = mcp_tool_get(mcp, "list_risk_reviews")?;
    require(
        mcp_risk_reviews
            .as_array()
            .map(|reviews| {
                reviews.iter().any(|review| {
                    value_str(review, "/outcome") == Some("Approved")
                        && value_str(review, "/bounty_id") == Some(mcp_reviewed_bounty_id)
                })
            })
            .unwrap_or(false),
        "MCP list_risk_reviews must include the approval record",
    )?;
    let mcp_review_solver = mcp_tool_post(
        mcp,
        "register_agent",
        serde_json::json!({
            "handle": format!("mcp-service-smoke-review-solver-{smoke_id}"),
            "payout_wallet": "0x2222222222222222222222222222222222222222"
        }),
    )?;
    let mcp_review_solver_id =
        value_str(&mcp_review_solver, "/id").context("MCP review solver id missing")?;
    let mcp_review_claim = mcp_tool_post(
        mcp,
        "claim_bounty",
        serde_json::json!({
            "bounty_id": mcp_reviewed_bounty_id,
            "solver_agent_id": mcp_review_solver_id
        }),
    )?;
    require(
        value_str(&mcp_review_claim, "/status") == Some("Claimed"),
        "MCP claim_bounty must claim reviewed high-value bounty",
    )?;
    let reviewed_artifact_body = "{\"mcp_reviewed\":true}";
    let mcp_review_submission = mcp_tool_post(
        mcp,
        "submit_result",
        serde_json::json!({
            "bounty_id": mcp_reviewed_bounty_id,
            "solver_agent_id": mcp_review_solver_id,
            "artifact_uri": "https://github.com/example/repo/pull/1",
            "artifact_body": reviewed_artifact_body
        }),
    )?;
    let mcp_review_submission_id = value_str(&mcp_review_submission, "/id")
        .context("MCP reviewed bounty submission id missing")?;
    let mcp_review_verification_block = post_json(
        &format!("{mcp}/tools/request_verification"),
        serde_json::json!({
            "bounty_id": mcp_reviewed_bounty_id,
            "submission_id": mcp_review_submission_id,
            "expected_artifact_digest": "not-used-by-github-ci",
            "verifier_kind": null,
            "rubric": null,
            "evidence": github_ci_evidence(),
            "approved_risk_event_id": null
        }),
    )?;
    require(
        value_str(&mcp_review_verification_block, "/error")
            .map(|error| error.contains("automatic release cap"))
            .unwrap_or(false),
        "MCP request_verification must require payout review for high-value Base USDC",
    )?;
    let mcp_payout_risk_events = mcp_tool_post(
        mcp,
        "list_risk_events",
        serde_json::json!({
            "action": "NeedsReview",
            "surface": "Payout",
            "bounty_id": mcp_reviewed_bounty_id,
            "agent_id": null,
            "limit": 10
        }),
    )?;
    let mcp_payout_risk_event_id = mcp_payout_risk_events
        .as_array()
        .and_then(|events| {
            events.iter().find(|event| {
                value_str(event, "/action") == Some("NeedsReview")
                    && event
                        .pointer("/reasons")
                        .and_then(|reasons| reasons.as_array())
                        .map(|reasons| {
                            reasons.iter().any(|reason| {
                                reason
                                    .as_str()
                                    .map(|text| text.contains("automatic release cap"))
                                    .unwrap_or(false)
                            })
                        })
                        .unwrap_or(false)
            })
        })
        .and_then(|event| value_str(event, "/id"))
        .context("MCP payout risk event id missing")?
        .to_string();
    let mcp_payout_review = mcp_tool_post(
        mcp,
        "approve_risk_payout",
        serde_json::json!({
            "risk_event_id": mcp_payout_risk_event_id.as_str(),
            "operator_id": "service-smoke-operator",
            "note": "Approved high-value payout during service smoke."
        }),
    )?;
    require(
        value_str(&mcp_payout_review, "/surface") == Some("Payout")
            && value_str(&mcp_payout_review, "/outcome") == Some("Approved"),
        "MCP approve_risk_payout must record an Approved payout review",
    )?;
    let mcp_reviewed_proof = mcp_tool_post(
        mcp,
        "request_verification",
        serde_json::json!({
            "bounty_id": mcp_reviewed_bounty_id,
            "submission_id": mcp_review_submission_id,
            "expected_artifact_digest": "not-used-by-github-ci",
            "verifier_kind": null,
            "rubric": null,
            "evidence": github_ci_evidence(),
            "approved_risk_event_id": mcp_payout_risk_event_id.as_str()
        }),
    )?;
    require(
        value_str(&mcp_reviewed_proof, "/proof_hash").is_some(),
        "MCP reviewed payout verification must return a proof hash",
    )?;
    let mcp_reviewed_status = mcp_tool_post(
        mcp,
        "get_bounty_status",
        serde_json::json!({ "bounty_id": mcp_reviewed_bounty_id }),
    )?;
    require(
        value_str(&mcp_reviewed_status, "/bounty/status") == Some("Payable"),
        "MCP reviewed high-value bounty must become Payable after payout approval",
    )?;

    let mcp_eval_loops = mcp_tool_get(mcp, "run_eval_loops")?;
    require(
        mcp_eval_loops
            .pointer("/loops")
            .and_then(|loops| loops.as_array())
            .map(|loops| loops.len() == 5)
            .unwrap_or(false),
        "MCP run_eval_loops must return all five loop reports",
    )?;
    require(
        mcp_eval_loops
            .pointer("/passed")
            .and_then(|passed| passed.as_bool())
            == Some(true),
        "MCP run_eval_loops must pass",
    )?;
    let mcp_eval_runs = mcp_tool_get(mcp, "get_eval_runs")?;
    require(
        mcp_eval_runs
            .as_array()
            .map(|runs| {
                runs.iter().any(|run| {
                    value_str(run, "/suite")
                        .map(|suite| suite == "EvalLoops/all-v0")
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false),
        "MCP get_eval_runs must include the recorded EvalLoops/all-v0 run",
    )?;

    let mcp_smoke_id = Uuid::new_v4();
    let mcp_solver = mcp_tool_post(
        mcp,
        "register_agent",
        serde_json::json!({
            "handle": format!("mcp-service-smoke-solver-{mcp_smoke_id}"),
            "payout_wallet": "0x2222222222222222222222222222222222222222"
        }),
    )?;
    let mcp_solver_id = value_str(&mcp_solver, "/id").context("MCP solver id missing")?;

    let mcp_base_log_query = mcp_tool_post(
        mcp,
        "plan_base_log_query",
        serde_json::json!({
            "escrow_contract": "0x1111111111111111111111111111111111111111",
            "from_block": 123,
            "to_block": null,
            "request_id": 8
        }),
    )?;
    require(
        value_str(&mcp_base_log_query, "/method") == Some("eth_getLogs"),
        "MCP plan_base_log_query must produce eth_getLogs request",
    )?;
    let mcp_base_rpc_logs = mcp_tool_post(
        mcp,
        "reconcile_base_rpc_logs",
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 8,
            "result": []
        }),
    )?;
    require(
        mcp_base_rpc_logs
            .pointer("/decoded_events")
            .and_then(|value| value.as_u64())
            == Some(0),
        "MCP reconcile_base_rpc_logs must accept an empty eth_getLogs response",
    )?;

    let mcp_capability = mcp_tool_post(
        mcp,
        "register_capability",
        serde_json::json!({
            "agent_id": mcp_solver_id,
            "class": "Coding",
            "template_slugs": ["small-code-change"],
            "min_price_minor": 500_000,
            "max_price_minor": 1_000_000,
            "currency": "usdc",
            "latency_seconds": 600,
            "supported_verifiers": ["JsonSchema"]
        }),
    )?;
    require(
        value_str(&mcp_capability, "/id").is_some(),
        "MCP register_capability must return a capability id",
    )?;

    let mcp_capability_search = mcp_tool_post(
        mcp,
        "search_capabilities",
        serde_json::json!({
            "class": "Coding",
            "template_slug": "small-code-change",
            "currency": "usdc",
            "max_price_minor": 1_000_000
        }),
    )?;
    let mcp_capability_items = mcp_capability_search
        .as_array()
        .context("MCP search_capabilities result must be an array")?;
    require(
        mcp_capability_items.iter().any(|item| {
            value_str(item, "/agent_id")
                .map(|id| id == mcp_solver_id)
                .unwrap_or(false)
        }),
        "MCP search_capabilities must include the registered solver",
    )?;

    let mcp_stripe_checkout = mcp_tool_post(
        mcp,
        "plan_stripe_checkout_top_up",
        serde_json::json!({
            "organization_id": smoke_id,
            "amount_minor": 5_000,
            "currency": "usd",
            "success_url": null,
            "cancel_url": null
        }),
    )?;
    require(
        value_str(&mcp_stripe_checkout, "/endpoint") == Some("/v1/checkout/sessions"),
        "MCP plan_stripe_checkout_top_up must produce a Checkout Sessions request intent",
    )?;

    let mcp_stripe_connect = mcp_tool_post(
        mcp,
        "plan_stripe_connect_account",
        serde_json::json!({ "agent_id": mcp_solver_id }),
    )?;
    require(
        value_str(&mcp_stripe_connect, "/request/endpoint") == Some("/v2/core/accounts"),
        "MCP plan_stripe_connect_account must produce an Accounts v2 request intent",
    )?;

    let mcp_github_issue = mcp_tool_post(
        mcp,
        "plan_github_issue_bounty",
        serde_json::json!({
            "repository": "agent-bounties/agent-bounties",
            "issue_url": "https://github.com/agent-bounties/agent-bounties/issues/1",
            "title": "[bounty]: Fix CI",
            "body": "### Goal\nFix the failing CI check.\n\n### Acceptance criteria\nThe test job is green and the patch explains the failure.\n\n### Template\nfix-ci-failure\n\n### Suggested amount\n10 USDC\n"
        }),
    )?;
    require(
        mcp_github_issue
            .pointer("/ready")
            .and_then(|value| value.as_bool())
            == Some(true),
        "MCP plan_github_issue_bounty must accept a valid paid bounty issue",
    )?;
    require(
        value_str(&mcp_github_issue, "/check/conclusion") == Some("Success"),
        "MCP plan_github_issue_bounty must produce a success check",
    )?;

    let mcp_github_funding = mcp_tool_post(
        mcp,
        "plan_github_funding_comment",
        serde_json::json!({
            "repository": "agent-bounties/agent-bounties",
            "issue_url": "https://github.com/agent-bounties/agent-bounties/issues/1",
            "title": "[bounty]: Fix CI",
            "body": "### Goal\nFix the failing CI check.\n\n### Acceptance criteria\nThe test job is green and the patch explains the failure.\n\n### Template\nfix-ci-failure\n\n### Suggested amount\n10 USDC\n",
            "comment_body": "/agent-bounty fund 5 USDC via BaseUsdcEscrow",
            "contributor_login": "service-smoke",
            "comment_id": "12345",
            "existing_idempotency_keys": []
        }),
    )?;
    require(
        mcp_github_funding
            .pointer("/ready")
            .and_then(|value| value.as_bool())
            == Some(true),
        "MCP plan_github_funding_comment must accept a valid funding signal",
    )?;
    require(
        mcp_github_funding
            .pointer("/signal/requires_operator_reconciliation")
            .and_then(|value| value.as_bool())
            == Some(true),
        "MCP plan_github_funding_comment must require operator reconciliation",
    )?;

    let mcp_github_claim = mcp_tool_post(
        mcp,
        "plan_github_claim_comment",
        serde_json::json!({
            "repository": "agent-bounties/agent-bounties",
            "issue_url": "https://github.com/agent-bounties/agent-bounties/issues/1",
            "title": "[bounty]: Fix CI",
            "body": "### Goal\nFix the failing CI check.\n\n### Acceptance criteria\nThe test job is green and the patch explains the failure.\n\n### Template\nfix-ci-failure\n\n### Suggested amount\n10 USDC\n",
            "comment_body": "/agent-bounty claim\nPlan: inspect CI logs and open a small fix.",
            "contributor_login": "service-smoke",
            "comment_id": "12346",
            "claim_age_minutes": 5,
            "progress_signal_count": 0,
            "active_claim_login": null
        }),
    )?;
    require(
        mcp_github_claim
            .pointer("/ready")
            .and_then(|value| value.as_bool())
            == Some(true),
        "MCP plan_github_claim_comment must accept a progress-backed claim",
    )?;
    require(
        mcp_github_claim
            .pointer("/signal/settlement_authority")
            .and_then(|value| value.as_bool())
            == Some(false),
        "MCP plan_github_claim_comment must not authorize settlement",
    )?;

    let mcp_github_proof = mcp_tool_post(
        mcp,
        "plan_github_proof_comment",
        serde_json::json!({
            "bounty_id": smoke_id,
            "proof_url": "https://agentbounties.local/public/proofs/smoke",
            "verifier_summary": "GitHub CI passed",
            "settlement_url": null
        }),
    )?;
    require(
        value_str(&mcp_github_proof, "/fingerprint")
            .map(|fingerprint| fingerprint.len() == 64)
            .unwrap_or(false),
        "MCP plan_github_proof_comment must produce a stable fingerprint",
    )?;

    let mcp_bounty = mcp_tool_post(
        mcp,
        "post_bounty",
        serde_json::json!({
            "title": "MCP service smoke paid bounty",
            "template_slug": "small-code-change",
            "amount_minor": 1_000_000,
            "currency": "usdc",
            "funding_mode": "BaseUsdcEscrow",
            "privacy": "Public"
        }),
    )?;
    let mcp_bounty_id = value_str(&mcp_bounty, "/id")
        .context("MCP bounty id missing")?
        .to_string();
    let mcp_bounty_terms_hash =
        value_str(&mcp_bounty, "/terms_hash").context("MCP bounty terms hash missing")?;
    require(
        value_str(&mcp_bounty, "/status") == Some("Unfunded"),
        "MCP post_bounty must create a funding-ready Base bounty",
    )?;
    let mcp_bounty_funding = mcp_reconcile_base_created(
        mcp,
        mcp_bounty_id.as_str(),
        1_000_000,
        mcp_bounty_terms_hash,
        smoke_escrow_seed + 2,
    )?;
    require(
        value_str(&mcp_bounty_funding, "/bounty/status") == Some("Claimable"),
        "MCP reconcile_base_escrow_event must make posted Base bounty claimable",
    )?;

    let mcp_claimable = mcp_tool_get(mcp, "list_claimable_bounties")?;
    let mcp_claimable_items = mcp_claimable
        .as_array()
        .context("MCP list_claimable_bounties result must be an array")?;
    require(
        mcp_claimable_items.iter().any(|item| {
            value_str(item, "/bounty_id")
                .map(|id| id == mcp_bounty_id.as_str())
                .unwrap_or(false)
        }),
        "MCP-posted public bounty must appear in MCP claimable list",
    )?;

    let mcp_claim = mcp_tool_post(
        mcp,
        "claim_bounty",
        serde_json::json!({
            "bounty_id": mcp_bounty_id.as_str(),
            "solver_agent_id": mcp_solver_id
        }),
    )?;
    require(
        value_str(&mcp_claim, "/status") == Some("Claimed"),
        "MCP claim_bounty must move bounty to Claimed",
    )?;

    let artifact_body = "{\"mcp_smoke\":true}";
    let mcp_submission = mcp_tool_post(
        mcp,
        "submit_result",
        serde_json::json!({
            "bounty_id": mcp_bounty_id.as_str(),
            "solver_agent_id": mcp_solver_id,
            "artifact_uri": "s3://agent-bounties/mcp-smoke/artifact.json",
            "artifact_body": artifact_body
        }),
    )?;
    let mcp_submission_id =
        value_str(&mcp_submission, "/id").context("MCP submission id missing")?;

    let mcp_proof = mcp_tool_post(
        mcp,
        "request_verification",
        serde_json::json!({
            "bounty_id": mcp_bounty_id.as_str(),
            "submission_id": mcp_submission_id,
            "expected_artifact_digest": hash_artifact(artifact_body),
            "verifier_kind": "JsonSchema",
            "rubric": null,
            "evidence": null
        }),
    )?;
    require(
        value_str(&mcp_proof, "/proof_hash").is_some(),
        "MCP request_verification must return a proof hash",
    )?;
    let mcp_proof_id = value_str(&mcp_proof, "/id").context("MCP proof id missing")?;
    let mcp_proof_comment_from_proof = mcp_tool_post(
        mcp,
        "plan_github_proof_comment_for_proof",
        serde_json::json!({
            "proof_id": mcp_proof_id,
            "settlement_url": null
        }),
    )?;
    require(
        value_str(&mcp_proof_comment_from_proof, "/comment/bounty_id")
            == Some(mcp_bounty_id.as_str()),
        "MCP proof-record proof comment planner must use the verified bounty",
    )?;
    require(
        value_str(&mcp_proof_comment_from_proof, "/comment/proof_url")
            .map(|url| url.ends_with(&format!("/public/proofs/{mcp_proof_id}")))
            .unwrap_or(false),
        "MCP proof-record proof comment planner must link the public proof page",
    )?;
    require(
        value_str(&mcp_proof_comment_from_proof, "/fingerprint")
            .map(|fingerprint| fingerprint.len() == 64)
            .unwrap_or(false),
        "MCP proof-record proof comment planner must produce a stable fingerprint",
    )?;

    let mcp_status = mcp_tool_post(
        mcp,
        "get_bounty_status",
        serde_json::json!({ "bounty_id": mcp_bounty_id.as_str() }),
    )?;
    require(
        value_str(&mcp_status, "/bounty/status") == Some("Payable"),
        "MCP verified Base bounty must be Payable pending chain release",
    )?;

    let mcp_paid_status = mcp_tool_post(
        mcp,
        "get_paid_status",
        serde_json::json!({ "bounty_id": mcp_bounty_id.as_str() }),
    )?;
    require(
        value_str(&mcp_paid_status, "/bounty_status") == Some("Payable"),
        "MCP paid status must expose payable pending payout state",
    )?;
    require(
        mcp_paid_status
            .pointer("/settlements")
            .and_then(|settlements| settlements.as_array())
            .map(|settlements| !settlements.is_empty())
            .unwrap_or(false),
        "MCP paid status must include settlement payout intents",
    )?;
    let mcp_agent_paid_status = mcp_tool_post(
        mcp,
        "get_paid_status",
        serde_json::json!({ "agent_id": mcp_solver_id }),
    )?;
    require(
        value_str(&mcp_agent_paid_status, "/scope") == Some("agent"),
        "MCP agent paid status must report agent scope",
    )?;
    require(
        mcp_agent_paid_status
            .pointer("/payouts")
            .and_then(|payouts| payouts.as_array())
            .map(|payouts| !payouts.is_empty())
            .unwrap_or(false),
        "MCP agent paid status must include pending payout lines",
    )?;
    require(
        mcp_agent_paid_status
            .pointer("/totals/0/pending_minor")
            .and_then(|value| value.as_i64())
            .map(|amount| amount > 0)
            .unwrap_or(false),
        "MCP agent paid status must include pending totals",
    )?;

    Ok(ServiceSmokeReport {
        api_base_url: api.to_string(),
        mcp_base_url: mcp.to_string(),
        bounty_id,
        feed_items: feed_items.len(),
        mcp_tools: tool_list.len(),
        pooled_bounty_id,
        mcp_reviewed_bounty_id: mcp_reviewed_bounty_id.to_string(),
        mcp_bounty_id,
        mcp_solver_id: mcp_solver_id.to_string(),
        mcp_final_status: value_str(&mcp_status, "/bounty/status")
            .unwrap_or("unknown")
            .to_string(),
    })
}

fn print_service_smoke_report(report: &ServiceSmokeReport) -> Result<()> {
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "service_smoke": "ok",
            "api_base_url": report.api_base_url,
            "mcp_base_url": report.mcp_base_url,
            "bounty_id": report.bounty_id,
            "feed_items": report.feed_items,
            "mcp_tools": report.mcp_tools,
            "pooled_bounty_id": report.pooled_bounty_id,
            "mcp_reviewed_bounty_id": report.mcp_reviewed_bounty_id,
            "mcp_bounty_id": report.mcp_bounty_id,
            "mcp_solver_id": report.mcp_solver_id,
            "mcp_final_status": report.mcp_final_status
        }))?
    );
    Ok(())
}

async fn service_smoke_spawn(
    api_base_url: String,
    mcp_base_url: String,
    database_url: Option<String>,
    verify_restart_persistence: bool,
) -> Result<()> {
    let api = normalize_base_url(&api_base_url);
    let mcp = normalize_base_url(&mcp_base_url);
    if verify_restart_persistence && database_url.is_none() {
        bail!("--verify-restart-persistence requires --database-url");
    }
    let api_bind = bind_addr_from_base_url(&api)?;
    let mcp_bind = bind_addr_from_base_url(&mcp)?;
    let api_bin = sibling_binary("api")?;
    let mcp_bin = sibling_binary("mcp-server")?;

    let mut api_child = spawn_service(
        &api_bin,
        &[
            ("API_BIND_ADDR", api_bind.as_str()),
            ("PUBLIC_BASE_URL", api.as_str()),
            ("MCP_BASE_URL", mcp.as_str()),
        ],
        database_url.as_deref(),
    )
    .with_context(|| format!("failed to spawn {}", api_bin.display()))?;
    let mut mcp_child = match spawn_service(
        &mcp_bin,
        &[
            ("MCP_BIND_ADDR", mcp_bind.as_str()),
            ("PUBLIC_BASE_URL", api.as_str()),
            ("MCP_BASE_URL", mcp.as_str()),
        ],
        database_url.as_deref(),
    ) {
        Ok(child) => child,
        Err(error) => {
            stop_child(&mut api_child);
            return Err(error).with_context(|| format!("failed to spawn {}", mcp_bin.display()));
        }
    };

    let result = service_smoke_check(&api, &mcp).await;
    stop_child(&mut api_child);
    stop_child(&mut mcp_child);
    let report = result?;

    if verify_restart_persistence {
        verify_service_smoke_restart_persistence(
            &api,
            &mcp,
            database_url.as_deref().unwrap(),
            &report,
        )?;
    }

    print_service_smoke_report(&report)
}

fn verify_service_smoke_restart_persistence(
    api: &str,
    mcp: &str,
    database_url: &str,
    report: &ServiceSmokeReport,
) -> Result<()> {
    let api_bind = bind_addr_from_base_url(api)?;
    let mcp_bind = bind_addr_from_base_url(mcp)?;
    let api_bin = sibling_binary("api")?;
    let mcp_bin = sibling_binary("mcp-server")?;

    let mut api_child = spawn_service(
        &api_bin,
        &[
            ("API_BIND_ADDR", api_bind.as_str()),
            ("PUBLIC_BASE_URL", api),
            ("MCP_BASE_URL", mcp),
        ],
        Some(database_url),
    )
    .with_context(|| format!("failed to restart {}", api_bin.display()))?;
    let mut mcp_child = match spawn_service(
        &mcp_bin,
        &[
            ("MCP_BIND_ADDR", mcp_bind.as_str()),
            ("PUBLIC_BASE_URL", api),
            ("MCP_BASE_URL", mcp),
        ],
        Some(database_url),
    ) {
        Ok(child) => child,
        Err(error) => {
            stop_child(&mut api_child);
            return Err(error).with_context(|| format!("failed to restart {}", mcp_bin.display()));
        }
    };

    let result = (|| -> Result<()> {
        wait_for_health(&format!("{api}/health"))?;
        wait_for_health(&format!("{mcp}/health"))?;

        let api_eval_runs = get_json(&format!("{api}/v1/evals/runs"))?;
        require(
            contains_eval_suite(&api_eval_runs, "EvalLoops/all-v0"),
            "restarted API must hydrate persisted EvalLoops/all-v0 run history",
        )?;

        let mcp_eval_runs = mcp_tool_get(mcp, "get_eval_runs")?;
        require(
            contains_eval_suite(&mcp_eval_runs, "EvalLoops/all-v0"),
            "restarted MCP must hydrate persisted EvalLoops/all-v0 run history",
        )?;

        let api_bounty_status = get_json(&format!("{api}/v1/bounties/{}", report.bounty_id))?;
        require(
            value_str(&api_bounty_status, "/bounty/status") == Some("Claimable"),
            "restarted API must hydrate API-posted claimable bounty from Postgres",
        )?;

        let mcp_bounty_status = get_json(&format!("{api}/v1/bounties/{}", report.mcp_bounty_id))?;
        require(
            value_str(&mcp_bounty_status, "/bounty/status") == Some("Payable"),
            "restarted API must hydrate MCP-created payable bounty from Postgres",
        )?;
        require(
            mcp_bounty_status
                .pointer("/settlements")
                .and_then(|settlements| settlements.as_array())
                .map(|settlements| !settlements.is_empty())
                .unwrap_or(false),
            "restarted API must hydrate MCP-created settlement records",
        )?;

        let pooled_bounty_status =
            get_json(&format!("{api}/v1/bounties/{}", report.pooled_bounty_id))?;
        let pooled_settlement_id = value_str(&pooled_bounty_status, "/settlements/0/id")
            .context("restarted pooled bounty settlement id missing")?;
        require(
            value_str(&pooled_bounty_status, "/bounty/status") == Some("Paid"),
            "restarted API must hydrate paid pooled bounty from Postgres",
        )?;
        require(
            pooled_bounty_status
                .pointer("/funding_contributions/0/funding_ledger_entry_id")
                .and_then(|value| value.as_str())
                .is_some(),
            "restarted API must hydrate pooled contribution funding ledger linkage",
        )?;
        require(
            value_str(
                &pooled_bounty_status,
                "/funding_contributions/0/settlement_id",
            ) == Some(pooled_settlement_id),
            "restarted API must hydrate pooled contribution settlement linkage",
        )?;

        let reviewed_bounty_status = get_json(&format!(
            "{api}/v1/bounties/{}",
            report.mcp_reviewed_bounty_id
        ))?;
        require(
            value_str(&reviewed_bounty_status, "/bounty/status") == Some("Payable"),
            "restarted API must hydrate MCP-approved reviewed payout bounty from Postgres",
        )?;
        require(
            reviewed_bounty_status
                .pointer("/settlements")
                .and_then(|settlements| settlements.as_array())
                .map(|settlements| !settlements.is_empty())
                .unwrap_or(false),
            "restarted API must hydrate reviewed payout settlement records",
        )?;
        let risk_reviews = get_json(&format!("{api}/v1/risk/reviews"))?;
        require(
            risk_reviews
                .as_array()
                .map(|reviews| {
                    reviews.iter().any(|review| {
                        value_str(review, "/outcome") == Some("Approved")
                            && value_str(review, "/bounty_id")
                                == Some(report.mcp_reviewed_bounty_id.as_str())
                    })
                })
                .unwrap_or(false),
            "restarted API must hydrate risk review records from Postgres",
        )?;

        let feed = get_json(&format!("{api}/v1/bounties/feed"))?;
        let feed_items = feed
            .as_array()
            .context("restarted API bounty feed must be an array")?;
        require(
            feed_items.iter().any(|item| {
                value_str(item, "/bounty_id")
                    .map(|id| id == report.bounty_id.as_str())
                    .unwrap_or(false)
            }),
            "restarted API public feed must include persisted claimable bounty",
        )?;

        let mcp_paid_status = mcp_tool_post(
            mcp,
            "get_paid_status",
            serde_json::json!({ "bounty_id": report.mcp_bounty_id.as_str() }),
        )?;
        require(
            value_str(&mcp_paid_status, "/bounty_status") == Some("Payable"),
            "restarted MCP must hydrate payable status from Postgres",
        )?;
        let api_agent_paid_status = get_json(&format!(
            "{api}/v1/agents/{}/paid-status",
            report.mcp_solver_id
        ))?;
        require_agent_paid_status(
            &api_agent_paid_status,
            "restarted API must hydrate MCP solver payout summary from Postgres",
        )?;

        let mcp_agent_paid_status = mcp_tool_post(
            mcp,
            "get_paid_status",
            serde_json::json!({ "agent_id": report.mcp_solver_id.as_str() }),
        )?;
        require(
            value_str(&mcp_agent_paid_status, "/scope") == Some("agent"),
            "restarted MCP agent paid status must report agent scope",
        )?;
        require_agent_paid_status(
            &mcp_agent_paid_status,
            "restarted MCP must hydrate MCP solver payout summary from Postgres",
        )?;
        Ok(())
    })();

    stop_child(&mut api_child);
    stop_child(&mut mcp_child);
    result
}

fn wait_for_health(url: &str) -> Result<()> {
    for _ in 0..80 {
        if get_text(url).map(|body| body == "ok").unwrap_or(false) {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(250));
    }
    bail!("{url} did not become healthy")
}

fn get_json(url: &str) -> Result<serde_json::Value> {
    Ok(serde_json::from_str(&get_text(url)?)?)
}

fn post_json(url: &str, body: serde_json::Value) -> Result<serde_json::Value> {
    Ok(serde_json::from_str(&http_request(
        "POST",
        url,
        Some(body.to_string()),
    )?)?)
}

fn github_ci_evidence() -> serde_json::Value {
    serde_json::json!({
        "repository": "example/repo",
        "pull_request_url": "https://github.com/example/repo/pull/1",
        "pull_request": {
            "author_login": "solver-agent",
            "merged": true,
            "merged_by_login": "maintainer",
            "reviews": [
                {
                    "author_login": "maintainer",
                    "state": "APPROVED"
                }
            ]
        },
        "commit_sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "check_run": {
            "id": 123456789_u64,
            "name": "full-check",
            "status": "completed",
            "conclusion": "success",
            "head_sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "html_url": "https://github.com/example/repo/actions/runs/123456789",
            "repository": {
                "full_name": "example/repo"
            }
        }
    })
}

fn mcp_tool_get(mcp_base_url: &str, tool_name: &str) -> Result<serde_json::Value> {
    let response = get_json(&format!("{mcp_base_url}/tools/{tool_name}"))?;
    mcp_tool_result(response, tool_name)
}

fn mcp_tool_post(
    mcp_base_url: &str,
    tool_name: &str,
    body: serde_json::Value,
) -> Result<serde_json::Value> {
    let response = post_json(&format!("{mcp_base_url}/tools/{tool_name}"), body)?;
    mcp_tool_result(response, tool_name)
}

fn mcp_tool_result(response: serde_json::Value, tool_name: &str) -> Result<serde_json::Value> {
    if let Some(error) = value_str(&response, "/error") {
        bail!("MCP tool {tool_name} returned error: {error}");
    }
    response
        .pointer("/content/0/json")
        .cloned()
        .with_context(|| format!("MCP tool {tool_name} response missing content[0].json"))
}

fn mcp_reconcile_base_created(
    mcp_base_url: &str,
    bounty_id: &str,
    amount_minor: i64,
    terms_hash: &str,
    onchain_escrow_id: u128,
) -> Result<serde_json::Value> {
    mcp_tool_post(
        mcp_base_url,
        "reconcile_base_escrow_event",
        base_created_event_json(bounty_id, amount_minor, terms_hash, onchain_escrow_id),
    )
}

fn base_created_event_json(
    bounty_id: &str,
    amount_minor: i64,
    terms_hash: &str,
    onchain_escrow_id: u128,
) -> serde_json::Value {
    let tx_seed = Uuid::new_v4().simple().to_string();
    serde_json::json!({
        "id": Uuid::new_v4(),
        "log_key": format!("base:{onchain_escrow_id}:{bounty_id}:created"),
        "tx_hash": format!("0x{tx_seed}{tx_seed}"),
        "block_number": onchain_escrow_id,
        "onchain_escrow_id": onchain_escrow_id,
        "bounty_id": bounty_id,
        "kind": "Created",
        "status": "Funded",
        "token": "0x3333333333333333333333333333333333333333",
        "amount": {
            "amount": amount_minor,
            "currency": "usdc"
        },
        "terms_hash": terms_hash,
        "proof_hash": null,
        "reason_hash": null,
        "dispute_hash": null,
        "occurred_at": Utc::now(),
    })
}

fn get_text(url: &str) -> Result<String> {
    http_request("GET", url, None)
}

async fn production_get_json(client: &reqwest::Client, url: &str) -> Result<serde_json::Value> {
    Ok(serde_json::from_str(
        &production_get_text(client, url).await?,
    )?)
}

async fn production_get_text(client: &reqwest::Client, url: &str) -> Result<String> {
    let response = client
        .get(url)
        .header(
            reqwest::header::ACCEPT,
            "application/json, text/plain, text/html",
        )
        .send()
        .await
        .with_context(|| format!("GET {url} failed"))?;
    let status = response.status();
    require(
        status.is_success(),
        &format!("GET {url} failed with HTTP {status}"),
    )?;
    Ok(response.text().await?)
}

fn http_request(method: &str, url: &str, body: Option<String>) -> Result<String> {
    let parsed = parse_http_url(url)?;
    let body = body.unwrap_or_default();
    let content_headers = if method == "POST" {
        format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            body.len()
        )
    } else {
        String::new()
    };
    let request = format!(
        "{method} {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nAccept: application/json\r\n{}\
         \r\n{}",
        parsed.path, parsed.authority, content_headers, body
    );
    let mut stream = TcpStream::connect((parsed.host.as_str(), parsed.port))
        .with_context(|| format!("failed to connect to {}", parsed.authority))?;
    stream.write_all(request.as_bytes())?;
    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    let (head, body) = response
        .split_once("\r\n\r\n")
        .context("HTTP response missing header separator")?;
    let status_line = head
        .lines()
        .next()
        .context("HTTP response missing status")?;
    require(
        status_line.starts_with("HTTP/1.1 2") || status_line.starts_with("HTTP/1.0 2"),
        &format!("{method} {url} failed with {status_line}"),
    )?;
    if head
        .lines()
        .any(|line| line.eq_ignore_ascii_case("transfer-encoding: chunked"))
    {
        return decode_chunked_body(body);
    }
    Ok(body.to_string())
}

struct ParsedHttpUrl {
    authority: String,
    host: String,
    port: u16,
    path: String,
}

fn parse_http_url(url: &str) -> Result<ParsedHttpUrl> {
    let without_scheme = url
        .strip_prefix("http://")
        .context("service smoke only supports http:// URLs")?;
    let (authority, path) = without_scheme
        .split_once('/')
        .map(|(authority, path)| (authority.to_string(), format!("/{path}")))
        .unwrap_or_else(|| (without_scheme.to_string(), "/".to_string()));
    let (host, port) = if let Some((host, port)) = authority.rsplit_once(':') {
        (
            host.to_string(),
            port.parse::<u16>()
                .with_context(|| format!("invalid port in {url}"))?,
        )
    } else {
        (authority.clone(), 80)
    };
    Ok(ParsedHttpUrl {
        authority,
        host,
        port,
        path,
    })
}

fn decode_chunked_body(body: &str) -> Result<String> {
    let mut rest = body;
    let mut decoded = String::new();
    loop {
        let (size_line, after_size) = rest
            .split_once("\r\n")
            .context("invalid chunked body size line")?;
        let size =
            usize::from_str_radix(size_line.trim(), 16).context("invalid chunked body size")?;
        if size == 0 {
            break;
        }
        if after_size.len() < size + 2 {
            bail!("invalid chunked body length");
        }
        decoded.push_str(&after_size[..size]);
        rest = &after_size[size + 2..];
    }
    Ok(decoded)
}

#[derive(Debug)]
struct DocsContractIssue {
    file: PathBuf,
    line: usize,
    message: String,
}

#[derive(Debug, Clone)]
struct RequestContract {
    required: Vec<&'static str>,
    allowed: Vec<&'static str>,
    numeric_fields: Vec<&'static str>,
}

fn docs_contract_check(root: PathBuf, contract_root: PathBuf) -> Result<()> {
    let root = fs::canonicalize(&root)
        .with_context(|| format!("docs root does not exist: {}", root.display()))?;
    let contract_root = fs::canonicalize(&contract_root)
        .with_context(|| format!("contract root does not exist: {}", contract_root.display()))?;
    let api_routes = load_api_routes(&contract_root)?;
    let mcp_tools = load_mcp_tools(&contract_root)?;
    let request_contracts = request_contracts();
    let mut files = Vec::new();
    collect_doc_files(&root, &mut files)?;

    let mut issues = Vec::new();
    check_agent_quickstart_contract(&root, &mut issues);
    check_production_env_contract(&root, &mut issues);
    check_contributor_first_protocol_contract(&root, &mut issues);
    for file in &files {
        let text = fs::read_to_string(file)
            .with_context(|| format!("failed to read docs file {}", file.display()))?;
        let rel = file.strip_prefix(&root).unwrap_or(file.as_path());
        check_doc_text(
            rel,
            &text,
            &api_routes,
            &mcp_tools,
            &request_contracts,
            &mut issues,
        );
    }

    if !issues.is_empty() {
        for issue in &issues {
            eprintln!("{}:{}: {}", issue.file.display(), issue.line, issue.message);
        }
        bail!("docs contract check failed with {} issue(s)", issues.len());
    }

    println!(
        "docs_contract_check=ok files={} api_routes={} mcp_tools={}",
        files.len(),
        api_routes.len(),
        mcp_tools.len()
    );
    Ok(())
}

fn check_agent_quickstart_contract(root: &Path, issues: &mut Vec<DocsContractIssue>) {
    let path = root.join("docs").join("agent-quickstart.md");
    if !path.exists() {
        push_doc_issue(
            issues,
            &PathBuf::from("docs/agent-quickstart.md"),
            1,
            "agent quickstart is required for autonomous contributor onboarding",
        );
        return;
    }
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) => {
            push_doc_issue(
                issues,
                &PathBuf::from("docs/agent-quickstart.md"),
                1,
                &format!("failed to read agent quickstart: {error}"),
            );
            return;
        }
    };
    for marker in [
        "cargo run -p cli -- demo",
        "cargo run -p cli -- service-smoke-spawn",
        "/.well-known/agent-bounties.json",
        "/llms.txt",
        "route_blocked_goal",
        "register_agent",
        "register_capability",
        "open_pooled_bounty",
        "create_funding_intent",
        "add_bounty_funding",
        "claim_bounty",
        "submit_result",
        "request_verification",
        "get_paid_status",
        "plan_base_funding",
        "reconcile_base_escrow_event",
        "Base Sepolia",
        "testnet",
        "simulated",
        "operator",
        "Copy-paste prompt",
    ] {
        if !text.contains(marker) {
            push_doc_issue(
                issues,
                &PathBuf::from("docs/agent-quickstart.md"),
                1,
                &format!("agent quickstart missing required marker `{marker}`"),
            );
        }
    }
}

fn check_production_env_contract(root: &Path, issues: &mut Vec<DocsContractIssue>) {
    let env_path = root.join(".env.example");
    let compose_path = root.join("docker-compose.production.yml");
    let env_text = match fs::read_to_string(&env_path) {
        Ok(text) => text.replace("\r\n", "\n"),
        Err(error) => {
            push_doc_issue(
                issues,
                &PathBuf::from(".env.example"),
                1,
                &format!("failed to read production env template: {error}"),
            );
            return;
        }
    };
    let compose_text = match fs::read_to_string(&compose_path) {
        Ok(text) => text.replace("\r\n", "\n"),
        Err(error) => {
            push_doc_issue(
                issues,
                &PathBuf::from("docker-compose.production.yml"),
                1,
                &format!("failed to read production compose file: {error}"),
            );
            return;
        }
    };
    let api_block = service_block(&compose_text, "api").unwrap_or_default();
    let mcp_block = service_block(&compose_text, "mcp").unwrap_or_default();
    if api_block.is_empty() {
        push_doc_issue(
            issues,
            &PathBuf::from("docker-compose.production.yml"),
            1,
            "production compose missing api service block",
        );
    }
    if mcp_block.is_empty() {
        push_doc_issue(
            issues,
            &PathBuf::from("docker-compose.production.yml"),
            1,
            "production compose missing mcp service block",
        );
    }

    for name in production_live_money_env_vars() {
        let env_decl = format!("{name}=");
        if !env_text
            .lines()
            .any(|line| line.trim_start().starts_with(&env_decl))
        {
            push_doc_issue(
                issues,
                &PathBuf::from(".env.example"),
                1,
                &format!("production env template missing `{name}`"),
            );
        }
        for (service_name, block) in [("api", api_block.as_str()), ("mcp", mcp_block.as_str())] {
            let compose_decl = format!("{name}:");
            let compose_ref = format!("${{{name}");
            if !block.contains(&compose_decl) || !block.contains(&compose_ref) {
                push_doc_issue(
                    issues,
                    &PathBuf::from("docker-compose.production.yml"),
                    1,
                    &format!("production compose {service_name} service does not pass `{name}`"),
                );
            }
        }
    }
}

fn service_block(compose_text: &str, service_name: &str) -> Option<String> {
    let service_header = format!("  {service_name}:");
    let mut lines = Vec::new();
    let mut in_service = false;
    for line in compose_text.lines() {
        let is_top_level_service =
            line.starts_with("  ") && !line.starts_with("    ") && line.trim_end().ends_with(':');
        if line.trim_end() == service_header {
            in_service = true;
            continue;
        }
        if in_service && is_top_level_service {
            break;
        }
        if in_service {
            lines.push(line);
        }
    }
    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

fn production_live_money_env_vars() -> &'static [&'static str] {
    &[
        "PUBLIC_BASE_URL",
        "MCP_BASE_URL",
        "DATABASE_URL",
        "BASE_SEPOLIA_RPC_URL",
        "BASE_MAINNET_RPC_URL",
        "BASE_SEPOLIA_USDC_TOKEN",
        "BASE_MAINNET_USDC_TOKEN",
        "BASE_SEPOLIA_ESCROW_CONTRACT",
        "BASE_MAINNET_ESCROW_CONTRACT",
        "BASE_SETTLEMENT_SIGNER",
        "BASE_PLATFORM_FEE_WALLET",
        "ENABLE_BASE_TX_BROADCAST",
        "ENABLE_STRIPE_LIVE_EXECUTION",
        "OPERATOR_API_TOKEN",
        "STRIPE_SECRET_KEY",
        "STRIPE_API_BASE_URL",
        "STRIPE_WEBHOOK_SECRET",
        "ALLOW_UNSIGNED_STRIPE_WEBHOOKS",
    ]
}

fn check_contributor_first_protocol_contract(root: &Path, issues: &mut Vec<DocsContractIssue>) {
    check_required_markers(
        root,
        issues,
        &PathBuf::from("docs/contributor-first-maintenance.md"),
        &[
            "Contributor-First Maintainer Protocol",
            "public maintainer notice",
            "open PR queue",
            "collaboration branch",
            "Distribution feedback request",
        ],
        "contributor-first maintainer protocol",
    );
    check_required_markers(
        root,
        issues,
        &PathBuf::from("AGENTS.md"),
        &[
            "docs/contributor-first-maintenance.md",
            "open PRs first",
            "public maintainer notice",
        ],
        "agent contributor guide",
    );
    check_required_markers(
        root,
        issues,
        &PathBuf::from(".github/PULL_REQUEST_TEMPLATE.md"),
        &[
            "Maintainer Change Notice",
            "Notice issue/comment",
            "Open PR queue checked before edits",
            "Active PR impact or repair path",
        ],
        "pull request template",
    );
    check_required_markers(
        root,
        issues,
        &PathBuf::from(".github/ISSUE_TEMPLATE/maintainer-change-notice.yml"),
        &[
            "Maintainer change notice",
            "Open PR queue check",
            "Contributor impact and repair path",
            "Distribution feedback request",
        ],
        "maintainer change notice issue template",
    );
}

fn check_required_markers(
    root: &Path,
    issues: &mut Vec<DocsContractIssue>,
    rel_path: &Path,
    markers: &[&str],
    label: &str,
) {
    let path = root.join(rel_path);
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) => {
            push_doc_issue(
                issues,
                rel_path,
                1,
                &format!("failed to read {label}: {error}"),
            );
            return;
        }
    };
    for marker in markers {
        if !text.contains(marker) {
            push_doc_issue(
                issues,
                rel_path,
                1,
                &format!("{label} missing required marker `{marker}`"),
            );
        }
    }
}

fn collect_doc_files(root: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    if !root.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        let file_name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        if path.is_dir() {
            if matches!(
                file_name,
                ".git" | "target" | "node_modules" | ".next" | "__pycache__"
            ) {
                continue;
            }
            collect_doc_files(&path, files)?;
        } else if is_doc_contract_file(&path) {
            files.push(path);
        }
    }
    files.sort();
    Ok(())
}

fn is_doc_contract_file(path: &Path) -> bool {
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    if matches!(file_name, "README.md" | "AGENTS.md" | "llms.txt") {
        return true;
    }
    matches!(
        path.extension().and_then(|value| value.to_str()),
        Some("md" | "txt")
    )
}

fn load_api_routes(contract_root: &Path) -> Result<BTreeSet<String>> {
    let source_path = contract_root.join("crates/api/src/main.rs");
    let source = fs::read_to_string(&source_path)
        .with_context(|| format!("failed to read {}", source_path.display()))?;
    let mut routes = BTreeSet::new();
    let mut expecting_route = false;
    for line in source.lines() {
        let trimmed = line.trim();
        if let Some(route_start) = trimmed.find(".route(") {
            if let Some(route) = first_string_literal(&trimmed[route_start..]) {
                routes.insert(normalize_route(route));
                expecting_route = false;
            } else {
                expecting_route = true;
            }
            continue;
        }
        if expecting_route {
            if let Some(route) = first_string_literal(trimmed) {
                routes.insert(normalize_route(route));
                expecting_route = false;
            }
        }
    }
    Ok(routes)
}

fn load_mcp_tools(contract_root: &Path) -> Result<BTreeSet<String>> {
    let source_path = contract_root.join("crates/mcp-server/src/main.rs");
    let source = fs::read_to_string(&source_path)
        .with_context(|| format!("failed to read {}", source_path.display()))?;
    let mut tools = BTreeSet::new();
    let mut expecting_name = false;
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed == "tool(" || trimmed == "operator_tool(" {
            expecting_name = true;
            continue;
        }
        if expecting_name {
            if let Some(name) = first_string_literal(trimmed) {
                tools.insert(name.to_string());
                expecting_name = false;
            }
        }
    }
    Ok(tools)
}

fn check_doc_text(
    file: &Path,
    text: &str,
    api_routes: &BTreeSet<String>,
    mcp_tools: &BTreeSet<String>,
    request_contracts: &BTreeMap<String, RequestContract>,
    issues: &mut Vec<DocsContractIssue>,
) {
    for (line_index, line) in text.lines().enumerate() {
        let line_number = line_index + 1;
        if line_mentions_mcp_port_for_api(line) {
            push_doc_issue(
                issues,
                file,
                line_number,
                "REST/API path is pointed at port 8090; API defaults to 8080 and MCP defaults to 8090",
            );
        }
        for alias in stale_discovery_aliases() {
            if line.contains(alias) {
                push_doc_issue(
                    issues,
                    file,
                    line_number,
                    &format!(
                        "stale discovery endpoint `{alias}`; use the manifest `endpoints` object keys"
                    ),
                );
            }
        }
        for tool in tool_names_from_line(line) {
            if !mcp_tools.contains(&tool) {
                push_doc_issue(
                    issues,
                    file,
                    line_number,
                    &format!("unknown MCP tool `{tool}`"),
                );
            }
        }
        for path in api_paths_from_line(line) {
            let normalized = normalize_route(&path);
            if is_checked_api_path(&normalized)
                && !is_external_api_path(&normalized)
                && !api_routes.contains(&normalized)
            {
                push_doc_issue(
                    issues,
                    file,
                    line_number,
                    &format!("unknown API route `{path}`"),
                );
            }
        }
    }

    for (start_line, block) in markdown_code_blocks(text) {
        check_curl_payload_block(file, start_line, &block, request_contracts, issues);
    }
}

fn check_curl_payload_block(
    file: &Path,
    start_line: usize,
    block: &str,
    request_contracts: &BTreeMap<String, RequestContract>,
    issues: &mut Vec<DocsContractIssue>,
) {
    if !block.contains("curl") || !(block.contains("--data") || block.contains(" -d ")) {
        return;
    }
    let Some(path) = block.lines().flat_map(api_paths_from_line).next() else {
        return;
    };
    let normalized = normalize_route(&path);
    let Some(contract) = request_contracts.get(&normalized) else {
        return;
    };
    let Some(json_text) = extract_first_json_object(block) else {
        push_doc_issue(
            issues,
            file,
            start_line,
            &format!("curl payload for `{path}` does not contain a JSON object"),
        );
        return;
    };
    let value: serde_json::Value = match serde_json::from_str(&json_text) {
        Ok(value) => value,
        Err(error) => {
            push_doc_issue(
                issues,
                file,
                start_line,
                &format!("curl payload for `{path}` is not valid JSON: {error}"),
            );
            return;
        }
    };
    let Some(object) = value.as_object() else {
        push_doc_issue(
            issues,
            file,
            start_line,
            &format!("curl payload for `{path}` must be a JSON object"),
        );
        return;
    };

    for field in &contract.required {
        if !object.contains_key(*field) {
            push_doc_issue(
                issues,
                file,
                start_line,
                &format!("curl payload for `{path}` is missing required field `{field}`"),
            );
        }
    }
    for field in object.keys() {
        if !contract.allowed.iter().any(|allowed| allowed == field) {
            push_doc_issue(
                issues,
                file,
                start_line,
                &format!("curl payload for `{path}` contains unknown field `{field}`"),
            );
        }
    }
    for field in &contract.numeric_fields {
        if let Some(value) = object.get(*field) {
            if !value.is_number() {
                push_doc_issue(
                    issues,
                    file,
                    start_line,
                    &format!("curl payload for `{path}` field `{field}` must be numeric"),
                );
            }
        }
    }
}

fn request_contracts() -> BTreeMap<String, RequestContract> {
    let mut contracts = BTreeMap::new();
    insert_request_contract(
        &mut contracts,
        "/v1/agents",
        &["handle"],
        &["handle", "payout_wallet"],
        &[],
    );
    insert_request_contract(
        &mut contracts,
        "/v1/capabilities",
        &[
            "agent_id",
            "class",
            "template_slugs",
            "min_price_minor",
            "max_price_minor",
            "currency",
            "latency_seconds",
            "supported_verifiers",
        ],
        &[
            "agent_id",
            "class",
            "template_slugs",
            "min_price_minor",
            "max_price_minor",
            "currency",
            "latency_seconds",
            "supported_verifiers",
        ],
        &["min_price_minor", "max_price_minor", "latency_seconds"],
    );
    insert_request_contract(
        &mut contracts,
        "/v1/capabilities/search",
        &["query"],
        &["query"],
        &[],
    );
    insert_request_contract(
        &mut contracts,
        "/v1/bounties/{param}/claim",
        &["bounty_id", "solver_agent_id"],
        &["bounty_id", "solver_agent_id"],
        &[],
    );
    insert_request_contract(
        &mut contracts,
        "/v1/bounties/{param}/submit",
        &[
            "bounty_id",
            "solver_agent_id",
            "artifact_uri",
            "artifact_body",
        ],
        &[
            "bounty_id",
            "solver_agent_id",
            "artifact_uri",
            "artifact_body",
        ],
        &[],
    );
    insert_request_contract(
        &mut contracts,
        "/v1/bounties/{param}/verify",
        &["bounty_id", "submission_id", "expected_artifact_digest"],
        &[
            "bounty_id",
            "submission_id",
            "expected_artifact_digest",
            "verifier_kind",
            "rubric",
            "evidence",
            "approved_risk_event_id",
        ],
        &[],
    );
    insert_request_contract(
        &mut contracts,
        "/v1/base/funding-plan",
        &["bounty_id", "escrow_contract", "payer", "token"],
        &["bounty_id", "escrow_contract", "payer", "token", "network"],
        &[],
    );
    insert_request_contract(
        &mut contracts,
        "/v1/base/fetch-rpc-logs",
        &["escrow_contract", "from_block"],
        &[
            "escrow_contract",
            "from_block",
            "to_block",
            "request_id",
            "network",
        ],
        &["from_block", "to_block", "request_id"],
    );
    insert_request_contract(
        &mut contracts,
        "/v1/base/release-queue",
        &[],
        &["escrow_contract", "platform_fee_wallet", "network"],
        &[],
    );
    insert_request_contract(
        &mut contracts,
        "/v1/base/release-plan",
        &["bounty_id", "escrow_contract", "platform_fee_wallet"],
        &[
            "bounty_id",
            "escrow_contract",
            "platform_fee_wallet",
            "network",
        ],
        &[],
    );
    insert_request_contract(
        &mut contracts,
        "/v1/base/refund-plan",
        &["bounty_id", "escrow_contract", "reason_hash"],
        &["bounty_id", "escrow_contract", "reason_hash", "network"],
        &[],
    );
    insert_request_contract(
        &mut contracts,
        "/v1/base/dispute-plan",
        &["bounty_id", "escrow_contract", "dispute_hash"],
        &["bounty_id", "escrow_contract", "dispute_hash", "network"],
        &[],
    );
    insert_request_contract(
        &mut contracts,
        "/v1/base/broadcast-signed-transaction",
        &["signed_transaction"],
        &["signed_transaction", "request_id", "network"],
        &["request_id"],
    );
    insert_request_contract(
        &mut contracts,
        "/v1/base/transaction-receipt",
        &["tx_hash"],
        &["tx_hash", "request_id", "network", "reconcile_logs"],
        &["request_id"],
    );
    contracts
}

fn insert_request_contract(
    contracts: &mut BTreeMap<String, RequestContract>,
    path: &str,
    required: &'static [&'static str],
    allowed: &'static [&'static str],
    numeric_fields: &'static [&'static str],
) {
    contracts.insert(
        normalize_route(path),
        RequestContract {
            required: required.to_vec(),
            allowed: allowed.to_vec(),
            numeric_fields: numeric_fields.to_vec(),
        },
    );
}

fn line_mentions_mcp_port_for_api(line: &str) -> bool {
    (line.contains("localhost:8090") || line.contains("127.0.0.1:8090"))
        && (line.contains("/v1/") || line.contains("/api-docs/") || line.contains("/public/"))
}

fn stale_discovery_aliases() -> &'static [&'static str] {
    &[
        "openapi_url",
        "mcp_url",
        "templates_url",
        "claimable_bounties_url",
        "capabilities_feed_url",
        "public_proofs_url",
    ]
}

fn tool_names_from_line(line: &str) -> Vec<String> {
    let mut names = Vec::new();
    if let Some((_, after)) = line.split_once("Tool:") {
        if let Some(name) = first_identifier(after) {
            names.push(name);
        }
    }

    let lower = line.to_ascii_lowercase();
    if lower.contains("mcp") || lower.contains("tool") {
        for name in backtick_identifiers(line) {
            if looks_like_tool_reference(&name) {
                names.push(name);
            }
        }
    }
    names.sort();
    names.dedup();
    names
}

fn looks_like_tool_reference(value: &str) -> bool {
    const VERBS: &[&str] = &[
        "route_",
        "request_",
        "post_",
        "claim_",
        "submit_",
        "get_",
        "register_",
        "list_",
        "search_",
        "plan_",
        "reconcile_",
        "fetch_",
        "broadcast_",
        "approve_",
        "reject_",
        "execute_",
        "run_",
        "fund_",
    ];
    VERBS.iter().any(|prefix| value.starts_with(prefix))
}

fn backtick_identifiers(line: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut rest = line;
    while let Some(start) = rest.find('`') {
        let after_start = &rest[start + 1..];
        let Some(end) = after_start.find('`') else {
            break;
        };
        let value = &after_start[..end];
        if is_snake_identifier(value) {
            values.push(value.to_string());
        }
        rest = &after_start[end + 1..];
    }
    values
}

fn is_snake_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.contains('_')
        && value
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
}

fn first_identifier(value: &str) -> Option<String> {
    let mut chars = value.trim_start().chars().peekable();
    let mut ident = String::new();
    while let Some(ch) = chars.peek().copied() {
        if ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_' {
            ident.push(ch);
            chars.next();
        } else {
            break;
        }
    }
    if ident.is_empty() {
        None
    } else {
        Some(ident)
    }
}

fn first_string_literal(line: &str) -> Option<&str> {
    let start = line.find('"')?;
    let after_start = &line[start + 1..];
    let end = after_start.find('"')?;
    Some(&after_start[..end])
}

fn api_paths_from_line(line: &str) -> Vec<String> {
    let chars = line.char_indices().collect::<Vec<_>>();
    let mut paths = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        let (byte_index, ch) = chars[index];
        if ch != '/' {
            index += 1;
            continue;
        }
        let prev = if index == 0 {
            None
        } else {
            Some(chars[index - 1].1)
        };
        let next = chars.get(index + 1).map(|(_, ch)| *ch);
        if prev == Some(':') || next == Some('/') {
            index += 1;
            continue;
        }
        let mut end = byte_index + ch.len_utf8();
        let mut cursor = index + 1;
        while let Some((next_byte, next_ch)) = chars.get(cursor).copied() {
            if is_path_char(next_ch) {
                end = next_byte + next_ch.len_utf8();
                cursor += 1;
            } else {
                break;
            }
        }
        let path = trim_path_token(&line[byte_index..end]);
        if is_checked_api_path(&normalize_route(path)) {
            paths.push(path.to_string());
        }
        index = cursor.max(index + 1);
    }
    paths
}

fn is_path_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric()
        || matches!(
            ch,
            '/' | '{' | '}' | ':' | '_' | '-' | '.' | '?' | '=' | '&'
        )
}

fn trim_path_token(value: &str) -> &str {
    value
        .trim_end_matches(['.', ',', ')', ']', '>', ';', ':'])
        .split('?')
        .next()
        .unwrap_or(value)
}

fn is_checked_api_path(path: &str) -> bool {
    path.starts_with("/v1/")
        || path.starts_with("/public/")
        || path.starts_with("/api-docs/")
        || path.starts_with("/.well-known/")
        || path.starts_with("/schemas/")
        || matches!(path, "/llms.txt" | "/docs" | "/health")
}

fn is_external_api_path(path: &str) -> bool {
    matches!(path, "/v1/checkout/sessions")
}

fn normalize_route(path: &str) -> String {
    let trimmed = trim_path_token(path.trim()).trim_end_matches('/');
    if trimmed.is_empty() {
        return "/".to_string();
    }
    let mut normalized = String::new();
    for segment in trimmed.split('/') {
        if segment.is_empty() {
            continue;
        }
        normalized.push('/');
        if segment.starts_with(':') || (segment.starts_with('{') && segment.ends_with('}')) {
            normalized.push_str("{param}");
        } else {
            normalized.push_str(segment);
        }
    }
    if normalized.is_empty() {
        "/".to_string()
    } else {
        normalized
    }
}

fn markdown_code_blocks(text: &str) -> Vec<(usize, String)> {
    let mut blocks = Vec::new();
    let mut in_block = false;
    let mut start_line = 0;
    let mut current = String::new();
    for (index, line) in text.lines().enumerate() {
        if line.trim_start().starts_with("```") {
            if in_block {
                blocks.push((start_line, current.clone()));
                current.clear();
                in_block = false;
            } else {
                in_block = true;
                start_line = index + 2;
            }
            continue;
        }
        if in_block {
            current.push_str(line);
            current.push('\n');
        }
    }
    blocks
}

fn extract_first_json_object(text: &str) -> Option<String> {
    let start = text.find('{')?;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (offset, ch) in text[start..].char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    let end = start + offset + ch.len_utf8();
                    return Some(text[start..end].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

fn push_doc_issue(issues: &mut Vec<DocsContractIssue>, file: &Path, line: usize, message: &str) {
    issues.push(DocsContractIssue {
        file: file.to_path_buf(),
        line,
        message: message.to_string(),
    });
}

fn require(condition: bool, message: &str) -> Result<()> {
    if !condition {
        bail!("{message}");
    }
    Ok(())
}

fn require_base_indexer_status_contract(value: &serde_json::Value, context: &str) -> Result<()> {
    require(
        value
            .pointer("/heartbeat_found")
            .and_then(|field| field.as_bool())
            .is_some(),
        &format!("{context} must expose heartbeat_found boolean"),
    )?;
    require(
        value
            .pointer("/worker_healthy")
            .is_some_and(|field| field.is_boolean() || field.is_null()),
        &format!("{context} must expose nullable worker_healthy boolean"),
    )?;
    for field in [
        "/last_poll_status",
        "/last_poll_started_at",
        "/last_poll_completed_at",
        "/last_poll_skipped_reason",
        "/last_poll_error_message",
        "/heartbeat_updated_at",
    ] {
        require(
            value
                .pointer(field)
                .is_some_and(|field| field.is_string() || field.is_null()),
            &format!("{context} must expose nullable string field {field}"),
        )?;
    }
    for field in [
        "/last_poll_latest_block",
        "/last_poll_confirmed_to_block",
        "/last_poll_from_block",
        "/last_poll_to_block",
        "/last_poll_fetched_logs",
        "/last_poll_persisted_cursor_block",
    ] {
        require(
            value
                .pointer(field)
                .is_some_and(|field| field.as_u64().is_some() || field.is_null()),
            &format!("{context} must expose nullable numeric field {field}"),
        )?;
    }
    require(
        value
            .pointer("/evidence_boundaries")
            .and_then(|field| field.as_array())
            .map(|boundaries| {
                boundaries.iter().any(|boundary| {
                    boundary
                        .as_str()
                        .is_some_and(|text| text.contains("does not fund"))
                }) && boundaries.iter().any(|boundary| {
                    boundary.as_str().is_some_and(|text| {
                        text.contains("heartbeat proves only the last recorded poll outcome")
                    })
                })
            })
            .unwrap_or(false),
        &format!("{context} must state cursor and heartbeat status are not settlement"),
    )
}

fn require_agent_paid_status(value: &serde_json::Value, message: &str) -> Result<()> {
    require(
        value
            .pointer("/payouts")
            .and_then(|payouts| payouts.as_array())
            .map(|payouts| !payouts.is_empty())
            .unwrap_or(false),
        message,
    )?;
    require(
        value
            .pointer("/totals/0/pending_minor")
            .and_then(|amount| amount.as_i64())
            .map(|amount| amount > 0)
            .unwrap_or(false),
        message,
    )
}

fn value_str<'a>(value: &'a serde_json::Value, pointer: &str) -> Option<&'a str> {
    value.pointer(pointer).and_then(serde_json::Value::as_str)
}

fn array_contains_name(value: &serde_json::Value, pointer: &str, expected: &str) -> bool {
    value
        .pointer(pointer)
        .and_then(|items| items.as_array())
        .map(|items| {
            items
                .iter()
                .any(|item| value_str(item, "/name") == Some(expected))
        })
        .unwrap_or(false)
}

fn push_query_param(params: &mut Vec<String>, key: &str, value: Option<String>) {
    if let Some(value) = value {
        params.push(format!("{key}={value}"));
    }
}

fn contains_eval_suite(value: &serde_json::Value, expected_suite: &str) -> bool {
    value.as_array().is_some_and(|runs| {
        runs.iter().any(|run| {
            value_str(run, "/suite")
                .map(|suite| suite == expected_suite)
                .unwrap_or(false)
        })
    })
}

fn normalize_base_url(value: &str) -> String {
    value.trim().trim_end_matches('/').to_string()
}

fn bind_addr_from_base_url(value: &str) -> Result<String> {
    let without_scheme = value
        .strip_prefix("http://")
        .or_else(|| value.strip_prefix("https://"))
        .context("base URL must start with http:// or https://")?;
    let authority = without_scheme
        .split('/')
        .next()
        .context("base URL must include host and port")?;
    if authority.is_empty() {
        bail!("base URL must include host and port");
    }
    Ok(authority.to_string())
}

fn sibling_binary(name: &str) -> Result<PathBuf> {
    let exe_dir = env::current_exe()?
        .parent()
        .context("current executable has no parent directory")?
        .to_path_buf();
    let file_name = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    let path = exe_dir.join(file_name);
    if !path.exists() {
        bail!(
            "{} is missing; run `cargo build -p api -p mcp-server` before service-smoke-spawn",
            path.display()
        );
    }
    Ok(path)
}

fn spawn_service(path: &Path, envs: &[(&str, &str)], database_url: Option<&str>) -> Result<Child> {
    let mut command = ProcessCommand::new(path);
    command.stdin(Stdio::null());
    command.stdout(Stdio::null());
    command.stderr(Stdio::null());
    if let Some(database_url) = database_url {
        command.env("DATABASE_URL", database_url);
    } else {
        command.env_remove("DATABASE_URL");
    }
    for (key, value) in envs {
        command.env(key, value);
    }
    Ok(command.spawn()?)
}

fn stop_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_report_handles_structured_noisy_partial_and_duplicate_records() {
        let report =
            build_discovery_report_from_str(include_str!("../fixtures/discovery_answers.json"))
                .expect("fixture should build a discovery report");

        assert_eq!(report.total_records, 8);
        assert_eq!(report.answered_records, 7);
        assert_eq!(report.partial_answer_records, 1);
        assert_eq!(report.missing_answer_records, 1);
        assert_eq!(report.unique_contributors, 6);
        assert_eq!(
            report.duplicate_contributors,
            vec!["codeboost-tr", "hyperxiaoerxz-hash"]
        );
        assert!(bucket_count(&report.discovery_sources, "github") >= 4);
        assert!(bucket_count(&report.discovery_sources, "machine-discovery") >= 1);
        assert!(bucket_count(&report.participation_reasons, "payout") >= 2);
        assert!(bucket_count(&report.participation_reasons, "clear-scope") >= 3);
        assert!(bucket_count(&report.agent_workflows, "codex") >= 1);
        assert!(bucket_count(&report.trust_payment_signals, "base-usdc-escrow") >= 1);
        assert!(bucket_count(&report.trust_payment_signals, "deterministic-verification") >= 2);
        assert!(bucket_count(&report.friction_points, "stale-docs-or-contract") >= 1);
        assert!(bucket_count(&report.friction_points, "unclear-payment-path") >= 1);
    }

    #[test]
    fn discovery_report_writes_parent_directories() {
        let root = std::env::temp_dir().join(format!(
            "agent-bounties-discovery-report-{}",
            uuid::Uuid::new_v4()
        ));
        let fixture = root.join("input").join("answers.json");
        let json_out = root.join("reports").join("discovery-report.json");
        let markdown_out = root.join("reports").join("discovery-report.md");
        fs::create_dir_all(fixture.parent().expect("fixture parent should exist"))
            .expect("should create temp fixture parent");
        fs::write(&fixture, include_str!("../fixtures/discovery_answers.json"))
            .expect("should write temp fixture");

        discovery_report(
            fixture.to_string_lossy().to_string(),
            Some(json_out.to_string_lossy().to_string()),
            Some(markdown_out.to_string_lossy().to_string()),
        )
        .expect("report command should write outputs");

        let json = fs::read_to_string(&json_out).expect("json report should exist");
        let markdown = fs::read_to_string(&markdown_out).expect("markdown report should exist");
        assert!(json.contains("\"duplicate_contributors\""));
        assert!(markdown.contains("# Contributor Discovery Report"));
        assert!(markdown.contains("base-usdc-escrow"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn production_env_contract_reports_missing_compose_vars() {
        let root = std::env::temp_dir().join(format!(
            "agent-bounties-env-contract-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&root).expect("should create temp root");
        fs::write(
            root.join(".env.example"),
            production_live_money_env_vars()
                .iter()
                .map(|name| format!("{name}=\n"))
                .collect::<String>(),
        )
        .expect("should write temp env template");
        fs::write(
            root.join("docker-compose.production.yml"),
            r#"services:
  api:
    environment:
      PUBLIC_BASE_URL: ${PUBLIC_BASE_URL:?Set PUBLIC_BASE_URL}
      MCP_BASE_URL: ${MCP_BASE_URL:?Set MCP_BASE_URL}
      DATABASE_URL: ${DATABASE_URL:?Set DATABASE_URL}
  mcp:
    environment:
      PUBLIC_BASE_URL: ${PUBLIC_BASE_URL:?Set PUBLIC_BASE_URL}
      MCP_BASE_URL: ${MCP_BASE_URL:?Set MCP_BASE_URL}
      DATABASE_URL: ${DATABASE_URL:?Set DATABASE_URL}
"#,
        )
        .expect("should write temp compose file");

        let mut issues = Vec::new();
        check_production_env_contract(&root, &mut issues);

        assert!(issues.iter().any(|issue| {
            issue.message.contains(
                "production compose api service does not pass `BASE_SEPOLIA_ESCROW_CONTRACT`",
            )
        }));
        assert!(issues.iter().any(|issue| {
            issue
                .message
                .contains("production compose mcp service does not pass `STRIPE_WEBHOOK_SECRET`")
        }));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn contributor_first_protocol_contract_reports_missing_artifacts() {
        let root = std::env::temp_dir().join(format!(
            "agent-bounties-contributor-first-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(root.join(".github").join("ISSUE_TEMPLATE"))
            .expect("should create temp github template dir");
        fs::create_dir_all(root.join("docs")).expect("should create temp docs dir");
        fs::write(
            root.join("docs").join("contributor-first-maintenance.md"),
            "# Contributor-First Maintainer Protocol\npublic maintainer notice\nopen PR queue\n",
        )
        .expect("should write partial protocol doc");
        fs::write(root.join("AGENTS.md"), "public maintainer notice\n")
            .expect("should write partial agents file");
        fs::write(
            root.join(".github").join("PULL_REQUEST_TEMPLATE.md"),
            "## Maintainer Change Notice\nNotice issue/comment\n",
        )
        .expect("should write partial PR template");
        fs::write(
            root.join(".github")
                .join("ISSUE_TEMPLATE")
                .join("maintainer-change-notice.yml"),
            "name: Maintainer change notice\nOpen PR queue check\n",
        )
        .expect("should write partial issue template");

        let mut issues = Vec::new();
        check_contributor_first_protocol_contract(&root, &mut issues);

        assert!(issues.iter().any(|issue| {
            issue
                .message
                .contains("contributor-first maintainer protocol missing required marker `collaboration branch`")
        }));
        assert!(issues.iter().any(|issue| {
            issue.message.contains(
                "agent contributor guide missing required marker `docs/contributor-first-maintenance.md`",
            )
        }));
        assert!(issues.iter().any(|issue| {
            issue
                .message
                .contains("pull request template missing required marker `Open PR queue checked before edits`")
        }));
        assert!(issues.iter().any(|issue| {
            issue.message.contains(
                "maintainer change notice issue template missing required marker `Contributor impact and repair path`",
            )
        }));

        let _ = fs::remove_dir_all(root);
    }

    fn bucket_count(buckets: &[ContributorDiscoveryReportBucket], name: &str) -> usize {
        buckets
            .iter()
            .find(|bucket| bucket.name == name)
            .map(|bucket| bucket.count)
            .unwrap_or_default()
    }
}
