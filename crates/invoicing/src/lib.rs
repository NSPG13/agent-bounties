//! Seller-of-record invoiced orders (ADR 0006).
//!
//! AgentBounties sells a verified outcome to a business buyer and invoices it through Stripe from
//! the US entity. Once the invoice is paid, AgentBounties funds an autonomous-v2 bounty from its
//! own treasury, gated so that only attested contractors
//! (`agent-bounties/invoice-contractor-v1`) can claim it.
//!
//! This crate is pure and deterministic. It covers quote math, the append-only order projection,
//! contractor eligibility, and the contractor payment ledger for 1099 reporting. Callers supply
//! signature-verified Stripe webhook evidence and confirmed canonical chain events. A redirect,
//! payment intent, plan, signature or transaction hash is never evidence here.

use serde::{Deserialize, Serialize};
use sha3::{Digest, Keccak256};
use std::collections::{BTreeMap, HashSet};
use thiserror::Error;

pub const INVOICE_CONTRACTOR_SOURCE: &str = "agent-bounties/invoice-contractor-v1";
/// USDC has six decimals. At the 1:1 invoicing rate, one US cent is 10,000 base units.
pub const USDC_BASE_UNITS_PER_CENT: u64 = 10_000;
pub const MAX_PLATFORM_FEE_BPS: u16 = 1_000;
/// `ParticipantEligibilityRegistry.register` accepts at most 365 days of validity.
pub const MAX_ATTESTATION_SECONDS: u64 = 365 * 86_400;
const BPS_DENOMINATOR: u64 = 10_000;
const MAX_TITLE_CHARS: usize = 200;

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum InvoicingError {
    #[error("invalid invoicing policy: {0}")]
    InvalidPolicy(String),
    #[error("invalid quote: {0}")]
    InvalidQuote(String),
    #[error("order event rejected: {0}")]
    RejectedEvent(String),
    #[error("contractor is not eligible: {0}")]
    ContractorIneligible(String),
    #[error("amount overflow")]
    Overflow,
}

type Result<T> = std::result::Result<T, InvoicingError>;

pub fn keccak_hex(bytes: &[u8]) -> String {
    format!("0x{}", hex::encode(Keccak256::digest(bytes)))
}

/// `keccak256("agent-bounties/invoice-contractor-v1")`: the claim-gate source committed by every
/// invoiced bounty and attested for every contractor wallet.
pub fn invoice_contractor_source_hash() -> String {
    keccak_hex(INVOICE_CONTRACTOR_SOURCE.as_bytes())
}

/// The contractor's stable registry identity. Every wallet the same contractor registers shares it,
/// and a wallet can never change identity.
pub fn contractor_participant_id(contractor_id: &str) -> String {
    keccak_hex(format!("{INVOICE_CONTRACTOR_SOURCE}:{contractor_id}").as_bytes())
}

fn ceil_div(numerator: u64, denominator: u64) -> u64 {
    numerator.div_ceil(denominator)
}

fn checked_add(left: u64, right: u64) -> Result<u64> {
    left.checked_add(right).ok_or(InvoicingError::Overflow)
}

fn checked_mul(left: u64, right: u64) -> Result<u64> {
    left.checked_mul(right).ok_or(InvoicingError::Overflow)
}

/// Lowercase `0x` + 40 hex address, or `None`.
pub fn normalize_address(value: &str) -> Option<String> {
    let value = value.trim();
    let hex = value.strip_prefix("0x")?;
    (hex.len() == 40 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| format!("0x{}", hex.to_ascii_lowercase()))
}

fn normalize_bytes32(value: &str) -> Option<String> {
    let value = value.trim();
    let hex = value.strip_prefix("0x")?;
    (hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| format!("0x{}", hex.to_ascii_lowercase()))
}

/// Operator-configured invoicing terms. `platform_fee_bps` must equal the v2 factory's immutable
/// fee so that the invoice covers exactly what the treasury escrows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvoicePolicy {
    pub platform_fee_bps: u16,
    /// Payment-processing pass-through added to the invoice. It is never escrowed.
    pub processing_fee_bps: u16,
    pub processing_fee_fixed_cents: u64,
    /// Risk cap for one order's invoice total.
    pub max_invoice_cents: u64,
    pub days_until_due: u32,
    /// The Stripe mode that invoice and credit-note evidence must match (`false` = test mode).
    pub livemode: bool,
}

