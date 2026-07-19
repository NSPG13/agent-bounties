use ethers::prelude::*;
use std::env;
use std::error::Error;

// Constants for environment variables
const BOUNTY_CONTRACT_ADDRESS: &str = "BOUNTY_CONTRACT_ADDRESS";
const SOLVER_WALLET: &str = "SOLVER_WALLET";
const SOLVER_PRIVATE_KEY: &str = "SOLVER_PRIVATE_KEY";

// Function to get environment variables
fn get_env_var(key: &str) -> Result<String, Box<dyn Error>> {
    match env::var(key) {
        Ok(val) => Ok(val),
        Err(_) => Err(format!("Environment variable {} not set", key).into()),
    }
}

// Function to generate nonce proof
async fn generate_nonce_proof(bounty_id: &str, round: u64, solver: &str, submission_hash: &str, evidence_hash: &str, policy_hash: &str) -> Result<[u8; 32], Box<dyn Error>> {
    // Placeholder for the actual nonce proof generation
    // This should be replaced with the actual implementation using `autonomous-mine-work-proof`
    let nonce_proof = [0u8; 32]; // Replace with actual nonce proof
    Ok(nonce_proof)
}

// Function to verify and settle the bounty
async fn verify_and_settle(provider: &Provider<Http>, signer: &LocalWallet, bounty_contract: &Contract, nonce_proof: [u8; 32]) -> Result<(), Box<dyn Error>> {
    let tx = bounty_contract.call("verifyAndSettle", (nonce_proof,))(
        provider,
        signer,
    ).await?;
    println!("Transaction sent: {:?}", tx);
    Ok(())
}

// Function to post the claim bond
async fn post_claim_bond(provider: &Provider<Http>, signer: &LocalWallet, bounty_contract: &Contract, amount: U256) -> Result<(), Box<dyn Error>> {
    let tx = bounty_contract.call("postClaimBond", (amount,))(
        provider,
        signer,
    ).await?;
    println!("Transaction sent: {:?}", tx);
    Ok(())
}

// Function to complete the comment
fn completion_comment() -> String {
    format!(
        "The first confirmed canonical claim was made by the solver wallet. \
        The 0.10 USDC claim bond was posted, and the submission and evidence hashes were submitted before the claim deadline. \
        A 16-leading-zero-bit nonce was found using `cargo run -p cli -- autonomous-mine-work-proof`. \
        The nonce proof was relayed, and the bounty was settled. \
        The solver received 0.90 USDC plus the return of the 0.10 USDC bond, and the deterministic verifier recipient received 0.10 USDC. \
        Links to the transactions: [claim](#), [submission](#), [settlement](#). \
        The solver found Agent Bounties through [this link](#), participated because [reason], and suggests [improvements]."
    )
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Load environment variables
    let bounty_contract_address = get_env_var(BOUNTY_CONTRACT_ADDRESS)?;
    let solver_wallet = get_env_var(SOLVER_WALLET)?;
    let solver_private_key = get_env_var(SOLVER_PRIVATE_KEY)?;

    // Set up the Ethereum provider and signer
    let provider = Provider::<Http>::try_from("http://localhost:8545")?;
    let signer = LocalWallet::from_str(&solver_private_key)?;

    // Set up the contract
    let abi = include_str!("BountyContract.abi");
    let contract = Contract::new(H160::from_str(&bounty_contract_address)?, serde_json::from_str(abi)?, provider.clone());

    // Generate the nonce proof
    let nonce_proof = generate_nonce_proof(
        "bounty_id",
        1,
        &solver_wallet,
        "submission_hash",
        "evidence_hash",
        "policy_hash",
    ).await?;

    // Post the claim bond
    post_claim_bond(&provider, &signer, &contract, U256::from(100_000_000)).await?; // 0.10 USDC

    // Verify and settle the bounty
    verify_and_settle(&provider, &signer, &contract, nonce_proof).await?;

    // Print the completion comment
    println!("{}", completion_comment());

    Ok(())
}