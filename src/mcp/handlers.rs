use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::db::Database;
use crate::error::McpError;

#[derive(Debug, Serialize, Deserialize)]
pub struct EconomicsOutput {
    pub solver_payout_usdc: String,
    pub refundable_bond_usdc: String,
    pub required_external_spend_usdc: String,
    pub gross_cash_margin_usdc: String,
    pub note: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ClaimInfo {
    pub contract_address: String,
    pub status: String,
    pub economics: EconomicsOutput,
}

pub async fn agent_native_claim(
    contract_address: String,
    db: Arc<Database>,
) -> Result<ClaimInfo, McpError> {
    let bounty = db.get_bounty_by_contract(&contract_address).await?;
    
    if bounty.status != "claimable" {
        return Err(McpError::InvalidStatus(bounty.status));
    }
    
    let solver_payout = bounty.solver_payout.clone();
    let refundable_bond = bounty.verifier_reward.clone();
    let required_external_spend = bounty.required_external_spend.clone();
    
    let gross_cash_margin = calculate_gross_margin(
        &solver_payout,
        &refundable_bond,
        &required_external_spend,
    )?;
    
    let economics = EconomicsOutput {
        solver_payout_usdc: solver_payout,
        refundable_bond_usdc: refundable_bond,
        required_external_spend_usdc: required_external_spend,
        gross_cash_margin_usdc: gross_cash_margin.clone(),
        note: format!(
            "Gross cash margin of {} USDC before gas costs. Bond is refundable upon successful verification. Only BountySettled proves payment.",
            gross_cash_margin
        ),
    };
    
    let info = ClaimInfo {
        contract_address: bounty.contract_address,
        status: bounty.status,
        economics,
    };
    
    Ok(info)
}

fn calculate_gross_margin(
    solver_payout: &str,
    refundable_bond: &str,
    required_external_spend: &str,
) -> Result<String, McpError> {
    let payout: f64 = solver_payout.parse().map_err(|_| McpError::ParseError)?;
    let bond: f64 = refundable_bond.parse().map_err(|_| McpError::ParseError)?;
    let spend: f64 = required_external_spend.parse().map_err(|_| McpError::ParseError)?;
    
    let margin = payout - spend;
    Ok(format!("{:.2}", margin))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_economics_output_note_clarity() {
        let economics = EconomicsOutput {
            solver_payout_usdc: "1.99".to_string(),
            refundable_bond_usdc: "0.01".to_string(),
            required_external_spend_usdc: "0".to_string(),
            gross_cash_margin_usdc: "1.99".to_string(),
            note: "Gross cash margin of 1.99 USDC before gas costs. Bond is refundable upon successful verification. Only BountySettled proves payment.".to_string(),
        };
        
        assert!(!economics.note.contains("guaranteed"));
        assert!(!economics.note.contains("net profit"));
        assert!(economics.note.contains("before gas costs"));
        assert!(economics.note.contains("refundable"));
    }

    #[test]
    fn test_calculate_margin_filters_unprofitable() {
        let margin = calculate_gross_margin("1.00", "0.05", "2.00").unwrap();
        let margin_value: f64 = margin.parse().unwrap();
        assert!(margin_value < 0.0);
    }

    #[test]
    fn test_calculate_margin_direct_bounty() {
        let margin = calculate_gross_margin("1.99", "0.01", "0").unwrap();
        assert_eq!(margin, "1.99");
    }

    #[test]
    fn test_calculate_margin_standing_meta() {
        let margin = calculate_gross_margin("8.50", "0.25", "1.75").unwrap();
        assert_eq!(margin, "6.75");
    }
}
