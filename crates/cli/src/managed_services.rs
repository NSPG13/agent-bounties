use clap::{Args, Subcommand};
use serde_json::{json, Value};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Args)]
pub struct Options {
    #[arg(
        long,
        env = "AGENT_BOUNTIES_API_URL",
        default_value = "https://api.agentizable.ai"
    )]
    api_url: String,
}
#[derive(Debug, Subcommand)]
pub enum ManagedServiceCommand {
    /// Cancel your paid service before funding; request a full refund to its original payment.
    Refund {
        id: Uuid,
        #[command(flatten)]
        options: Options,
    },
    /// Merchant operator: issue exact immutable quote JSON. Does not charge anyone.
    IssueQuote {
        file: PathBuf,
        #[command(flatten)]
        options: Options,
    },
    /// Price a posting draft (JSON file) for card payment, bound to one exact v2 bounty.
    BountyQuote {
        file: PathBuf,
        #[arg(long)]
        solver_reward_micro_usdc: u64,
        #[command(flatten)]
        options: Options,
    },
    /// Read the itemized price/terms with your existing website account session.
    Quote {
        id: Uuid,
        #[command(flatten)]
        options: Options,
    },
    /// Accept the exact quote digest; retain the same idempotency key for retries.
    Accept {
        id: Uuid,
        digest: String,
        idempotency_key: String,
        #[command(flatten)]
        options: Options,
    },
    /// Read your order without claiming bank settlement or funding from payment alone.
    Order {
        id: Uuid,
        #[command(flatten)]
        options: Options,
    },
    /// Open/recover Checkout. Pay on Stripe's returned page; never pass card data here.
    Checkout {
        id: Uuid,
        #[command(flatten)]
        options: Options,
    },
    /// Show a funded card bounty's submission awaiting your review and its public evidence.
    Review {
        id: Uuid,
        #[command(flatten)]
        options: Options,
    },
    /// Record your one verdict for a round after inspecting its artifact. Not settlement.
    RecordReview {
        id: Uuid,
        #[arg(long)]
        round: u64,
        #[arg(long)]
        submission_hash: String,
        #[arg(long)]
        evidence_hash: String,
        /// Pass the submission (pays the solver); omit to reject it. A rejection waits 48 hours
        /// for the solver's appeal, then forfeits the solver's bond.
        #[arg(long)]
        pass: bool,
        /// Public reason; a rejection needs 20 to 2,000 characters.
        #[arg(long, default_value = "")]
        reason: String,
        #[command(flatten)]
        options: Options,
    },
    /// Public review state of a card bounty, with the typed data a rejected solver signs.
    CardReview {
        network: String,
        contract: String,
        #[command(flatten)]
        options: Options,
    },
    /// Appeal a rejection of your own submission with your wallet's EIP-712 signature over the
    /// card review's appeal_typed_data and this exact reason. Never pass a private key.
    Appeal {
        network: String,
        contract: String,
        #[arg(long)]
        round: u64,
        #[arg(long)]
        solver: String,
        /// Public reason of 20 to 2,000 characters; it is part of the signed message.
        #[arg(long)]
        reason: String,
        #[arg(long)]
        signature: String,
        #[command(flatten)]
        options: Options,
    },
    /// Operator: appealed rejections awaiting a ruling, earliest deadline first.
    Appeals {
        #[command(flatten)]
        options: Options,
    },
    /// Operator: rule on an appeal. --reverse pays the solver and the customer's review
    /// deposit; without it the rejection is upheld. The review key signs the ruling.
    RuleAppeal {
        network: String,
        contract: String,
        #[arg(long)]
        round: u64,
        #[arg(long)]
        reverse: bool,
        /// Public reason of 20 to 2,000 characters.
        #[arg(long)]
        reason: String,
        #[command(flatten)]
        options: Options,
    },
    /// Operator: review deposits owed to solvers after reversed rejections.
    DepositPayouts {
        #[command(flatten)]
        options: Options,
    },
    /// Operator: record the USDC transfer that paid a forfeited deposit; it is verified on chain.
    RecordPayout {
        network: String,
        contract: String,
        #[arg(long)]
        tx_hash: String,
        #[command(flatten)]
        options: Options,
    },
}

/// Who a request speaks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Authority {
    Account,
    Operator,
    Public,
}

