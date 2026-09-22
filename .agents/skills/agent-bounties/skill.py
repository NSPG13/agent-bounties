import os
import json
from pathlib import Path
from typing import Dict, Any

from web3 import Web3
from web3.exceptions import ContractLogicError

# Contract ABI (minimal subset required for state queries)
CONTRACT_ABI = [
    {
        "constant": True,
        "inputs": [],
        "name": "isClaimable",
        "outputs": [{"name": "", "type": "bool"}],
        "type": "function",
    },
    {
        "constant": True,
        "inputs": [],
        "name": "isFunded",
        "outputs": [{"name": "", "type": "bool"}],
        "type": "function",
    },
    {
        "constant": True,
        "inputs": [],
        "name": "isVerifierReady",
        "outputs": [{"name": "", "type": "bool"}],
        "type": "function",
    },
    {
        "constant": True,
        "inputs": [],
        "name": "isSubmitted",
        "outputs": [{"name": "", "type": "bool"}],
        "type": "function",
    },
    {
        "constant": True,
        "inputs": [],
        "name": "isPaid",
        "outputs": [{"name": "", "type": "bool"}],
        "type": "function",
    },
]

# Default RPC and contract address
BASE_RPC_URL = os.getenv("BASE_RPC_URL", "https://mainnet.base.org")
CONTRACT_ADDRESS = os.getenv(
    "CONTRACT_ADDRESS", "0x294cda5faa1b1a9dd7eca2cb52daff1fa843ad22"
)

# Verification directory
VERIFICATION_DIR = Path(".openhands") / "verification"
FOCUSED_CHECKS_FILE = VERIFICATION_DIR / "focused_checks.json"
EVIDENCE_FILE = VERIFICATION_DIR / "evidence.json"


def _get_web3() -> Web3:
    """Instantiate a Web3 instance connected to Base mainnet."""
    return Web3(Web3.HTTPProvider(BASE_RPC_URL))


def _get_contract(w3: Web3):
    """Return a contract instance for the bounty contract."""
    return w3.eth.contract(address=Web3.to_checksum_address(CONTRACT_ADDRESS), abi=CONTRACT_ABI)


def _query_state(contract) -> Dict[str, bool]:
    """Query the contract for all relevant state booleans."""
    try:
        return {
            "is_claimable": contract.functions.isClaimable().call(),
            "is_funded": contract.functions.isFunded().call(),
            "is_verifier_ready": contract.functions.isVerifierReady().call(),
            "is_submitted": contract.functions.isSubmitted().call(),
            "is_paid": contract.functions.isPaid().call(),
        }
    except ContractLogicError as e:
        # In case the contract does not expose the expected functions
        raise RuntimeError(f"Failed to query contract state: {e}") from e


def _determine_next_action(state: Dict[str, bool]) -> str:
    """
    Determine the next action based on the current contract state.
    The mapping follows the acceptance criteria:
    - claimable -> "claim"
    - unfunded -> "wait for funding"
    - verifier-unready -> "wait for verifier readiness"
    - submitted-not-paid -> "wait for payment"
    - paid -> "done"
    """
    if not state["is_funded"]:
        return "wait for funding"
    if state["is_funded"] and not state["is_verifier_ready"]:
        return "wait for verifier readiness"
    if state["is_claimable"]:
        return "claim"
    if state["is_submitted"] and not state["is_paid"]:
        return "wait for payment"
    if state["is_paid"]:
        return "done"
    # Fallback: if none of the above, default to waiting
    return "wait for next state"


def run() -> Dict[str, Any]:
    """
    Main entry point for the skill.
    Returns a dictionary containing the next action and the raw state.
    """
    w3 = _get_web3()
    contract = _get_contract(w3)
    state = _query_state(contract)
    next_action = _determine_next_action(state)
    return {"next_action": next_action, "state": state}


def stop() -> bool:
    """
    Deterministic stop hook.
    The skill will not report completion until both focused checks and
    submission evidence exist in the verification directory.
    """
    return FOCUSED_CHECKS_FILE.is_file() and EVIDENCE_FILE.is_file()


# If the file is executed directly, print the next action for debugging.
if __name__ == "__main__":
    result = run()
    print(json.dumps(result, indent=2))
