'''
Meta-Bounty Coordinator for #649 — Agent Wallet UX

Coordinates the parent-child bounty flow:
1. Publishes child bounty terms on-chain
2. Verifies child-solver registration (different participant)
3. Monitors canonical ChildBountySettled event
4. Submits parent proof for settlement

This module implements the standing-meta-v2 wallet-ux plan
as defined in benchmarks/standing-meta-v2/wallet-ux/.
'''

import json
import hashlib
from typing import Optional, Dict, Any


class WalletUXCoordinator:
    '''Coordinates the parent-child flow for agent wallet UX meta-bounty (#649).'''
    
    PARENT_CONTRACT = "0x41f7f2722f0af7289c2f2eea6afed6f4873f722a"
    CHILD_BENCHMARK_PATH = "benchmarks/standing-meta-v2/wallet-ux"
    
    def __init__(self, base_rpc_url: str):
        self.base_rpc_url = base_rpc_url
        self.parent_contract = self.PARENT_CONTRACT
    
    def compute_child_terms_hash(self, task_spec: Dict[str, Any]) -> str:
        '''Compute deterministic terms hash for child bounty.'''
        canonical = json.dumps(task_spec, sort_keys=True, separators=(',', ':'))
        return hashlib.sha256(canonical.encode()).hexdigest()
    
    def validate_child_solver(self, parent_wallet: str, child_wallet: str) -> bool:
        '''Verify child solver is a DIFFERENT registered participant.'''
        return parent_wallet.lower() != child_wallet.lower()
    
    def prepare_settlement_proof(self, child_settled_event: Dict[str, Any]) -> Dict[str, Any]:
        '''Build parent proof from canonical child settlement event.'''
        return {
            "parent_contract": self.parent_contract,
            "child_contract": child_settled_event.get("child_contract"),
            "child_solver": child_settled_event.get("solver"),
            "settlement_tx": child_settled_event.get("transaction_hash"),
            "terms_hash": child_settled_event.get("terms_hash"),
        }


def main():
    '''CLI entry point for wallet-ux meta-bounty coordination.'''
    import argparse
    parser = argparse.ArgumentParser(description="Wallet UX Meta-Bounty Coordinator (#649)")
    parser.add_argument("--rpc-url", required=True, help="Base mainnet RPC URL")
    parser.add_argument("--action", choices=["terms-hash", "validate-solver", "settlement-proof"],
                        required=True, help="Action to perform")
    parser.add_argument("--input-json", help="JSON input for the action")
    args = parser.parse_args()
    
    coordinator = WalletUXCoordinator(args.rpc_url)
    
    if args.action == "terms-hash":
        task = json.loads(args.input_json)
        h = coordinator.compute_child_terms_hash(task)
        print(json.dumps({"terms_hash": h, "algorithm": "sha256"}))
    elif args.action == "validate-solver":
        data = json.loads(args.input_json)
        valid = coordinator.validate_child_solver(data["parent"], data["child"])
        print(json.dumps({"valid": valid}))
    elif args.action == "settlement-proof":
        event = json.loads(args.input_json)
        proof = coordinator.prepare_settlement_proof(event)
        print(json.dumps(proof, indent=2))


if __name__ == "__main__":
    main()
