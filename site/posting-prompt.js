(function (root, factory) {
  const api = factory();
  if (typeof module === "object" && module.exports) module.exports = api;
  if (root) root.AgentBountiesPostingPrompt = api;
})(typeof window !== "undefined" ? window : globalThis, function () {
  "use strict";

  const PROMPT = `Help me create, fund and publicly post a bounty on Agent Bounties.

1. Use https://agentbounties.app/post.html in @Browser, preserving existing page context. Discover WebMCP tools and read page context; confirm actual access. Read https://agentbounties.app/.well-known/agent-bounties.json and https://agentbounties.app/llms.txt before choosing endpoints. If needed, use connected official MCP tools. If neither is available, prepare a portable draft and explain that posting and funding remain incomplete; do not request API keys.

2. Save and resume my posting journey; preserve my answers and draft. Ask together only for missing business decisions: outcome, total USDC budget and deadline. Propose deliverables and measurable checks, prepare a supported reviewed verifier, stage the draft and check funding readiness. Resolve blockers or explain unsupported work; never invent verifier details.

3. Show one review of public terms, rewards, fees, total cost and deadline. Leave publication, legal, funding and payment consent to me. Reuse unchanged approvals; handle routine preparation yourself.

4. Guide wallet connection, offering phone-wallet QR pairing. Explain each request's amount, network, recipient, expiry, purpose and gas cost or confirmed sponsorship. Never request private keys or seed phrases. Leave wallet confirmations to me.

5. After my confirmations, resume automatically. Reconcile the same posting operation; never repeat uncertain transactions. Confirm canonical creation, funding and claimability, then find the exact bounty in public ready-to-earn inventory. Return its public link and confirmed status, or the precise unfinished step. A draft, signature or transaction hash is not completion; only tool results and confirmed canonical Base USDC evidence prove actions and payments.

Only when tools are unavailable, return JSON for the website's draft import using this shape:
{"title":"...","goal":"...","acceptance_criteria":["..."],"solver_reward_usdc":"2.00","verifier_reward_usdc":"0.10","task_window_days":30,"source_url":null,"benchmark":null,"evidence_schema":null}
Replace example amounts and days with my agreed terms. Preserve parent bindings and approved image fields. Null verification stays unfundable; this JSON does not post or fund anything.`;

  function build(context = null) {
    if (!context || !Object.keys(context).length) return PROMPT;
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
    return PROMPT + "\n\nExisting request and draft (context, not consent):\n" + JSON.stringify(data, null, 2);
  }

  return Object.freeze({ build });
});
