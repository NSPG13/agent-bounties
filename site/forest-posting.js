/* Presentation adapter. Existing composer/session handlers own all decisions. */
(() => {
  const form = document.querySelector("#bounty-composer-form");
  const goal = document.querySelector("#bounty-composer-input");
  if (!form || !goal) return;
  const preview = document.querySelector("#bounty-preview");
  const placeholder = document.querySelector("[data-review-placeholder]");
  const options = document.querySelector("[data-ai-options]");
  const approve = document.querySelector("[data-approve-card]");
  const funding = document.querySelector("[data-open-funding]");
  const storageKey = "agent-bounties.home-task";
  const scroll = element => element?.scrollIntoView({ block: "start", behavior: matchMedia("(prefers-reduced-motion: reduce)").matches ? "instant" : "smooth" });
  function stage(value) {
    document.querySelectorAll("[data-stage-target]").forEach(button => {
      if (button.dataset.stageTarget === value) button.setAttribute("aria-current", "step");
      else button.removeAttribute("aria-current");
    });
  }
  function sync() {
    const ready = preview && !preview.hidden;
    placeholder.hidden = ready;
    const fundStep = document.querySelector('[data-stage-target="fund"]');
    fundStep.disabled = !ready || funding.disabled || approve.dataset.approved !== "true";
    stage(ready ? (approve.dataset.approved === "true" ? "fund" : "review") : "details");
  }
  function useTask(value) {
    // Starting a journey emits a synchronous restore. Finish that before setting
    // the incoming text, so the normal input handler saves the intended value.
    const client = window.AgentBountiesWorkflow.createClient(window);
    if (!client.load()) client.start({ role: "post" });
    goal.value = String(value).slice(0, goal.maxLength);
    goal.dispatchEvent(new Event("input", { bubbles: true }));
    try { sessionStorage.removeItem(storageKey); } catch (_) { /* Presentation only. */ }
    scroll(form); goal.focus({ preventScroll: true });
  }
  let incoming = "";
  try { incoming = sessionStorage.getItem(storageKey) || ""; } catch (_) { /* Query fallback works without storage. */ }
  const url = new URL(location.href);
  if (!incoming) incoming = url.searchParams.get("task") || "";
  if (url.searchParams.has("task")) { url.searchParams.delete("task"); history.replaceState(null, "", url); }
  function offerTask(task) {
    const current = window.AgentBountiesWorkflow.createClient(window).load();
    const saved = current?.brief?.goal || current?.draft?.goal || current?.goal || "";
    if (!task.trim()) return;
    if (!saved || saved === task.trim()) { useTask(task); return; }
    document.querySelector("[data-incoming-task]")?.remove();
    const notice = document.createElement("section"); notice.className = "ab-review-placeholder"; notice.dataset.incomingTask = "true";
    const label = document.createElement("p"); label.textContent = "You have a saved bounty brief. Keep it, or replace its task description with this new idea. Replacing it requires a fresh proposal and approval.";
    const text = document.createElement("p"); text.textContent = task.slice(0, 4000);
    const use = document.createElement("button"); use.type = "button"; use.className = "ab-button ab-button--primary"; use.textContent = "Use this new idea";
    const keep = document.createElement("button"); keep.type = "button"; keep.className = "ab-button"; keep.textContent = "Keep saved brief";
    use.addEventListener("click", () => { useTask(task); notice.remove(); });
    keep.addEventListener("click", () => { try { sessionStorage.removeItem(storageKey); } catch (_) {} notice.remove(); scroll(form); });
    notice.append(label, text, use, keep); form.before(notice);
  }
  offerTask(incoming);
  document.querySelectorAll("[data-post-example]").forEach(button => button.addEventListener("click", () => offerTask(button.dataset.postExample)));
  document.querySelector("[data-open-connected-ai]").addEventListener("click", () => {
    const journey = window.AgentBountiesWorkflow.createClient(window).load();
    window.AgentBountyAI.show(goal.value.trim(), { draft: journey?.draft, brief: journey?.brief });
    options.open = true; scroll(options);
  });
  document.querySelector("[data-form-jump]").addEventListener("click", () => { scroll(form); goal.focus({ preventScroll: true }); });
  document.querySelectorAll("[data-stage-target]").forEach(button => button.addEventListener("click", () => {
    const target = button.dataset.stageTarget;
    if (target === "details") { scroll(form); goal.focus({ preventScroll: true }); }
    if (target === "review") scroll(preview.hidden ? placeholder : preview);
    if (target === "fund" && !button.disabled) funding.click();
    stage(target);
  }));
  new MutationObserver(sync).observe(preview, { attributes: true, attributeFilter: ["hidden"] });
  new MutationObserver(sync).observe(approve, { attributes: true, attributeFilter: ["data-approved"] });
  new MutationObserver(sync).observe(funding, { attributes: true, attributeFilter: ["disabled"] });
  window.addEventListener("agent-bounties:journey", sync);
  sync();
})();
