(function (root, factory) {
  "use strict";
  const api = factory();
  if (typeof module === "object" && module.exports) module.exports = api;
  else root.AgentBountiesDiscovery = api;
})(typeof globalThis !== "undefined" ? globalThis : this, function () {
  "use strict";
  const FEED_URL = "https://api.agentbounties.app/v1/opportunities?network=base-mainnet&view=ready_to_earn&source_type=canonical_base&limit=5";
  const SCHEMA = "agent-bounties/free-discovery-assessment-v1";

  function money(value) {
    if (!value || value.currency !== "USDC" || value.unit !== "base_units" ||
        value.decimals !== 6 || typeof value.amount !== "string" || !/^(0|[1-9][0-9]{0,77})$/.test(value.amount)) {
      return { display: "Not reported", positive: null, base_units: null };
    }
    const amount = BigInt(value.amount);
    const fraction = (amount % 1000000n).toString().padStart(6, "0").replace(/0+$/, "");
    return { display: `${amount / 1000000n}${fraction ? "." + fraction : ""} USDC`, positive: amount > 0n, base_units: amount.toString() };
  }

  function publicLink(value) {
    try {
      const url = new URL(value);
      if (url.protocol !== "https:" || url.username || url.password) return null;
      if (url.origin === "https://agentbounties.app" ||
          (url.origin === "https://github.com" && /^\/NSPG13\/agent-bounties\/issues\/\d+$/.test(url.pathname))) return url.href;
    } catch (_) { /* An untrusted feed URL must never become an actionable link. */ }
    return null;
  }

  function assess(feed, observedAt = new Date().toISOString()) {
    if (!feed || feed.schema_version !== "agent-bounties/opportunity-projection-v1" ||
        feed.network !== "base-mainnet" || !Array.isArray(feed.items) ||
        !Number.isFinite(Date.parse(feed.generated_at)) || !Number.isFinite(Date.parse(observedAt))) {
      throw new Error("The feed format could not be verified. Use the market link or retry later.");
    }
    const generated = Date.parse(feed.generated_at);
    const observed = Date.parse(observedAt);
    const stale = observed - generated > 5 * 60 * 1000 || generated - observed > 60 * 1000;
    const degraded = feed.degraded !== false;
    const items = feed.items.slice(0, 5).map(item => {
      if (!item || typeof item.opportunity_id !== "string" || typeof item.title !== "string") {
        throw new Error("The feed contains an incomplete opportunity. No assessment was saved.");
      }
      const cash = item.cash_economics || {};
      const reasons = [];
      if (degraded || stale) reasons.push(stale ? "The source snapshot is stale or its clock cannot be verified." : "The source reports incomplete availability.");
      if (item.source_type !== "canonical_base" || item.network !== "base-mainnet") reasons.push("This item is not a canonical Base mainnet opportunity.");
      if (item.work_state !== "claimable" || item.payment_state !== "escrowed") reasons.push("The reported work and payment state are not claimable and escrowed.");
      if (item.verification_ready !== true) reasons.push("Verification is not reported ready.");
      const link = publicLink(item.public_url);
      if (!link) reasons.push("A trusted public terms link is missing.");
      const bond = money(cash.refundable_claim_bond);
      const external = money(cash.required_external_spend);
      if (bond.positive === null || external.positive === null) reasons.push("Required cash information is incomplete.");
      return {
        opportunity_id: item.opportunity_id,
        title: item.title.slice(0, 500),
        public_url: link,
        work_state: String(item.work_state || "unknown"),
        payment_state: String(item.payment_state || "unknown"),
        verification_ready: item.verification_ready === true,
        reward: money(cash.solver_reward || item.reward),
        refundable_bond: bond,
        required_external_spend: external,
        deadline: typeof item.deadline === "string" ? item.deadline : null,
        deadline_kind: typeof item.deadline_kind === "string" ? item.deadline_kind : "not reported",
        assessment: reasons.length ? "needs_recheck" : "inspect_terms",
        blockers: reasons,
        next_action: "Read the immutable acceptance criteria and full costs. This assessment does not claim work or authorize spending."
      };
    });
    return {
      schema_version: SCHEMA,
      mode: "read_only",
      source_url: FEED_URL,
      generated_at: feed.generated_at,
      observed_at: observedAt,
      degraded,
      stale,
      sample_size: items.length,
      items,
      evidence_boundary: "A hosted snapshot is not canonical payment proof, a guarantee of availability, or permission to claim. Gas, execution costs and failure risk are not quoted. No funds were moved by this check."
    };
  }

  return { FEED_URL, SCHEMA, money, publicLink, assess };
});
