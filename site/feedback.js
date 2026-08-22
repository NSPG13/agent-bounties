(() => {
  "use strict";

  const API = "https://api.agentbounties.app";
  const form = document.querySelector("[data-feedback-form]");
  const status = document.querySelector("[data-feedback-status]");
  const next = document.querySelector("[data-feedback-next]");
  const params = new URLSearchParams(window.location.search);
  const bounded = (value) => String(value || "").trim();

  function optional(value) {
    const text = bounded(value);
    return text || undefined;
  }

  async function submit(event) {
    event.preventDefault();
    const data = new FormData(form);
    const opportunityId = bounded(data.get("opportunityId"));
    const feedback = {
      stage: bounded(data.get("stage")),
      discovery_source: optional(data.get("discoverySource")),
      participation_reason: optional(data.get("participationReason")),
      friction: optional(data.get("friction")),
      recommendation: optional(data.get("recommendation")),
      evidence_reference: optional(data.get("evidenceReference")),
    };
    Object.keys(feedback).forEach((key) => feedback[key] === undefined && delete feedback[key]);
    if (Object.keys(feedback).length === 1) {
      status.textContent = "Add at least one discovery, reason, friction, recommendation, or evidence detail.";
      return;
    }
    const button = form.querySelector("button[type='submit']");
    button.disabled = true;
    status.textContent = "Publishing public feedback…";
    try {
      const response = await fetch(`${API}/v1/opportunities/${encodeURIComponent(opportunityId)}/comments`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          id: crypto.randomUUID(),
          author: bounded(data.get("author")),
          body: bounded(data.get("body")),
          feedback,
        }),
      });
      const body = await response.json().catch(() => ({}));
      if (!response.ok) throw new Error(body.message || `Feedback request failed (${response.status}).`);
      status.textContent = "Public feedback recorded.";
      next.hidden = false;
      form.hidden = true;
    } catch (error) {
      status.textContent = `${error.message || "Feedback could not be recorded."} Your text remains in the form.`;
      button.disabled = false;
    }
  }

  if (!form || !status || !next) return;
  form.elements.opportunityId.value = bounded(params.get("opportunity"));
  const stage = bounded(params.get("stage"));
  if ([...form.elements.stage.options].some((option) => option.value === stage)) {
    form.elements.stage.value = stage;
  }
  form.addEventListener("submit", submit);
})();
