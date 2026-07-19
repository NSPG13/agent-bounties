from web3 import Web3
from eth_account import Account
import json

# Constants
INFURA_URL = 'https://base-mainnet.infura.io/v3/YOUR_INFURA_PROJECT_ID'
CONTRACT_ABI_PATH = 'path/to/contract_abi.json'
ONCHAIN_TERMS_REGISTRY_CONTRACT = '0x35e5d49c12b75c119d33951c2c4f054c5732208c'
PARENT_BOUNTY_CONTRACT = '0xe8c1d3f046f3e4690bef59ba4abd5d02d2a6984b'
USDC_CONTRACT = '0xYOUR_USDC_CONTRACT_ADDRESS'

# Initialize Web3
w3 = Web3(Web3.HTTPProvider(INFURA_URL))

# Load contract ABI
with open(CONTRACT_ABI_PATH) as f:
    contract_abi = json.load(f)

# Parent Bounty Contract
parent_bounty_contract = w3.eth.contract(address=PARENT_BOUNTY_CONTRACT, abi=contract_abi)

# Onchain Terms Registry Contract
onchain_terms_registry_contract = w3.eth.contract(address=ONCHAIN_TERMS_REGISTRY_CONTRACT, abi=contract_abi)

# USDC Contract
usdc_contract = w3.eth.contract(address=USDC_CONTRACT, abi=contract_abi)

# Wallets
parent_wallet_private_key = 'YOUR_PARENT_WALLET_PRIVATE_KEY'
child_solver_wallet = '0xCHILD_SOLVER_WALLET_ADDRESS'

# Register wallets
def register_wallet(wallet_address):
    tx = parent_bounty_contract.functions.register(wallet_address).buildTransaction({
        'from': parent_wallet_private_key,
        'gas': 2000000,
        'gasPrice': w3.toWei('5', 'gwei')
    })
    signed_tx = w3.eth.account.sign_transaction(tx, private_key=parent_wallet_private_key)
    tx_hash = w3.eth.send_raw_transaction(signed_tx.rawTransaction)
    print(f"Registered wallet: {wallet_address}, TX Hash: {tx_hash.hex()}")

# Publish terms
def publish_terms(terms):
    tx = onchain_terms_registry_contract.functions.publishTerms(terms).buildTransaction({
        'from': parent_wallet_private_key,
        'gas': 2000000,
        'gasPrice': w3.toWei('5', 'gwei')
    })
    signed_tx = w3.eth.account.sign_transaction(tx, private_key=parent_wallet_private_key)
    tx_hash = w3.eth.send_raw_transaction(signed_tx.rawTransaction)
    print(f"Published terms: {terms}, TX Hash: {tx_hash.hex()}")

# Fund the bounty
def fund_bounty(amount):
    tx = usdc_contract.functions.transfer(PARENT_BOUNTY_CONTRACT, amount).buildTransaction({
        'from': parent_wallet_private_key,
        'gas': 2000000,
        'gasPrice': w3.toWei('5', 'gwei')
    })
    signed_tx = w3.eth.account.sign_transaction(tx, private_key=parent_wallet_private_key)
    tx_hash = w3.eth.send_raw_transaction(signed_tx.rawTransaction)
    print(f"Funded bounty with {amount} USDC, TX Hash: {tx_hash.hex()}")

# Start the parent claim
def start_parent_claim():
    tx = parent_bounty_contract.functions.startParentClaim().buildTransaction({
        'from': parent_wallet_private_key,
        'gas': 2000000,
        'gasPrice': w3.toWei('5', 'gwei')
    })
    signed_tx = w3.eth.account.sign_transaction(tx, private_key=parent_wallet_private_key)
    tx_hash = w3.eth.send_raw_transaction(signed_tx.rawTransaction)
    print(f"Started parent claim, TX Hash: {tx_hash.hex()}")

# Main function
def main():
    # Step 1: Register wallets
    register_wallet(parent_wallet_private_key)
    register_wallet(child_solver_wallet)

    # Step 2: Publish terms
    terms = {
        'title': 'Wallet UX Coding Task',
        'description': 'Implement a new feature for the wallet UX',
       'reward': 0.90 * 10**6,  # 0.90 USDC
        'verifiers': [
            '0xbe6292b9e465f549e2363b918d6dd9187038431e',
            '0xb7c2ce6430b66fb986e27b6140b29309550d487a'
        ],
        'threshold': 2
    }
    publish_terms(terms)

    # Step 3: Fund the bounty
    amount = 0.90 * 10**6  # 0.90 USDC
    fund_bounty(amount)

    # Step 4: Start the parent claim
    start_parent_claim()

if __name__ == "__main__":
    main()