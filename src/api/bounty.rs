use axum::{extract::Path, Json};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::db::Database;
use crate::error::ApiError;

#[derive(Debug, Serialize, Deserialize)]
pub struct BountyEconomics {
    pub solver_payout: String,
    pub refundable_bond: String,
    pub required_external_spend: String,
    pub gross_cash_margin: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BountyDetails {
    pub id: String,
    pub status: String,
    pub contract_address: String,
    pub total_funding: String,
    pub economics: BountyEconomics,
}

pub async fn get_bounty(
    Path(id): Path<String>,
    db: Arc<Database>,
) -> Result<Json<BountyDetails>, ApiError> {
    let bounty = db.get_bounty(&id).await?;
    
    let solver_payout = bounty.solver_payout.clone();
    let refundable_bond = bounty.verifier_reward.clone();
    let required_external_spend = bounty.required_external_spend.clone();
    
    let gross_cash_margin = calculate_gross_margin(
        &solver_payout,
        &refundable_bond,
        &required_external_spend,
    )?;
    
    let economics = BountyEconomics {
        solver_payout,
        refundable_bond,
        required_external_spend,
        gross_cash_margin,
    };
    
    let details = BountyDetails {
        id: bounty.id,
        status: bounty.status,
        contract_address: bounty.contract_address,
        total_funding: bounty.total_funding,
        economics,
    };
    
    Ok(Json(details))
}

fn calculate_gross_margin(
    solver_payout: &str,
    refundable_bond: &str,
    required_external_spend: &str,
) -> Result<String, ApiError> {
    let payout: f64 = solver_payout.parse().map_err(|_| ApiError::ParseError)?;
    let bond: f64 = refundable_bond.parse().map_err(|_| ApiError::ParseError)?;
    let spend: f64 = required_external_spend.parse().map_err(|_| ApiError::ParseError)?;
    
    let margin = payout - spend;
    Ok(format!("{:.2}", margin))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_gross_margin_direct() {
        let result = calculate_gross_margin("1.99", "0.01", "0").unwrap();
        assert_eq!(result, "1.99");
    }

    #[test]
    fn test_calculate_gross_margin_with_external_spend() {
        let result = calculate_gross_margin("5.00", "0.10", "2.00").unwrap();
        assert_eq!(result, "3.00");
    }

    #[test]
    fn test_calculate_gross_margin_unprofitable() {
        let result = calculate_gross_margin("1.00", "0.05", "1.50").unwrap();
        assert_eq!(result, "-0.50");
    }

    #[test]
    fn test_calculate_gross_margin_standing_meta() {
        let result = calculate_gross_margin("10.00", "0.50", "3.25").unwrap();
        assert_eq!(result, "6.75");
    }
}
