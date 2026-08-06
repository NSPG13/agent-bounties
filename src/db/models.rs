use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bounty {
    pub id: String,
    pub status: String,
    pub contract_address: String,
    pub total_funding: String,
    pub solver_payout: String,
    pub verifier_reward: String,
    pub required_external_spend: String,
    pub verification_type: String,
    pub created_at: i64,
    pub updated_at: i64,
}

impl Bounty {
    pub fn new(
        id: String,
        contract_address: String,
        total_funding: String,
        solver_payout: String,
        verifier_reward: String,
        required_external_spend: String,
        verification_type: String,
    ) -> Self {
        let now = chrono::Utc::now().timestamp();
        Self {
            id,
            status: "claimable".to_string(),
            contract_address,
            total_funding,
            solver_payout,
            verifier_reward,
            required_external_spend,
            verification_type,
            created_at: now,
            updated_at: now,
        }
    }
}
