"use strict";
// Synthetic only. This fixture cannot be mistaken for a public funded bounty.
const contract = "0x" + "1".repeat(40), id = "0x" + "2".repeat(64), hash = "0x" + "3".repeat(64);
const event = (kind, index, data = {}) => ({ id: `fixture-${index}`, kind, bounty_id: id, tx_hash: hash, contract_address: contract, block_number: 100, log_index: index, data });
const item = {
  bounty_contract: contract, bounty_id: id, creator: "0x" + "4".repeat(40), status: "claimable", terms_valid: true, validation_errors: [],
  solver_reward: "15068098", verifier_reward: "2000000", claim_bond: "2000000", funded_amount: "17068098", target_amount: "17068098",
  verification_ready: true, terms_hash: "0x" + "5".repeat(64),
  terms: { document: { title: "An editable rainwater collector with assembly drawings", goal: "Design a modular collector that can be measured, adapted and assembled from clear source files.",
    acceptance_criteria: ["Deliver editable CAD files and STEP and STL exports that open without errors.", "Include a dimensioned adapter, assembly drawings and a bill of materials.", "List the measurements needed before fabrication."],
    contract_terms: { claim_window_seconds: 604800, verification_window_seconds: 172800 }, benchmark: {}, verification_policy: {} } },
  events: [event("canonical_bounty_created", 0, { bounty_contract: contract }), event("funding_added", 1, { funded_amount: 17068098 }), event("bounty_became_claimable", 2)]
};
const amount = value => ({ amount: value, unit: "base_units", decimals: 6, asset: "USDC" });
const opportunity = { opportunity_id: `canonical:base-mainnet:${contract}`, source_id: contract, network: "base-mainnet", source_type: "canonical_base", source_status: "claimable", work_state: "claimable", payment_state: "escrowed", payment_committed: true, verification_ready: true, terms_hash: item.terms_hash,
  title: item.terms.document.title, goal: item.terms.document.goal, reward: amount(item.solver_reward), funded_amount: amount(item.funded_amount), funding_target: amount(item.target_amount), categories: ["Design", "CAD"], skills: ["3D modelling"] };
module.exports = { contract, id, hash, item, opportunity };
