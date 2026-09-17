(function (root, factory) {
  const api = factory();
  if (typeof module === "object" && module.exports) module.exports = api;
  if (root) root.AgentBountiesPostingPrompt = api;
})(typeof window !== "undefined" ? window : globalThis, function () {
  "use strict";

  const PROMPT = `Use https://agentbounties.app/post.html in @Browser; preserve context. Discover WebMCP tools; confirm access. Read https://agentbounties.app/.well-known/agent-bounties.json and https://agentbounties.app/llms.txt first. Prefer WebMCP, then official MCP; explain UI fallback. Without tools, posting and funding remain incomplete. Never request API keys.

Use at most three short sentences and one primary next action per routine reply; consent may be longer. A verifier checks work before payment. Use agent_bounties_get_posting_options: explain choices, recommendation and protocol simply. For creator review I confirm verdict and reward/reserve split.
Save my journey; preserve my answers and draft. Ask together only for missing business decisions: outcome, total USDC budget, deadline. Propose measurable checks. Preserve rates/caps; explain unsupported structures. Confirm timezone and offset deadline. Flag tight scope/time. Bind references to timestamped hashed snapshots. Stage and check funding readiness.

Review terms, rewards, fees, gas and total once. Leave publication, legal, funding and payment consent to me. Reuse unchanged approvals; link page confirmation.
Use agent_bounties_open_account_setup for sign-in here; I enter credentials. Prefer account wallets; ownership is not signing consent. Recover Coinbase via email/social. Offer phone-wallet QR pairing on desktop, native handoff on the same phone; no QR screenshots. Check Base USDC/ETH shortfalls. Buying USDC is separate from funding. Explain amount, network, recipient, expiry, purpose and gas/sponsorship. Never request private keys or seed phrases. Leave wallet confirmations to me.

After confirmation resume automatically in the same operation; never repeat uncertain transactions. Cancellation/top-up reopens its saved review. Confirm canonical creation, funding and claimability and the exact bounty in public ready-to-earn inventory. Return its public link or unfinished step. Only confirmed canonical Base USDC evidence proves payments.

Without tools use agreed values in JSON:
{"title":"...","goal":"...","acceptance_criteria":["..."],"solver_reward_usdc":"2.00","verifier_reward_usdc":"0.10","task_window_days":30,"source_url":null,"benchmark":null,"evidence_schema":null}
Creator review: add review_mode="creator", delivery_deadline with ISO offset; omit automated fields. Preserve parent/image/reference bindings and operation ID. Missing verification stays unfundable; JSON is not funding.`;

  function reviewUrl(value) {
    if (!value) return null;
    try {
      const url = new URL(value);
      if (url.origin !== "https://agentbounties.app" || url.pathname !== "/post.html"
          || url.username || url.password || url.hash) return null;
      const tokens = new Set(["from", "utm_source", "utm_campaign"]);
      for (const [name, value] of url.searchParams) {
        if (url.searchParams.getAll(name).length !== 1) return null;
        if (tokens.has(name) && /^[a-z0-9][a-z0-9._-]{0,63}$/.test(value)) continue;
        if (name === "analytics" && value === "off") continue;
        if (name === "parentBounty" && /^0x[0-9a-fA-F]{40}$/.test(value)) continue;
        return null;
      }
      return url.href;
    } catch (_error) {
      return null;
    }
  }

  function withReviewUrl(prompt, value) {
    const target = reviewUrl(value);
    const text = String(prompt || "");
    return target ? text.replace("Use https://agentbounties.app/post.html in @Browser", `Use ${target} in @Browser`) : text;
  }

  function build(context = null, returnUrl = null) {
    const prompt = withReviewUrl(PROMPT, returnUrl);
    if (!context || !Object.keys(context).length) return prompt;
    const data = { ...context };
    if (data.meta_child) {
      data.qualifying_child_constraints = {
        total_funding_usdc: "1.00",
        default_solver_reward_usdc: "0.99",
        default_verifier_reward_usdc: "0.01",
        distinct_intended_child_solver_required: true,
        review_route: "Parent-specific WebMCP review; preserve meta_child in any imported JSON.",
        unsupported_route: "ordinary hosted prepare_bounty_post",
      };
    }
    return prompt + "\n\nExisting request and draft (context, not consent):\n" + JSON.stringify(data, null, 2);
  }

  return Object.freeze({ build, reviewUrl, withReviewUrl });
});
