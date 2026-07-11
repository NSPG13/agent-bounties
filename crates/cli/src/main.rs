use anyhow::{anyhow, bail, Context, Result};
use app::{
    build_live_money_readiness_report, hash_artifact, stripe_secret_key_mode_from_secret,
    AddFundingContributionRequest, BountyNetwork, ClaimBountyRequest, CreateHelpRequestRequest,
    FundQuoteRequest, LiveMoneyReadinessConfig, OpenPooledBountyRequest, RegisterAgentRequest,
    RegisterCapabilityRequest, RequestQuotesRequest, SubmitResultRequest, VerifySubmissionRequest,
};
use chain_base::{
    base_network_descriptor, broadcast_signed_transaction, eth_get_transaction_receipt_request,
    eth_send_raw_transaction_request, fetch_transaction_receipt, BaseRpcUrlConfig,
};
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
    execute_stripe_request, CheckoutTopUpRequest, StripePlanner, StripeRequestIntent,
    STRIPE_API_BASE_URL,
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
        funding_mode: Some(FundingMode::Simulated),
    })?;

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
    let status = network.status(bounty.id)?;

    println!("demo_status={:?}", status.bounty.status);
    println!("template={}", status.bounty.template_slug);
    println!("quotes={}", quote_set.quotes.len());
    println!("proof={}", proof.proof_hash);
    println!("ledger_entries={}", network.ledger.entries().len());
    println!("settlements={}", status.settlements.len());
    println!("reputation_events={}", status.reputation_events.len());
    println!("template_signals={}", status.template_signals.len());
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
            "next_step": "Use the receipt only to confirm inclusion. The autonomous indexer independently reconciles canonical factory and bounty logs; a receipt alone never proves settlement."
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
    verification_modes: usize,
    payment_rails: usize,
    claimable_requirements: usize,
    evidence_boundaries: usize,
    eval_runs: usize,
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
            .is_some_and(|schema| schema.ends_with("discovery-manifest.v2.json")),
        "discovery manifest must expose the v2 schema",
    )?;
    require(
        value_str(&discovery, "/protocol/version") == Some("agent-bounties/autonomous-v1"),
        "discovery manifest must advertise autonomous-v1",
    )?;
    require(
        discovery
            .pointer("/protocol/operator_settlement_signer")
            .and_then(|value| value.as_bool())
            == Some(false),
        "autonomous-v1 must not advertise an operator settlement signer",
    )?;
    require(
        value_str(&discovery, "/protocol/payout_authority")
            .is_some_and(|authority| authority.contains("BountySettled")),
        "discovery manifest must bind payout evidence to BountySettled",
    )?;
    require(
        value_str(&discovery, "/endpoints/api_base") == Some(api),
        "discovery manifest api_base must match the checked API URL",
    )?;
    require(
        value_str(&discovery, "/endpoints/mcp_tools").is_some_and(|url| url.starts_with(mcp)),
        "discovery manifest must point MCP tools at the checked MCP URL",
    )?;

    let autonomous_endpoints = [
        "protocol_status",
        "autonomous_terms_publish",
        "autonomous_terms_get",
        "autonomous_submission_evidence_publish",
        "autonomous_submission_evidence_get",
        "autonomous_bounty_feed",
        "autonomous_verification_jobs",
        "autonomous_events",
        "autonomous_creation_plan",
        "autonomous_authorized_creation_plan",
        "autonomous_contribution_plan",
        "autonomous_authorized_contribution_plan",
        "autonomous_claim_plan",
        "autonomous_authorized_claim_plan",
        "autonomous_submission_plan",
        "autonomous_verification_attestation_plan",
        "autonomous_module_settlement_plan",
        "autonomous_attestation_settlement_plan",
        "autonomous_expire_claim_plan",
        "autonomous_expire_submission_plan",
        "autonomous_cancel_plan",
        "autonomous_refund_withdrawal_plan",
    ];
    for endpoint in autonomous_endpoints {
        require(
            value_str(&discovery, &format!("/endpoints/{endpoint}")).is_some(),
            &format!("discovery manifest missing autonomous endpoint {endpoint}"),
        )?;
    }
    for retired in [
        "base_escrow_events",
        "base_indexer_status",
        "base_funding_plan",
        "base_release_queue",
        "base_refund_plan",
        "base_dispute_plan",
    ] {
        require(
            discovery
                .pointer(&format!("/endpoints/{retired}"))
                .is_none(),
            &format!("retired V1 endpoint leaked into discovery: {retired}"),
        )?;
    }

    let agent_tools = discovery
        .pointer("/agent_tools")
        .and_then(|value| value.as_array())
        .context("discovery manifest agent_tools must be an array")?;
    for expected in [
        "list_autonomous_bounties",
        "list_autonomous_verification_jobs",
        "publish_autonomous_bounty_terms",
        "plan_autonomous_bounty_creation",
        "plan_autonomous_bounty_contribution",
        "plan_autonomous_bounty_claim",
        "plan_autonomous_bounty_submission",
        "plan_autonomous_module_settlement",
        "plan_autonomous_attestation_settlement",
        "list_autonomous_bounty_events",
    ] {
        require(
            agent_tools
                .iter()
                .any(|tool| tool.as_str() == Some(expected)),
            &format!("discovery manifest missing autonomous agent tool {expected}"),
        )?;
    }
    require(
        agent_tools.iter().all(|tool| {
            tool.as_str().is_none_or(|name| {
                !name.starts_with("plan_base_") && !name.starts_with("reconcile_base_")
            })
        }),
        "retired V1 Base tools must not be advertised",
    )?;

    let verification_modes = discovery
        .pointer("/verification_modes")
        .and_then(|value| value.as_array())
        .context("discovery manifest verification_modes must be an array")?;
    for mode in ["deterministic_module", "signed_quorum", "ai_judge_quorum"] {
        require(
            verification_modes
                .iter()
                .any(|value| value_str(value, "/name") == Some(mode)),
            &format!("discovery manifest missing verification mode {mode}"),
        )?;
    }
    let payment_rails = discovery
        .pointer("/payment_rails")
        .and_then(|value| value.as_array())
        .context("discovery manifest payment_rails must be an array")?;
    require(
        payment_rails
            .iter()
            .any(|rail| value_str(rail, "/name") == Some("Base native USDC")),
        "discovery manifest must advertise Base native USDC",
    )?;
    require(
        payment_rails.iter().all(|rail| {
            rail.pointer("/funding_required_before_claim")
                .and_then(|value| value.as_bool())
                == Some(true)
        }),
        "all advertised payment rails must require funding before claim",
    )?;
    let claimable_requirements = discovery
        .pointer("/claimable_requirements")
        .and_then(|value| value.as_array())
        .context("discovery manifest claimable_requirements must be an array")?;
    require(
        claimable_requirements.len() >= 5,
        "discovery manifest must expose autonomous claim safety requirements",
    )?;
    let evidence_boundaries = discovery
        .pointer("/evidence_boundaries")
        .and_then(|value| value.as_array())
        .context("discovery manifest evidence_boundaries must be an array")?;
    require(
        evidence_boundaries.iter().any(|boundary| {
            boundary
                .as_str()
                .is_some_and(|text| text.contains("BountySettled"))
        }),
        "discovery manifest must identify canonical payout evidence",
    )?;
    require(
        value_str(&discovery, "/post_value_loop/default_cta") == Some("Post your own bounty"),
        "post-value loop must default to Post your own bounty",
    )?;
    let post_value_actions = discovery
        .pointer("/post_value_loop/actions")
        .and_then(|value| value.as_array())
        .context("post-value loop actions must be an array")?;
    for kind in [
        "share_verified_value",
        "tell_your_human",
        "star_upvote_repo",
        "post_own_bounty",
    ] {
        require(
            post_value_actions
                .iter()
                .any(|action| value_str(action, "/kind") == Some(kind)),
            &format!("post-value loop missing action {kind}"),
        )?;
    }

    let discovery_schema_url = value_str(&discovery, "/endpoints/discovery_schema")
        .context("discovery schema url missing")?;
    let discovery_schema = production_get_json(&client, discovery_schema_url).await?;
    require(
        value_str(&discovery_schema, "/$id") == value_str(&discovery, "/schema"),
        "discovery schema $id must match manifest schema id",
    )?;
    let schema_endpoint_requirements = discovery_schema
        .pointer("/properties/endpoints/required")
        .and_then(|value| value.as_array())
        .context("discovery schema must require autonomous endpoints")?;
    for endpoint in autonomous_endpoints {
        require(
            schema_endpoint_requirements
                .iter()
                .any(|value| value.as_str() == Some(endpoint)),
            &format!("discovery schema must require {endpoint}"),
        )?;
    }

    let mcp_schema = production_get_json(
        &client,
        &format!("{mcp}/schemas/discovery-manifest.v2.json"),
    )
    .await?;
    require(
        value_str(&mcp_schema, "/$id") == value_str(&discovery_schema, "/$id"),
        "MCP discovery schema endpoint must serve the v2 manifest schema",
    )?;
    let mcp_discovery =
        production_get_json(&client, &format!("{mcp}/.well-known/agent-bounties.json")).await?;
    require(
        value_str(&mcp_discovery, "/protocol/version") == Some("agent-bounties/autonomous-v1"),
        "MCP discovery must expose autonomous-v1",
    )?;

    let openapi_url =
        value_str(&discovery, "/endpoints/openapi_json").context("openapi url missing")?;
    let openapi = production_get_json(&client, openapi_url).await?;
    let paths = openapi
        .pointer("/paths")
        .and_then(|value| value.as_object())
        .context("OpenAPI must include paths")?;
    for path in [
        "/v1/base/autonomous-bounties/creation-plan",
        "/v1/base/autonomous-bounties/authorized-creation-plan",
        "/v1/base/autonomous-bounties/contribution-plan",
        "/v1/base/autonomous-bounties/authorized-contribution-plan",
        "/v1/base/autonomous-bounties/claim-plan",
        "/v1/base/autonomous-bounties/authorized-claim-plan",
        "/v1/base/autonomous-bounties/submission-plan",
        "/v1/base/autonomous-bounties/verification-attestation-plan",
        "/v1/base/autonomous-bounties/module-settlement-plan",
        "/v1/base/autonomous-bounties/attestation-settlement-plan",
        "/v1/base/autonomous-bounties/expire-claim-plan",
        "/v1/base/autonomous-bounties/expire-submission-plan",
        "/v1/base/autonomous-bounties/cancel-plan",
        "/v1/base/autonomous-bounties/refund-withdrawal-plan",
        "/v1/base/autonomous-bounties/decode-events",
        "/v1/base/autonomous-bounties/events",
        "/v1/base/autonomous-bounties/terms",
        "/v1/base/autonomous-bounties/submission-evidence",
        "/v1/base/autonomous-bounties/feed",
        "/v1/base/autonomous-bounties/verification-jobs",
        "/v1/base/transaction-receipt",
    ] {
        require(
            paths.contains_key(path),
            &format!("OpenAPI missing autonomous path {path}"),
        )?;
    }
    for retired in [
        "/v1/base/indexer-status",
        "/v1/base/escrow-events",
        "/v1/base/evm-logs",
        "/v1/base/log-query",
        "/v1/base/rpc-logs",
        "/v1/base/fetch-rpc-logs",
        "/v1/base/funding-plan",
        "/v1/base/release-queue",
        "/v1/base/release-plan",
        "/v1/base/refund-plan",
        "/v1/base/dispute-plan",
    ] {
        require(
            !paths.contains_key(retired),
            &format!("retired V1 path leaked into OpenAPI: {retired}"),
        )?;
    }
    let receipt_operation = paths
        .get("/v1/base/transaction-receipt")
        .and_then(|path| path.get("post"))
        .context("OpenAPI receipt operation missing")?;
    require(
        receipt_operation.get("security").is_none(),
        "transaction receipt reads must not advertise operator authorization",
    )?;

    let api_llms = production_get_text(&client, &format!("{api}/llms.txt")).await?;
    for expected in [
        "agent-bounties/autonomous-v1",
        "list_autonomous_bounties",
        "plan_autonomous_bounty_creation",
        "BountySettled",
        "Post your own bounty",
    ] {
        require(
            api_llms.contains(expected),
            &format!("API llms.txt missing {expected}"),
        )?;
    }
    let mcp_llms = production_get_text(&client, &format!("{mcp}/llms.txt")).await?;
    require(
        mcp_llms.contains("MCP tools") && mcp_llms.contains("list_autonomous_bounties"),
        "MCP llms.txt must orient agents to autonomous tools",
    )?;

    let mcp_tools_url =
        value_str(&discovery, "/endpoints/mcp_tools").context("MCP tools url missing")?;
    let tools = production_get_json(&client, mcp_tools_url).await?;
    let tool_list = tools.as_array().context("MCP tools must be an array")?;
    for tool in tool_list {
        let name = value_str(tool, "/name").unwrap_or("<unnamed>");
        require(
            tool.pointer("/input_schema/type").is_some(),
            &format!("MCP tool {name} missing input_schema.type"),
        )?;
    }
    for expected in [
        "plan_autonomous_bounty_creation",
        "plan_autonomous_bounty_authorized_creation",
        "plan_autonomous_bounty_contribution",
        "plan_autonomous_bounty_authorized_contribution",
        "plan_autonomous_bounty_claim",
        "plan_autonomous_bounty_authorized_claim",
        "plan_autonomous_bounty_submission",
        "plan_autonomous_verification_attestation",
        "plan_autonomous_module_settlement",
        "plan_autonomous_attestation_settlement",
        "decode_autonomous_bounty_events",
        "list_autonomous_bounty_events",
        "publish_autonomous_bounty_terms",
        "get_autonomous_bounty_terms",
        "publish_autonomous_submission_evidence",
        "get_autonomous_submission_evidence",
        "list_autonomous_bounties",
        "list_autonomous_verification_jobs",
    ] {
        require(
            tool_list
                .iter()
                .any(|tool| value_str(tool, "/name") == Some(expected)),
            &format!("MCP tool list missing {expected}"),
        )?;
    }
    for retired in [
        "plan_base_log_query",
        "reconcile_base_escrow_event",
        "reconcile_base_evm_logs",
        "reconcile_base_rpc_logs",
        "fetch_base_rpc_logs",
        "get_base_indexer_status",
        "plan_base_funding",
        "plan_base_release",
        "plan_base_refund",
        "plan_base_dispute",
        "list_base_release_queue",
    ] {
        require(
            tool_list
                .iter()
                .all(|tool| value_str(tool, "/name") != Some(retired)),
            &format!("retired V1 MCP tool leaked: {retired}"),
        )?;
    }
    let receipt_tool = tool_list
        .iter()
        .find(|tool| value_str(tool, "/name") == Some("get_base_transaction_receipt"))
        .context("MCP receipt tool missing")?;
    require(
        receipt_tool
            .pointer("/input_schema/properties/reconcile_logs")
            .is_none()
            && receipt_tool.pointer("/authorization").is_none(),
        "MCP receipt tool must be read-only and unauthenticated",
    )?;

    let eval_runs = production_get_json(&client, &format!("{api}/v1/evals/runs")).await?;
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

    Ok(ProductionSmokeReport {
        api_base_url: api.to_string(),
        mcp_base_url: mcp.to_string(),
        verification_modes: verification_modes.len(),
        payment_rails: payment_rails.len(),
        claimable_requirements: claimable_requirements.len(),
        evidence_boundaries: evidence_boundaries.len(),
        eval_runs: eval_run_count,
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
            "verification_modes": report.verification_modes,
            "payment_rails": report.payment_rails,
            "claimable_requirements": report.claimable_requirements,
            "evidence_boundaries": report.evidence_boundaries,
            "eval_runs": report.eval_runs,
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
    paid_bounty_id: String,
    solver_id: String,
    final_status: String,
    autonomous_events: usize,
    mcp_tools: usize,
}

async fn service_smoke_check(api: &str, mcp: &str) -> Result<ServiceSmokeReport> {
    wait_for_health(&format!("{api}/health"))?;
    wait_for_health(&format!("{mcp}/health"))?;
    let _production_contract = production_smoke_check(api, mcp, false).await?;

    let discovery = get_json(&format!("{api}/.well-known/agent-bounties.json"))?;
    require(
        value_str(&discovery, "/protocol/version") == Some("agent-bounties/autonomous-v1"),
        "service discovery must expose autonomous-v1",
    )?;
    require(
        discovery
            .pointer("/endpoints/autonomous_creation_plan")
            .is_some()
            && discovery
                .pointer("/endpoints/autonomous_verification_jobs")
                .is_some()
            && discovery.pointer("/endpoints/autonomous_events").is_some(),
        "service discovery must expose autonomous creation, verification, and event surfaces",
    )?;

    let route = post_json(
        &format!("{api}/v1/route-blocked-goal"),
        serde_json::json!({
            "goal": "Fix the failing autonomous protocol smoke check",
            "context": "The task has deterministic acceptance criteria and should route to a coding bounty.",
            "budget_minor": 1_000_000,
            "currency": "usdc",
            "privacy": "Public"
        }),
    )?;
    require(
        route.pointer("/capability_class").is_some(),
        "route response must include capability_class",
    )?;

    let decoded_events = post_json(
        &format!("{api}/v1/base/autonomous-bounties/decode-events"),
        serde_json::json!({ "logs": [] }),
    )?;
    require(
        decoded_events.as_array().is_some_and(Vec::is_empty),
        "autonomous event decoder must accept an empty confirmed-log batch",
    )?;
    let indexed_events = get_json(&format!(
        "{api}/v1/base/autonomous-bounties/events?network=base-mainnet"
    ))?;
    let autonomous_event_count = indexed_events
        .as_array()
        .context("autonomous event feed must be an array")?
        .len();

    let smoke_id = Uuid::new_v4();
    let solver = post_json(
        &format!("{api}/v1/agents"),
        serde_json::json!({
            "handle": format!("autonomous-service-smoke-solver-{smoke_id}"),
            "payout_wallet": "0x2222222222222222222222222222222222222222"
        }),
    )?;
    let solver_id = value_str(&solver, "/id")
        .context("service smoke solver id missing")?
        .to_string();

    let bounty = post_json(
        &format!("{api}/v1/bounties/pooled"),
        serde_json::json!({
            "title": "Autonomous protocol local paid-loop smoke",
            "template_slug": "extract-data-to-schema",
            "target_amount_minor": 1_000,
            "currency": "usdc",
            "funding_mode": "Simulated",
            "privacy": "Public"
        }),
    )?;
    let bounty_id = value_str(&bounty, "/id")
        .context("service smoke bounty id missing")?
        .to_string();
    let funding = post_json(
        &format!("{api}/v1/bounties/{bounty_id}/funding-contributions"),
        serde_json::json!({
            "bounty_id": bounty_id,
            "contributor_agent_id": null,
            "source_organization_id": null,
            "amount_minor": 1_000,
            "currency": "usdc",
            "rail": "Simulated",
            "external_reference": format!("autonomous-service-smoke-{smoke_id}")
        }),
    )?;
    require(
        value_str(&funding, "/bounty/status") == Some("Claimable"),
        "simulated funding must make the local smoke bounty claimable",
    )?;
    let claim = post_json(
        &format!("{api}/v1/bounties/{bounty_id}/claim"),
        serde_json::json!({
            "bounty_id": bounty_id,
            "solver_agent_id": solver_id
        }),
    )?;
    require(
        value_str(&claim, "/status") == Some("Claimed"),
        "local smoke bounty must become claimed",
    )?;
    let artifact_body = "{\"autonomous_service_smoke\":true}";
    let submission = post_json(
        &format!("{api}/v1/bounties/{bounty_id}/submit"),
        serde_json::json!({
            "bounty_id": bounty_id,
            "solver_agent_id": solver_id,
            "artifact_uri": "memory://autonomous-service-smoke/artifact.json",
            "artifact_body": artifact_body
        }),
    )?;
    let submission_id =
        value_str(&submission, "/id").context("service smoke submission id missing")?;
    let proof = post_json(
        &format!("{api}/v1/bounties/{bounty_id}/verify"),
        serde_json::json!({
            "bounty_id": bounty_id,
            "submission_id": submission_id,
            "expected_artifact_digest": hash_artifact(artifact_body),
            "verifier_kind": "JsonSchema",
            "rubric": null,
            "evidence": null,
            "approved_risk_event_id": null
        }),
    )?;
    require(
        value_str(&proof, "/proof_hash").is_some(),
        "local smoke verification must return a proof hash",
    )?;
    let status = get_json(&format!("{api}/v1/bounties/{bounty_id}"))?;
    let final_status = value_str(&status, "/bounty/status")
        .context("service smoke final bounty status missing")?
        .to_string();
    require(
        final_status == "Paid",
        "simulated local bounty loop must finish Paid",
    )?;

    let tools = get_json(&format!("{mcp}/tools"))?;
    let tool_list = tools.as_array().context("MCP tools must be an array")?;
    let mcp_route = mcp_tool_post(
        mcp,
        "route_blocked_goal",
        serde_json::json!({
            "goal": "Fix an autonomous MCP lifecycle failure",
            "context": "A deterministic digital task needs a funded bounty.",
            "budget_minor": 1_000_000,
            "currency": "usdc",
            "privacy": "Public"
        }),
    )?;
    require(
        mcp_route.pointer("/capability_class").is_some(),
        "MCP route_blocked_goal must return capability_class",
    )?;
    let mcp_decoded = mcp_tool_post(
        mcp,
        "decode_autonomous_bounty_events",
        serde_json::json!({ "logs": [] }),
    )?;
    require(
        mcp_decoded.as_array().is_some_and(Vec::is_empty),
        "MCP autonomous event decoder must accept an empty log batch",
    )?;
    let mcp_events = mcp_tool_post(
        mcp,
        "list_autonomous_bounty_events",
        serde_json::json!({ "network": "base-mainnet", "bounty_id": null }),
    )?;
    require(
        mcp_events.as_array().is_some(),
        "MCP autonomous event listing must return an array",
    )?;
    let eval_loops = mcp_tool_get(mcp, "run_eval_loops")?;
    require(
        eval_loops
            .pointer("/passed")
            .and_then(|value| value.as_bool())
            == Some(true),
        "MCP eval loops must pass during service smoke",
    )?;

    Ok(ServiceSmokeReport {
        api_base_url: api.to_string(),
        mcp_base_url: mcp.to_string(),
        paid_bounty_id: bounty_id,
        solver_id,
        final_status,
        autonomous_events: autonomous_event_count,
        mcp_tools: tool_list.len(),
    })
}

fn print_service_smoke_report(report: &ServiceSmokeReport) -> Result<()> {
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "service_smoke": "ok",
            "api_base_url": report.api_base_url,
            "mcp_base_url": report.mcp_base_url,
            "paid_bounty_id": report.paid_bounty_id,
            "solver_id": report.solver_id,
            "final_status": report.final_status,
            "autonomous_events": report.autonomous_events,
            "mcp_tools": report.mcp_tools
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

        let api_bounty_status = get_json(&format!("{api}/v1/bounties/{}", report.paid_bounty_id))?;
        require(
            value_str(&api_bounty_status, "/bounty/status") == Some("Paid"),
            "restarted API must hydrate the paid local-loop bounty from Postgres",
        )?;
        require(
            api_bounty_status
                .pointer("/settlements")
                .and_then(|settlements| settlements.as_array())
                .map(|settlements| !settlements.is_empty())
                .unwrap_or(false),
            "restarted API must hydrate settlement records for the paid local loop",
        )?;
        require(
            api_bounty_status
                .pointer("/funding_contributions/0/funding_ledger_entry_id")
                .and_then(|value| value.as_str())
                .is_some(),
            "restarted API must hydrate the contribution ledger linkage",
        )?;

        let mcp_paid_status = mcp_tool_post(
            mcp,
            "get_paid_status",
            serde_json::json!({ "bounty_id": report.paid_bounty_id.as_str() }),
        )?;
        require(
            value_str(&mcp_paid_status, "/bounty_status") == Some("Paid"),
            "restarted MCP must hydrate paid status from Postgres",
        )?;
        let api_agent_paid_status =
            get_json(&format!("{api}/v1/agents/{}/paid-status", report.solver_id))?;
        require_agent_paid_status(
            &api_agent_paid_status,
            "restarted API must hydrate solver payout summary from Postgres",
        )?;

        let mcp_agent_paid_status = mcp_tool_post(
            mcp,
            "get_paid_status",
            serde_json::json!({ "agent_id": report.solver_id.as_str() }),
        )?;
        require(
            value_str(&mcp_agent_paid_status, "/scope") == Some("agent"),
            "restarted MCP agent paid status must report agent scope",
        )?;
        require_agent_paid_status(
            &mcp_agent_paid_status,
            "restarted MCP must hydrate solver payout summary from Postgres",
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
        "/protocol.json",
        "/.well-known/agent-bounties.json",
        "/llms.txt",
        "route_blocked_goal",
        "list_autonomous_bounties",
        "publish_autonomous_bounty_terms",
        "plan_autonomous_bounty_creation",
        "plan_autonomous_bounty_contribution",
        "plan_autonomous_bounty_claim",
        "plan_autonomous_bounty_submission",
        "publish_autonomous_submission_evidence",
        "list_autonomous_verification_jobs",
        "plan_autonomous_module_settlement",
        "plan_autonomous_attestation_settlement",
        "list_autonomous_bounty_events",
        "FundingAdded",
        "BountyBecameClaimable",
        "BountySettled",
        "Base Sepolia",
        "testnet",
        "operator",
        "Local demo credits are not money",
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
        "BASE_SEPOLIA_BOUNTY_FACTORY",
        "BASE_SEPOLIA_BOUNTY_IMPLEMENTATION",
        "BASE_MAINNET_BOUNTY_FACTORY",
        "BASE_MAINNET_BOUNTY_IMPLEMENTATION",
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
    if is_historical_v1_document(text) {
        return;
    }
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

fn is_historical_v1_document(text: &str) -> bool {
    text.lines().take(8).any(|line| {
        line.trim()
            == "> Historical V1 material only. The operator-controlled escrow was refunded and"
    })
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
        "/v1/base/autonomous-bounties/creation-plan",
        &["create"],
        &["network", "create"],
        &[],
    );
    insert_request_contract(
        &mut contracts,
        "/v1/base/autonomous-bounties/authorized-creation-plan",
        &["create", "signature"],
        &["network", "create", "signature", "relayer"],
        &[],
    );
    insert_request_contract(
        &mut contracts,
        "/v1/base/autonomous-bounties/contribution-plan",
        &["contribution"],
        &["network", "contribution"],
        &[],
    );
    insert_request_contract(
        &mut contracts,
        "/v1/base/autonomous-bounties/authorized-contribution-plan",
        &["contribution", "signature"],
        &["network", "contribution", "signature", "relayer"],
        &[],
    );
    insert_request_contract(
        &mut contracts,
        "/v1/base/autonomous-bounties/claim-plan",
        &["bounty_contract", "solver"],
        &[
            "network",
            "bounty_contract",
            "solver",
            "authorization_nonce",
            "authorization_valid_before",
        ],
        &["authorization_valid_before"],
    );
    insert_request_contract(
        &mut contracts,
        "/v1/base/autonomous-bounties/authorized-claim-plan",
        &[
            "bounty_contract",
            "solver",
            "authorization_nonce",
            "authorization_valid_before",
            "signature",
        ],
        &[
            "network",
            "bounty_contract",
            "solver",
            "authorization_nonce",
            "authorization_valid_before",
            "signature",
            "relayer",
        ],
        &["authorization_valid_before"],
    );
    insert_request_contract(
        &mut contracts,
        "/v1/base/autonomous-bounties/submission-plan",
        &[
            "bounty_contract",
            "solver",
            "submission_hash",
            "evidence_hash",
        ],
        &[
            "network",
            "bounty_contract",
            "solver",
            "submission_hash",
            "evidence_hash",
        ],
        &[],
    );
    insert_request_contract(
        &mut contracts,
        "/v1/base/autonomous-bounties/verification-attestation-plan",
        &["attestation"],
        &["network", "attestation"],
        &[],
    );
    insert_request_contract(
        &mut contracts,
        "/v1/base/autonomous-bounties/module-settlement-plan",
        &["bounty_contract", "proof"],
        &["network", "bounty_contract", "caller", "proof"],
        &[],
    );
    insert_request_contract(
        &mut contracts,
        "/v1/base/autonomous-bounties/attestation-settlement-plan",
        &["bounty_contract", "attestations"],
        &["network", "bounty_contract", "caller", "attestations"],
        &[],
    );
    for path in [
        "/v1/base/autonomous-bounties/expire-claim-plan",
        "/v1/base/autonomous-bounties/expire-submission-plan",
        "/v1/base/autonomous-bounties/cancel-plan",
        "/v1/base/autonomous-bounties/refund-withdrawal-plan",
    ] {
        insert_request_contract(
            &mut contracts,
            path,
            &["bounty_contract"],
            &["network", "bounty_contract", "caller"],
            &[],
        );
    }
    insert_request_contract(
        &mut contracts,
        "/v1/base/autonomous-bounties/decode-events",
        &["logs"],
        &["logs"],
        &[],
    );
    insert_request_contract(
        &mut contracts,
        "/v1/base/autonomous-bounties/terms",
        &["creator_wallet", "document"],
        &["creator_wallet", "document"],
        &[],
    );
    insert_request_contract(
        &mut contracts,
        "/v1/base/autonomous-bounties/submission-evidence",
        &[
            "bounty_contract",
            "bounty_id",
            "round",
            "solver_wallet",
            "artifact_reference",
            "evidence",
        ],
        &[
            "network",
            "bounty_contract",
            "bounty_id",
            "round",
            "solver_wallet",
            "artifact_reference",
            "evidence",
        ],
        &["round"],
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
        &["tx_hash", "request_id", "network"],
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

fn require_agent_paid_status(value: &serde_json::Value, message: &str) -> Result<()> {
    require(
        value
            .pointer("/payouts")
            .and_then(|payouts| payouts.as_array())
            .map(|payouts| {
                payouts.iter().any(|payout| {
                    value_str(payout, "/status") == Some("Paid")
                        && payout
                            .pointer("/amount/amount")
                            .and_then(|amount| amount.as_i64())
                            .is_some_and(|amount| amount > 0)
                })
            })
            .unwrap_or(false),
        message,
    )?;
    require(
        value
            .pointer("/totals")
            .and_then(|totals| totals.as_array())
            .map(|totals| {
                totals.iter().any(|total| {
                    total
                        .pointer("/paid_minor")
                        .and_then(|amount| amount.as_i64())
                        .is_some_and(|amount| amount > 0)
                })
            })
            .unwrap_or(false),
        message,
    )
}

fn value_str<'a>(value: &'a serde_json::Value, pointer: &str) -> Option<&'a str> {
    value.pointer(pointer).and_then(serde_json::Value::as_str)
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

        assert_eq!(report.total_records, 10);
        assert_eq!(report.answered_records, 9);
        assert_eq!(report.partial_answer_records, 1);
        assert_eq!(report.missing_answer_records, 1);
        assert_eq!(report.unique_contributors, 8);
        assert_eq!(
            report.duplicate_contributors,
            vec!["codeboost-tr", "hyperxiaoerxz-hash"]
        );
        assert!(bucket_count(&report.discovery_sources, "github") >= 4);
        assert!(bucket_count(&report.discovery_sources, "machine-discovery") >= 1);
        assert!(bucket_count(&report.participation_reasons, "payout") >= 2);
        assert!(bucket_count(&report.participation_reasons, "clear-scope") >= 3);
        assert!(bucket_count(&report.agent_workflows, "codex") >= 1);
        assert!(bucket_count(&report.agent_workflows, "Hermes Agent") >= 1);
        assert!(bucket_count(&report.agent_workflows, "BountyScout") >= 1);
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
                "production compose api service does not pass `BASE_SEPOLIA_BOUNTY_FACTORY`",
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
    fn docs_contract_exempts_only_explicit_historical_v1_documents() {
        let retired = "> Historical V1 material only. The operator-controlled escrow was refunded and\n\n`POST /v1/base/release-plan`\n";
        let active = "# Active runbook\n\n`POST /v1/base/release-plan`\n";
        let api_routes = BTreeSet::new();
        let mcp_tools = BTreeSet::new();
        let request_contracts = request_contracts();

        let mut historical_issues = Vec::new();
        check_doc_text(
            Path::new("docs/historical.md"),
            retired,
            &api_routes,
            &mcp_tools,
            &request_contracts,
            &mut historical_issues,
        );
        assert!(historical_issues.is_empty());

        let mut active_issues = Vec::new();
        check_doc_text(
            Path::new("docs/active.md"),
            active,
            &api_routes,
            &mcp_tools,
            &request_contracts,
            &mut active_issues,
        );
        assert!(active_issues.iter().any(|issue| issue
            .message
            .contains("unknown API route `/v1/base/release-plan`")));
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
