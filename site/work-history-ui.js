(function () {
  "use strict";
  const reader = window.AgentBountiesWorkHistory, form = document.querySelector("[data-work-history-form]");
  if (!reader || !form) return;
  const input = form.querySelector("input"), button = form.querySelector("button"), status = document.querySelector("[data-work-history-status]"), output = document.querySelector("[data-work-history-results]");
  const element = (tag, text) => { const node = document.createElement(tag); if (text !== undefined) node.textContent = text; return node; };
  let busy = false;
  try { const value = new URLSearchParams(location.hash.slice(1)).get("wallet"); if (value) input.value = reader.address(value); } catch (_) { /* Invalid fragments never start a lookup. */ }
  form.addEventListener("submit", async event => {
    event.preventDefault(); if (busy) return;
    let wallet;
    try { wallet = reader.address(input.value.trim()); } catch (error) { status.textContent = error.message; input.focus(); return; }
    busy = true; button.disabled = true; input.readOnly = true; output.replaceChildren();
    status.textContent = "Reading three public event sources…";
    try {
      const snapshot = await reader.inspect(wallet);
      history.replaceState(history.state, "", `${location.pathname}${location.search}#wallet=${wallet}`);
      const prefix = snapshot.status === "unavailable" ? "Sources unavailable. No history or zero earnings can be inferred." : snapshot.status === "partial" ? "Partial results: at least one source or matching record could not be used." : "The three inspected sources responded. Lifetime coverage and chain freshness remain unverified.";
      status.textContent = `${prefix} ${snapshot.records.length ? `${snapshot.records.length} settlement observations; ${reader.formatAmount(snapshot.solver_reward_total_base_units)} USDC in reported solver rewards.` : snapshot.status === "unavailable" ? "" : "No matching indexed settlement in the usable sources. This does not mean this wallet has never completed work."} You can retry with Inspect work history.`;
      const sourceList = element("ul");
      for (const source of snapshot.sources) {
        const item = element("li"), link = element("a", source.id); link.href = source.url;
        item.append(link, document.createTextNode(` — ${source.available ? `${source.matched_records} matches; ${source.rejected_matching_records} unusable matching records` : source.error}. Read at ${source.observed_at}.`)); sourceList.append(item);
      }
      output.append(sourceList);
      for (const row of snapshot.records) {
        const card = element("article"), title = element("h3", row.protocol.replace("agent-bounties/", ""));
        const bounty = element("a", row.bounty_contract); bounty.href = `participate.html?bountyContract=${row.bounty_contract}&network=base-mainnet`;
        const proof = element("a", `Transaction and log ${row.log_index}`); proof.href = `https://basescan.org/tx/${row.tx_hash}#eventlog`;
        card.append(title, bounty, element("p", `Reported event: ${row.event_time} · block ${row.block_number}`), element("p", `Solver reward: ${reader.formatAmount(row.solver_reward_base_units)} USDC. Returned bond: ${reader.formatAmount(row.returned_bond_base_units)}${row.returned_bond_base_units === null ? "" : " USDC"}. Timeout bonus: ${reader.formatAmount(row.timeout_bonus_base_units)}${row.timeout_bonus_base_units === null ? "" : " USDC"}.`), proof);
        output.append(card);
      }
      const details = element("details"), summary = element("summary", "Inspect or copy the same JSON for your agent"), json = element("textarea");
      json.readOnly = true; json.rows = 10; json.value = JSON.stringify(snapshot, null, 2); json.setAttribute("aria-label", "Wallet work history JSON");
      details.append(summary, json); output.append(details);
    } catch (_) { status.textContent = "The lookup could not finish. Your wallet address is preserved; try Inspect work history again."; }
    finally { busy = false; button.disabled = false; input.readOnly = false; }
  });
})();
