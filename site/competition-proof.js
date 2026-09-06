(function (root, factory) {
  const api = factory();
  if (typeof module === "object" && module.exports) module.exports = api;
  if (root?.document) api.start(root, root.document).catch((error) => {
    const status = root.document.querySelector("[data-proof-status]");
    if (status) status.textContent = error.message;
  });
})(typeof window !== "undefined" ? window : globalThis, function () {
  "use strict";
  const ROOT = "/v1/base/open-competition-v2-beta3", TOKEN = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913";
  const ADDRESS = /^0x[0-9a-f]{40}$/i, HASH = /^0x[0-9a-f]{64}$/i, UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
  const lower = (value) => String(value || "").toLowerCase();
  const stable = (value) => value && typeof value === "object" ? Array.isArray(value) ? `[${value.map(stable).join(",")}]` : `{${Object.keys(value).sort().map((key) => `${JSON.stringify(key)}:${stable(value[key])}`).join(",")}}` : JSON.stringify(value);
  const units = (value) => { if (!/^\d+$/.test(String(value))) throw new Error("The quote contains an invalid amount."); return BigInt(value); };
  const money = (value) => { const signed = BigInt(value), n = signed < 0n ? -signed : signed; return `${signed < 0n ? "−" : ""}${n / 1000000n}.${String(n % 1000000n).padStart(6, "0").replace(/0{1,4}$/, "")} USDC`; };
  function validateQuote(result, request, now) {
    const q = result?.quote, c = result?.payment_required, a = c?.accepts?.[0], e = a?.extra;
    if (!UUID.test(result?.proof_job_id) || q?.network !== "eip155:8453" || lower(q.competition_contract) !== request.competition_contract
      || lower(q.solver) !== request.solver || q.solver_nonce !== request.solver_nonce || !HASH.test(q.artifact_hash)
      || (request.artifact_hash && lower(q.artifact_hash) !== lower(request.artifact_hash)) || !HASH.test(q.quote_id)
      || !["groth16", "plonk"].includes(q.proof_system) || !["first_proven", "best_score"].includes(q.winner_mode)
      || !Number.isSafeInteger(q.quote_expiration) || q.quote_expiration <= now + 30 || q.quote_expiration > now + 300
      || c?.x402Version !== 2 || c.accepts?.length !== 1 || a.scheme !== "exact" || a.network !== "eip155:8453"
      || lower(a.asset) !== TOKEN || !ADDRESS.test(a.payTo) || lower(a.payTo) === "0x" + "00".repeat(20)
      || a.amount !== q.maximum_charge || e?.maximumCharge !== q.maximum_charge || e?.quoteId !== q.quote_id
      || lower(e?.competition) !== request.competition_contract || lower(e?.solver) !== request.solver
      || lower(e?.artifactHash) !== lower(q.artifact_hash) || e?.proofSystem !== q.proof_system
      || e?.assetTransferMethod !== "eip3009" || e?.name !== "USD Coin" || e?.version !== "2"
      || a.maxTimeoutSeconds !== 300) throw new Error("The service quote does not match this exact entry and supported payment rail.");
    if (units(q.maximum_charge) <= 0n || units(q.maximum_charge) !== units(q.proof_fee_quote) + units(q.relay_fee_quote)
      || BigInt(q.net_prize_if_win) !== units(q.gross_prize) - units(q.maximum_charge)) throw new Error("The quote totals do not reconcile.");
    const resource = new URL(c.resource?.url);
    if (resource.pathname !== `${ROOT}/proof-jobs/${result.proof_job_id}/payment` || resource.search || resource.hash
      || resource.origin !== "https://api.agentbounties.app") throw new Error("The payment challenge points to an unexpected service.");
    return q;
  }
  function paymentData(result, wallet, nonce, evm) {
    return { types: evm.transferWithAuthorizationTypes(), primaryType: "TransferWithAuthorization",
      domain: { name: "USD Coin", version: "2", chainId: 8453, verifyingContract: TOKEN },
      message: { from: wallet, to: lower(result.payment_required.accepts[0].payTo), value: result.quote.maximum_charge,
        validAfter: "0", validBefore: String(result.quote.quote_expiration), nonce } };
  }
  function artifactHash(text, evm) {
    if (typeof text !== "string") throw new Error("Prepare the exact artifact text before quoting.");
    const bytes = evm.textHex(text).slice(2);
    // competition-metric-core::structured_artifact_submission_hash: domain, u32 BE byte length, UTF-8 bytes.
    return evm.keccak256Hex(`0x6c3e2c182e83869d996ddb7c5a78d3d43a611c656ef04d03c053e39fd2315659${(bytes.length / 2).toString(16).padStart(8, "0")}${bytes}`);
  }
  function serviceEvidence(job, refund = false) {
    const e = refund ? job?.refund_evidence : job?.payment_evidence;
    return Boolean(e && e.schema_version === `agent-bounties/open-competition-v2-proof-${refund ? "refund" : "payment"}-evidence-v1`
      && lower(e.asset) === TOKEN && e.amount === job.maximum_charge && HASH.test(e.transaction_hash)
      && e.transaction_hash === (refund ? job.refund_tx_hash : job.payment_tx_hash)
      && Number.isSafeInteger(e.block_number) && e.block_number > 0 && e.block_number === (refund ? job.refund_block_number : job.payment_block_number)
      && Number.isSafeInteger(e.safe_block_number) && e.safe_block_number >= e.block_number && HASH.test(e.block_hash) && HASH.test(e.safe_block_hash)
      && lower(refund ? e.recipient : e.payer) === lower(job.payer));
  }
  function relayData(plan, job, projection, deadline, evm) {
    const values = job.public_values, proof = job.proof;
    if (!/^0x[0-9a-f]{1280}$/i.test(values) || !/^0x(?:[0-9a-f]{2})+$/i.test(proof) || proof.length > 8388610
      || lower(values) !== lower(job.expected_public_values)) throw new Error("The finished proof differs from the quoted entry.");
    const words = values.slice(2).match(/.{64}/g);
    const bindings = { 0: evm.keccak256Hex(evm.textHex("agent-bounties/open-competition-v2-beta3/journal")),
      1: `0x${evm.uint256Word(8453)}`, 2: `0x${evm.addressWord(job.competition_contract)}`, 3: projection.bounty_id,
      4: `0x${evm.addressWord(job.solver)}`, 5: `0x${evm.uint256Word(job.solver_nonce)}`, 6: job.artifact_hash,
      9: projection.program_vkey, 10: projection.source_hash, 11: projection.elf_hash, 12: projection.journal_schema_hash,
      13: projection.metric_program_hash, 14: projection.execution_policy_hash, 15: projection.verification_policy_hash,
      16: projection.settlement_policy_hash, 17: projection.beta_risk_hash, 18: `0x${evm.uint256Word(1)}` };
    for (const [i, expected] of Object.entries(bindings)) if (!HASH.test(expected) || `0x${lower(words[i])}` !== lower(expected)) throw new Error("The proof is bound to different competition terms or a different entry.");
    const fields = (pairs) => pairs.map(([name, type]) => ({ name, type }));
    const expected = { types: {
      EIP712Domain: fields([["name", "string"], ["version", "string"], ["chainId", "uint256"], ["verifyingContract", "address"]]),
      SubmitProof: fields([["solver", "address"], ["solverNonce", "uint256"], ["publicValuesHash", "bytes32"], ["proofHash", "bytes32"], ["authorizationDeadline", "uint256"]]) },
      primaryType: "SubmitProof", domain: { name: "Agent Bounties Open Competition V2 Beta3", version: "1", chainId: 8453, verifyingContract: lower(job.competition_contract) },
      message: { solver: lower(job.solver), solverNonce: job.solver_nonce, publicValuesHash: evm.keccak256Hex(values), proofHash: evm.keccak256Hex(proof), authorizationDeadline: String(deadline) } };
    if (stable(plan?.relay_authorization) !== stable(expected) || lower(job.proof_hash) !== lower(expected.message.proofHash)
      || lower(job.public_values_hash) !== lower(expected.message.publicValuesHash)) throw new Error("The relay request differs from the exact finished proof.");
    return expected;
  }
  function evidence(job, projection, events) {
    const same = (e) => e.id && HASH.test(e.tx_hash) && Number.isSafeInteger(e.block_number) && e.block_number > 0
      && lower(e.contract_address) === lower(job.competition_contract) && lower(e.bounty_id) === lower(projection.bounty_id)
      && lower(e.data?.solver) === lower(job.solver) && lower(e.data?.submission_hash) === lower(job.artifact_hash);
    const entry = events.find((e) => e.kind === "entry_qualified" && same(e) && String(e.data.solver_nonce) === job.solver_nonce);
    const settlement = entry && events.find((e) => e.kind === "competition_settled" && same(e)
      && String(e.data.winning_sequence) === String(entry.data.sequence) && lower(e.data.evidence_hash) === lower(entry.data.evidence_hash));
    return { entry: entry || null, settlement: settlement || null, paid: Boolean(settlement) };
  }
  async function start(win, doc) {
    const section = doc.querySelector("[data-proof-workspace]");
    if (!section) return;
    const flow = win.AgentBountiesWorkflow, client = flow.createClient(win), evm = win.AgentBountiesEvm;
    const params = new URLSearchParams(win.location.search), contract = lower(params.get("bountyContract"));
    if (!ADDRESS.test(contract) || (params.get("network") || "base-mainnet") !== "base-mainnet") return;
    const key = `agent-bounties.competition-proof.v1:${contract}`;
    let record = JSON.parse(win.sessionStorage.getItem(key) || "{}"), provider = null, wallet = null, job = null, projection = null, opportunity = null, currentEvidence = null, busy = false;
    const find = (s) => doc.querySelector(s), put = (s, value) => { const node = find(s); if (node) node.textContent = value; };
    const now = () => Math.floor(Date.now() / 1000);
    const save = () => win.sessionStorage.setItem(key, JSON.stringify(record));
    const paymentPath = () => `${ROOT}/proof-jobs/${record.id}/payment`;
    const relayPath = () => `${ROOT}/proof-jobs/${record.id}/relay-authorization`;
    function boundJob(value) {
      if (value?.id !== record.id || value.network !== "base-mainnet" || lower(value.competition_contract) !== contract
        || !ADDRESS.test(value.solver) || !/^\d+$/.test(value.solver_nonce) || !HASH.test(value.artifact_hash)
        || (record.request && (lower(value.solver) !== record.request.solver || value.solver_nonce !== record.request.solver_nonce))
        || (record.quote && (value.maximum_charge !== record.quote.quote.maximum_charge || lower(value.artifact_hash) !== lower(record.quote.quote.artifact_hash)))) throw new Error("The proof job does not match this review.");
      return value;
    }
    async function canonical() {
      const inventory = await client.request(`${ROOT}/inventory?network=base-mainnet`);
      const found = inventory.competitions?.find((c) => lower(c.record?.projection?.competition) === contract)?.record?.projection;
      if (inventory.network !== "base-mainnet" || !found) throw new Error("Current competition state is unavailable.");
      projection = found; return found;
    }
    function requireOpen() {
      if (projection?.state !== "active" || !Number.isSafeInteger(projection.proof_deadline) || projection.proof_deadline <= now() + 30) throw new Error("This competition no longer has enough time to accept a proof.");
    }
    function next() {
      if (!record.id && opportunity?.evidence_requirements?.program_profile === "forward-canonical-gmv-attribution-metric-v2" && flow.phase(opportunity) !== "ended") return {
        action: "generate_score", tool: "agent_bounties_get_competition_manifest", message: "Prepare qualifying child work for the displayed scoring window. A proof quote comes after the window closes and the scoring snapshot is published." };
      if (!record.id) return { action: "prepare_quote", tool: "agent_bounties_prepare_proof_quote", message: "Your AI prepares the entry and exact service quote. Connect your wallet once if its address is not already known." };
      if (currentEvidence?.paid) return { action: "complete", message: "Your prize payment is confirmed on Base." };
      if (record.paymentEnvelope || record.relayEnvelope || job?.state === "payment_pending") return { action: "resume", message: "Continue the same authorized request. Do not create or pay another quote." };
      if (job?.state === "quoted") return { action: "pay", message: "Review the exact service charge, then approve it in your wallet." };
      if (job?.state === "proved") return { action: "relay", message: "Your proof is ready. Authorize submission of this exact entry; its relay fee was included in your service quote." };
      if (job?.state === "refund_due") return { action: "wait", message: "The proof service reported a failure. Track the service refund; do not pay again." };
      if (job?.state === "refunded") return { action: "complete", message: "Check the attached canonical service-refund evidence. A refund is not a competition prize." };
      if (job?.state === "lost_competition") return { action: "complete", message: "Another entry won. A valid completed proof service is not refundable merely because an entry loses." };
      return { action: "wait", message: currentEvidence?.entry ? "Your entry qualified. Waiting for the competition result and canonical prize settlement." : "Waiting for payment confirmation, proving, or submission confirmation. Your AI can check progress without another permission request." };
    }
    function status() {
      const q = record.quote?.quote || job;
      return { competition_contract: contract, wallet_connected: Boolean(wallet), wallet, proof_job_id: record.id || null,
        state: job?.state || "not_quoted", quote: q ? Object.fromEntries(["gross_prize", "proof_fee_quote", "relay_fee_quote", "net_prize_if_win", "maximum_charge", "winner_mode", "competition_risk"].map((k) => [k, q[k]])) : null,
        solver: job?.solver || record.request?.solver || null, artifact_hash: job?.artifact_hash || q?.artifact_hash || null,
        quote_expires_at: job?.quote_expires_at || null, proof_deadline: projection?.proof_deadline || null,
        service_payment_confirmed: serviceEvidence(job),
        service_refund_confirmed: serviceEvidence(job, true),
        payment_transaction: job?.payment_tx_hash || null, refund_transaction: job?.refund_tx_hash || null,
        relay_transaction: job?.relay_tx_hash || null, qualified: Boolean(currentEvidence?.entry), paid: Boolean(currentEvidence?.paid),
        canonical_entry: currentEvidence?.entry || null, canonical_settlement: currentEvidence?.settlement || null,
        failure: job?.failure_message || null, next_action: next(), poll_after_seconds: 15,
        evidence_boundary: "Service payment buys computation and relay. It is not a prize. Only a matching canonical CompetitionSettledV2 proves your prize payment." };
    }
    function render() {
      const s = status(), q = record.quote?.quote || job;
      put("[data-proof-status]", s.next_action.message);
      put("[data-proof-wallet-status]", wallet ? `Connected: ${wallet}` : "Connect your Base wallet when you are ready to enter.");
      put("[data-proof-cost]", q ? `Proof ${money(q.proof_fee_quote)} + submission ${money(q.relay_fee_quote)} = ${money(q.maximum_charge)} maximum service charge. Prize ${money(q.gross_prize)}; after service fees if you win ${money(q.net_prize_if_win)}. If you lose, the service charge is spent. Other work and funding costs are additional.` : "No quote or charge yet.");
      put("[data-proof-entry]", q ? `Entrant ${job?.solver || record.request?.solver}. Mode: ${q.winner_mode}. Quote expires ${job?.quote_expires_at || new Date(q.quote_expiration * 1000).toISOString()}. Service recipient ${record.quote?.payment_required?.accepts?.[0]?.payTo || "shown in the saved payment receipt"}.` : "");
      const setup = find("[data-proof-wallet-setup]");
      if (setup) { const url = new URL("onramp.html", win.location.href); url.searchParams.set("return", win.location.href); url.searchParams.set("purpose", "earn"); if (q) url.searchParams.set("amount", String(Number(q.maximum_charge) / 1e6)); setup.href = url.href; }
      put("[data-proof-evidence]", JSON.stringify(s, null, 2));
      put("[data-proof-artifact]", record.request?.metric?.artifact_utf8 || (record.request?.metric ? JSON.stringify(record.request.metric, null, 2) : "Your exact entry appears here when your AI prepares it."));
      const consent = find("[data-proof-workspace] [data-legal-consent]"); if (consent) consent.hidden = !["pay", "relay"].includes(s.next_action.action);
      for (const action of ["pay", "relay", "resume"]) { const b = find(`[data-proof-${action}]`); if (b) { b.hidden = s.next_action.action !== action; b.disabled = busy; } }
      return s;
    }
    async function refresh() {
      await canonical();
      if (!record.id) opportunity = await client.opportunity(`open-competition-v2:base-mainnet:${contract}`);
      if (record.id) {
        job = boundJob((await client.request(`${ROOT}/proof-jobs/${record.id}`)).job);
        if (job.state === "quoted" && !record.quote && new Date(job.quote_expires_at).getTime() > Date.now() + 30000) {
          // A public job link can restore its exact unsigned review without creating another quote.
          const response = await win.fetch(`${flow.apiBase(win.location)}${paymentPath()}`, { method: "POST", cache: "no-store", credentials: "omit", headers: { Accept: "application/json" } });
          if (response.status === 402) {
            const payload = await response.json();
            const q = Object.fromEntries(["competition_contract", "solver", "solver_nonce", "artifact_hash", "proof_system", "gross_prize", "proof_fee_quote", "relay_fee_quote", "net_prize_if_win", "maximum_charge", "winner_mode", "competition_risk"].map((k) => [k, job[k]]));
            Object.assign(q, { network: "eip155:8453", quote_id: job.idempotency_key, quote_expiration: Math.floor(new Date(job.quote_expires_at).getTime() / 1000) });
            const restored = { quote: q, proof_job_id: job.id, payment_required: payload.paymentRequired };
            record.request = { competition_contract: contract, solver: lower(job.solver), solver_nonce: job.solver_nonce, artifact_hash: job.artifact_hash };
            validateQuote(restored, record.request, now()); record.quote = restored;
          } else if (!response.ok) throw new Error("The saved quote cannot be restored yet. Check this same proof job again.");
          else job = boundJob((await client.request(`${ROOT}/proof-jobs/${record.id}`)).job);
        }
        // Discard bearer capabilities once the server has accepted them. They never enter tool results.
        if (job.state !== "quoted") delete record.paymentEnvelope;
        if (["relaying", "confirmed", "lost_competition", "refund_due", "refunded"].includes(job.state)) delete record.relayEnvelope;
        const result = await client.request(`${ROOT}/events?network=base-mainnet&bounty_id=${encodeURIComponent(projection.bounty_id)}`);
        currentEvidence = evidence(job, projection, result.events || []); save();
      }
      return render();
    }
    async function quote(input = {}) {
      if (busy) throw new Error("A wallet decision is in progress. Keep the current review unchanged.");
      busy = true;
      try {
        await refresh(); requireOpen();
        const solver = lower(input.solver || wallet || record.request?.solver);
        if (!ADDRESS.test(solver)) return { ...render(), next_action: { action: "connect_wallet", message: "Connect your wallet on this page so the quote belongs to you." } };
        const item = opportunity || await client.opportunity(`open-competition-v2:base-mainnet:${contract}`);
        let metric;
        if (item.evidence_requirements?.program_profile === "forward-canonical-gmv-attribution-metric-v2") {
          if (flow.phase(item) !== "ended") throw new Error("The scoring window is still open. Finish qualifying work first; no proof charge is needed yet.");
          const url = new URL(item.evidence_requirements.snapshot_url, win.location.href);
          if (!([win.location.origin, "https://agentbounties.app"].includes(url.origin)) || !/^\/generated\/gmv-snapshots\/[a-z0-9-]+\.json$/.test(url.pathname)) throw new Error("The published scoring snapshot is unavailable.");
          const response = await win.fetch(new URL(url.pathname, win.location.origin).href, { cache: "no-store", credentials: "omit" });
          if (!response.ok) throw new Error("The scoring snapshot is not published yet. Wait for its two committed attestations; do not pay for a different metric.");
          const snapshot = await response.json();
          if (!snapshot.campaign || !snapshot.snapshot) throw new Error("The published snapshot is incomplete.");
          metric = { profile_id: "forward-canonical-gmv-attribution-metric-v2", campaign: snapshot.campaign, snapshot: snapshot.snapshot };
        } else metric = flow.publicJson(input.metric || record.request?.metric);
        const artifact = metric.profile_id === "structured-artifact-metric-v1" ? artifactHash(metric.artifact_utf8, evm) : input.artifact_hash;
        if (metric.profile_id !== "forward-canonical-gmv-attribution-metric-v2" && !HASH.test(artifact)) throw new Error("The AI must prepare the exact artifact hash and committed metric input.");
        const fingerprint = stable({ solver, metric, artifact: artifact || null });
        if (record.id) {
          if (record.fingerprint === fingerprint && (job.state !== "quoted" || new Date(job.quote_expires_at).getTime() > Date.now() + 30000)) return render();
          if (job.state !== "quoted" || record.paymentData || record.paymentEnvelope || record.relayEnvelope) throw new Error("Continue the existing proof job before changing this entry. Its payment authorization must be reconciled first.");
        }
        const request = { network: "base-mainnet", competition_contract: contract, solver,
          solver_nonce: record.fingerprint === fingerprint ? record.request.solver_nonce : BigInt(`0x${evm.randomBytes32().slice(2, 34)}`).toString(), relay: true, metric,
          ...(metric.profile_id === "forward-canonical-gmv-attribution-metric-v2" ? {} : { artifact_hash: artifact }) };
        record = { request, fingerprint }; save(); // Only a free quote can be retried before a job ID exists.
        const result = await client.request(`${ROOT}/proof-quotes`, request);
        validateQuote(result, request, now());
        record.id = result.proof_job_id; record.quote = result; save();
        const url = new URL(win.location.href); url.searchParams.set("proofJob", record.id); win.history.replaceState(null, "", url.href);
        return await refresh();
      } finally { busy = false; render(); }
    }
    const providers = [];
    win.addEventListener("eip6963:announceProvider", (event) => { if (event.detail?.provider && !providers.some((p) => p.provider === event.detail.provider)) providers.push(event.detail); });
    win.dispatchEvent(new win.Event("eip6963:requestProvider"));
    async function connect(selected) {
      provider = selected;
      let accounts = await provider.request({ method: "eth_accounts" });
      if (!accounts?.length) accounts = await provider.request({ method: "eth_requestAccounts" });
      wallet = lower(accounts?.[0]);
      if (!ADDRESS.test(wallet)) throw new Error("Choose a wallet account.");
      render();
    }
    async function signer() {
      if (!provider) throw new Error("Connect the entrant wallet first.");
      const accounts = await provider.request({ method: "eth_accounts" });
      if (lower(accounts?.[0]) !== lower(job?.solver)) throw new Error("Reconnect the entrant wallet shown in this review.");
      wallet = lower(accounts[0]);
      if (BigInt(await provider.request({ method: "eth_chainId" })) !== 8453n) await provider.request({ method: "wallet_switchEthereumChain", params: [{ chainId: "0x2105" }] });
      const acceptance = await win.AgentBountiesLegal.requireAcceptance({ action: "submit_result", walletAddress: wallet });
      if (!acceptance?.durable) throw new Error("The agreement could not be recorded. Retry before signing.");
      return wallet;
    }
    async function postPayment(envelope) {
      const response = await win.fetch(`${flow.apiBase(win.location)}${paymentPath()}`, { method: "POST", cache: "no-store", credentials: "omit", referrerPolicy: "no-referrer",
        headers: { Accept: "application/json", ...(envelope ? { "PAYMENT-SIGNATURE": win.btoa(JSON.stringify(envelope)) } : {}) } });
      if (!response.ok) throw new Error(`The service has not acknowledged payment (${response.status}). Resume this same job; do not create a new authorization.`);
      return refresh();
    }
    async function resume() {
      if (busy) throw new Error("Wait for the current wallet step to finish.");
      busy = true;
      try {
        await refresh();
        if (record.paymentEnvelope && job.state === "quoted") await postPayment(record.paymentEnvelope);
        else if (job.state === "payment_pending") await postPayment();
        if (record.relayEnvelope && job.state === "proved") await client.request(relayPath(), record.relayEnvelope);
        return await refresh();
      } finally { busy = false; render(); }
    }
    async function pay() {
      await refresh(); requireOpen();
      if (job?.state !== "quoted" || record.paymentEnvelope) throw new Error("Resume the existing job; a new payment signature is unnecessary.");
      if (!record.quote) throw new Error("Open this quote in the browser where its exact payment review was prepared.");
      validateQuote(record.quote, record.request, now());
      await signer();
      const balance = await provider.request({ method: "eth_call", params: [{ to: TOKEN, data: `0x70a08231${evm.addressWord(wallet)}` }, "latest"] });
      if (BigInt(balance) < units(job.maximum_charge)) throw new Error("Your wallet needs the displayed service fee in USDC on Base. Use the wallet setup link, then return to this same review.");
      validateQuote(record.quote, record.request, now());
      if (!record.paymentData) { record.paymentData = paymentData(record.quote, wallet, evm.randomBytes32(), evm); save(); }
      if (stable(record.paymentData) !== stable(paymentData(record.quote, wallet, record.paymentData.message.nonce, evm))) throw new Error("The saved payment differs from this quote.");
      let signature;
      try { signature = await provider.request({ method: "eth_signTypedData_v4", params: [wallet, JSON.stringify(record.paymentData)] }); }
      catch (error) { if (error.code === 4001) { delete record.paymentData; save(); } throw error; }
      if (!/^0x[0-9a-f]{130}$/i.test(signature)) throw new Error("The wallet did not return a supported payment signature. Preserve this authorization nonce when retrying.");
      record.paymentEnvelope = { x402Version: 2, resource: record.quote.payment_required.resource, accepted: record.quote.payment_required.accepts[0],
        payload: { signature, authorization: record.paymentData.message }, ...(record.quote.payment_required.extensions ? { extensions: record.quote.payment_required.extensions } : {}) };
      save(); await postPayment(record.paymentEnvelope);
    }
    async function relay() {
      await refresh(); requireOpen();
      if (job?.state !== "proved" || record.relayEnvelope) throw new Error("The job is not awaiting a new relay authorization. Check or resume its current progress.");
      await signer();
      const deadline = Math.min(now() + 300, projection.proof_deadline);
      const result = await client.request(relayPath(), { authorization_deadline: deadline });
      const typed = relayData(result.plan, job, projection, deadline, evm);
      const signature = await provider.request({ method: "eth_signTypedData_v4", params: [wallet, JSON.stringify(typed)] });
      if (!/^0x(?:[0-9a-f]{2})+$/i.test(signature) || signature.length > 32770) throw new Error("The wallet returned an invalid relay signature.");
      record.relayEnvelope = { authorization_deadline: deadline, solver_signature: signature }; save();
      await client.request(relayPath(), record.relayEnvelope); await refresh();
    }
    const human = (fn) => async (event) => {
      if (!event.isTrusted || busy) return;
      busy = true; render();
      try { await fn(); } catch (error) { put("[data-proof-status]", error.message); }
      finally { busy = false; for (const name of ["pay", "relay", "resume"]) if (find(`[data-proof-${name}]`)) find(`[data-proof-${name}]`).disabled = false; }
    };
    find("[data-proof-connect]")?.addEventListener("click", human(async () => {
      if (providers.length <= 1) return connect(providers[0]?.provider || win.ethereum || (() => { throw new Error("Use the wallet setup link, then return to this review."); })());
      const choices = find("[data-proof-wallet-choices]"); choices.replaceChildren();
      for (const choice of providers) { const button = doc.createElement("button"); button.type = "button"; button.textContent = choice.info?.name || "Wallet"; button.addEventListener("click", human(() => connect(choice.provider))); choices.append(button); }
    }));
    find("[data-proof-pay]")?.addEventListener("click", human(pay));
    find("[data-proof-relay]")?.addEventListener("click", human(relay));
    find("[data-proof-resume]")?.addEventListener("click", async (event) => { if (event.isTrusted) try { await resume(); } catch (error) { put("[data-proof-status]", error.message); } });
    if (!record.id && UUID.test(params.get("proofJob"))) { record.id = params.get("proofJob"); save(); }
    win.AgentBountiesProofWorkspace = Object.freeze({ prepareQuote: quote, refresh: () => busy ? Promise.resolve(status()) : refresh(), resume, status,
      openReview: async () => { const result = await refresh(); section.scrollIntoView({ behavior: "smooth", block: "start" }); return result; } });
    render();
    await refresh();
    win.setInterval(() => { if (!doc.hidden && !busy && record.id) refresh().catch((error) => put("[data-proof-status]", error.message)); }, 15000);
    return win.AgentBountiesProofWorkspace;
  }
  return { validateQuote, paymentData, artifactHash, relayData, evidence, serviceEvidence, start };
});
