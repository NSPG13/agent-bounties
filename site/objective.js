(function () {
  "use strict";

  const API = "https://api.bountyboard.global";
  const EXAMPLE = {
    objective: "Ship Agent Bounties as reliable coordination rails where AI agents decompose ambitious digital objectives, complete verifier-ready work, and receive canonical Base USDC payment without a human settlement gate.",
    constraints: [
      "Keep payment authority deterministic.",
      "Use existing canonical paid-loop evidence.",
      "Produce a judge-runnable developer tool.",
    ],
    budget: "12.00",
  };

  const previewPlan = {
    title: "Coordinate a verified agent objective",
    model: "GPT-5.6 preview",
    parallel_layers: [["define_terms"], ["build_work", "build_evals"], ["verify_release"]],
    tasks: [
      {
        task_id: "define_terms",
        title: "Commit measurable terms",
        goal: "Define the bounded digital outcome and replayable acceptance criteria.",
        depends_on: [],
        acceptance_criteria: ["Terms validate against the committed schema."],
        verifier: { kind: "schema", command: null },
        evidence_schema: { required: ["terms_digest"] },
        suggested_solver_reward_usdc: "2.000000",
      },
      {
        task_id: "build_work",
        title: "Implement the outcome",
        goal: "Produce the requested digital artifact against immutable terms.",
        depends_on: ["define_terms"],
        acceptance_criteria: ["The committed regression command exits successfully."],
        verifier: { kind: "command", command: "cargo test --workspace" },
        evidence_schema: { required: ["commit_sha", "test_digest"] },
        suggested_solver_reward_usdc: "5.000000",
      },
      {
        task_id: "build_evals",
        title: "Challenge the result",
        goal: "Replay negative fixtures and confirm the verifier fails closed.",
        depends_on: ["define_terms"],
        acceptance_criteria: ["Every known-bad fixture is rejected."],
        verifier: { kind: "github_ci", command: "cargo run -p cli -- bountybench" },
        evidence_schema: { required: ["ci_run_url", "fixture_digest"] },
        suggested_solver_reward_usdc: "3.000000",
      },
      {
        task_id: "verify_release",
        title: "Prove the paid release",
        goal: "Publish the canonical completion and settlement evidence.",
        depends_on: ["build_work", "build_evals"],
        acceptance_criteria: ["A confirmed canonical BountySettled event matches the solver and amount."],
        verifier: { kind: "http", endpoint: `${API}/v1/base/autonomous-bounties/events`, expected_status: 200 },
        evidence_schema: { required: ["settlement_event_id", "transaction_hash"] },
        suggested_solver_reward_usdc: "2.000000",
      },
    ],
  };

  const form = document.getElementById("objective-compiler-form");
  const graph = document.getElementById("objective-graph");
  const inspector = document.getElementById("task-inspector");
  const status = document.querySelector("[data-compiler-status]");
  const submit = form.querySelector("button[type='submit']");
  let currentPlan = previewPlan;

  function safeUrl(value) {
    try {
      const url = new URL(value);
      return url.protocol === "https:" ? url.href : null;
    } catch (_) {
      return null;
    }
  }

  function compactAddress(value) {
    if (!value || value.length < 18) return value || "unknown";
    return `${value.slice(0, 10)}...${value.slice(-6)}`;
  }

  function formatUsdc(value) {
    const number = Number(value);
    return Number.isFinite(number) ? number.toFixed(2) : "--";
  }

  function taskById(id) {
    return currentPlan.tasks.find((task) => task.task_id === id);
  }

  function dependencyClosure(task) {
    const related = new Set([task.task_id]);
    const queue = [...(task.depends_on || [])];
    while (queue.length) {
      const id = queue.shift();
      if (related.has(id)) continue;
      related.add(id);
      const dependency = taskById(id);
      if (dependency) queue.push(...(dependency.depends_on || []));
    }
    currentPlan.tasks.forEach((candidate) => {
      if ((candidate.depends_on || []).includes(task.task_id)) related.add(candidate.task_id);
    });
    return related;
  }

  function clearRelationshipHighlight() {
    graph.querySelectorAll(".graph-node").forEach((node) => node.removeAttribute("data-related"));
  }

  function highlightTask(task) {
    const related = dependencyClosure(task);
    graph.querySelectorAll(".graph-node").forEach((node) => {
      node.dataset.related = String(related.has(node.dataset.taskId));
      node.dataset.selected = String(node.dataset.taskId === task.task_id);
    });
  }

  function inspectTask(task) {
    highlightTask(task);
    const criteria = (task.acceptance_criteria || []).map((item) => `<li>${escapeHtml(item)}</li>`).join("");
    const evidence = ((task.evidence_schema || {}).required || []).map(escapeHtml).join(", ") || "No fields supplied";
    const verifier = task.verifier || {};
    const verifierDetail = verifier.command || verifier.endpoint || verifier.kind;
    inspector.innerHTML = `<strong>${escapeHtml(task.title)}</strong> <code>${escapeHtml(task.task_id)}</code><br>${escapeHtml(task.goal)}<br><strong>Verify:</strong> ${escapeHtml(verifierDetail || "unspecified")}<br><strong>Evidence:</strong> ${evidence}<ol>${criteria}</ol>`;
  }

  function escapeHtml(value) {
    const element = document.createElement("span");
    element.textContent = String(value == null ? "" : value);
    return element.innerHTML;
  }

  function renderGraph(plan, state) {
    currentPlan = plan;
    graph.textContent = "";
    document.querySelector("[data-graph-title]").textContent = plan.title;
    document.querySelector("[data-graph-model]").textContent = `${plan.model} | ${plan.tasks.length} validated tasks`;
    document.querySelector(".graph-shell").dataset.graphState = state;
    plan.parallel_layers.forEach((layer, layerIndex) => {
      const column = document.createElement("div");
      column.className = "graph-layer";
      column.dataset.layer = String(layerIndex);
      layer.forEach((taskId, taskIndex) => {
        const task = plan.tasks.find((candidate) => candidate.task_id === taskId);
        if (!task) return;
        const node = document.createElement("button");
        node.type = "button";
        node.className = "graph-node";
        node.dataset.taskId = task.task_id;
        node.style.animationDelay = `${(layerIndex * 80) + (taskIndex * 40)}ms`;
        const reward = task.suggested_solver_reward_usdc
          ? `${formatUsdc(task.suggested_solver_reward_usdc)} USDC solver budget`
          : "Budget not assigned";
        node.innerHTML = `<span class="graph-node-id">${escapeHtml(task.task_id)}</span><strong>${escapeHtml(task.title)}</strong><span class="graph-node-meta">${escapeHtml(task.verifier.kind)} verifier<br>${escapeHtml(reward)}</span>`;
        node.addEventListener("click", () => inspectTask(task));
        node.addEventListener("mouseenter", () => highlightTask(task));
        node.addEventListener("mouseleave", clearRelationshipHighlight);
        column.append(node);
      });
      graph.append(column);
    });
    const first = plan.tasks[0];
    if (first) inspectTask(first);
  }

  async function loadReadiness() {
    const container = document.querySelector(".compiler-readiness");
    const text = document.querySelector("[data-readiness-text]");
    try {
      const response = await fetch(`${API}/v1/cloud-agent/readiness`, { cache: "no-store" });
      if (!response.ok) throw new Error(`HTTP ${response.status}`);
      const readiness = await response.json();
      const ready = readiness.available
        && readiness.protocol === "open_ai_responses"
        && String(readiness.model || "").startsWith("gpt-5.6");
      container.dataset.compilerReadiness = ready ? "ready" : "unavailable";
      text.textContent = ready
        ? `${readiness.model} hosted and ready`
        : `Hosted compiler unavailable: ${(readiness.missing_configuration || []).join(", ") || "GPT-5.6 configuration mismatch"}`;
    } catch (error) {
      container.dataset.compilerReadiness = "unavailable";
      text.textContent = `Readiness check failed: ${error.message}`;
    }
  }

  async function compileObjective(event) {
    event.preventDefault();
    submit.disabled = true;
    submit.setAttribute("aria-busy", "true");
    status.textContent = "GPT-5.6 is decomposing the objective. Deterministic validation runs before anything is shown.";
    const data = new FormData(form);
    const constraints = String(data.get("constraints") || "")
      .split("\n")
      .map((item) => item.trim())
      .filter(Boolean);
    const body = {
      objective: String(data.get("objective") || "").trim(),
      context: "OpenAI Build Week developer-tool entry using the production Agent Bounties protocol.",
      constraints,
      max_tasks: Number(data.get("max_tasks") || 5),
      solver_budget_usdc: String(data.get("solver_budget_usdc") || "").trim() || null,
      source_url: "https://github.com/NSPG13/agent-bounties/issues/421",
      idempotency_key: `build-week:${crypto.randomUUID()}`,
    };
    try {
      const response = await fetch(`${API}/v1/cloud-agent/objective-plans`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(body),
      });
      const result = await response.json().catch(() => ({}));
      if (!response.ok) throw new Error(result.error || `Compiler returned HTTP ${response.status}`);
      renderGraph(result, "compiled");
      status.textContent = `${result.tasks.length} tasks passed graph, verifier, evidence, and budget validation. Review before publishing.`;
      if (typeof window.agentBountiesTrack === "function") {
        window.agentBountiesTrack("objective_compiled", { task_count: result.tasks.length });
      }
    } catch (error) {
      status.textContent = `${error.message}. The preview remains visible; no bounty, signature, or payment was created.`;
    } finally {
      submit.disabled = false;
      submit.removeAttribute("aria-busy");
    }
  }

  function resetExample() {
    form.elements.objective.value = EXAMPLE.objective;
    form.elements.constraints.value = EXAMPLE.constraints.join("\n");
    form.elements.solver_budget_usdc.value = EXAMPLE.budget;
    renderGraph(previewPlan, "preview");
    status.textContent = "The model proposes. Deterministic code validates. Contracts settle.";
  }

  function opportunityReward(item) {
    if (!item.reward || item.reward.currency !== "USDC") return 0;
    const raw = Number(item.reward.amount || 0);
    if (!Number.isFinite(raw)) return 0;
    return item.reward.unit === "base_units" ? raw / 1_000_000 : raw;
  }

  async function loadEvidence() {
    const updated = document.querySelector("[data-evidence-updated]");
    try {
      const [claimResponse, opportunityResponse] = await Promise.all([
        fetch(`${API}/v1/base/autonomous-bounties/claim-funnel?window_hours=720`, { cache: "no-store" }),
        fetch(`${API}/v1/opportunities?network=base-mainnet&limit=300`, { cache: "no-store" }),
      ]);
      if (!claimResponse.ok || !opportunityResponse.ok) throw new Error("Canonical evidence endpoint unavailable");
      const [claim, projection] = await Promise.all([claimResponse.json(), opportunityResponse.json()]);
      const outcomes = claim.canonical_outcomes || {};
      const paid = (projection.items || [])
        .filter((item) => item.source_type === "canonical_base" && item.payment_state === "paid")
        .sort((left, right) => Date.parse(right.updated_at) - Date.parse(left.updated_at));
      const rewards = paid.reduce((total, item) => total + opportunityReward(item), 0);
      document.querySelector("[data-evidence-settlements]").textContent = String(outcomes.settlements_confirmed || 0);
      document.querySelector("[data-evidence-rewards]").textContent = formatUsdc(rewards);
      document.querySelector("[data-evidence-solvers]").textContent = String(outcomes.unique_paid_solver_wallets || 0);
      document.querySelector("[data-evidence-repeat]").textContent = String(outcomes.repeat_paid_solver_wallets || 0);
      updated.textContent = `Verified ${new Date(claim.generated_at).toLocaleString()}`;
      updated.dateTime = claim.generated_at;

      const list = document.querySelector("[data-proof-list]");
      list.textContent = "";
      paid.slice(0, 4).forEach((item, index) => {
        const proofUrl = safeUrl((item.proof_urls || [])[0] || item.public_url);
        const row = document.createElement("li");
        const reward = formatUsdc(opportunityReward(item));
        row.innerHTML = `<span class="proof-index">${String(index + 1).padStart(2, "0")}</span><span><strong>${escapeHtml(item.title || "Settled bounty")}</strong><br><small>${escapeHtml(compactAddress(item.bounty_contract || item.opportunity_id))} | ${reward} USDC</small></span>`;
        if (proofUrl) {
          const link = document.createElement("a");
          link.href = proofUrl;
          link.textContent = "Inspect proof";
          row.append(link);
        }
        list.append(row);
      });
      if (!paid.length) list.innerHTML = "<li>No paid proof is currently projected.</li>";
    } catch (error) {
      updated.textContent = `${error.message}. Retry from the canonical API.`;
    }
  }

  form.addEventListener("submit", compileObjective);
  document.querySelector("[data-load-example]").addEventListener("click", resetExample);
  renderGraph(previewPlan, "preview");
  loadReadiness();
  loadEvidence();
}());
