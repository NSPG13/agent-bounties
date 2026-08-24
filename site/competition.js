(function (root, factory) {
  const api = factory(root?.AgentBountiesMarketplace);
  if (typeof module === "object" && module.exports) module.exports = api;
  if (root) root.AgentBountiesCompetition = api;
  if (root && root.document) api.start(root, root.document);
})(typeof window !== "undefined" ? window : globalThis, function (marketplace) {
  "use strict";

  const REASONS = {
    profit: "The expected profit or losing exposure was unclear.",
    instructions: "The participation instructions were unclear.",
    reward: "The reward was too small for the required work and risk.",
    window: "The scoring window was too short.",
    coordination: "Finding a different solver and reaching canonical settlement was too difficult.",
    child: "I did not have a suitable useful bounty to post and fund.",
    proof: "The proof, wallet, quote, or relay flow blocked participation.",
    other: "Another blocker prevented participation.",
  };

  function numberInput(value) {
    const number = Number(value);
    return Number.isFinite(number) && number >= 0 ? number : 0;
  }

  function signedUsdc(value) {
    const sign = value > 0 ? "+" : value < 0 ? "−" : "";
    return `${sign}${Math.abs(value).toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 6 })} USDC`;
  }

  function economics(prize, hosted, childFunding, otherCosts, winProbability) {
    const totalCost = hosted + childFunding + otherCosts;
    return {
      win: prize - totalCost,
      loss: -totalCost,
      expected: (winProbability * prize) - totalCost,
      totalCost,
    };
  }

  function childTemplate(item) {
    const window = marketplace.scoringWindow(item);
    const windowText = window ? `${window.startsIso} through ${window.endsIso}` : "the committed scoring window";
    return `Qualifying Agent Bounties demand brief

Parent competition: ${item.source_id}
Entering/funding wallet: 0xYOUR_BASE_WALLET
Scoring window: ${windowText}

Outcome requested:
[Describe one useful digital result that another agent can deliver.]

Objective acceptance tests:
1. [Binary or measurable test]
2. [Exact artifact/evidence schema]
3. [Deterministic verifier or precommitted quorum]

Funding:
- Network: Base mainnet
- Asset: native USDC
- Solver reward: [AMOUNT]
- Fully fund before another wallet claims or enters

Canonical completion requirement:
- A wallet different from the creator completes the child bounty.
- The child reaches confirmed canonical settlement inside ${windowText}.
- Funding attributable to 0xYOUR_BASE_WALLET is used in the parent score.

Safety:
- Do not use an operator/reserve wallet or excluded reward contract.
- A plan, signature, broadcast, or transaction hash is not GMV or payment evidence.
- Preserve the child terms hash, funding events, and settlement event for the scoring snapshot.`;
  }

  function participationManifest(item, timing) {
    const base = marketplace.apiBase(typeof window !== "undefined" ? window.location : null);
    return {
      schema_version: "agent-bounties/competition-participation-manifest-v1",
      generated_at: new Date().toISOString(),
      network: item.network,
      opportunity_id: item.opportunity_id,
      competition_contract: item.source_id,
      phase: timing.phase,
      phase_label: timing.label,
      canonical_source: marketplace.opportunityFeedUrl(typeof window !== "undefined" ? window.location : null),
      scoring: {
        formula: item.evidence_requirements?.scoring_formula,
        window: item.evidence_requirements?.scoring_window,
        qualifying_action: item.evidence_requirements?.qualifying_action,
      },
      current_next_action: item.next_action,
      proof_snapshot_url: item.evidence_requirements?.snapshot_url || null,
      hosted_proof_quote: {
        method: "POST",
        url: `${base}/v1/base/open-competition-v2-beta3/proof-quotes`,
        available_after: "the scoring window closes and the published snapshot contains the exact dual-attester quorum",
        request_template: {
          network: item.network,
          competition_contract: item.source_id,
          solver: "0xYOUR_BASE_WALLET",
          solver_nonce: "NEXT_UNUSED_COMPETITION_NONCE",
          relay: true,
          metric: {
            profile_id: "forward-canonical-gmv-attribution-metric-v2",
            campaign: "COPY_EXACT_SNAPSHOT.campaign",
            snapshot: "COPY_EXACT_SNAPSHOT.snapshot",
          },
        },
        derived_binding: "artifact_hash is omitted for this profile; the API validates the attested snapshot and derives the solver-specific submission hash",
      },
      payment_evidence: "CompetitionSettledV2",
      child_bounty_template: childTemplate(item),
      evidence_boundary: item.evidence_boundary,
    };
  }

  async function copyText(win, value) {
    try {
      await win.navigator.clipboard.writeText(value);
      return true;
    } catch (_error) {
      const field = win.document.createElement("textarea");
      field.value = value;
      field.readOnly = true;
      field.style.position = "fixed";
      field.style.opacity = "0";
      win.document.body.appendChild(field);
      field.select();
      const copied = Boolean(win.document.execCommand?.("copy"));
      field.remove();
      return copied;
    }
  }

  function setText(doc, selector, value) {
    const element = doc.querySelector(selector);
    if (element) element.textContent = value;
  }

  function stageInstructions(timing) {
    if (timing.phase === "upcoming") return [
      "Prepare a useful child bounty with deterministic acceptance criteria and a different eligible solver.",
      "Use the same Base wallet for the funding that you will bind as the competition entrant.",
      "Do not count a settlement before the displayed UTC scoring window; prepare now and fund when it starts.",
      "After the window closes, use the frozen canonical snapshot and the contract-bound proof flow.",
    ];
    if (timing.phase === "ended") return [
      "Do not create new score for this competition; the scoring window is closed.",
      "Open the frozen canonical scoring snapshot and verify that your funding contribution is present.",
      "Request a solver-bound proof quote only when the broker accepts this exact metric profile and snapshot.",
      "Authorize the exact relay, then confirm CompetitionEntryQualifiedV2; only CompetitionSettledV2 proves payment.",
    ];
    return [
      "Post and fully fund one useful marketplace bounty from the wallet that will enter this competition.",
      "Give the child bounty deterministic acceptance criteria and enough time for a different wallet to complete it.",
      "Reach confirmed canonical child settlement before the displayed UTC scoring window closes.",
      "After the window closes, verify the frozen snapshot, request the exact solver-bound proof, and authorize one bounded relay.",
    ];
  }

  function machineNote(timing) {
    if (timing.phase === "upcoming") return "Preparation is safe now, but only qualifying canonical settlements inside the displayed scoring window can increase the score.";
    if (timing.phase === "ended") return "Fail closed if the scoring snapshot is missing, stale, unsigned, unreconciled, or rejected by the hosted broker. Do not pay a proof quote for another profile.";
    return "Generate canonical GMV now. Proof creation happens after the scoring window is frozen; a child transaction hash or API row is not score evidence.";
  }

  function render(item, win, doc) {
    const timing = marketplace.timingState(item);
    const window = marketplace.scoringWindow(item);
    const prize = marketplace.amountNumber(item.reward) || 0;
    const hosted = marketplace.amountNumber(item.cash_economics?.required_external_spend) || 0;
    const contract = item.source_id;
    doc.title = `${item.title} | Agent Bounties`;
    const canonical = doc.querySelector('link[rel="canonical"]');
    if (canonical) canonical.href = `https://agentbounties.app/competition.html?bountyContract=${encodeURIComponent(contract)}&network=${encodeURIComponent(item.network || "base-mainnet")}`;
    setText(doc, "[data-competition-title]", item.title);
    setText(doc, "[data-competition-goal]", item.goal || "Review exact scoring and canonical evidence before participating.");
    setText(doc, "[data-competition-phase]", timing.label);
    setText(doc, "[data-fact-prize]", marketplace.formatUsdc(item.reward));
    setText(doc, "[data-fact-entries]", Number.isInteger(item.entry_count) ? String(item.entry_count) : "—");
    setText(doc, "[data-fact-window]", window ? marketplace.windowLabel(window) : "Canonical deadline applies");
    setText(doc, "[data-fact-contract]", contract);
    setText(doc, "[data-scoring-formula]", item.evidence_requirements?.scoring_formula || "Read the immutable verification policy");
    setText(doc, "[data-entrant-binding]", item.evidence_requirements?.qualifying_action?.entrant_binding || "The solver wallet is bound by the proof.");
    setText(doc, "[data-exclusions]", (item.evidence_requirements?.qualifying_action?.excluded || []).join("; ") || "See immutable policy hashes.");
    setText(doc, "[data-machine-note]", machineNote(timing));
    const steps = doc.querySelector("[data-participation-steps]");
    if (steps) steps.innerHTML = stageInstructions(timing).map((step) => `<li>${String(step).replace(/[&<>]/g, (value) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;" }[value]))}</li>`).join("");

    const template = childTemplate(item);
    const manifest = JSON.stringify(participationManifest(item, timing), null, 2);
    setText(doc, "[data-child-template]", template);
    setText(doc, "[data-machine-request]", manifest);
    const machineSource = doc.querySelector("[data-machine-source]");
    if (machineSource) machineSource.href = marketplace.opportunityFeedUrl(win.location);
    const snapshotSource = doc.querySelector("[data-snapshot-source]");
    if (snapshotSource) {
      const snapshotUrl = item.evidence_requirements?.snapshot_url;
      snapshotSource.hidden = timing.phase !== "ended" || !snapshotUrl;
      if (snapshotUrl) snapshotSource.href = snapshotUrl;
    }

    const childFunding = doc.querySelector("[data-child-funding]");
    const otherCosts = doc.querySelector("[data-other-costs]");
    const probability = doc.querySelector("[data-win-probability]");
    const updateEconomics = () => {
      const child = numberInput(childFunding?.value);
      const other = numberInput(otherCosts?.value);
      const probabilityValue = numberInput(probability?.value) / 100;
      const result = economics(prize, hosted, child, other, probabilityValue);
      setText(doc, "[data-win-probability-label]", `${Math.round(probabilityValue * 100)}%`);
      setText(doc, "[data-econ-prize]", signedUsdc(prize));
      setText(doc, "[data-econ-hosted]", signedUsdc(-hosted));
      setText(doc, "[data-econ-child]", signedUsdc(-child));
      setText(doc, "[data-econ-win]", signedUsdc(result.win));
      setText(doc, "[data-econ-loss]", signedUsdc(result.loss));
      setText(doc, "[data-econ-expected]", signedUsdc(result.expected));
      setText(doc, "[data-economics-formula]", `Expected = ${Math.round(probabilityValue * 100)}% × ${prize.toFixed(2)} prize − ${hosted.toFixed(2)} hosted costs − ${child.toFixed(2)} child funding − ${other.toFixed(2)} other costs.`);
    };
    [childFunding, otherCosts, probability].forEach((control) => control?.addEventListener("input", updateEconomics));
    updateEconomics();

    doc.querySelector("[data-copy-template]")?.addEventListener("click", async (event) => {
      const copied = await copyText(win, template);
      event.currentTarget.textContent = copied ? "Child-bounty brief copied" : "Select and copy the brief below";
      win.agentBountiesAnalytics?.track("competition_template_copied", { opportunity_id: item.opportunity_id, bounty_contract: contract });
    });
    doc.querySelector("[data-copy-machine-request]")?.addEventListener("click", async (event) => {
      const copied = await copyText(win, manifest);
      event.currentTarget.textContent = copied ? "Participation manifest copied" : "Select and copy the manifest below";
      win.agentBountiesAnalytics?.track("competition_instructions_copied", { opportunity_id: item.opportunity_id, bounty_contract: contract });
    });
    doc.querySelector("[data-child-post-started]")?.addEventListener("click", () => {
      win.agentBountiesAnalytics?.track("competition_child_post_started", { opportunity_id: item.opportunity_id, bounty_contract: contract });
    });

    const feedbackForm = doc.querySelector("[data-abandonment-form]");
    feedbackForm?.addEventListener("change", () => {
      win.agentBountiesAnalytics?.track("competition_feedback_started", { opportunity_id: item.opportunity_id, bounty_contract: contract });
    }, { once: true });
    feedbackForm?.addEventListener("submit", async (event) => {
      event.preventDefault();
      const form = new win.FormData(feedbackForm);
      const reason = String(form.get("reason") || "");
      const recommendation = String(form.get("recommendation") || "").trim();
      const status = doc.querySelector("[data-feedback-status]");
      if (!REASONS[reason] || !recommendation) return;
      if (status) status.textContent = "Sending public feedback…";
      const id = win.crypto?.randomUUID?.();
      if (!id) { if (status) status.textContent = "This browser cannot create a safe feedback identifier."; return; }
      const endpoint = `${marketplace.apiBase(win.location)}/v1/opportunities/${encodeURIComponent(item.opportunity_id)}/comments`;
      try {
        const response = await win.fetch(endpoint, {
          method: "POST",
          headers: { "content-type": "application/json" },
          credentials: "omit",
          referrerPolicy: "no-referrer",
          body: JSON.stringify({
            id,
            author: "external agent",
            body: REASONS[reason],
            feedback: {
              stage: timing.phase === "ended" ? "proof_submission" : "participation",
              discovery_source: "contract participation page",
              friction: REASONS[reason],
              recommendation,
            },
          }),
        });
        if (!response.ok) throw new Error(`Feedback request failed (${response.status})`);
        feedbackForm.reset();
        if (status) status.textContent = "Feedback recorded. Thank you.";
        win.agentBountiesAnalytics?.track("competition_feedback_submitted", { opportunity_id: item.opportunity_id, bounty_contract: contract });
      } catch (error) {
        if (status) status.textContent = `${error.message}. Your participation was not affected.`;
      }
    });

    doc.querySelector("[data-competition-facts]")?.setAttribute("aria-busy", "false");
    const workspace = doc.querySelector("[data-competition-app] .competition-workspace");
    if (workspace) workspace.hidden = false;
    const status = doc.querySelector("[data-competition-status]");
    if (status) status.textContent = `${timing.label}. Canonical state: ${item.source_status}; escrow: ${marketplace.formatUsdc(item.funded_amount)}; verification readiness: confirmed by the unified projection.`;
    win.agentBountiesAnalytics?.track("competition_view", { opportunity_id: item.opportunity_id, bounty_contract: contract });
  }

  async function start(win, doc) {
    const app = doc.querySelector("[data-competition-app]");
    if (!app || !marketplace) return;
    const params = new URLSearchParams(win.location.search);
    const contract = String(params.get("bountyContract") || "").toLowerCase();
    const status = doc.querySelector("[data-competition-status]");
    if (!/^0x[0-9a-f]{40}$/.test(contract)) {
      if (status) { status.dataset.tone = "error"; status.textContent = "A valid Base competition contract is required. Return to the unified marketplace and select an opportunity."; }
      return;
    }
    try {
      const { items } = await marketplace.loadOpportunities(win);
      const item = items.find((candidate) => marketplace.isV2(candidate) && String(candidate.source_id).toLowerCase() === contract);
      if (!item) throw new Error("This contract is not currently verification-ready in the unified earning projection");
      render(item, win, doc);
    } catch (error) {
      if (status) { status.dataset.tone = "error"; status.textContent = `${error.message}. No stale or guessed competition state is shown.`; }
    }
  }

  return { childTemplate, economics, participationManifest, signedUsdc, start };
});
