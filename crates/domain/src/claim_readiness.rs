use serde::{Deserialize, Serialize};

/// A solver-facing claim-readiness diagnostic response.
/// Exposes reward, refundable bond, external spend, gross cash margin, and an actionable blocker.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClaimReadinessDiagnostic {
    pub scenario: String,
    pub reward: String,
    pub refundable_bond: String,
    pub external_spend: String,
    pub gross_cash_margin: String,
    /// Gross cash margin is clearly distinguished from guaranteed net profit.
    pub is_guaranteed_net_profit: bool,
    pub next_action: String,
    pub blocker: Option<String>,
}

impl ClaimReadinessDiagnostic {
    /// Validates the diagnostic against security requirements.
    pub fn validate(&self) -> Result<(), &'static str> {
        // Never request private keys or seed phrases.
        let json = serde_json::to_string(self).unwrap_or_default().to_lowercase();
        if json.contains("private key") || json.contains("seed phrase") {
            return Err("Diagnostic must never request a private key or seed phrase");
        }
        
        // Reject any result that describes a plan, signature, transaction hash, or hosted row as payment.
        for forbidden in &["payment: plan", "payment: signature", "payment: transaction hash", "payment: hosted row", "as payment"] {
            if json.contains(forbidden) {
                return Err("Diagnostic must not describe a plan, signature, tx hash, or hosted row as payment");
            }
        }
        
        // Gross cash margin MUST NOT be misrepresented as guaranteed net profit.
        if self.is_guaranteed_net_profit {
            return Err("Gross cash margin cannot be represented as guaranteed net profit");
        }
        
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE_JSON: &str = include_str!("../../../fixtures/claim-readiness-diagnostics.json");

    #[test]
    fn parses_and_validates_committed_fixture() {
        let diagnostics: Vec<ClaimReadinessDiagnostic> = serde_json::from_str(FIXTURE_JSON)
            .expect("Fixture must be valid JSON");
            
        assert_eq!(diagnostics.len(), 4, "Fixture must cover 4 specific scenarios");
        
        let healthy = diagnostics.iter().find(|d| d.scenario == "healthy_direct_bounty").unwrap();
        assert_eq!(healthy.next_action, "sign_claim_transaction");
        assert!(healthy.blocker.is_none());
        assert!(healthy.validate().is_ok());

        let recovery = diagnostics.iter().find(|d| d.scenario == "recovery_reserved").unwrap();
        assert_eq!(recovery.next_action, "wait_for_recovery");
        assert!(recovery.blocker.as_ref().unwrap().contains("recovery"));
        assert!(recovery.validate().is_ok());

        let unprofitable = diagnostics.iter().find(|d| d.scenario == "unprofitable_bounty").unwrap();
        assert_eq!(unprofitable.next_action, "abort_claim");
        assert!(unprofitable.blocker.as_ref().unwrap().contains("negative"));
        assert!(unprofitable.validate().is_ok());

        let non_creator = diagnostics.iter().find(|d| d.scenario == "non_creator_failure").unwrap();
        assert_eq!(non_creator.next_action, "abort_claim");
        assert!(non_creator.blocker.as_ref().unwrap().contains("solver"));
        assert!(non_creator.validate().is_ok());
    }

    #[test]
    fn rejects_private_key_requests() {
        let mut diag = ClaimReadinessDiagnostic {
            scenario: "malicious".to_string(),
            reward: "0".to_string(),
            refundable_bond: "0".to_string(),
            external_spend: "0".to_string(),
            gross_cash_margin: "0".to_string(),
            is_guaranteed_net_profit: false,
            next_action: "Provide your private key to claim".to_string(),
            blocker: None,
        };
        assert!(diag.validate().is_err());
        
        diag.next_action = "Enter seed phrase".to_string();
        assert!(diag.validate().is_err());
    }

    #[test]
    fn rejects_misrepresented_payment() {
        let diag = ClaimReadinessDiagnostic {
            scenario: "misrepresented".to_string(),
            reward: "0".to_string(),
            refundable_bond: "0".to_string(),
            external_spend: "0".to_string(),
            gross_cash_margin: "0".to_string(),
            is_guaranteed_net_profit: false,
            next_action: "submit".to_string(),
            blocker: Some("Transaction hash accepted as payment".to_string()),
        };
        assert!(diag.validate().is_err());
    }

    #[test]
    fn rejects_guaranteed_profit_claim() {
        let diag = ClaimReadinessDiagnostic {
            scenario: "too_good_to_be_true".to_string(),
            reward: "0".to_string(),
            refundable_bond: "0".to_string(),
            external_spend: "0".to_string(),
            gross_cash_margin: "0".to_string(),
            is_guaranteed_net_profit: true,
            next_action: "submit".to_string(),
            blocker: None,
        };
        assert!(diag.validate().is_err());
    }
}
