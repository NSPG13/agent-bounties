(function (root, factory) {
  const api = factory();
  if (typeof module === "object" && module.exports) module.exports = api;
  else root.AgentBountiesMetaChild = api;
})(typeof window === "object" ? window : globalThis, function () {
  "use strict";
  const ADDRESS = /^0x[0-9a-f]{40}$/i;
  const PROTOCOL = "agent-bounties/independent-child-v3-routed";
  const REGISTRY = "0x35e5d49c12b75c119d33951c2c4f054c5732208c";
  const PARTICIPANTS = "0x9875dcaf570bde8ff1aa62275d3c8985f4fd1294";
  const SET_HASH = "0x2c5a10915ca1fb99d4a11e2222b4f32b986b4e0f5599f55d70e9c8f9725a28cd";
  const VERIFIERS = ["0xbe6292b9e465f549e2363b918d6dd9187038431e", "0xb7c2ce6430b66fb986e27b6140b29309550d487a"];
  const FACTORY = "0x082c52131aaf0c56e76b075f895eab6fcab6d2f9";
  const TOKEN = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913";
  const lower = (v) => String(v || "").toLowerCase();
  const canonical = (v) => JSON.stringify(sort(v));
  function sort(v) { return Array.isArray(v) ? v.map(sort) : v && typeof v === "object" ? Object.fromEntries(Object.keys(v).sort().map((k) => [k, sort(v[k])])) : v; }
  function check(ok, message) { if (!ok) throw new Error(message); }
  function normalize(value) {
    if (value == null) return null;
    check(value && typeof value === "object" && !Array.isArray(value), "Supply one meta_child context.");
    check(ADDRESS.test(value.parent_bounty_contract) && !/^0x0{40}$/i.test(value.parent_bounty_contract), "Choose a canonical parent bounty.");
    if (value.intended_child_solver != null) check(ADDRESS.test(value.intended_child_solver) && !/^0x0{40}$/i.test(value.intended_child_solver), "The intended child solver must be a public wallet address.");
    return { parent_bounty_contract: lower(value.parent_bounty_contract), intended_child_solver: value.intended_child_solver ? lower(value.intended_child_solver) : null };
  }
  async function resolve(value, client) {
    const context = normalize(value);
    if (!context) return null;
    const item = await client.opportunity(context.parent_bounty_contract, true);
    const inspection = await client.inspect(item.opportunity_id);
    const record = inspection.terms, terms = record?.document, benchmark = terms?.benchmark;
    check(item.network === "base-mainnet" && item.work_state === "claimable" && item.verification_ready === true
      && item.payment_committed === true && lower(item.source_id) === context.parent_bounty_contract,
    "The parent must still be canonically funded, claimable and verification-ready.");
    check(benchmark?.engine === "standing_meta_v3_routed_parent" && benchmark.minimum_child_target === 1000000
      && benchmark.minimum_parent_gross_margin === 1000000 && benchmark.required_child_engine === "sandboxed_regression_v1"
      && benchmark.required_child_verifier_threshold === 2 && lower(benchmark.required_child_verifier_set_hash) === SET_HASH
      && lower(benchmark.terms_registry) === REGISTRY && lower(benchmark.participant_registry) === PARTICIPANTS
      && terms.verification_policy?.module_id === "policy_bound_verifier_router_v1"
      && lower(terms.verification_policy?.verifier_module) === "0x380c1af742593dd88b6f20387e9ee693a0536731"
      && record.terms_hash === item.terms_hash
      && record.acceptance_criteria_hash === "0xba3b04ab970dfd91f5ccf1b7eda6670b5a38a854bca16dc980ec8362ed2bcaf9",
    "This parent does not support the qualifying 1 USDC routed-V3 child exception.");
    check(Number(terms.contract_terms?.solver_reward?.amount) >= 2000000, "The parent cannot preserve the required 1 USDC margin.");
    return { ...context, protocol: PROTOCOL, total_base_units: "1000000", title: item.title, terms_hash: item.terms_hash, terms };
  }
  function rewards(solver, verifier, parent) {
    check(parent?.protocol === PROTOCOL && parent.total_base_units === "1000000", "Resolve the canonical parent before using its child minimum.");
    check(solver > 0n && verifier >= 10000n && verifier % 2n === 0n && solver + verifier === 1000000n,
      "This parent requires exactly 1 USDC total: a positive solver reward and at least 0.01 USDC divided evenly between two verifiers.");
    return { solver, verifier, total: solver + verifier };
  }
  function request(draft, parent, account, split, days, nonce) {
    rewards(split.solver, split.verifier, parent);
    check(ADDRESS.test(account) && lower(account) !== lower(parent.terms.contract_terms.creator_wallet), "The parent creator cannot be the parent solver.");
    check(parent.intended_child_solver && parent.intended_child_solver !== lower(account), "Choose a different intended child solver; both participants must be registered before the parent claim.");
    check(draft.benchmark?.source && draft.benchmark?.runner_manifest, "Prepare the exact executable child benchmark before funding.");
    return { network: "base-mainnet", parent_bounty_contract: parent.parent_bounty_contract, parent_solver: lower(account),
      intended_child_solver: parent.intended_child_solver, title: draft.title, goal: draft.goal, acceptance_criteria: draft.acceptance_criteria,
      benchmark_source: draft.benchmark.source, runner_manifest: draft.benchmark.runner_manifest, evidence_schema: draft.evidence_schema,
      verifier_reward: { amount: Number(split.verifier), currency: "usdc" }, claim_window_seconds: days * 86400,
      verification_window_seconds: 48 * 3600, funding_deadline: parent.terms.contract_terms.funding_deadline,
      source_url: draft.source_url || null, creation_nonce: nonce, discovery_source: "WebMCP routed-V3 child review" };
  }
  // Reconstruct all three wallet calls from the reviewed commitment, rather than
  // treating an arbitrary planner-provided transaction as authorization.
  function validatePlan(plan, input, split, evm) {
    const record = plan?.terms, doc = record?.document, committed = doc?.contract_terms, create = plan?.child_create;
    check(plan?.protocol_version === PROTOCOL && plan.network?.chain_id === 8453
      && lower(plan.parent_bounty_contract) === input.parent_bounty_contract && lower(plan.parent_solver) === input.parent_solver
      && lower(plan.intended_child_solver) === input.intended_child_solver && Number.isSafeInteger(plan.parent_round) && plan.parent_round > 0
      && lower(plan.terms_registry) === REGISTRY && plan.task_verifier_threshold === 2
      && lower(plan.task_verifier_set_hash) === SET_HASH && canonical(plan.task_verifiers) === canonical(VERIFIERS), "The child planner changed the parent, wallet, network or verifier quorum.");
    check(doc?.title === input.title && doc.goal === input.goal && (doc.source_url || null) === (input.source_url || null)
      && doc.discovery_source === input.discovery_source && canonical(doc.acceptance_criteria) === canonical(input.acceptance_criteria)
      && canonical(doc.evidence_schema) === canonical(input.evidence_schema)
      && canonical(doc.benchmark) === canonical({ engine: "sandboxed_regression_v1", source: input.benchmark_source, runner_manifest: input.runner_manifest,
        parent_binding: { protocol: PROTOCOL, parent_bounty_contract: input.parent_bounty_contract, parent_bounty_id: plan.parent_bounty_id, parent_round: plan.parent_round } }),
    "The child planner changed the reviewed work or executable benchmark.");
    check(["Base", "base-mainnet"].includes(committed?.network) && lower(committed.creator_wallet) === input.parent_solver && lower(committed.settlement_token) === TOKEN
      && ["solver_reward", "verifier_reward", "claim_bond", "initial_funding"].every(key => committed[key]?.currency === "usdc")
      && BigInt(committed.solver_reward?.amount ?? -1) === split.solver && BigInt(committed.verifier_reward?.amount ?? -1) === split.verifier
      && BigInt(committed.claim_bond?.amount ?? -1) === split.verifier && BigInt(committed.initial_funding?.amount ?? -1) === split.total
      && committed.claim_window_seconds === input.claim_window_seconds && committed.verification_window_seconds === input.verification_window_seconds
      && committed.creation_nonce === input.creation_nonce && committed.funding_deadline === input.funding_deadline && committed.funding_deadline > Date.now() / 1000,
    "The child planner changed the reviewed funding, bond, nonce or work windows.");
    check(doc.verification_policy?.mechanism === "signed_quorum" && doc.verification_policy.engine === "sandboxed_regression_v1"
      && doc.verification_policy.threshold === 2 && canonical(doc.verification_policy.verifiers) === canonical(VERIFIERS), "The child requires the parent's exact two-verifier policy.");
    const hash = (v) => evm.keccak256Hex(evm.textHex(canonical(v)));
    for (const [key, value] of [["terms_hash", doc], ["policy_hash", doc.verification_policy], ["acceptance_criteria_hash", doc.acceptance_criteria], ["benchmark_hash", doc.benchmark], ["evidence_schema_hash", doc.evidence_schema]]) {
      check(record[key] === hash(value) && create?.[key] === record[key], `The child ${key} does not match its reviewed preimage.`);
    }
    check(plan.canonical_terms_json === canonical(doc) && lower(plan.canonical_terms_hex) === evm.textHex(canonical(doc)), "The on-chain child terms differ from the reviewed document.");
    const w = evm.uint256Word, b = evm.bytes32Word, a = evm.addressWord;
    const selector = (signature) => evm.keccak256Hex(evm.textHex(signature)).slice(0, 10);
    const raw = plan.canonical_terms_hex.slice(2), padded = raw.padEnd(Math.ceil(raw.length / 64) * 64, "0");
    const publish = selector("publish(bytes,(bytes32,uint64,bytes32,bytes32,bytes32,bytes32,bytes32,uint8))")
      + [w(9 * 32), b(plan.parent_bounty_id), w(plan.parent_round), b(record.policy_hash), b(record.acceptance_criteria_hash), b(record.benchmark_hash), b(record.evidence_schema_hash), b(SET_HASH), w(2), w(raw.length / 2), padded].join("");
    const approve = selector("approve(address,uint256)") + a(FACTORY) + w(split.total);
    const zero = "0x" + "0".repeat(40);
    const creation = selector("createBounty((uint256,uint256,bytes32,bytes32,bytes32,bytes32,bytes32,uint64,uint64,uint64,uint8,address,address,uint8),address[],uint256,bytes32)")
      + [w(split.solver), w(split.verifier), b(record.terms_hash), b(record.policy_hash), b(record.acceptance_criteria_hash), b(record.benchmark_hash), b(record.evidence_schema_hash),
        w(committed.funding_deadline), w(committed.claim_window_seconds), w(committed.verification_window_seconds), w(1), a(zero), a(zero), w(2),
        w(17 * 32), w(split.total), b(input.creation_nonce), w(2), ...VERIFIERS.map(a)].join("");
    const expected = [[REGISTRY, publish], [TOKEN, approve], [FACTORY, creation]];
    check(plan.pre_claim_wallet_calls?.length === expected.length, "Unexpected child wallet actions.");
    expected.forEach(([to, data], i) => {
      const call = plan.pre_claim_wallet_calls[i];
      check(lower(call.from) === input.parent_solver && lower(call.to) === to && String(call.value_wei) === "0" && lower(call.data) === data,
        "The child wallet action does not match the reviewed destination, amount or terms.");
    });
    check(plan.child_creation?.network?.chain_id === 8453 && lower(plan.child_creation.factory_contract) === FACTORY
      && ADDRESS.test(plan.child_creation.predicted_bounty_contract) && /^0x[0-9a-f]{64}$/i.test(plan.child_creation.bounty_id), "Invalid canonical child creation plan.");
    return plan;
  }
  return { PROTOCOL, REGISTRY, PARTICIPANTS, SET_HASH, VERIFIERS, normalize, resolve, rewards, request, validatePlan, canonical, check, lower };
});