impl InvoicePolicy {
    pub fn validate(&self) -> Result<()> {
        let reject = |message: &str| Err(InvoicingError::InvalidPolicy(message.to_string()));
        if self.platform_fee_bps > MAX_PLATFORM_FEE_BPS {
            return reject("platform fee exceeds the 1,000 bps protocol cap");
        }
        if self.processing_fee_bps > 1_000 {
            return reject("processing pass-through exceeds 1,000 bps");
        }
        if self.max_invoice_cents == 0 {
            return reject("max_invoice_cents must be positive");
        }
        if !(1..=90).contains(&self.days_until_due) {
            return reject("days_until_due must be between 1 and 90");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvoiceLineItem {
    /// `outcome`, `platform_fee`, or `processing`.
    pub code: String,
    pub description: String,
    pub amount_cents: u64,
}

/// A buyer quote. The treasury escrows `funding_target_usdc` exactly. The invoice rounds the fee
/// line up to whole cents and adds any processing pass-through, so it always covers the escrow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvoiceQuote {
    pub title: String,
    pub solver_reward_usdc: u64,
    pub verifier_reward_usdc: u64,
    pub platform_fee_bps: u16,
    pub platform_fee_usdc: u64,
    /// Exact autonomous-v2 target: solver reward + verifier reward + platform fee.
    pub funding_target_usdc: u64,
    pub line_items: Vec<InvoiceLineItem>,
    pub invoice_total_cents: u64,
    pub currency: String,
}

/// `ceil(solver_reward * bps / 10_000)`, matching `agentBountyV2PlatformFee`.
pub fn platform_fee_usdc(solver_reward_usdc: u64, platform_fee_bps: u16) -> Result<u64> {
    Ok(ceil_div(
        checked_mul(solver_reward_usdc, u64::from(platform_fee_bps))?,
        BPS_DENOMINATOR,
    ))
}

pub fn quote_invoice_order(
    policy: &InvoicePolicy,
    title: &str,
    solver_reward_cents: u64,
    verifier_reward_cents: u64,
) -> Result<InvoiceQuote> {
    policy.validate()?;
    let reject = |message: &str| Err(InvoicingError::InvalidQuote(message.to_string()));
    let title = title.trim();
    if title.is_empty() || title.chars().count() > MAX_TITLE_CHARS {
        return reject("title must be 1 to 200 characters");
    }
    if solver_reward_cents == 0 || verifier_reward_cents == 0 {
        return reject("solver and verifier rewards must be positive");
    }
    let solver_reward_usdc = checked_mul(solver_reward_cents, USDC_BASE_UNITS_PER_CENT)?;
    let verifier_reward_usdc = checked_mul(verifier_reward_cents, USDC_BASE_UNITS_PER_CENT)?;
    let platform_fee_usdc = platform_fee_usdc(solver_reward_usdc, policy.platform_fee_bps)?;
    let funding_target_usdc = checked_add(
        checked_add(solver_reward_usdc, verifier_reward_usdc)?,
        platform_fee_usdc,
    )?;
    let outcome_cents = checked_add(solver_reward_cents, verifier_reward_cents)?;
    let fee_cents = ceil_div(platform_fee_usdc, USDC_BASE_UNITS_PER_CENT);
    let mut line_items = vec![InvoiceLineItem {
        code: "outcome".to_string(),
        description: format!("Verified outcome: {title}"),
        amount_cents: outcome_cents,
    }];
    if fee_cents > 0 {
        line_items.push(InvoiceLineItem {
            code: "platform_fee".to_string(),
            description: format!(
                "Platform fee ({}.{:02}% of the solver reward)",
                policy.platform_fee_bps / 100,
                policy.platform_fee_bps % 100
            ),
            amount_cents: fee_cents,
        });
    }
    let subtotal_cents = checked_add(outcome_cents, fee_cents)?;
    let processing_cents = checked_add(
        ceil_div(
            checked_mul(subtotal_cents, u64::from(policy.processing_fee_bps))?,
            BPS_DENOMINATOR,
        ),
        policy.processing_fee_fixed_cents,
    )?;
    if processing_cents > 0 {
        line_items.push(InvoiceLineItem {
            code: "processing".to_string(),
            description: "Payment processing".to_string(),
            amount_cents: processing_cents,
        });
    }
    let invoice_total_cents = checked_add(subtotal_cents, processing_cents)?;
    if invoice_total_cents > policy.max_invoice_cents {
        return reject("invoice total exceeds the configured per-order cap");
    }
    debug_assert!(
        u128::from(subtotal_cents) * u128::from(USDC_BASE_UNITS_PER_CENT)
            >= u128::from(funding_target_usdc)
    );
    Ok(InvoiceQuote {
        title: title.to_string(),
        solver_reward_usdc,
        verifier_reward_usdc,
        platform_fee_bps: policy.platform_fee_bps,
        platform_fee_usdc,
        funding_target_usdc,
        line_items,
        invoice_total_cents,
        currency: "usd".to_string(),
    })
}

/// Facts from a signature-verified Stripe `invoice.paid` webhook.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvoicePaidEvidence {
    pub stripe_event_id: String,
    pub invoice_id: String,
    /// The invoice's `metadata.order_id`.
    pub order_id: String,
    pub status: String,
    pub amount_paid_cents: u64,
    pub amount_remaining_cents: u64,
    pub currency: String,
    pub livemode: bool,
}

/// One append-only order event. Stripe events must arrive signature-verified, and chain events
/// must be confirmed canonical logs. Each evidence id (Stripe event id or log key) applies at most
/// once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OrderEvent {
    Quoted {
        quote: InvoiceQuote,
        /// SHA-256 of the business terms the invoice references (seller of record, contractors).
        terms_sha256: String,
    },
    InvoiceIssued {
        invoice_id: String,
        total_cents: u64,
    },
    InvoicePaid(InvoicePaidEvidence),
    InvoiceVoided {
        invoice_id: String,
        stripe_event_id: String,
    },
    FundingPlanned {
        treasury: String,
        bounty_id: String,
        bounty_contract: String,
        funding_target_usdc: u64,
    },
    /// Canonical `FundingAdded` from the planned bounty contract.
    FundingObserved {
        bounty_contract: String,
        contributor: String,
        amount_usdc: u64,
        log_key: String,
    },
    /// Canonical `BountySettled`. `reportable_usdc` is the solver reward plus the completion bonus;
    /// the returned claim bond was the solver's own money.
    SettlementObserved {
        bounty_contract: String,
        solver: String,
        reportable_usdc: u64,
        settled_at: u64,
        log_key: String,
    },
    CancellationRequested {
        reason: String,
    },
    /// Canonical `RefundWithdrawn` paying the treasury after on-chain cancellation.
    RefundObserved {
        bounty_contract: String,
        contributor: String,
        amount_usdc: u64,
        log_key: String,
    },
    /// Signature-verified Stripe `credit_note.created` refunding the buyer.
    CreditNoteIssued {
        credit_note_id: String,
        invoice_id: String,
        refund_cents: u64,
        stripe_event_id: String,
        livemode: bool,
    },
}

