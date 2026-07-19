import requests
import json
from web3 import Web3
from eth_account.messages import encode_defunct

# Constants
GITHUB_API_URL = "https://api.github.com/repos/{owner}/{repo}/issues/{issue_number}/comments"
BOUNTYBOARD_API_URL = "https://api.bountyboard.global/v1/base/autonomous-bounties/standing-meta-v2-child-preparation"
PARENT_CONTRACT = "0x43d42cb227d76588ab16693f14efd6cff851fa7a"
VERIFIER_MODULE = "0xe573cb4f471d38b5bf10ce82237251ac902c9867"
RUNNER_MANIFEST = "sandboxed_regression_v1"

# Function to register wallet via GitHub comment
def register_wallet(owner, repo, issue_number, token, wallet_address):
    url = GITHUB_API_URL.format(owner=owner, repo=repo, issue_number=issue_number)
    headers = {
        "Authorization": f"token {token}",
        "Content-Type": "application/json"
    }
    data = {
        "body": f"/agent-bounty register {wallet_address}"
    }
    response = requests.post(url, headers=headers, data=json.dumps(data))
    if response.status_code == 201:
        print(f"Wallet {wallet_address} registered successfully.")
    else:
        print(f"Failed to register wallet: {response.text}")
        raise Exception("Wallet registration failed")

# Function to prepare child bounty
def prepare_child_bounty(parent_wallet, child_wallet, task_criteria, github_commit, github_subdirectory):
    url = BOUNTYBOARD_API_URL
    headers = {
        "Content-Type": "application/json"
    }
    data = {
        "parent_contract": PARENT_CONTRACT,
        "parent_wallet": parent_wallet,
        "child_wallet": child_wallet,
        "task_criteria": task_criteria,
        "github_commit": github_commit,
        "github_subdirectory": github_subdirectory,
        "runner_manifest": RUNNER_MANIFEST
    }
    response = requests.post(url, headers=headers, data=json.dumps(data))
    if response.status_code == 200:
        return response.json()
    else:
        print(f"Failed to prepare child bounty: {response.text}")
        raise Exception("Child bounty preparation failed")

# Function to sign and execute transactions
def execute_transactions(web3, private_key, ordered_batch):
    for tx in ordered_batch:
        signed_tx = web3.eth.account.sign_transaction(tx, private_key)
        try:
            tx_hash = web3.eth.send_raw_transaction(signed_tx.rawTransaction)
            receipt = web3.eth.wait_for_transaction_receipt(tx_hash)
            print(f"Transaction {tx_hash.hex()} executed successfully.")
        except Exception as e:
            print(f"Transaction failed: {e}")
            raise Exception("Transaction execution failed")

# Main function
def main():
    # Input parameters
    owner = "your_github_owner"
    repo = "your_github_repo"
    issue_number = "your_issue_number"
    github_token = "your_github_token"
    parent_wallet = "0xYourParentBaseWallet"
    child_wallet = "0xYourChildBaseWallet"
    task_criteria = "Your task criteria"
    github_commit = "your_github_commit"
    github_subdirectory = "your_github_subdirectory"
    private_key = "your_private_key"

    # Initialize Web3
    web3 = Web3(Web3.HTTPProvider("https://mainnet.base.org"))

    # Register wallets
    try:
        register_wallet(owner, repo, issue_number, github_token, parent_wallet)
        register_wallet(owner, repo, issue_number, github_token, child_wallet)
    except Exception as e:
        print(f"Error registering wallets: {e}")
        return

    # Prepare child bounty
    try:
        child_bounty_data = prepare_child_bounty(parent_wallet, child_wallet, task_criteria, github_commit, github_subdirectory)
        ordered_batch = child_bounty_data.get("ordered_batch")
    except Exception as e:
        print(f"Error preparing child bounty: {e}")
        return

    # Execute transactions
    try:
        execute_transactions(web3, private_key, ordered_batch)
    except Exception as e:
        print(f"Error executing transactions: {e}")

if __name__ == "__main__":
    main()