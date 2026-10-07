import os
import json
from web3 import Web3

def relay_action(envelope):
    """
    Core relay logic refactored to accept an envelope instead of a GH event.
    envelope: { 'bounty_id': int, 'proof': str, 'sender': str }
    """
    w3 = Web3(Web3.HTTPProvider(os.getenv("RPC_URL")))
    # Logic for relaying verifyAndSettle using shared gas-sponsorship
    # ... (implementation details for contract interaction)
    print(f"Relaying {envelope['bounty_id']} with proof {envelope['proof']}")
    return True

def handle_comment_event(event_data):
    # Legacy support for GH comments
    envelope = parse_comment(event_data)
    return relay_action(envelope)
