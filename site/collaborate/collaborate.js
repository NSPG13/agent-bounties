(function () {
  "use strict";
  const api = window.AgentBountiesDiscovery;
  const key = "agent-bounties/free-discovery-assessment-v1";
  const status = document.getElementById("discovery-status");
  const results = document.getElementById("discovery-results");
  const run = document.getElementById("check-discovery");
  const exportButton = document.getElementById("download-assessment");
  const clearButton = document.getElementById("clear-assessment");
  let assessment = null;

  function node(tag, text, className) {
    const element = document.createElement(tag);
    if (text !== undefined) element.textContent = text;
    if (className) element.className = className;
    return element;
  }

  function render(value, restored) {
    results.replaceChildren();
    results.appendChild(node("p", `${restored ? "Saved" : "Observed"} ${value.observed_at}. Source snapshot: ${value.generated_at}. Recheck before acting.`, "snapshot-time"));
    if (!value.items.length) results.appendChild(node("p", "No opportunities were returned in this sample. You can still contribute an integration, a task brief or a reproducible usability report below."));
    value.items.forEach(item => {
      const card = node("article", undefined, "opportunity-card");
      card.appendChild(node("h3", item.title));
      card.appendChild(node("p", item.assessment === "inspect_terms" ? "Next: inspect the terms" : "Needs a fresh check", "result-label"));
      const facts = node("dl");
      [["Solver reward", item.reward.display], ["Refundable bond", item.refundable_bond.display],
       ["Required external spend", item.required_external_spend.display],
       ["Verification reported ready", item.verification_ready ? "Yes — snapshot only" : "No"],
       ["Work / payment", `${item.work_state} / ${item.payment_state}`],
       ["Deadline", item.deadline ? `${item.deadline} (${item.deadline_kind})` : "Not reported"]].forEach(([label, value]) => {
        facts.append(node("dt", label), node("dd", value));
      });
      card.appendChild(facts);
      if (item.blockers.length) {
        const list = node("ul");
        item.blockers.forEach(reason => list.appendChild(node("li", reason)));
        card.appendChild(list);
      }
      const href = api.publicLink(item.public_url);
      if (href) {
        const link = node("a", "Read public terms →");
        link.href = href;
        card.appendChild(link);
      }
      results.appendChild(card);
    });
    results.appendChild(node("p", value.evidence_boundary, "fine-print"));
    exportButton.disabled = false;
    clearButton.disabled = false;
  }

  run.addEventListener("click", async () => {
    run.disabled = true;
    results.setAttribute("aria-busy", "true");
    status.textContent = "Reading a small sample of public opportunities…";
    const controller = new AbortController();
    const timeout = window.setTimeout(() => controller.abort(), 10000);
    try {
      const response = await fetch(api.FEED_URL, { signal: controller.signal, credentials: "omit", cache: "no-store", headers: { accept: "application/json" } });
      if (!response.ok) throw new Error(`The market returned ${response.status}. Retry or use Browse work.`);
      const text = await response.text();
      if (text.length > 1000000) throw new Error("The response was larger than expected. Use Browse work.");
      assessment = api.assess(JSON.parse(text));
      let saved = false;
      try { localStorage.setItem(key, JSON.stringify(assessment)); saved = true; } catch (_) { /* Export remains available without browser storage. */ }
      render(assessment, false);
      status.textContent = `Assessment ready. ${saved ? "Saved in this browser; return here to resume." : "Browser storage is unavailable; download your assessment to keep it."} Nothing was submitted or paid.`;
    } catch (error) {
      status.textContent = error.name === "AbortError" ? "The market took too long. Retry when ready. Your previous saved assessment is unchanged." : "The discovery check could not finish. Retry or use Browse work. Your previous saved assessment is unchanged.";
    } finally {
      window.clearTimeout(timeout);
      run.disabled = false;
      results.setAttribute("aria-busy", "false");
    }
  });

  exportButton.addEventListener("click", () => {
    if (!assessment) return;
    const url = URL.createObjectURL(new Blob([JSON.stringify(assessment, null, 2) + "\n"], { type: "application/json" }));
    const link = node("a");
    link.href = url;
    link.download = "agent-bounties-discovery-assessment.json";
    document.body.appendChild(link);
    link.click();
    link.remove();
    window.setTimeout(() => URL.revokeObjectURL(url), 1000);
  });
  clearButton.addEventListener("click", () => {
    try { localStorage.removeItem(key); } catch (_) { status.textContent = "Browser storage is unavailable. Clear site data in your browser to remove any stored copy."; return; }
    assessment = null;
    results.replaceChildren();
    exportButton.disabled = clearButton.disabled = true;
    status.textContent = "Saved assessment cleared from this browser.";
  });

  document.querySelectorAll("[data-copy]").forEach(button => button.addEventListener("click", async () => {
    const target = document.getElementById(button.dataset.copy);
    try { await navigator.clipboard.writeText(target.textContent); button.textContent = "Copied"; }
    catch (_) { button.textContent = "Select and copy the text below"; target.focus(); }
  }));

  try {
    const saved = JSON.parse(localStorage.getItem(key) || "null");
    if (saved && saved.schema_version === api.SCHEMA && saved.mode === "read_only" &&
        saved.source_url === api.FEED_URL && Number.isFinite(Date.parse(saved.observed_at)) &&
        Number.isFinite(Date.parse(saved.generated_at)) && Array.isArray(saved.items) && saved.items.length <= 5) {
      // Rendering uses textContent and revalidates link destinations, including restored data.
      render(saved, true);
      assessment = saved;
      status.textContent = "Your saved assessment is restored. It is a historical snapshot; recheck for current availability.";
    }
  } catch (_) {
    results.replaceChildren();
    assessment = null;
    exportButton.disabled = clearButton.disabled = true;
    status.textContent = "No readable saved assessment. Run a new check when ready.";
  }
})();
