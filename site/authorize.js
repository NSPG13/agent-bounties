(() => {
  "use strict";

  const byId = (id) => document.getElementById(id);
  const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;
  let protocol = null;
  let intent = null;

  function output(lines, tone = "") {
    const element = byId("authorization-output");
    element.textContent = Array.isArray(lines) ? lines.join("\n") : lines;
    element.dataset.tone = tone;
  }

  async function requestJson(url) {
    const response = await fetch(url, { cache: "no-store" });
    const body = await response.json().catch(() => null);
    if (!response.ok) {
      throw new Error(body?.message || body?.error || `Request failed (${response.status}).`);
    }
    return body;
  }

  function intentId() {
    const value = new URLSearchParams(location.search).get("intent");
    if (!UUID.test(value || "")) throw new Error("This authorization link is invalid.");
    return value;
  }

  function destinationFor(value) {
    const params = new URLSearchParams({ intent: value.intent_id, from: "chatgpt-app" });
    if (value.bounty_contract) params.set("bountyContract", value.bounty_contract);
    if (value.action === "fund" && value.amount_base_units) {
      params.set("amount", (Number(value.amount_base_units) / 1_000_000).toFixed(6));
    }
    if (value.action === "solve" || value.action === "compete") {
      params.set("source", "chatgpt-app");
    }
    const page = {
      post: "post.html",
      fund: "funding.html",
      solve: "earn.html",
      compete: "earn.html",
      complete: "earn.html",
      verify: "verify.html",
    }[value.action];
    if (!page) throw new Error("This action type is not supported.");
    return `${page}?${params}`;
  }

  function actionTitle(action) {
    return {
      post: "Post a bounty",
      fund: "Fund this bounty",
      solve: "Solve this bounty",
      compete: "Solve this bounty",
      complete: "Submit completed work",
      verify: "Verify this submission",
    }[action] || "Review this bounty action";
  }

  function render(value) {
    intent = value;
    byId("authorization-title").textContent = actionTitle(value.action);
    byId("authorization-summary").textContent =
      value.status === "confirmed"
        ? "The matching canonical event is confirmed."
        : "Review the exact destination and evidence requirement before continuing.";
    byId("authorization-network").textContent = value.network;
    byId("authorization-contract").textContent =
      value.bounty_contract || "Created during authorization";
    byId("authorization-amount").textContent = value.amount_base_units
      ? `${(Number(value.amount_base_units) / 1_000_000).toFixed(6)} USDC`
      : "No transfer amount in this step";
    byId("authorization-events").textContent = value.expected_canonical_events.join(" or ");
    byId("authorization-details").textContent = JSON.stringify(value.details || {}, null, 2);
    byId("authorization-boundary").textContent = value.evidence_boundary;
    const status = byId("authorization-status");
    status.textContent = value.status.replaceAll("_", " ");
    status.dataset.tone = value.status === "confirmed"
      ? "success"
      : value.status === "expired" || value.status === "failed"
        ? "error"
        : "pending";
    const button = byId("authorization-continue");
    button.hidden = value.status !== "review_required";
    if (!button.hidden) button.href = destinationFor(value);
    if (value.status === "confirmed") {
      output([
        `Confirmed: ${value.canonical_event_kind}.`,
        value.paid
          ? "BountySettled confirms solver payment."
          : "This confirms the action, but it is not payout evidence.",
        "Return to ChatGPT and refresh the bounty card, then share the result.",
      ], "success");
    } else if (value.status === "pending_confirmation") {
      output([
        `Transaction observed: ${value.transaction_hash}.`,
        "Waiting for the exact indexed canonical event. Do not approve the action again.",
      ], "pending");
    } else if (value.status === "expired" || value.status === "failed") {
      output(value.next_action, "error");
    } else {
      output([
        "No wallet request is made on this review screen.",
        "Continue only if the action, contract, amount, and evidence requirement are correct.",
      ], "pending");
    }
  }

  async function refresh() {
    try {
      if (!protocol) {
        protocol = await requestJson("protocol.json");
      }
      const api = protocol.api_base_url.replace(/\/$/, "");
      render(await requestJson(`${api}/v1/chatgpt/action-intents/${intentId()}`));
    } catch (error) {
      byId("authorization-status").textContent = "unavailable";
      byId("authorization-status").dataset.tone = "error";
      output(error.message || String(error), "error");
    }
  }

  byId("authorization-refresh").addEventListener("click", refresh);
  refresh();
})();
