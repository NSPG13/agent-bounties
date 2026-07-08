use anyhow::{bail, Context, Result};
use app::{
    hash_artifact, BaseReleaseQueueRequest, BountyNetwork, ClaimBountyRequest,
    CreateHelpRequestRequest, FundQuoteRequest, PostBountyRequest, RegisterAgentRequest,
    RegisterCapabilityRequest, RequestQuotesRequest, SubmitResultRequest, VerifySubmissionRequest,
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
use clap::{Parser, Subcommand};
use domain::{CapabilityClass, FundingMode, Money, PrivacyLevel, VerifierKind};
use eval_harness::{
    bundled_abuse_fixtures, bundled_fixtures, bundled_judge_fixtures, run_eval_loops, AbuseBench,
    BountyBench, JudgeBench,
};
use github_app::{
    bounty_check_output, parse_issue_form_bounty, proof_check_output, proof_comment_fingerprint,
    GitHubProofComment,
};
use payments_stripe::{
    execute_stripe_request, CheckoutTopUpRequest, StripePlanner, STRIPE_API_BASE_URL,
};
use std::{
    env, fs,
    io::{Read, Write},
    net::TcpStream,
    path::{Path, PathBuf},
    process::{Child, Command as ProcessCommand, Stdio},
    thread,
    time::Duration,
};
use uuid::Uuid;

#[derive(Parser)]
#[command(name = "agent-bounties")]
#[command(about = "Open-source agent bounty network CLI")]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Demo,
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

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    match args.command {
        Command::Demo => demo().await,
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
        Command::GithubPlan {
            repository,
            issue_url,
            title,
            body_file,
        } => github_plan(repository, issue_url, title, body_file),
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
    let markdown = comment.markdown();
    let fingerprint = proof_comment_fingerprint(&comment);
    let check = proof_check_output(&comment);
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "comment": comment,
            "markdown": markdown,
            "fingerprint": fingerprint,
            "check": check
        }))?
    );
    Ok(())
}

fn discovery(public_base_url: String, mcp_base_url: String) -> Result<()> {
    let manifest = web_public::discovery_manifest(&public_base_url, &mcp_base_url);
    println!("{}", serde_json::to_string_pretty(&manifest)?);
    Ok(())
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
        "/endpoints/templates",
        "/endpoints/bounty_feed",
        "/endpoints/capability_feed",
        "/endpoints/eval_runs",
        "/endpoints/risk_policy",
        "/endpoints/risk_events",
        "/endpoints/risk_reviews",
        "/endpoints/base_release_queue",
        "/endpoints/base_funding_plan",
        "/endpoints/risk_payout_approvals",
        "/endpoints/base_broadcast_signed_transaction",
        "/endpoints/base_transaction_receipt",
        "/endpoints/stripe_live_checkout_top_ups",
        "/endpoints/stripe_live_connect_accounts",
        "/endpoints/github_issue_bounty_plan",
        "/endpoints/github_proof_comment_plan",
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
            })
            .unwrap_or(false),
        "discovery schema must require agent entrypoints and payment rails",
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
                        .any(|value| value.as_str() == Some("discovery_schema"))
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
        "list_base_release_queue",
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
        "Risk policy",
        "AI judges",
        "Stripe live execution is gated",
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
        "/v1/stripe/live/connect-accounts",
        "/v1/stripe/connect-snapshots",
        "/v1/stripe/checkout-webhooks",
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
            && template_page
                .contains("https://github.com/agent-bounties/agent-bounties/issues/new?template=paid-bounty.yml"),
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
            && public_bounties.contains("Machine-readable feed"),
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
        "claim_bounty",
        "submit_result",
        "request_verification",
        "get_paid_status",
        "plan_base_funding",
        "list_base_release_queue",
        "execute_stripe_checkout_top_up",
        "execute_stripe_connect_account",
        "list_risk_events",
        "list_risk_reviews",
        "approve_risk_bounty",
        "approve_risk_payout",
        "reject_risk_event",
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
        "reconcile_stripe_connect_snapshot",
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
        discovery.pointer("/endpoints/capability_feed").is_some(),
        "discovery manifest must include capability feed",
    )?;
    require(
        discovery.pointer("/endpoints/risk_events").is_some(),
        "discovery manifest must include risk review events endpoint",
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
            .pointer("/endpoints/github_proof_comment_plan")
            .is_some(),
        "discovery manifest must include GitHub proof comment planning",
    )?;
    let api_llms = get_text(&format!("{api}/llms.txt"))?;
    require(
        api_llms.contains("route_blocked_goal")
            && api_llms.contains("/.well-known/agent-bounties.json"),
        "API llms.txt must orient agents to discovery and routing",
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
        "plan_stripe_checkout_top_up",
        "plan_stripe_connect_account",
        "execute_stripe_checkout_top_up",
        "execute_stripe_connect_account",
        "plan_github_issue_bounty",
        "plan_github_proof_comment",
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
            "artifact_uri": "https://github.com/example/repo/actions/runs/1",
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
            "evidence": {
                "check_conclusion": "success",
                "check_name": "test"
            },
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
            "evidence": {
                "check_conclusion": "success",
                "check_name": "test"
            },
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

fn require(condition: bool, message: &str) -> Result<()> {
    if !condition {
        bail!("{message}");
    }
    Ok(())
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
