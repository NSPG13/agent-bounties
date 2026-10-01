/* Exact, independently checked authority for the versioned creator-open flow. */
(function (root, factory) {
  const api = factory();
  if (typeof module === "object" && module.exports) module.exports = api;
  else root.AgentBountiesCreatorOpen = api;
})(typeof window === "object" ? window : globalThis, function () {
  "use strict";
  const PROTOCOL = "agent-bounties/creator-open-v1";
  const DISCLOSURE = "The creator judges the published criteria. The first passing creator verdict pays one solver. Rejected entries pay their bond to the creator; unreviewed entries recover only their bond after their review deadline or another winner. There is no exclusive claim or guarantee of payment for work.";
  const ADDRESS = /^0x[0-9a-f]{40}$/i, HASH = /^0x[0-9a-f]{64}$/i;
  const lower = v => String(v || "").toLowerCase();
  const stable = v => v && typeof v === "object" ? Array.isArray(v) ? `[${v.map(stable).join(",")}]` : `{${Object.keys(v).sort().map(k => `${JSON.stringify(k)}:${stable(v[k])}`).join(",")}}` : JSON.stringify(v);
  const fail = message => { throw new Error(message); };
  const integer = (v, min = 0, max = Number.MAX_SAFE_INTEGER) => Number.isSafeInteger(v) && v >= min && v <= max;
  const network = name => name === "base-mainnet" ? { chain: 8453, token: "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913", name: "USD Coin" } : name === "base-sepolia" ? { chain: 84532, token: "0x036cbd53842c5426634e7929541ec2318f3dcf7e", name: "USDC" } : fail("Choose Base or Base Sepolia.");
  const schema = { type: "object", required: ["artifact_url", "artifact_sha256"], properties: { artifact_url: { type: "string", pattern: "^https://" }, artifact_sha256: { type: "string", pattern: "^sha256:[0-9a-f]{64}$" } } };
  const fields = spec => spec.split(",").map(pair => { const [name, type] = pair.split(":"); return { name, type }; });
  const TYPES = {
    ReceiveWithAuthorization: "from:address,to:address,value:uint256,validAfter:uint256,validBefore:uint256,nonce:bytes32",
    OpenSubmission: "solver:address,submissionHash:bytes32,evidenceHash:bytes32,termsHash:bytes32,revision:uint64,tokenNonce:bytes32,deadline:uint256",
    CreatorReview: "entry:uint64,solver:address,submissionHash:bytes32,evidenceHash:bytes32,termsHash:bytes32,revision:uint64,passed:bool,responseHash:bytes32,deadline:uint256",
    IncreaseReward: "revision:uint64,termsHash:bytes32,newTermsHash:bytes32,newReward:uint256,tokenNonce:bytes32,deadline:uint256",
    CancelBounty: "revision:uint64,termsHash:bytes32,deadline:uint256",
  };
  const DOMAIN = fields("name:string,version:string,chainId:uint256,verifyingContract:address");
  function typed(primaryType, domain, message) { return { primaryType, domain, types: { EIP712Domain: DOMAIN, [primaryType]: fields(TYPES[primaryType]) }, message }; }
  function hashJson(value, evm) { return evm.keccak256Hex(evm.textHex(stable(value))); }
  function units(value) {
    if (!/^(?:0|[1-9]\d{0,9})(?:\.\d{1,6})?$/.test(String(value))) fail("Enter a positive USDC amount with at most six decimal places.");
    const [whole, fraction = ""] = String(value).split(".");
    const result = BigInt(whole) * 1000000n + BigInt(fraction.padEnd(6, "0"));
    if (result <= 0n || result > 9000000000000000n) fail("USDC amount is outside the supported range.");
    return Number(result);
  }
  function money(value) { const n = BigInt(value); return `${n / 1000000n}.${(n % 1000000n).toString().padStart(6, "0").replace(/0+$/, "") || "00"} USDC`; }
  function evidence(document, evm) {
    if (!document || Object.keys(document).sort().join() !== "artifact_sha256,artifact_url,notes") fail("The public evidence fields changed.");
    let url; try { url = new URL(document.artifact_url); } catch (_) { fail("Use an HTTPS link to the public artifact."); }
    if (url.protocol !== "https:" || url.username || url.password || document.artifact_url.length > 2048 || !/^sha256:[0-9a-f]{64}$/.test(document.artifact_sha256) || typeof document.notes !== "string" || new TextEncoder().encode(document.notes).length > 8000) fail("Check the public artifact link, SHA-256 and notes.");
    return hashJson(document, evm);
  }
  function config(terms, evm) {
    if (terms.schema !== PROTOCOL || terms.disclosure !== DISCLOSURE || !ADDRESS.test(terms.creator) || /^0x0{40}$/i.test(terms.creator) || !HASH.test(terms.bounty_id)
      || !integer(terms.solver_reward_micro_usdc, 1, 9000000000000000) || !integer(terms.review_reward_micro_usdc, 1, 9000000000000000)
      || !integer(terms.submission_deadline, 1) || !integer(terms.review_window_seconds, 3600, 2592000) || !integer(terms.max_pending_entries, 1, 64)
      || typeof terms.title !== "string" || !terms.title.trim() || new TextEncoder().encode(terms.title).length > 200
      || typeof terms.goal !== "string" || !terms.goal.trim() || new TextEncoder().encode(terms.goal).length > 16000
      || !Array.isArray(terms.acceptance_criteria) || !terms.acceptance_criteria.length || terms.acceptance_criteria.length > 64
      || terms.acceptance_criteria.some(v => typeof v !== "string" || !v.trim() || new TextEncoder().encode(v).length > 4000)) fail("The exact public terms are incomplete or invalid.");
    network(terms.network);
    return evm.bytes32Word(terms.bounty_id) + evm.addressWord(terms.creator) + evm.uint256Word(terms.solver_reward_micro_usdc) + evm.uint256Word(terms.review_reward_micro_usdc)
      + evm.bytes32Word(hashJson(terms, evm)) + evm.bytes32Word(hashJson(terms.acceptance_criteria, evm)) + evm.bytes32Word(hashJson(schema, evm))
      + evm.uint256Word(terms.submission_deadline) + evm.uint256Word(terms.review_window_seconds) + evm.uint256Word(terms.max_pending_entries);
  }
  function predict(release, request, evm) {
    if (release?.protocol !== PROTOCOL || release.network !== request.original_terms.network || !ADDRESS.test(release.factory) || !ADDRESS.test(release.implementation)
      || !HASH.test(release.factory_code_hash) || !HASH.test(release.implementation_code_hash) || !integer(release.deployment_block, 1)) fail("The reviewed release is unavailable.");
    if (!HASH.test(request.user_salt)) fail("The bounty salt is invalid.");
    const salt = evm.keccak256Hex(`0x${config(request.original_terms, evm)}${lower(request.user_salt).slice(2)}`);
    const code = evm.keccak256Hex(`0x3d602d80600a3d3981f3363d3d373d3d3d363d73${lower(release.implementation).slice(2)}5af43d82803e903d91602b57fd5bf3`);
    return `0x${evm.keccak256Hex(`0xff${lower(release.factory).slice(2)}${salt.slice(2)}${code.slice(2)}`).slice(-40)}`;
  }
  function expected(release, r, evm, now = Math.floor(Date.now() / 1000), recovery = false) {
    const net = network(r.original_terms.network), bounty = predict(release, r, evm), a = r.action;
    if (!integer(r.expected_revision, 1) || !integer(r.expected_solver_reward_micro_usdc, r.original_terms.solver_reward_micro_usdc, 9000000000000000)
      || !integer(r.valid_before, 1) || (!recovery && (r.valid_before < now + 30 || r.valid_before > now + 7200))) fail("The request or signature deadline changed. Prepare a fresh review before signing.");
    const current = { ...r.original_terms, solver_reward_micro_usdc: r.expected_solver_reward_micro_usdc };
    const termsHash = hashJson(current, evm), creator = lower(current.creator), domain = { name: "Agent Bounties Creator Open", version: "1", chainId: String(net.chain), verifyingContract: bounty };
    let token = null, action = null, principal = 0n, signer = creator;
    const authorize = (from, value) => {
      principal = BigInt(value); evm.bytes32Word(a.token_nonce);
      token = { signer: from, typed_data: typed("ReceiveWithAuthorization", { name: net.name, version: "2", chainId: String(net.chain), verifyingContract: net.token }, { from, to: bounty, value: String(principal), validAfter: "0", validBefore: String(r.valid_before), nonce: a.token_nonce }) };
    };
    const authority = (type, message) => { action = { signer, typed_data: typed(type, domain, message) }; };
    if (a.kind === "create") {
      if (r.expected_revision !== 1 || r.expected_solver_reward_micro_usdc !== current.solver_reward_micro_usdc || r.expected_solver_reward_micro_usdc !== r.original_terms.solver_reward_micro_usdc
        || (!recovery && (current.submission_deadline <= now || current.submission_deadline > now + 366 * 86400))) fail("The creation terms changed or its work deadline is invalid.");
      authorize(creator, BigInt(current.solver_reward_micro_usdc) + BigInt(current.review_reward_micro_usdc));
    } else if (a.kind === "submit") {
      if (!ADDRESS.test(a.solver) || lower(a.solver) === creator) fail("Use a solver wallet different from the creator.");
      signer = lower(a.solver); evm.bytes32Word(a.submission_hash); evm.bytes32Word(a.evidence_hash);
      if (r.public_evidence && evidence(r.public_evidence, evm) !== lower(a.evidence_hash)) fail("The evidence no longer matches its signed commitment.");
      authorize(signer, current.review_reward_micro_usdc);
      authority("OpenSubmission", { solver: signer, submissionHash: a.submission_hash, evidenceHash: a.evidence_hash, termsHash, revision: r.expected_revision, tokenNonce: a.token_nonce, deadline: String(r.valid_before) });
    } else if (a.kind === "review") {
      if (!integer(a.entry, 1) || !ADDRESS.test(a.solver) || typeof a.passed !== "boolean") fail("Choose the exact submission and verdict.");
      [a.submission_hash, a.evidence_hash, a.response_hash].forEach(v => evm.bytes32Word(v));
      authority("CreatorReview", { entry: a.entry, solver: a.solver, submissionHash: a.submission_hash, evidenceHash: a.evidence_hash, termsHash, revision: r.expected_revision, passed: a.passed, responseHash: a.response_hash, deadline: String(r.valid_before) });
    } else if (a.kind === "increase") {
      if (!integer(a.new_reward_micro_usdc, current.solver_reward_micro_usdc + 1, 9000000000000000)) fail("The new reward must exceed the current reward.");
      authorize(creator, BigInt(a.new_reward_micro_usdc) - BigInt(current.solver_reward_micro_usdc));
      authority("IncreaseReward", { revision: r.expected_revision, termsHash, newTermsHash: hashJson({ ...current, solver_reward_micro_usdc: a.new_reward_micro_usdc }, evm), newReward: String(a.new_reward_micro_usdc), tokenNonce: a.token_nonce, deadline: String(r.valid_before) });
    } else if (a.kind === "cancel") authority("CancelBounty", { revision: r.expected_revision, termsHash, deadline: String(r.valid_before) });
    else if (a.kind === "refund_bond") { if (!integer(a.entry, 1)) fail("Choose the exact bond entry."); }
    else if (!["expire", "refund_principal"].includes(a.kind)) fail("Unsupported creator-review action.");
    if (r.public_evidence && a.kind !== "submit") fail("Only submissions can publish evidence.");
    return { bounty, approval_hash: hashJson({ release, request: r }, evm), token_authorization: token, action_authorization: action, principal: String(principal), gas: "0" };
  }
  function sameTyped(actual, wanted) {
    if (!actual || actual.primaryType !== wanted.primaryType || stable(actual.types) !== stable(wanted.types)) return false;
    for (const [part, descriptors] of [["domain", DOMAIN], ["message", wanted.types[wanted.primaryType]]]) {
      if (Object.keys(actual[part] || {}).sort().join() !== Object.keys(wanted[part]).sort().join()) return false;
      for (const { name, type } of descriptors) {
        const a = actual[part][name], b = wanted[part][name];
        if (type.startsWith("uint")) { try { if (BigInt(a) !== BigInt(b)) return false; } catch (_) { return false; } }
        else if (type === "address" || type === "bytes32") { if (lower(a) !== lower(b)) return false; }
        else if (a !== b) return false;
      }
    }
    return true;
  }
  function validatePrepared(p, release, request, evm, now) {
    const local = expected(release, request, evm, now);
    if (p?.protocol !== PROTOCOL || p.network !== release.network || stable(p.request) !== stable(request) || lower(p.bounty_contract) !== local.bounty
      || lower(p.approval_hash) !== local.approval_hash || p.customer_principal_micro_usdc !== local.principal || p.customer_gas_wei !== "0") fail("The prepared action differs from your reviewed terms, amount or release.");
    for (const key of ["token_authorization", "action_authorization"]) {
      if (!local[key] ? p[key] != null : !p[key] || lower(p[key].signer) !== local[key].signer || !sameTyped(p[key].typed_data, local[key].typed_data)) fail("The wallet authority differs from the exact action you reviewed.");
    }
    return local;
  }
  function actionBlocker(item, kind, wallet, now = Math.floor(Date.now() / 1000)) {
    if (!item) return "Refresh the canonical bounty before preparing an action.";
    if (!ADDRESS.test(wallet)) return "Connect the wallet that will authorize this action.";
    const creator = lower(item.terms.creator) === lower(wallet), open = item.status === "open", entries = item.entries || [];
    const pending = entries.filter(v => v.entry.status === "pending");
    if (["review", "increase", "cancel"].includes(kind) && !creator) return "Connect the creator wallet for this action.";
    if (kind === "submit") {
      if (creator) return "The creator cannot submit to their own bounty. Connect a separate solver wallet.";
      if (!open || now >= item.terms.submission_deadline) return "Submissions are closed.";
      if (pending.some(v => lower(v.entry.solver) === lower(wallet))) return "Your existing submission is awaiting review. Finish or recover it before submitting again.";
      if (pending.length >= item.terms.max_pending_entries) return "The review queue is full. Wait for a review or eligible bond refund.";
    }
    if (kind === "review" && (!open || !pending.some(v => v.entry.review_deadline > now + 30))) return "No pending submission has enough time left for a new review authorization.";
    if (["cancel", "increase"].includes(kind) && (!open || item.entry_count !== 0)) return "This action is available only before the first submission.";
    if (kind === "increase" && now >= item.terms.submission_deadline) return "The reward cannot increase after the submission deadline.";
    if (kind === "expire" && (!open || now <= item.terms.submission_deadline + item.terms.review_window_seconds)) return "Closing becomes available after the submission deadline plus the full review window.";
    if (kind === "refund_principal" && (item.status !== "cancelled" || !/^[1-9][0-9]*$/.test(item.principal_micro_usdc))) return "Close the bounty first; only remaining creator funds can be returned.";
    if (kind === "refund_bond" && !pending.some(v => !open || now > v.entry.review_deadline)) return "No pending bond is refundable yet. Wait until its review deadline or a different winner.";
    return null;
  }
  return Object.freeze({ PROTOCOL, DISCLOSURE, ADDRESS, HASH, stable, network, units, money, hashJson, evidence, config, predict, expected, sameTyped, validatePrepared, actionBlocker });
});
