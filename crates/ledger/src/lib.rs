use chrono::{DateTime, Utc};
use domain::{Id, Money};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LedgerError {
    #[error("ledger entry must balance debits and credits")]
    UnbalancedEntry,
    #[error("ledger event already applied")]
    DuplicateExternalEvent,
    #[error("currency mismatch in ledger entry")]
    CurrencyMismatch,
    #[error("entry must contain at least two postings")]
    TooFewPostings,
}

pub type LedgerResult<T> = Result<T, LedgerError>;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AccountCode(pub String);

impl AccountCode {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PostingSide {
    Debit,
    Credit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Posting {
    pub account: AccountCode,
    pub side: PostingSide,
    pub amount: Money,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LedgerEntry {
    pub id: Id,
    pub external_event_id: Option<String>,
    pub memo: String,
    pub postings: Vec<Posting>,
    pub created_at: DateTime<Utc>,
}

impl LedgerEntry {
    pub fn new(
        memo: impl Into<String>,
        external_event_id: Option<String>,
        postings: Vec<Posting>,
    ) -> LedgerResult<Self> {
        validate_postings(&postings)?;

        Ok(Self {
            id: Uuid::new_v4(),
            external_event_id,
            memo: memo.into(),
            postings,
            created_at: Utc::now(),
        })
    }
}

#[derive(Debug, Default)]
pub struct Ledger {
    entries: Vec<LedgerEntry>,
    applied_external_events: HashSet<String>,
}

impl Ledger {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn append(&mut self, entry: LedgerEntry) -> LedgerResult<()> {
        if let Some(external_id) = &entry.external_event_id {
            if self.applied_external_events.contains(external_id) {
                return Err(LedgerError::DuplicateExternalEvent);
            }
        }

        validate_postings(&entry.postings)?;

        if let Some(external_id) = &entry.external_event_id {
            self.applied_external_events.insert(external_id.clone());
        }

        self.entries.push(entry);
        Ok(())
    }

    pub fn entries(&self) -> &[LedgerEntry] {
        &self.entries
    }

    pub fn has_external_event(&self, external_event_id: &str) -> bool {
        self.applied_external_events.contains(external_event_id)
    }

    pub fn from_entries(entries: Vec<LedgerEntry>) -> LedgerResult<Self> {
        let mut ledger = Self::new();
        for entry in entries {
            ledger.append(entry)?;
        }
        Ok(ledger)
    }

    pub fn balance(&self, account: &AccountCode, currency: &str) -> i64 {
        self.entries
            .iter()
            .flat_map(|entry| &entry.postings)
            .filter(|posting| &posting.account == account && posting.amount.currency == currency)
            .map(|posting| match posting.side {
                PostingSide::Debit => posting.amount.amount,
                PostingSide::Credit => -posting.amount.amount,
            })
            .sum()
    }

    pub fn balances(&self) -> HashMap<(AccountCode, String), i64> {
        let mut balances = HashMap::new();
        for posting in self.entries.iter().flat_map(|entry| &entry.postings) {
            let key = (posting.account.clone(), posting.amount.currency.clone());
            let delta = match posting.side {
                PostingSide::Debit => posting.amount.amount,
                PostingSide::Credit => -posting.amount.amount,
            };
            *balances.entry(key).or_insert(0) += delta;
        }
        balances
    }
}

pub fn debit(account: impl Into<String>, amount: Money) -> Posting {
    Posting {
        account: AccountCode::new(account),
        side: PostingSide::Debit,
        amount,
    }
}

pub fn credit(account: impl Into<String>, amount: Money) -> Posting {
    Posting {
        account: AccountCode::new(account),
        side: PostingSide::Credit,
        amount,
    }
}

pub fn validate_postings(postings: &[Posting]) -> LedgerResult<()> {
    if postings.len() < 2 {
        return Err(LedgerError::TooFewPostings);
    }

    let currency = postings[0].amount.currency.clone();
    if postings
        .iter()
        .any(|posting| posting.amount.currency != currency)
    {
        return Err(LedgerError::CurrencyMismatch);
    }

    let debits: i64 = postings
        .iter()
        .filter(|posting| posting.side == PostingSide::Debit)
        .map(|posting| posting.amount.amount)
        .sum();
    let credits: i64 = postings
        .iter()
        .filter(|posting| posting.side == PostingSide::Credit)
        .map(|posting| posting.amount.amount)
        .sum();

    if debits != credits {
        return Err(LedgerError::UnbalancedEntry);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::Money;
    use proptest::prelude::*;

    #[test]
    fn rejects_unbalanced_entries() {
        let entry = LedgerEntry::new(
            "bad",
            None,
            vec![
                debit("cash", Money::new(100, "usd").unwrap()),
                credit("revenue", Money::new(99, "usd").unwrap()),
            ],
        );

        assert_eq!(entry.unwrap_err(), LedgerError::UnbalancedEntry);
    }

    #[test]
    fn duplicate_external_event_is_rejected() {
        let mut ledger = Ledger::new();
        let entry = || {
            LedgerEntry::new(
                "stripe topup",
                Some("evt_1".to_string()),
                vec![
                    debit("stripe_cash", Money::new(5000, "usd").unwrap()),
                    credit("user_balance", Money::new(5000, "usd").unwrap()),
                ],
            )
            .unwrap()
        };

        ledger.append(entry()).unwrap();
        assert_eq!(
            ledger.append(entry()).unwrap_err(),
            LedgerError::DuplicateExternalEvent
        );
    }

    proptest! {
        #[test]
        fn balanced_entries_preserve_system_total(amount in 1i64..1_000_000) {
            let mut ledger = Ledger::new();
            let entry = LedgerEntry::new(
                "reserve bounty",
                None,
                vec![
                    debit("reserved_bounties", Money::new(amount, "usdc").unwrap()),
                    credit("available_balance", Money::new(amount, "usdc").unwrap()),
                ],
            ).unwrap();

            ledger.append(entry).unwrap();
            let total: i64 = ledger.balances().values().sum();
            prop_assert_eq!(total, 0);
        }
    }
}
