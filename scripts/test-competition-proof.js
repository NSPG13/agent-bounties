"use strict";
const { test } = require("node:test"), assert = require("node:assert/strict"), fs = require("node:fs"), vm = require("node:vm");
const { webcrypto } = require("node:crypto");
const proof = require("../site/competition-proof.js"), flow = require("../site/marketplace-workflow.js");
const contract = "0x" + "11".repeat(20), wallet = "0x" + "22".repeat(20), broker = "0x" + "33".repeat(20);
const hash = (byte) => "0x" + byte.repeat(64), id = "10000000-0000-4000-8000-000000000001";
const evmWindow = {};
vm.runInNewContext(fs.readFileSync(require.resolve("../site/evm.js"), "utf8"), { window: evmWindow, crypto: webcrypto, TextEncoder });
const evm = evmWindow.AgentBountiesEvm, clone = (v) => JSON.parse(JSON.stringify(v));
function responseQuote(request) {
  const q = { network: "eip155:8453", competition_contract: contract, solver: request.solver, solver_nonce: request.solver_nonce,
    artifact_hash: request.artifact_hash || hash("a"), proof_system: "groth16", quote_id: hash("b"), gross_prize: "3000000", proof_fee_quote: "100000", relay_fee_quote: "10000",
    net_prize_if_win: "2890000", maximum_charge: "110000", winner_mode: "best_score", competition_risk: "Another entry can win.", quote_expiration: Math.floor(Date.now() / 1000) + 299 };
  return { proof_job_id: id, quote: q, payment_required: { x402Version: 2,
    resource: { url: `https://api.agentbounties.app/v1/base/open-competition-v2-beta3/proof-jobs/${id}/payment` },
    accepts: [{ scheme: "exact", network: "eip155:8453", asset: "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913", amount: "110000", payTo: broker, maxTimeoutSeconds: 300,
      extra: { maximumCharge: "110000", quoteId: q.quote_id, competition: contract, solver: request.solver, artifactHash: q.artifact_hash, proofSystem: "groth16", assetTransferMethod: "eip3009", name: "USD Coin", version: "2" } }] } };
}
function setup(storage = new Map(), server = {}) {
  const elements = new Map(), calls = [], requests = [], windowEvents = new Map();
  const element = () => ({ hidden: false, disabled: false, textContent: "", listeners: new Map(), addEventListener(k, fn) { this.listeners.set(k, fn); }, scrollIntoView() {}, replaceChildren() {}, append() {} });
  const doc = { hidden: false, querySelector(k) { if (!elements.has(k)) elements.set(k, element()); return elements.get(k); }, createElement: element };
  server.projection ||= { competition: contract, bounty_id: hash("c"), state: "active", proof_deadline: Math.floor(Date.now() / 1000) + 3600,
    program_vkey: hash("1"), source_hash: hash("2"), elf_hash: hash("3"), journal_schema_hash: hash("4"), metric_program_hash: hash("5"), execution_policy_hash: hash("6"), verification_policy_hash: hash("7"), settlement_policy_hash: hash("8"), beta_risk_hash: hash("9") };
  server.events ||= [];
  server.item ||= { opportunity_id: `open-competition-v2:base-mainnet:${contract}`, source_id: contract, network: "base-mainnet", evidence_requirements: { program_profile: "structured-artifact-metric-v1" } };
  const provider = { async request(req) {
    calls.push(req);
    if (req.method === "eth_accounts") return [server.account || wallet];
    if (req.method === "eth_chainId") return "0x2105";
    if (req.method === "eth_call") return "0x989680";
    if (req.method === "eth_signTypedData_v4") {
      if (server.signError) throw Object.assign(new Error("Wallet response interrupted"), { code: server.signError });
      return "0x" + "ab".repeat(65);
    }
    throw new Error(`Unexpected wallet method ${req.method}`);
  } };
  const win = { document: doc, location: new URL(`https://agentbounties.app/competition.html?bountyContract=${contract}`), history: { replaceState() {} }, crypto: webcrypto,
    AgentBountiesEvm: evm, AgentBountiesWorkflow: flow, AgentBountiesLegal: { requireAcceptance: async () => ({ durable: true }) }, ethereum: provider,
    sessionStorage: { getItem: (k) => storage.get(k) || null, setItem: (k, v) => { if (server.noStorage) throw new Error("Storage unavailable"); storage.set(k, v); } },
    addEventListener(name, listener) { windowEvents.set(name, listener); }, dispatchEvent(event) { windowEvents.get(event.type)?.(event); }, Event: class {}, setInterval() {}, btoa: (s) => Buffer.from(s, "binary").toString("base64"),
    async fetch(url, options = {}) {
      requests.push({ url, ...options });
      const body = options.body && JSON.parse(options.body);
      let data;
      if (url.includes("/inventory?")) data = { network: "base-mainnet", competitions: [{ record: { projection: server.projection } }] };
      else if (url.includes("/opportunities?")) data = { schema_version: "agent-bounties/opportunity-projection-v1", items: [server.item] };
      else if (url.endsWith("/proof-quotes")) {
        server.quoteCount = (server.quoteCount || 0) + 1;
        data = responseQuote(body); server.quoted = clone(data);
        server.job = { ...data.quote, id, idempotency_key: data.quote.quote_id, network: "base-mainnet", requested_relay: true, state: "quoted", quote_expires_at: new Date(data.quote.quote_expiration * 1000).toISOString() };
      } else if (url.endsWith("/payment")) {
        if (server.job.state === "quoted" && !options.headers["PAYMENT-SIGNATURE"]) return { ok: false, status: 402, json: async () => ({ paymentRequired: server.quoted.payment_required }) };
        if (server.dropPayment) { server.dropPayment = false; throw new Error("Lost payment response"); }
        server.paymentCount = (server.paymentCount || 0) + 1;
        if (options.headers["PAYMENT-SIGNATURE"]) server.envelope = JSON.parse(Buffer.from(options.headers["PAYMENT-SIGNATURE"], "base64").toString());
        server.job.state = server.pending ? "payment_pending" : "paid"; data = {};
      } else if (url.endsWith("/relay-authorization")) {
        if (body.solver_signature) {
          if (server.dropRelay) { server.dropRelay = false; throw new Error("Lost relay response"); }
          server.relayCount = (server.relayCount || 0) + 1; server.job.state = "relaying"; data = {};
        } else data = { plan: { relay_authorization: expectedRelay(server.job, body.authorization_deadline) } };
        if (server.badRelay && data.plan) data.plan.relay_authorization.message.proofHash = hash("f");
      } else if (url.includes("/proof-jobs/")) data = { job: clone(server.job) };
      else if (url.includes("/events?")) data = { events: server.events };
      else if (url.includes("/generated/gmv-snapshots/")) { if (server.noSnapshot) return { ok: false, status: 404 }; data = server.snapshot; }
      else throw new Error(`Unexpected API call ${url}`);
      return { ok: true, json: async () => data };
    } };
  const click = (name, trusted = true) => doc.querySelector(`[data-proof-${name}]`).listeners.get("click")({ isTrusted: trusted });
  return { win, doc, calls, requests, server, storage, elements, click, async start() { return proof.start(win, doc); } };
}
function expectedRelay(job, deadline) {
  const fields = (rows) => rows.map(([name, type]) => ({ name, type }));
  return { types: { EIP712Domain: fields([["name", "string"], ["version", "string"], ["chainId", "uint256"], ["verifyingContract", "address"]]),
    SubmitProof: fields([["solver", "address"], ["solverNonce", "uint256"], ["publicValuesHash", "bytes32"], ["proofHash", "bytes32"], ["authorizationDeadline", "uint256"]]) },
    primaryType: "SubmitProof", domain: { name: "Agent Bounties Open Competition V2 Beta3", version: "1", chainId: 8453, verifyingContract: contract },
    message: { solver: wallet, solverNonce: job.solver_nonce, publicValuesHash: job.public_values_hash, proofHash: job.proof_hash, authorizationDeadline: String(deadline) } };
}
function proved(server) {
  const p = server.projection, j = server.job;
  const fields = [evm.keccak256Hex(evm.textHex("agent-bounties/open-competition-v2-beta3/journal")).slice(2), evm.uint256Word(8453), evm.addressWord(contract), p.bounty_id.slice(2), evm.addressWord(wallet), evm.uint256Word(j.solver_nonce), j.artifact_hash.slice(2), hash("d").slice(2), hash("e").slice(2), ...[p.program_vkey, p.source_hash, p.elf_hash, p.journal_schema_hash, p.metric_program_hash, p.execution_policy_hash, p.verification_policy_hash, p.settlement_policy_hash, p.beta_risk_hash].map((v) => v.slice(2)), evm.uint256Word(1), evm.uint256Word(25)];
  j.state = "proved"; j.proof = "0x12345678"; j.public_values = "0x" + fields.join(""); j.expected_public_values = j.public_values;
  j.proof_hash = evm.keccak256Hex(j.proof); j.public_values_hash = evm.keccak256Hex(j.public_values);
}
const input = () => ({ solver: wallet, metric: { profile_id: "structured-artifact-metric-v1", threshold: "1", artifact_utf8: "Hello 🌍", requirements: [{ kind: "utf8_contains", needle: "Hello", minimum_occurrences: 1, weight: 1 }] } });
const sigs = (env) => env.calls.filter((r) => r.method === "eth_signTypedData_v4");
test("resuming before a proof quote returns guidance without wallet or service mutations", async () => {
  const e = setup(), api = await e.start();
  for (let attempt = 0; attempt < 2; attempt++) {
    const result = await api.resume();
    assert.equal(result.state, "not_quoted");
    assert.equal(result.next_action.action, "prepare_quote");
    assert.equal(result.proof_job_id, null);
  }
  assert.equal(e.calls.length, 0);
  assert.equal(e.requests.some((request) => request.method === "POST"), false);
  assert.equal(e.server.quoteCount, undefined);
  assert.equal(e.doc.querySelector("[data-proof-workspace] [data-legal-consent]").hidden, true);
});
test("a phone connection is adopted by the proof review without another connect click or signature", async () => {
  const e = setup(); await e.start();
  e.win.AgentBountiesPhoneWallet = { provider: e.win.ethereum };
  e.win.dispatchEvent({ type: "agent-bounties:phone-wallet-state", detail: { connected: true } });
  await new Promise(setImmediate);
  assert.match(e.doc.querySelector("[data-proof-wallet-status]").textContent, new RegExp(wallet));
  assert.equal(sigs(e).length, 0); assert.equal(e.calls.some((request) => request.method === "eth_requestAccounts"), false);
});

