(function (root, factory) {
  const api = factory();
  if (typeof module === "object" && module.exports) module.exports = api;
  if (root) root.AgentBountiesPostingPrompt = api;
})(typeof window !== "undefined" ? window : globalThis, function () {
  "use strict";

  const PROMPT = `Help me create, fund and publicly post a bounty on Agent Bounties.

1. Use https://agentbounties.app/post.html in @Browser, preserving existing page context. Discover WebMCP tools and read page context; confirm actual access. Read https://agentbounties.app/.well-known/agent-bounties.json and https://agentbounties.app/llms.txt before choosing endpoints. Fallback to connected official MCP tools; otherwise prepare a portable draft and say posting and funding remain incomplete. Never request API keys.

2. Save and resume my posting journey; preserve my answers and draft. Ask together only for missing business decisions: outcome, total USDC budget and deadline. Propose deliverables and measurable checks. Use a supported reviewed verifier; for design work, propose creator review with my exact calendar deadline and explain that I confirm the verdict. Stage the draft and check funding readiness. Never invent verifier details or replace a calendar deadline with days after claim.

3. Show one review of public terms, rewards, fees, total cost and deadline. Leave publication, legal, funding and payment consent to me. Reuse unchanged approvals; handle routine preparation yourself.

4. Guide wallet connection, offering phone-wallet QR pairing. Explain each request's amount, network, recipient, expiry, purpose and gas cost or confirmed sponsorship. Never request private keys or seed phrases. Leave wallet confirmations to me.

5. After my confirmations, resume automatically. Reconcile the same posting operation; never repeat uncertain transactions. Confirm canonical creation, funding and claimability, then find the exact bounty in public ready-to-earn inventory. Return its public link and confirmed status, or the precise unfinished step. A draft, signature or transaction hash is not completion; only tool results and confirmed canonical Base USDC evidence prove actions and payments.

Without tools, return importable JSON:
{"title":"...","goal":"...","acceptance_criteria":["..."],"solver_reward_usdc":"2.00","verifier_reward_usdc":"0.10","task_window_days":30,"source_url":null,"benchmark":null,"evidence_schema":null}
Use my agreed amounts and days. For creator review add "review_mode":"creator" and "delivery_deadline" as the agreed ISO timestamp with timezone offset; omit automated benchmark fields. Preserve parent bindings and approved image fields. Missing verification stays unfundable; JSON is not publication or funding.`;

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
