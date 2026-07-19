from web3 import Web3
import hashlib
import json

# Initialize Web3
w3 = Web3(Web3.HTTPProvider('https://mainnet.infura.io/v3/YOUR_INFURA_PROJECT_ID'))

# Define the bounty contract ABI and address
bounty_contract_abi = json.loads(open('BountyContractABI.json').read())
bounty_contract_address = '0xYourBountyContractAddress'
bounty_contract = w3.eth.contract(address=bounty_contract_address, abi=bounty_contract_abi)

# Define the solver's wallet
solver_wallet = '0xYourSolverWalletAddress'
solver_private_key = 'YourSolverPrivateKey'

# Step 1: Claim the bounty
def claim_bounty():
    # Get the current round and other necessary details
    round_info = bounty_contract.functions.getActiveRound().call()
    round_id = round_info[0]
    
    # Prepare the transaction
    tx = bounty_contract.functions.claimBounty(round_id).buildTransaction({
        'chainId': 1,
        'gas': 2000000,
        'gasPrice': w3.toWei('50', 'gwei'),
        'nonce': w3.eth.getTransactionCount(solver_wallet),
        'value': w3.toWei(0.01, 'ether')  # 0.01 USDC bond
    })
    
    # Sign and send the transaction
    signed_tx = w3.eth.account.sign_transaction(tx, private_key=solver_private_key)
    tx_hash = w3.eth.sendRawTransaction(signed_tx.rawTransaction)
    print(f"Bounty claimed. TX Hash: {tx_hash.hex()}")

# Step 2: Submit hashes
def submit_hashes(submission_data, evidence_data):
    # Generate submission and evidence hashes
    submission_hash = hashlib.sha256(submission_data.encode()).hexdigest()
    evidence_hash = hashlib.sha256(evidence_data.encode()).hexdigest()
    
    # Get the current round and other necessary details
    round_info = bounty_contract.functions.getActiveRound().call()
    round_id = round_info[0]
    
    # Prepare the transaction
    tx = bounty_contract.functions.submitHashes(round_id, submission_hash, evidence_hash).buildTransaction({
        'chainId': 1,
        'gas': 2000000,
        'gasPrice': w3.toWei('50', 'gwei'),
        'nonce': w3.eth.getTransactionCount(solver_wallet),
    })
    
    # Sign and send the transaction
    signed_tx = w3.eth.account.sign_transaction(tx, private_key=solver_private_key)
    tx_hash = w3.eth.sendRawTransaction(signed_tx.rawTransaction)
    print(f"Hashes submitted. TX Hash: {tx_hash.hex()}")

# Step 3: Generate work proof
def generate_work_proof(bounty_id, round_id, solver, submission_hash, evidence_hash, policy_hash):
    # Concatenate the required data
    data = f"{bounty_id}{round_id}{solver}{submission_hash}{evidence_hash}{policy_hash}"
    
    # Generate the hash
    data_hash = hashlib.sha256(data.encode()).hexdigest()
    
    # Find a 16-bit leading-zero work proof
    nonce = 0
    while True:
        candidate = f"{data_hash}{nonce}".encode()
        candidate_hash = hashlib.sha256(candidate).hexdigest()
        if candidate_hash.startswith('0' * 4):  # 16 bits (4 hex characters)
            return nonce, candidate_hash
        nonce += 1

# Step 4: Relay the work proof
def relay_work_proof(nonce, work_proof):
    # Get the current round and other necessary details
    round_info = bounty_contract.functions.getActiveRound().call()
    round_id = round_info[0]
    
    # Prepare the transaction
    tx = bounty_contract.functions.relayWorkProof(round_id, nonce, work_proof).buildTransaction({
        'chainId': 1,
        'gas': 2000000,
        'gasPrice': w3.toWei('50', 'gwei'),
        'nonce': w3.eth.getTransactionCount(solver_wallet),
    })
    
    # Sign and send the transaction
    signed_tx = w3.eth.account.sign_transaction(tx, private_key=solver_private_key)
    tx_hash = w3.eth.sendRawTransaction(signed_tx.rawTransaction)
    print(f"Work proof relayed. TX Hash: {tx_hash.hex()}")

# Main function to orchestrate the process
def main():
    # Step 1: Claim the bounty
    claim_bounty()
    
    # Step 2: Submit hashes
    submission_data = "Your submission data"
    evidence_data = "Your evidence data"
    submit_hashes(submission_data, evidence_data)
    
    # Step 3: Generate work proof
    bounty_id = 'YourBountyID'
    round_id = 'YourRoundID'
    policy_hash = 'YourPolicyHash'
    nonce, work_proof = generate_work_proof(bounty_id, round_id, solver_wallet, submission_data, evidence_data, policy_hash)
    print(f"Generated work proof: Nonce={nonce}, Proof={work_proof}")
    
    # Step 4: Relay the work proof
    relay_work_proof(nonce, work_proof)

if __name__ == "__main__":
    main()