from bounty_management import Bounty, Wallet, EventManager

# Initialize wallet and event manager
wallet = Wallet()
event_manager = EventManager()

# Bounties to keep and cancel
bounty_ids_to_keep = [217]
bounty_ids_to_cancel = [218, 219, 220]

# New bounties to create
new_bounty_ids = [244, 248, 249, 250]
solver_reward = 2.00
verifier_reward = 0.01
solver_bond = 0.01

# Step 1: Keep bounty #217 funded and claimable
for bounty_id in bounty_ids_to_keep:
    bounty = Bounty.get(bounty_id)
    if not bounty.is_funded:
        wallet.transfer(bounty.funding_amount, bounty.wallet_address)
    bounty.set_claimable(True)

# Step 2: Cancel bounties #218, #219, and #220, then withdraw the creator's 3.00 USDC refunds
total_refund = 0.0
for bounty_id in bounty_ids_to_cancel:
    bounty = Bounty.get(bounty_id)
    if not bounty.is_claimed:
        bounty.cancel()
        refund_amount = bounty.funding_amount
        total_refund += refund_amount
        wallet.withdraw(refund_amount, bounty.creator_wallet)

# Step 3: Create and fully fund new bounties
for bounty_id in new_bounty_ids:
    bounty = Bounty.create(bounty_id, solver_reward, verifier_reward, solver_bond)
    funding_amount = solver_reward + verifier_reward
    wallet.transfer(funding_amount, bounty.wallet_address)
    bounty.set_funded(True)

# Step 4: Require a 0.01 USDC solver bond
for bounty_id in new_bounty_ids:
    bounty = Bounty.get(bounty_id)
    bounty.set_solver_bond(solver_bond)

# Step 5: Reconcile events
for bounty_id in bounty_ids_to_keep + new_bounty_ids:
    bounty = Bounty.get(bounty_id)
    event_manager.publish_event('CanonicalCreation', bounty_id=bounty_id)
    event_manager.publish_event('FundingAdded', bounty_id=bounty_id, amount=bounty.funding_amount)
    if bounty.is_claimable:
        event_manager.publish_event('Claimability', bounty_id=bounty_id)
    if bounty.is_cancelled:
        event_manager.publish_event('Cancellation', bounty_id=bounty_id)
    if bounty.is_refunded:
        event_manager.publish_event('Refund', bounty_id=bounty_id, amount=total_refund)

# Update public status
event_manager.update_public_status()

print("Bounty maintenance completed successfully.")