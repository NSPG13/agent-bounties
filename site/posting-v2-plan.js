(() => {
  "use strict";

  // The direct-wallet recovery route uses the existing immutable v2 factory.
  // This module never signs, broadcasts, publishes terms, or contacts a relay.
  const VERSION = "agent-bounties/autonomous-v2";
  const ZERO = `0x${"0".repeat(40)}`, ZERO32 = `0x${"0".repeat(64)}`;
  const lower = value => String(value || "").toLowerCase();
  const fail = () => { throw new Error("The V2 plan differs from the reviewed terms, fee, factory or wallet. No wallet request was sent."); };
  function uint(value) {
    if (!["string", "number", "bigint"].includes(typeof value)
      || (typeof value === "number" && !Number.isSafeInteger(value))
      || !/^(0|[1-9][0-9]*)$/.test(String(value))) return fail();
    const result = BigInt(value);
    if (result >= (1n << 256n)) return fail();
    return result;
  }
  const word = value => uint(value).toString(16).padStart(64, "0");
  function address(value) {
    if (!/^0x[0-9a-f]{40}$/.test(lower(value))) return fail();
    return lower(value).slice(2).padStart(64, "0");
  }
  function bytes32(value) {
    if (!/^0x[0-9a-f]{64}$/.test(lower(value))) return fail();
    return lower(value).slice(2);
  }
  function money(value) {
    if (lower(value?.currency) !== "usdc") return fail();
    return uint(value.amount);
  }
  function economics(solver, verifier, protocol) {
    solver = uint(solver); verifier = uint(verifier);
    if (protocol?.protocol_version !== VERSION || solver === 0n
      || uint(protocol.platform_fee_bps) !== 750n || lower(protocol.platform_fee_recipient) === ZERO) return fail();
    address(protocol.platform_fee_recipient);
    const fee = (solver * 750n + 9999n) / 10000n;
    const total = solver + verifier + fee;
    if (total > BigInt(Number.MAX_SAFE_INTEGER)) return fail();
    return { solver, verifier, fee, total };
  }
  function validateQuote(quote, create, protocol) {
    const amounts = economics(money(create.solver_reward), money(create.verifier_reward), protocol);
    if (quote?.protocol_version !== VERSION || uint(quote.platform_fee_bps) !== 750n
      || lower(quote.platform_fee_recipient) !== lower(protocol.platform_fee_recipient)
      || uint(quote.solver_reward) !== amounts.solver || uint(quote.verifier_reward) !== amounts.verifier
      || uint(quote.claim_bond) !== amounts.verifier || uint(quote.platform_fee) !== amounts.fee
      || uint(quote.target_amount) !== amounts.total || money(create.initial_funding) !== amounts.total) return fail();
    return amounts;
  }
  function encode(create, protocol) {
    const evm = window.AgentBountiesEvm;
    if (!evm || ![8453, 84532].includes(Number(protocol.chain_id))) return fail();
    // Creator review and the existing signed quorum share this ABI. Module and
    // claim-gated profiles need their own reviewed UI, so are not inferred here.
    if (create.verification_mode !== "signed_quorum" || create.verifier_module != null
      || create.verifier_reward_recipient != null || create.claim_eligibility_registry != null
      || create.claim_eligibility_source != null || !Array.isArray(create.verifiers)
      || !create.verifiers.length || create.verifiers.length > 255
      || uint(create.threshold) < 1n || uint(create.threshold) > BigInt(create.verifiers.length)
      || new Set(create.verifiers.map(lower)).size !== create.verifiers.length
      || create.verifiers.some(v => lower(v) === ZERO)) return fail();
    for (const key of ["funding_deadline", "claim_window_seconds", "verification_window_seconds"])
      if (uint(create[key]) < 1n || uint(create[key]) >= (1n << 64n)) return fail();
    const params = [word(money(create.solver_reward)), word(money(create.verifier_reward)),
      ...["terms_hash", "policy_hash", "acceptance_criteria_hash", "benchmark_hash", "evidence_schema_hash"].map(k => bytes32(create[k])),
      word(create.funding_deadline), word(create.claim_window_seconds), word(create.verification_window_seconds),
      word(1), address(ZERO), address(ZERO), word(create.threshold), address(ZERO), bytes32(ZERO32)];
    const tail = word(create.verifiers.length) + create.verifiers.map(address).join("");
    const data = "0xed6662a2" + params.join("") + word(19 * 32)
      + word(money(create.initial_funding)) + bytes32(create.creation_nonce) + tail;
    const bountyId = evm.keccak256Hex("0x" + word(protocol.chain_id) + address(protocol.factory)
      + address(create.creator) + bytes32(create.creation_nonce) + params.join("") + word(21 * 32) + tail);
    const initCodeHash = evm.keccak256Hex("0x3d602d80600a3d3981f3363d3d373d3d3d363d73"
      + address(protocol.implementation).slice(-40) + "5af43d82803e903d91602b57fd5bf3");
    const bounty = "0x" + evm.keccak256Hex("0xff" + address(protocol.factory).slice(-40)
      + bytes32(bountyId) + bytes32(initCodeHash)).slice(-40);
    return { data, bountyId, bounty };
  }
  function validatePlan(plan, create, protocol) {
    const amounts = validateQuote(plan?.quote, create, protocol), encoded = encode(create, protocol);
    if (plan.protocol_version !== VERSION || Number(plan.network?.chain_id) !== Number(protocol.chain_id)
      || lower(plan.network?.native_usdc_token_address) !== lower(protocol.native_usdc)
      || lower(plan.factory_contract) !== lower(protocol.factory)
      || lower(plan.implementation_contract) !== lower(protocol.implementation)
      || lower(plan.bounty_id) !== encoded.bountyId || lower(plan.predicted_bounty_contract) !== encoded.bounty
      || !Array.isArray(plan.wallet_calls) || plan.wallet_calls.length !== 2) return fail();
    const expected = [
      { to: protocol.native_usdc, data: "0x095ea7b3" + address(protocol.factory) + word(amounts.total) },
      { to: protocol.factory, data: encoded.data },
    ];
    for (const [index, call] of plan.wallet_calls.entries()) {
      if (lower(call.from) !== lower(create.creator) || lower(call.to) !== lower(expected[index].to)
        || lower(call.data) !== expected[index].data || uint(call.value_wei) !== 0n
        || (call.value != null && uint(call.value) !== 0n)) return fail();
    }
    // Copy only checked transaction fields; callers must not reuse mutable API
    // objects after validation or accidentally select the signature/relay path.
    return { ...amounts, bountyId: encoded.bountyId, bounty: encoded.bounty,
      calls: expected.map(call => ({ ...call, from: create.creator, value_wei: 0 })) };
  }
  window.AgentBountiesPostingV2Plan = Object.freeze({ VERSION, economics, validateQuote, encode, validatePlan });
})();