fn card_bounty(network: &str, contract: &str) -> anyhow::Result<String> {
    anyhow::ensure!(
        matches!(network, "base-mainnet" | "base-sepolia")
            && contract.len() == 42
            && contract.starts_with("0x")
            && contract[2..].bytes().all(|b| b.is_ascii_hexdigit()),
        "card bounty needs base-mainnet or base-sepolia and a 0x contract address"
    );
    Ok(format!("{network}/{}", contract.to_ascii_lowercase()))
}

fn public_reason(reason: &str) -> anyhow::Result<()> {
    let chars = reason.chars().count();
    anyhow::ensure!(
        reason == reason.trim() && (20..=2_000).contains(&chars),
        "reason must be a trimmed public reason of 20 to 2,000 characters"
    );
    Ok(())
}
pub async fn execute(command: ManagedServiceCommand) -> anyhow::Result<()> {
    use Authority::{Account, Operator, Public};
    let (options, path, body, authority) = match command {
        ManagedServiceCommand::Refund { id, options } => (
            options,
            format!("/v1/managed-service/orders/{id}/refund"),
            Some(json!({})),
            Account,
        ),
        ManagedServiceCommand::IssueQuote { file, options } => {
            anyhow::ensure!(
                std::fs::metadata(&file)?.len() <= 64 * 1024,
                "quote exceeds 64 KiB"
            );
            let body: Value = serde_json::from_slice(&std::fs::read(file)?)?;
            (
                options,
                "/v1/managed-service/quotes".into(),
                Some(body),
                Operator,
            )
        }
        ManagedServiceCommand::BountyQuote {
            file,
            solver_reward_micro_usdc,
            options,
        } => {
            anyhow::ensure!(
                solver_reward_micro_usdc > 0,
                "solver reward must be positive"
            );
            anyhow::ensure!(
                std::fs::metadata(&file)?.len() <= 64 * 1024,
                "draft exceeds 64 KiB"
            );
            let document: Value = serde_json::from_slice(&std::fs::read(file)?)?;
            anyhow::ensure!(document.is_object(), "draft must be a JSON object");
            (
                options,
                "/v1/managed-service/bounty-quotes".into(),
                Some(
                    json!({"document": document, "solver_reward_micro_usdc": solver_reward_micro_usdc}),
                ),
                Account,
            )
        }
        ManagedServiceCommand::Quote { id, options } => (
            options,
            format!("/v1/managed-service/quotes/{id}"),
            None,
            Account,
        ),
        ManagedServiceCommand::Order { id, options } => (
            options,
            format!("/v1/managed-service/orders/{id}"),
            None,
            Account,
        ),
        ManagedServiceCommand::Accept {
            id,
            digest,
            idempotency_key,
            options,
        } => (
            options,
            format!("/v1/managed-service/quotes/{id}/accept"),
            Some(json!({"quote_digest":digest,"idempotency_key":idempotency_key})),
            Account,
        ),
        ManagedServiceCommand::Checkout { id, options } => (
            options,
            format!("/v1/managed-service/orders/{id}/checkout"),
            Some(json!({})),
            Account,
        ),
        ManagedServiceCommand::Review { id, options } => (
            options,
            format!("/v1/managed-service/orders/{id}/review"),
            None,
            Account,
        ),
        ManagedServiceCommand::RecordReview {
            id,
            round,
            submission_hash,
            evidence_hash,
            pass,
            reason,
            options,
        } => {
            let hash = |value: &str| {
                value.len() == 66
                    && value.starts_with("0x")
                    && value[2..]
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            };
            anyhow::ensure!(
                round > 0 && hash(&submission_hash) && hash(&evidence_hash),
                "round must be positive and hashes 0x-prefixed lowercase 32-byte hex"
            );
            let chars = reason.chars().count();
            anyhow::ensure!(
                reason == reason.trim() && chars <= 2_000 && (pass || chars >= 20),
                "a rejection needs a trimmed public reason of 20 to 2,000 characters"
            );
            (
                options,
                format!("/v1/managed-service/orders/{id}/review"),
                Some(json!({"round": round, "submission_hash": submission_hash,
                    "evidence_hash": evidence_hash, "passed": pass, "reason": reason})),
                Account,
            )
        }
        ManagedServiceCommand::CardReview {
            network,
            contract,
            options,
        } => (
            options,
            format!(
                "/v1/managed-service/bounties/{}/review",
                card_bounty(&network, &contract)?
            ),
            None,
            Public,
        ),
        ManagedServiceCommand::Appeal {
            network,
            contract,
            round,
            solver,
            reason,
            signature,
            options,
        } => {
            public_reason(&reason)?;
            anyhow::ensure!(
                round > 0
                    && solver.len() == 42
                    && solver.starts_with("0x")
                    && solver[2..].bytes().all(|b| b.is_ascii_hexdigit())
                    && signature.len() >= 4
                    && signature.len() % 2 == 0
                    && signature.starts_with("0x")
                    && signature[2..].bytes().all(|b| b.is_ascii_hexdigit()),
                "appeal needs a positive round, the solver address and its 0x signature"
            );
            (
                options,
                format!(
                    "/v1/managed-service/bounties/{}/appeal",
                    card_bounty(&network, &contract)?
                ),
                Some(
                    json!({"round": round, "solver": solver.to_ascii_lowercase(),
                    "reason": reason, "signature": signature}),
                ),
                Public,
            )
        }
        ManagedServiceCommand::Appeals { options } => (
            options,
            "/v1/managed-service/appeals".into(),
            None,
            Operator,
        ),
        ManagedServiceCommand::RuleAppeal {
            network,
            contract,
            round,
            reverse,
            reason,
            options,
        } => {
            public_reason(&reason)?;
            anyhow::ensure!(round > 0, "round must be positive");
            (
                options,
                format!(
                    "/v1/managed-service/appeals/{}/{round}/ruling",
                    card_bounty(&network, &contract)?
                ),
                Some(json!({"reversed": reverse, "reason": reason})),
                Operator,
            )
        }
        ManagedServiceCommand::DepositPayouts { options } => (
            options,
            "/v1/managed-service/deposit-payouts".into(),
            None,
            Operator,
        ),
        ManagedServiceCommand::RecordPayout {
            network,
            contract,
            tx_hash,
            options,
        } => {
            anyhow::ensure!(
                tx_hash.len() == 66
                    && tx_hash.starts_with("0x")
                    && tx_hash[2..].bytes().all(|b| b.is_ascii_hexdigit()),
                "tx hash must be a 0x-prefixed 32-byte hex hash"
            );
            (
                options,
                format!(
                    "/v1/managed-service/deposit-payouts/{}",
                    card_bounty(&network, &contract)?
                ),
                Some(json!({"tx_hash": tx_hash.to_ascii_lowercase()})),
                Operator,
            )
        }
    };
    let url = reqwest::Url::parse(&options.api_url)?;
    anyhow::ensure!(
        url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
            && url.path() == "/"
            && (url.scheme() == "https"
                || (url.scheme() == "http"
                    && matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]")))),
        "API URL must be an HTTPS origin (loopback HTTP only for local tests)"
    );
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(100))
        .build()?;
    let url = url.join(&path)?;
    let mut request = match body {
        Some(body) => client.post(url).json(&body),
        None => client.get(url),
    };
    if authority == Operator {
        let token = std::env::var("OPERATOR_API_TOKEN")
            .map_err(|_| anyhow::anyhow!("Operator commands require OPERATOR_API_TOKEN"))?;
        request = request.header("x-operator-token", token);
    } else if authority == Account {
        let cookie=std::env::var("AGENT_BOUNTIES_ACCOUNT_COOKIE").map_err(|_|anyhow::anyhow!("Use your verified website account session via AGENT_BOUNTIES_ACCOUNT_COOKIE; wallet/analytics tokens do not grant account access"))?;
        let origin = std::env::var("AGENT_BOUNTIES_WEBSITE_ORIGIN")
            .unwrap_or_else(|_| "https://agentizable.ai".into());
        request = request.header("cookie", cookie).header("origin", origin);
    }
    let response=request.send().await.map_err(|_|anyhow::anyhow!("Managed-service request did not finish; read order state and retain the same retry key"))?;
    anyhow::ensure!(
        response.status().is_success(),
        "Managed-service API returned HTTP {}; no new retry key or replacement order is authorized",
        response.status()
    );
    let value: Value = response.json().await?;
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}
