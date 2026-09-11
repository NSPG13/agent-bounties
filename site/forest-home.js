/* Landing-page interactions. No model calls, account writes, or wallet actions. */
(() => {
  const form = document.querySelector("[data-home-task]");
  const input = document.querySelector("#home-task");
  if (!form || !input) return;
  const storageKey = "agent-bounties.home-task";
  try { input.value = sessionStorage.getItem(storageKey) || ""; } catch (_) { /* Input remains usable. */ }
  input.addEventListener("input", () => {
    try { sessionStorage.setItem(storageKey, input.value); } catch (_) { /* Submission reports unavailable storage. */ }
  });
  form.addEventListener("submit", event => {
    event.preventDefault();
    document.querySelector("#post-a-bounty")?.click();
  });
  document.querySelectorAll("[data-task-example]").forEach(button => button.addEventListener("click", () => {
    input.value = button.dataset.taskExample;
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.scrollIntoView({ block: "center", behavior: matchMedia("(prefers-reduced-motion: reduce)").matches ? "instant" : "smooth" });
    input.focus({ preventScroll: true });
  }));
  // A cycling placeholder never replaces user input and stops during editing.
  const examples = Array.from(new Set(Array.from(document.querySelectorAll("[data-task-example]"), button => button.dataset.taskExample)));
  let example = 0;
  const reduced = matchMedia("(prefers-reduced-motion: reduce)");
  const outcome = document.querySelector("[data-outcome-word]");
  const outcomes = ["Faster", "Cheaper", "Better"];
  let outcomeIndex = 0, outcomeAnimation;
  window.setInterval(() => {
    if (!outcome || document.hidden || reduced.matches || outcome.getBoundingClientRect().bottom < 0) return;
    outcomeIndex = (outcomeIndex + 1) % outcomes.length;
    outcome.textContent = outcomes[outcomeIndex];
    outcomeAnimation?.cancel();
    outcomeAnimation = outcome.animate?.([
      { opacity: 0, transform: "translateY(.3em)" },
      { opacity: 1, transform: "translateY(0)" },
    ], { duration: 420, easing: "cubic-bezier(.2,.8,.2,1)" });
  }, 2800);
  reduced.addEventListener("change", () => { if (reduced.matches) outcomeAnimation?.cancel(); });
  if (!reduced.matches && "IntersectionObserver" in window) {
    const reveal = new IntersectionObserver(entries => entries.forEach(entry => {
      if (!entry.isIntersecting) return;
      entry.target.dataset.enter = "visible";
      reveal.unobserve(entry.target);
    }), { threshold: .12 });
    document.querySelectorAll(".ab-process-step").forEach(step => { step.dataset.enter = "waiting"; reveal.observe(step); });
    reduced.addEventListener("change", () => {
      if (!reduced.matches) return;
      reveal.disconnect();
      document.querySelectorAll("[data-enter]").forEach(step => { step.dataset.enter = "visible"; });
    });
  }
  window.setInterval(() => {
    if (!examples.length || document.hidden || reduced.matches || input.value || document.activeElement === input) return;
    example = (example + 1) % examples.length;
    input.placeholder = examples[example];
  }, 5000);
  const deck = document.querySelector("[data-example-deck]");
  const cards = Array.from(deck.querySelectorAll(".ab-example-card"));
  let active = 0, pointerX = null;
  function show(direction) {
    active = (active + direction + cards.length) % cards.length;
    cards.forEach((card, index) => { card.dataset.slot = String((index - active + cards.length) % cards.length); card.setAttribute("aria-hidden", String(index !== active)); });
    document.querySelector("[data-deck-status]").textContent = `Example ${active + 1} of ${cards.length}`;
  }
  document.querySelector("[data-deck-prev]").addEventListener("click", () => show(-1));
  document.querySelector("[data-deck-next]").addEventListener("click", () => show(1));
  deck.addEventListener("keydown", event => {
    if (!["ArrowLeft", "ArrowRight"].includes(event.key)) return;
    event.preventDefault(); show(event.key === "ArrowRight" ? 1 : -1);
  });
  deck.addEventListener("pointerdown", event => { pointerX = event.clientX; });
  deck.addEventListener("pointerup", event => { if (pointerX !== null && Math.abs(event.clientX - pointerX) > 45) show(event.clientX < pointerX ? 1 : -1); pointerX = null; });
  deck.addEventListener("pointercancel", () => { pointerX = null; });
  show(0);
})();
