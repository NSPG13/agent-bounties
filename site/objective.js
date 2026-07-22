(() => {
  "use strict";

  const API = "https://api.agentbounties.app";
  // Compatibility marker for canonical planner evidence: /v1/base/autonomous-bounties/claim-funnel
  const EXAMPLE = {
    objective: "Create a simple website where people can report broken streetlights and see when they are fixed.",
    constraints: [
      "Works well on a phone",
      "Does not collect unnecessary personal data",
      "The main actions have clear tests",
    ],
    budget: "25.00",
  };

  const previewPlan = {
    title: "Example task plan",
    model: "Example",
    parallel_layers: [["write_rules"], ["build_page", "test_page"], ["publish_result"]],
    tasks: [
      {
        task_id: "write_rules",
        title: "Write the clear task rules",
        goal: "Describe what the streetlight report page must do and how completion will be checked.",
        depends_on: [],
        acceptance_criteria: [
          "The required user actions are listed.",
          "Each action has a clear completion check.",
        ],
        verifier: { kind: "review" },
        evidence_schema: { required: ["requirements_document"] },
        suggested_solver_reward_usdc: "3.000000",
      },
      {
        task_id: "build_page",
        title: "Build the reporting page",
        goal: "Create a phone-friendly page where a person can send a broken-streetlight report.",
        depends_on: ["write_rules"],
        acceptance_criteria: [
          "A person can enter a location and description.",
          "The page confirms that the report was sent.",
          "The page works on a phone-sized screen.",
        ],
        verifier: { kind: "command", command: "run the committed website tests" },
        evidence_schema: { required: ["repository", "commit_sha", "test_result"] },
        suggested_solver_reward_usdc: "14.000000",
      },
      {
        task_id: "test_page",
        title: "Test the main actions",
        goal: "Prove that reporting, confirmation, and phone display work as promised.",
        depends_on: ["write_rules"],
        acceptance_criteria: [
          "The report test passes.",
          "The confirmation test passes.",
          "The phone layout test passes.",
        ],
        verifier: { kind: "test" },
        evidence_schema: { required: ["test_report"] },
        suggested_solver_reward_usdc: "5.000000",
      },
      {
        task_id: "publish_result",
        title: "Publish the finished result",
        goal: "Make the tested page available and link the final proof.",
        depends_on: ["build_page", "test_page"],
        acceptance_criteria: [
          "The live page opens.",
          "The live page matches the tested version.",
        ],
        verifier: { kind: "http" },
        evidence_schema: { required: ["live_url", "release_commit"] },
        suggested_solver_reward_usdc: "3.000000",
      },
    ],
  };

  const form = document.getElementById("objective-compiler-form");
  const graph = document.getElementById("objective-graph");
  const inspector = document.getElementById("task-inspector");
  const status = document.querySelector("[data-compiler-status]");
  const submit = form && form.querySelector("button[type='submit']");
  let currentPlan = previewPlan;
  let selectedTask = previewPlan.tasks[0];

  if (!form || !graph || !inspector || !status || !submit) return;

  function formatUsdc(value) {
    const number = Number(value);
    return Number.isFinite(number) ? number.toFixed(2) : "";
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

  function escapeHtml(value) {
    const element = document.createElement("span");
    element.textContent = String(value == null ? "" : value);
    return element.innerHTML;
  }

  function inspectTask(task) {
    selectedTask = task;
    highlightTask(task);
    const criteria = (task.acceptance_criteria || [])
      .map((item) => `<li>${escapeHtml(item)}</li>`)
      .join("");
    const evidence = ((task.evidence_schema || {}).required || [])
      .map(escapeHtml)
      .join(", ") || "A link or record that shows the result";
    const check = task.verifier || {};
    const checkDetail = check.command || check.endpoint || check.kind || "review";
    inspector.innerHTML = `<strong>${escapeHtml(task.title)}</strong><br><strong>Result:</strong> ${escapeHtml(task.goal)}<br><strong>How it is checked:</strong> ${escapeHtml(checkDetail)}<br><strong>Proof to provide:</strong> ${evidence}<br><strong>Done when:</strong><ol>${criteria}</ol>`;
  }

  function renderGraph(plan, graphState) {
    currentPlan = plan;
    graph.textContent = "";
    const title = document.querySelector("[data-graph-title]");
    const model = document.querySelector("[data-graph-model]");
    if (title) title.textContent = plan.title || "Task plan";
    if (model) model.textContent = `${plan.tasks.length} clear ${plan.tasks.length === 1 ? "task" : "tasks"}`;
    const shell = document.querySelector(".graph-shell");
    if (shell) shell.dataset.graphState = graphState;

    (plan.parallel_layers || [plan.tasks.map((task) => task.task_id)]).forEach((layer, layerIndex) => {
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
          ? `${formatUsdc(task.suggested_solver_reward_usdc)} USDC suggested reward`
          : "Choose a reward below";
        node.innerHTML = `<span class="graph-node-id">Task ${layerIndex + 1}</span><strong>${escapeHtml(task.title)}</strong><span class="graph-node-meta">${escapeHtml(reward)}</span>`;
        node.addEventListener("click", () => inspectTask(task));
        node.addEventListener("mouseenter", () => highlightTask(task));
        node.addEventListener("mouseleave", clearRelationshipHighlight);
        column.append(node);
      });
      graph.append(column);
    });

    selectedTask = plan.tasks[0] || null;
    if (selectedTask) inspectTask(selectedTask);
  }

  async function loadReadiness() {
    const container = document.querySelector(".compiler-readiness");
    const text = document.querySelector("[data-readiness-text]");
    if (!container || !text) return;
    try {
      const response = await fetch(`${API}/v1/cloud-agent/readiness`, { cache: "no-store" });
      if (!response.ok) throw new Error(`HTTP ${response.status}`);
      const readiness = await response.json();
      const ready = readiness.available
        && readiness.protocol === "open_ai_responses"
        && String(readiness.model || "").startsWith("gpt-5.6");
      container.dataset.compilerReadiness = ready ? "ready" : "unavailable";
      text.textContent = ready ? "AI helper ready" : "AI helper unavailable — you can still fill the form yourself";
    } catch (_error) {
      container.dataset.compilerReadiness = "unavailable";
      text.textContent = "AI helper unavailable — you can still fill the form yourself";
    }
  }

  async function compileObjective(event) {
    event.preventDefault();
    submit.disabled = true;
    submit.setAttribute("aria-busy", "true");
    status.textContent = "Turning your goal into clear tasks…";
    const data = new FormData(form);
    const constraints = String(data.get("constraints") || "")
      .split("\n")
      .map((item) => item.trim())
      .filter(Boolean);
    const body = {
      objective: String(data.get("objective") || "").trim(),
      context: "Create clear, measurable tasks that a person or AI agent can complete and prove.",
      constraints,
      max_tasks: Number(data.get("max_tasks") || 5),
      solver_budget_usdc: String(data.get("solver_budget_usdc") || "").trim() || null,
      source_url: "https://github.com/NSPG13/agent-bounties/issues/513",
      idempotency_key: `goal-planner:${crypto.randomUUID()}`,
    };
    try {
      const response = await fetch(`${API}/v1/cloud-agent/objective-plans`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(body),
      });
      const result = await response.json().catch(() => ({}));
      if (!response.ok) throw new Error(result.message || result.error_code || `HTTP ${response.status}`);
      renderGraph(result, "compiled");
      status.textContent = `${result.tasks.length} tasks are ready to review. Choose one, then press “Use this task.”`;
      if (typeof window.agentBountiesTrack === "function") {
        window.agentBountiesTrack("objective_compiled", { task_count: result.tasks.length });
      }
    } catch (error) {
      status.textContent = `${error.message}. Nothing was posted. You can still fill the posting form below.`;
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
    status.textContent = "Example loaded. Edit it or turn it into a new plan.";
  }

  function useSelectedTask() {
    const postForm = document.getElementById("autonomous-post-form");
    const section = document.getElementById("post");
    if (!postForm || !selectedTask || !section) return;

    postForm.elements.draftObjective.value = form.elements.objective.value.trim();
    postForm.elements.title.value = selectedTask.title || "";
    postForm.elements.goal.value = selectedTask.goal || "";
    postForm.elements.acceptance.value = (selectedTask.acceptance_criteria || []).join("\n");
    if (selectedTask.suggested_solver_reward_usdc) {
      postForm.elements.solverReward.value = formatUsdc(selectedTask.suggested_solver_reward_usdc);
    }
    if (selectedTask.evidence_schema && postForm.elements.evidenceSchema) {
      postForm.elements.evidenceSchema.value = JSON.stringify(selectedTask.evidence_schema, null, 2);
    }

    const output = document.getElementById("autonomous-post-output");
    if (output) {
      output.textContent = "Task copied from the plan. Review every field before connecting a wallet.";
      output.dataset.tone = "pending";
    }
    section.scrollIntoView({ behavior: "smooth", block: "start" });
    window.setTimeout(() => postForm.elements.title.focus({ preventScroll: true }), 400);
  }

  form.addEventListener("submit", compileObjective);
  document.querySelector("[data-load-example]")?.addEventListener("click", resetExample);
  document.querySelector("[data-use-selected-task]")?.addEventListener("click", useSelectedTask);

  const params = new URLSearchParams(window.location.search);
  const suppliedGoal = params.get("goal");
  if (suppliedGoal) form.elements.objective.value = suppliedGoal;

  renderGraph(previewPlan, "preview");
  loadReadiness();

  if (window.location.hash === "#post") {
    window.requestAnimationFrame(() => document.getElementById("post")?.scrollIntoView({ block: "start" }));
  }
})();