test("quote preparation derives the domain-bound UTF-8 artifact hash, reuses the job and never signs", async () => {
  const e = setup(), api = await e.start(); await api.prepareQuote(input()); await api.prepareQuote(input());
  assert.equal(e.server.quoteCount, 1); assert.equal(sigs(e).length, 0);
  const bytes = Buffer.from("Hello 🌍", "utf8"), length = Buffer.alloc(4); length.writeUInt32BE(bytes.length);
  const expected = evm.keccak256Hex("0x" + "6c3e2c182e83869d996ddb7c5a78d3d43a611c656ef04d03c053e39fd2315659" + length.toString("hex") + bytes.toString("hex"));
  assert.equal(e.server.job.artifact_hash, expected);
  assert.equal(api.status().next_action.action, "pay");
  assert.match(e.doc.querySelector("[data-proof-wallet-setup]").href, /return=/);
});
test("only a trusted human action signs the exact capped native-USDC charge", async () => {
  const e = setup(), api = await e.start(); await api.prepareQuote(input()); await e.click("connect");
  await e.click("pay", false); assert.equal(sigs(e).length, 0);
  await e.click("pay"); assert.equal(sigs(e).length, 1);
  const typed = JSON.parse(sigs(e)[0].params[1]);
  assert.equal(typed.domain.chainId, 8453); assert.equal(typed.primaryType, "TransferWithAuthorization");
  assert.equal(typed.message.value, "110000"); assert.equal(typed.message.to, broker); assert.equal(typed.message.from, wallet);
  assert.equal(e.server.envelope.payload.authorization.nonce, typed.message.nonce);
  assert.equal(api.status().paid, false); await e.click("pay"); assert.equal(sigs(e).length, 1);
});
test("a lost payment response survives reload and retries the same signed authorization without another wallet prompt", async () => {
  const e = setup(), api = await e.start(); await api.prepareQuote(input()); await e.click("connect"); e.server.dropPayment = true; await e.click("pay");
  assert.match(e.doc.querySelector("[data-proof-status]").textContent, /Lost payment response/);
  const saved = [...e.storage.values()][0]; assert.match(saved, /paymentEnvelope/);
  const reload = setup(e.storage, e.server), restored = await reload.start();
  await restored.resume(); assert.equal(sigs(reload).length, 0); assert.equal(e.server.paymentCount, 1);
  assert.doesNotMatch(JSON.stringify(restored.status()), /signature|paymentEnvelope|authorization/);
  assert.doesNotMatch([...e.storage.values()][0], /paymentEnvelope/);
});
test("pending payment resumes server reconciliation with no replacement signature or quote", async () => {
  const e = setup(), api = await e.start(); await api.prepareQuote(input()); await e.click("connect"); e.server.pending = true; await e.click("pay");
  e.server.pending = false; await api.resume(); assert.equal(sigs(e).length, 1);
  assert.equal(e.requests.filter((r) => r.url.endsWith("/payment")).at(-1).headers["PAYMENT-SIGNATURE"], undefined);
  assert.equal(api.status().state, "paid");
});
test("wallet account changes and unavailable recovery storage fail before signing", async () => {
  const e = setup(), api = await e.start(); await api.prepareQuote(input()); await e.click("connect"); e.server.account = broker; await e.click("pay"); assert.equal(sigs(e).length, 0);
  e.server.account = wallet; e.server.noStorage = true; await e.click("pay"); assert.equal(sigs(e).length, 0);
});
test("an ambiguous wallet reply retains the payment nonce and blocks changing the entry", async () => {
  const e = setup(), api = await e.start(); await api.prepareQuote(input()); await e.click("connect"); e.server.signError = -32000; await e.click("pay");
  const nonce = JSON.parse([...e.storage.values()][0]).paymentData.message.nonce;
  await assert.rejects(api.prepareQuote({ ...input(), metric: { ...input().metric, artifact_utf8: "Changed" } }), /reconciled/);
  e.server.signError = null; await e.click("pay"); assert.equal(JSON.parse(sigs(e).at(-1).params[1]).message.nonce, nonce);
});
test("relay authorization binds the exact journal, proof, wallet, nonce and deadline and survives a lost response", async () => {
  const e = setup(), api = await e.start(); await api.prepareQuote(input()); await e.click("connect"); await e.click("pay"); proved(e.server); await api.refresh();
  e.server.dropRelay = true; await e.click("relay"); assert.equal(sigs(e).length, 2);
  const typed = JSON.parse(sigs(e)[1].params[1]); assert.equal(typed.primaryType, "SubmitProof"); assert.equal(typed.message.proofHash, e.server.job.proof_hash);
  const reload = setup(e.storage, e.server), restored = await reload.start(); await restored.resume();
  assert.equal(e.server.relayCount, 1); assert.equal(sigs(reload).length, 0); assert.equal(restored.status().paid, false);
});
test("a changed relay payload or proof journal cannot obtain a signature", async () => {
  const e = setup(), api = await e.start(); await api.prepareQuote(input()); await e.click("connect"); await e.click("pay"); proved(e.server); e.server.badRelay = true;
  await e.click("relay"); assert.equal(sigs(e).length, 1);
  e.server.badRelay = false; e.server.job.public_values = e.server.job.public_values.replace(e.server.projection.source_hash.slice(2), hash("f").slice(2));
  await e.click("relay"); assert.equal(sigs(e).length, 1);
});
test("qualified entries, another wallet and another entry never become this entrant's prize payment", async () => {
  const e = setup(), api = await e.start(); await api.prepareQuote(input());
  const entry = { id: "entry", kind: "entry_qualified", tx_hash: hash("1"), block_number: 12, contract_address: contract, bounty_id: e.server.projection.bounty_id,
    data: { solver: wallet, solver_nonce: e.server.job.solver_nonce, submission_hash: e.server.job.artifact_hash, evidence_hash: hash("d"), sequence: 1 } };
  const settlement = { ...entry, id: "settle", kind: "competition_settled", data: { ...entry.data, winning_sequence: 1 } };
  e.server.events = [entry]; assert.equal((await api.refresh()).paid, false);
  e.server.events = [entry, { ...settlement, data: { ...settlement.data, solver: broker } }]; assert.equal((await api.refresh()).paid, false);
  e.server.events = [entry, { ...settlement, data: { ...settlement.data, winning_sequence: 2 } }]; assert.equal((await api.refresh()).paid, false);
  e.server.events = [entry, settlement]; assert.equal((await api.refresh()).paid, true);
});
test("an expired or altered charge, recipient resource, chain or proof binding is rejected", () => {
  const request = { competition_contract: contract, solver: wallet, solver_nonce: "7", artifact_hash: hash("a") };
  const good = responseQuote(request), now = Math.floor(Date.now() / 1000);
  proof.validateQuote(good, request, now);
  for (const alter of [(x) => x.quote.maximum_charge = "200000", (x) => x.quote.solver = broker,
    (x) => x.payment_required.accepts[0].network = "eip155:1", (x) => x.payment_required.resource.url = "https://evil.example/payment",
    (x) => x.quote.quote_expiration = now - 1, (x) => x.payment_required.accepts[0].extra.artifactHash = hash("f")]) {
    const bad = clone(good); alter(bad); assert.throws(() => proof.validateQuote(bad, request, now));
  }
});
test("forward GMV fetches the exact published snapshot and never quotes before the scoring window closes", async () => {
  const e = setup(); e.server.item.evidence_requirements = { program_profile: "forward-canonical-gmv-attribution-metric-v2", snapshot_url: "https://agentbounties.app/generated/gmv-snapshots/test-seed.json", scoring_window: { starts_at: new Date(Date.now() - 3600000).toISOString(), ends_at: new Date(Date.now() + 3600000).toISOString() } };
  const api = await e.start(); await assert.rejects(api.prepareQuote({ solver: wallet }), /still open/); assert.equal(e.server.quoteCount, undefined);
  e.server.item.evidence_requirements.scoring_window.ends_at = new Date(Date.now() - 1000).toISOString(); e.server.noSnapshot = true;
  await assert.rejects(api.prepareQuote({ solver: wallet }), /not published/); assert.equal(e.server.quoteCount, undefined);
  e.server.noSnapshot = false; e.server.snapshot = { campaign: { policy: "exact" }, snapshot: { attestations: [{ signature: "public-snapshot-one" }, { signature: "public-snapshot-two" }] } };
  await api.prepareQuote({ solver: wallet });
  const sent = JSON.parse(e.requests.find((r) => r.url.endsWith("/proof-quotes")).body);
  assert.deepEqual(sent.metric.campaign, e.server.snapshot.campaign); assert.deepEqual(sent.metric.snapshot, e.server.snapshot.snapshot); assert.equal(sent.artifact_hash, undefined);
});
test("service payment and refund labels require matching canonical safe-block evidence", () => {
  const job = { payer: wallet, maximum_charge: "110000", refund_tx_hash: hash("1"), refund_block_number: 12 };
  job.refund_evidence = { schema_version: "agent-bounties/open-competition-v2-proof-refund-evidence-v1", asset: "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913", recipient: wallet, amount: "110000", transaction_hash: hash("1"), block_number: 12, block_hash: hash("2"), safe_block_number: 15, safe_block_hash: hash("3") };
  assert.equal(proof.serviceEvidence(job, true), true); job.refund_evidence.recipient = broker; assert.equal(proof.serviceEvidence(job, true), false);
});
test("a public proof-job link restores its unpaid review without creating or paying another quote", async () => {
  const e = setup(), api = await e.start(); await api.prepareQuote(input());
  const another = setup(new Map(), e.server); another.win.location.searchParams.set("proofJob", id);
  const restored = await another.start(); assert.equal(restored.status().state, "quoted");
  assert.equal(e.server.quoteCount, 1); assert.equal(e.server.paymentCount, undefined); assert.equal(sigs(another).length, 0);
  await another.click("connect"); await another.click("pay"); assert.equal(e.server.paymentCount, 1); assert.equal(sigs(another).length, 1);
});
test("an expired unsigned quote renews from saved input while an issued authorization blocks replacement", async () => {
  const e = setup(), api = await e.start(); await api.prepareQuote(input()); const nonce = e.server.job.solver_nonce;
  e.server.job.quote_expires_at = new Date(Date.now() - 1000).toISOString(); await api.prepareQuote({});
  assert.equal(e.server.quoteCount, 2); assert.equal(e.server.job.solver_nonce, nonce); assert.equal(sigs(e).length, 0);
});
test("restoring a job from another competition fails before a payment or relay request", async () => {
  const e = setup(), api = await e.start(); await api.prepareQuote(input()); e.server.job.competition_contract = broker;
  const another = setup(new Map(), e.server); another.win.location.searchParams.set("proofJob", id);
  await assert.rejects(another.start(), /does not match/); assert.equal(another.requests.some((r) => r.url.endsWith("/payment")), false);
});
test("wallet funding returns only to allowed same-origin competition and participation workspaces", () => {
  const source = fs.readFileSync(require.resolve("../site/moonpay-onramp.js"), "utf8");
  const helper = source.slice(source.indexOf("  function safeReturnUrl()"), source.indexOf("  function ", source.indexOf("  function safeReturnUrl()") + 12));
  for (const target of ["https://agentbounties.app/competition.html?proofJob=" + id, "https://agentbounties.app/participate.html?intent=" + id, "https://evil.example/competition.html"]) {
    const location = new URL("https://agentbounties.app/onramp.html?return=" + encodeURIComponent(target));
    const restored = vm.runInNewContext(helper + "\nsafeReturnUrl().href", { location, URL, URLSearchParams });
    assert.equal(restored, target.includes("evil.example") ? "https://agentbounties.app/post.html" : target);
  }
});