impl OrderEvent {
    /// The external evidence id that must apply at most once, when the event has one.
    pub fn evidence_id(&self) -> Option<&str> {
        match self {
            OrderEvent::InvoicePaid(evidence) => Some(&evidence.stripe_event_id),
            OrderEvent::InvoiceVoided {
                stripe_event_id, ..
            }
            | OrderEvent::CreditNoteIssued {
                stripe_event_id, ..
            } => Some(stripe_event_id),
            OrderEvent::FundingObserved { log_key, .. }
            | OrderEvent::SettlementObserved { log_key, .. }
            | OrderEvent::RefundObserved { log_key, .. } => Some(log_key),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderStatus {
    Quoted,
    Invoiced,
    Paid,
    FundingPlanned,
    Funded,
    Settled,
    CancelRequested,
    Refunded,
    Voided,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractorSettlement {
    pub solver: String,
    pub reportable_usdc: u64,
    pub settled_at: u64,
    pub log_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderState {
    pub order_id: String,
    pub status: OrderStatus,
    pub quote: InvoiceQuote,
    pub terms_sha256: String,
    pub invoice_id: Option<String>,
    pub paid_cents: u64,
    pub treasury: Option<String>,
    pub bounty_id: Option<String>,
    pub bounty_contract: Option<String>,
    pub funded: bool,
    pub refunded_to_treasury: bool,
    pub settlement: Option<ContractorSettlement>,
    pub credit_note_id: Option<String>,
    /// What the operator or the system must do next.
    pub next_action: String,
}

impl OrderState {
    /// The amount owed to the buyer: positive only after cancellation, once no escrow remains at
    /// risk, and before a credit note has been issued.
    pub fn refund_due_cents(&self) -> u64 {
        if self.status == OrderStatus::CancelRequested
            && (!self.funded || self.refunded_to_treasury)
        {
            self.paid_cents
        } else {
            0
        }
    }

    fn refresh_next_action(&mut self) {
        self.next_action = match self.status {
            OrderStatus::Quoted => "issue_invoice",
            OrderStatus::Invoiced => "await_verified_invoice_paid",
            OrderStatus::Paid => "plan_treasury_funding",
            OrderStatus::FundingPlanned => "sign_treasury_funding_then_await_funding_added",
            OrderStatus::Funded => "await_bounty_settled",
            OrderStatus::CancelRequested if self.refund_due_cents() > 0 => {
                "issue_credit_note_refund"
            }
            OrderStatus::CancelRequested => "cancel_bounty_then_withdraw_treasury_refund",
            OrderStatus::Settled => "record_contractor_payment",
            OrderStatus::Refunded | OrderStatus::Voided => "none",
        }
        .to_string();
    }
}

fn reject<T>(message: impl Into<String>) -> Result<T> {
    Err(InvoicingError::RejectedEvent(message.into()))
}

fn same_address(expected: Option<&str>, actual: &str) -> bool {
    match (expected, normalize_address(actual)) {
        (Some(expected), Some(actual)) => expected == actual,
        _ => false,
    }
}

/// Replays an order's event log. Every transition is checked against the quote and the previously
/// applied evidence, and any out-of-order, duplicate, or mismatched event fails the whole
/// projection closed.
pub fn project_order(
    order_id: &str,
    policy: &InvoicePolicy,
    events: &[OrderEvent],
) -> Result<OrderState> {
    policy.validate()?;
    let Some((
        OrderEvent::Quoted {
            quote,
            terms_sha256,
        },
        rest,
    )) = events.split_first()
    else {
        return reject("an order log must start with exactly one quote");
    };
    if quote.platform_fee_bps != policy.platform_fee_bps {
        return reject("quote fee does not match the configured v2 factory fee");
    }
    let Some(terms_sha256) = terms_sha256
        .strip_prefix("sha256:")
        .filter(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .map(|hex| format!("sha256:{}", hex.to_ascii_lowercase()))
    else {
        return reject("terms_sha256 must be sha256:<64 hex>");
    };
    let mut state = OrderState {
        order_id: order_id.to_string(),
        status: OrderStatus::Quoted,
        quote: quote.clone(),
        terms_sha256,
        invoice_id: None,
        paid_cents: 0,
        treasury: None,
        bounty_id: None,
        bounty_contract: None,
        funded: false,
        refunded_to_treasury: false,
        settlement: None,
        credit_note_id: None,
        next_action: String::new(),
    };
    let mut applied_evidence = HashSet::new();
    for event in rest {
        if let Some(id) = event.evidence_id() {
            if id.trim().is_empty() || !applied_evidence.insert(id.to_string()) {
                return reject("evidence id is empty or already applied");
            }
        }
        apply_order_event(&mut state, policy, event)?;
    }
    state.refresh_next_action();
    Ok(state)
}

fn apply_order_event(
    state: &mut OrderState,
    policy: &InvoicePolicy,
    event: &OrderEvent,
) -> Result<()> {
    use OrderStatus as Status;
    match event {
        OrderEvent::Quoted { .. } => reject("an order has exactly one quote"),
        OrderEvent::InvoiceIssued {
            invoice_id,
            total_cents,
        } => {
            if state.status != Status::Quoted {
                return reject("invoice issued outside the quoted state");
            }
            if !invoice_id.starts_with("in_") || *total_cents != state.quote.invoice_total_cents {
                return reject("issued invoice does not match the quote total");
            }
            state.invoice_id = Some(invoice_id.clone());
            state.status = Status::Invoiced;
            Ok(())
        }
        OrderEvent::InvoicePaid(evidence) => {
            if state.status != Status::Invoiced {
                return reject("invoice payment outside the invoiced state");
            }
            if state.invoice_id.as_deref() != Some(evidence.invoice_id.as_str())
                || evidence.order_id != state.order_id
            {
                return reject("paid invoice is not this order's invoice");
            }
            if evidence.status != "paid"
                || evidence.amount_remaining_cents != 0
                || evidence.amount_paid_cents != state.quote.invoice_total_cents
                || evidence.currency != state.quote.currency
            {
                return reject("invoice is not fully paid in the quoted amount and currency");
            }
            if evidence.livemode != policy.livemode {
                return reject("invoice evidence is from the wrong Stripe mode");
            }
            state.paid_cents = evidence.amount_paid_cents;
            state.status = Status::Paid;
            Ok(())
        }
        OrderEvent::InvoiceVoided { invoice_id, .. } => {
            if state.status != Status::Invoiced
                || state.invoice_id.as_deref() != Some(invoice_id.as_str())
            {
                return reject("only this order's unpaid invoice can be voided");
            }
            state.status = Status::Voided;
            Ok(())
        }
        OrderEvent::FundingPlanned {
            treasury,
            bounty_id,
            bounty_contract,
            funding_target_usdc,
        } => {
            if state.status != Status::Paid {
                return reject("treasury funding can be planned only after verified payment");
            }
            let (Some(treasury), Some(bounty_id), Some(bounty_contract)) = (
                normalize_address(treasury),
                normalize_bytes32(bounty_id),
                normalize_address(bounty_contract),
            ) else {
                return reject("funding plan has an invalid treasury, bounty id, or contract");
            };
            if *funding_target_usdc != state.quote.funding_target_usdc {
                return reject("funding plan target differs from the quote");
            }
            state.treasury = Some(treasury);
            state.bounty_id = Some(bounty_id);
            state.bounty_contract = Some(bounty_contract);
            state.status = Status::FundingPlanned;
            Ok(())
        }
        OrderEvent::FundingObserved {
            bounty_contract,
            contributor,
            amount_usdc,
            ..
        } => {
            if state.funded
                || !matches!(
                    state.status,
                    Status::FundingPlanned | Status::CancelRequested
                )
                || !same_address(state.bounty_contract.as_deref(), bounty_contract)
            {
                return reject("funding observed for an unplanned or already funded bounty");
            }
            if !same_address(state.treasury.as_deref(), contributor)
                || *amount_usdc != state.quote.funding_target_usdc
            {
                return reject("funding is not the treasury's exact quoted target");
            }
            state.funded = true;
            if state.status == Status::FundingPlanned {
                state.status = Status::Funded;
            }
            Ok(())
        }
        OrderEvent::SettlementObserved {
            bounty_contract,
            solver,
            reportable_usdc,
            settled_at,
            log_key,
        } => {
            if !state.funded
                || !matches!(state.status, Status::Funded | Status::CancelRequested)
                || state.refunded_to_treasury
                || !same_address(state.bounty_contract.as_deref(), bounty_contract)
            {
                return reject("settlement observed for a bounty this order did not fund");
            }
            let Some(solver) = normalize_address(solver) else {
                return reject("settlement solver is not an address");
            };
            if *reportable_usdc < state.quote.solver_reward_usdc {
                return reject("settlement paid less than the quoted solver reward");
            }
            state.settlement = Some(ContractorSettlement {
                solver,
                reportable_usdc: *reportable_usdc,
                settled_at: *settled_at,
                log_key: log_key.clone(),
            });
            state.status = Status::Settled;
            Ok(())
        }
        OrderEvent::CancellationRequested { reason } => {
            if !matches!(
                state.status,
                Status::Paid | Status::FundingPlanned | Status::Funded
            ) || reason.trim().is_empty()
            {
                return reject("only a paid, unsettled order can be cancelled with a reason; void unpaid invoices instead");
            }
            state.status = Status::CancelRequested;
            Ok(())
        }
        OrderEvent::RefundObserved {
            bounty_contract,
            contributor,
            amount_usdc,
            ..
        } => {
            if state.status != Status::CancelRequested
                || !state.funded
                || state.refunded_to_treasury
                || !same_address(state.bounty_contract.as_deref(), bounty_contract)
                || !same_address(state.treasury.as_deref(), contributor)
                || *amount_usdc < state.quote.funding_target_usdc
            {
                return reject(
                    "refund is not the treasury's full principal from this cancelled bounty",
                );
            }
            state.refunded_to_treasury = true;
            Ok(())
        }
        OrderEvent::CreditNoteIssued {
            credit_note_id,
            invoice_id,
            refund_cents,
            livemode,
            ..
        } => {
            if state.refund_due_cents() == 0 {
                return reject(
                    "a credit note is due only after cancellation with no escrow at risk",
                );
            }
            if state.invoice_id.as_deref() != Some(invoice_id.as_str())
                || *refund_cents != state.paid_cents
                || *livemode != policy.livemode
                || !credit_note_id.starts_with("cn_")
            {
                return reject("credit note does not refund this order's full paid invoice");
            }
            state.credit_note_id = Some(credit_note_id.clone());
            state.status = Status::Refunded;
            Ok(())
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaxFormKind {
    /// US person. A matched TIN is required before payment.
    W9,
    /// Foreign individual.
    W8Ben,
    /// Foreign entity.
    W8BenE,
}

/// A tax form held by the tax-form provider. Only the provider reference is stored here, never the
/// TIN.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaxFormRecord {
    pub kind: TaxFormKind,
    pub provider: String,
    pub provider_reference: String,
    /// The provider's IRS TIN-matching result. Required for a W-9.
    pub tin_matched: bool,
    pub received_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractorRecord {
    pub contractor_id: String,
    pub wallet: String,
    pub agreement_sha256: String,
    pub agreement_accepted_at: u64,
    pub tax_form: Option<TaxFormRecord>,
}

/// The fields an operator's attester key signs for `ParticipantEligibilityRegistry.register`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractorAttestation {
    pub wallet: String,
    pub participant_id: String,
    pub source_hash: String,
    pub valid_until: u64,
}

/// Admits a contractor to the claim gate only with the current agreement and a usable tax form.
/// That way an automatic on-chain payout never reaches a US payee without a matched TIN.
pub fn contractor_attestation(
    record: &ContractorRecord,
    required_agreement_sha256: &str,
    now: u64,
    validity_seconds: u64,
) -> Result<ContractorAttestation> {
    let ineligible = |message: &str| Err(InvoicingError::ContractorIneligible(message.to_string()));
    let id = record.contractor_id.trim();
    if id.is_empty() || id.len() > 128 {
        return ineligible("contractor id must be 1 to 128 characters");
    }
    let Some(wallet) = normalize_address(&record.wallet) else {
        return ineligible("wallet is not an address");
    };
    if wallet == format!("0x{}", "0".repeat(40)) {
        return ineligible("wallet is the zero address");
    }
    if !record
        .agreement_sha256
        .eq_ignore_ascii_case(required_agreement_sha256)
        || record.agreement_accepted_at > now
    {
        return ineligible("the current contractor agreement has not been accepted");
    }
    let Some(form) = &record.tax_form else {
        return ineligible("no tax form on file");
    };
    if form.provider.trim().is_empty() || form.provider_reference.trim().is_empty() {
        return ineligible("tax form lacks a provider reference");
    }
    if form.kind == TaxFormKind::W9 && !form.tin_matched {
        return ineligible("a W-9 payee needs a matched TIN before any automatic payout");
    }
    if validity_seconds == 0 || validity_seconds > MAX_ATTESTATION_SECONDS {
        return ineligible("attestation validity must be 1 second to 365 days");
    }
    Ok(ContractorAttestation {
        wallet,
        participant_id: contractor_participant_id(id),
        source_hash: invoice_contractor_source_hash(),
        valid_until: now
            .checked_add(validity_seconds)
            .ok_or(InvoicingError::Overflow)?,
    })
}

/// One reportable contractor payment derived from a settled invoiced order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractorPayment {
    pub contractor_id: String,
    pub tax_form_kind: TaxFormKind,
    pub order_id: String,
    pub reportable_usdc: u64,
    pub paid_at: u64,
    pub log_key: String,
}

/// Civil year of a Unix timestamp (UTC), without a calendar dependency.
pub fn utc_year(unix_seconds: u64) -> i64 {
    // Howard Hinnant's days-to-civil algorithm.
    let days = (unix_seconds / 86_400) as i64 + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    year_of_era + era * 400 + i64::from(month <= 2)
}

/// Federal Form 1099-NEC threshold. It was $600 through 2025 and $2,000 for payments made after
/// December 31, 2025 (P.L. 119-21). The threshold is inflation-indexed after 2027, so later years
/// need an explicit, confirmed amount.
pub fn form_1099_nec_threshold_cents(tax_year: i64, indexed_override_cents: Option<u64>) -> u64 {
    match tax_year {
        ..=2025 => 60_000,
        2026 | 2027 => 200_000,
        _ => indexed_override_cents.unwrap_or(200_000),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Form1099NecLine {
    pub contractor_id: String,
    pub tax_year: i64,
    pub payments: usize,
    pub total_usdc: u64,
    /// Rounded half up to whole cents.
    pub total_cents: u64,
    pub requires_1099_nec: bool,
}

/// Annual per-contractor totals for W-9 payees. Foreign payees (W-8BEN/W-8BEN-E) are listed in
/// `foreign_payee_totals` for the tax provider's 1042-S review, not 1099-NEC.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractorTaxSummary {
    pub tax_year: i64,
    pub threshold_cents: u64,
    pub form_1099_nec: Vec<Form1099NecLine>,
    pub foreign_payee_totals: Vec<Form1099NecLine>,
}

pub fn summarize_contractor_payments(
    payments: &[ContractorPayment],
    tax_year: i64,
    indexed_override_cents: Option<u64>,
) -> Result<ContractorTaxSummary> {
    let threshold_cents = form_1099_nec_threshold_cents(tax_year, indexed_override_cents);
    let mut seen = HashSet::new();
    let mut domestic: BTreeMap<&str, (usize, u64)> = BTreeMap::new();
    let mut foreign: BTreeMap<&str, (usize, u64)> = BTreeMap::new();
    for payment in payments
        .iter()
        .filter(|payment| utc_year(payment.paid_at) == tax_year)
    {
        if !seen.insert(payment.log_key.as_str()) {
            return Err(InvoicingError::RejectedEvent(
                "a settlement was recorded twice".to_string(),
            ));
        }
        let bucket = if payment.tax_form_kind == TaxFormKind::W9 {
            &mut domestic
        } else {
            &mut foreign
        };
        let entry = bucket.entry(payment.contractor_id.as_str()).or_default();
        entry.0 += 1;
        entry.1 = checked_add(entry.1, payment.reportable_usdc)?;
    }
    let lines = |bucket: BTreeMap<&str, (usize, u64)>, domestic: bool| {
        bucket
            .into_iter()
            .map(|(contractor_id, (payments, total_usdc))| {
                let total_cents =
                    (total_usdc + USDC_BASE_UNITS_PER_CENT / 2) / USDC_BASE_UNITS_PER_CENT;
                Form1099NecLine {
                    contractor_id: contractor_id.to_string(),
                    tax_year,
                    payments,
                    total_usdc,
                    total_cents,
                    requires_1099_nec: domestic && total_cents >= threshold_cents,
                }
            })
            .collect()
    };
    Ok(ContractorTaxSummary {
        tax_year,
        threshold_cents,
        form_1099_nec: lines(domestic, true),
        foreign_payee_totals: lines(foreign, false),
    })
}

#[cfg(test)]
mod tests;
