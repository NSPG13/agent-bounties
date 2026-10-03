(function (root, factory) {
  const api = factory();
  if (typeof module === "object" && module.exports) module.exports = api;
  if (root) root.AgentBountiesPostingBrief = api;
})(typeof window !== "undefined" ? window : globalThis, function () {
  "use strict";
  const MINUTE = 60000;
  function offsetText(minutes) {
    return `${minutes < 0 ? "-" : "+"}${String(Math.floor(Math.abs(minutes) / 60)).padStart(2, "0")}:${String(Math.abs(minutes) % 60).padStart(2, "0")}`;
  }
  function fixedOffset(zone) {
    const match = /^([+-])(\d{2}):(\d{2})$/.exec(zone);
    if (!match) return null;
    const hours = Number(match[2]), minutes = Number(match[3]);
    if (hours > 14 || minutes > 59 || (hours === 14 && minutes)) throw new Error("Use a UTC offset between -14:00 and +14:00.");
    return (match[1] === "-" ? -1 : 1) * (hours * 60 + minutes);
  }
  function formatter(zone) {
    if (!zone || (!zone.includes("/") && zone !== "UTC")) throw new Error("Choose a location such as America/Mexico_City or an exact offset such as -06:00. Abbreviations such as CST are ambiguous.");
    try {
      return new Intl.DateTimeFormat("en-CA", { timeZone: zone, year: "numeric", month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit", hourCycle: "h23" });
    } catch (_) { throw new Error("Choose a valid location time zone or an exact UTC offset such as -06:00."); }
  }
  function wallTime(timestamp, zone) {
    const offset = fixedOffset(zone);
    if (offset !== null) return new Date(timestamp + offset * MINUTE).toISOString().slice(0, 16);
    const parts = Object.fromEntries(formatter(zone).formatToParts(timestamp).map(part => [part.type, part.value]));
    return `${parts.year}-${parts.month}-${parts.day}T${parts.hour}:${parts.minute}`;
  }
  function resolveDeadline(local, zone) {
    if (!local) return { iso: null, error: null };
    try {
      if (!/^\d{4}-\d\d-\d\dT\d\d:\d\d$/.test(local)) throw new Error("Enter a complete calendar date and time.");
      const naive = Date.parse(local + ":00Z");
      if (!Number.isFinite(naive) || new Date(naive).toISOString().slice(0, 16) !== local) throw new Error("Enter a valid calendar date and time.");
      const fixed = fixedOffset(zone);
      if (fixed !== null) return { iso: local + ":00" + offsetText(fixed), error: null };
      formatter(zone);
      // Check both sides of clock changes. Round-trips reject nonexistent
      // times and expose repeated times, rather than silently choosing one.
      const offsets = new Set();
      for (const hours of [-36, -12, 0, 12, 36]) {
        const timestamp = naive + hours * 3600000;
        offsets.add((Date.parse(wallTime(timestamp, zone) + ":00Z") - timestamp) / MINUTE);
      }
      const matches = [...offsets].filter(offset => wallTime(naive - offset * MINUTE, zone) === local);
      if (!matches.length) throw new Error("That local time does not exist because the clocks change. Choose another time.");
      if (matches.length > 1) throw new Error("That local time occurs twice because the clocks change. Enter its exact UTC offset to choose which occurrence.");
      return { iso: local + ":00" + offsetText(matches[0]), error: null };
    } catch (error) { return { iso: null, error: error.message }; }
  }
  function cents(value) {
    const text = String(value ?? "").trim();
    if (!/^\d+(?:\.\d{1,2})?$/.test(text)) return null;
    const [whole, fraction = ""] = text.split(".");
    const result = Number(whole) * 100 + Number(fraction.padEnd(2, "0"));
    return Number.isSafeInteger(result) && result > 0 ? result : null;
  }
  function proposedSplit(value) {
    const total = cents(value);
    if (total === null || total < 2) return null;
    const reserve = Math.max(1, Math.round(total / 10));
    return { solver: ((total - reserve) / 100).toFixed(2), reserve: (reserve / 100).toFixed(2), total: (total / 100).toFixed(2) };
  }
  function estimateBudget(hours, hourlyRate, reviewReserve) {
    // Hundredths of an hour times cents/hour; round the work budget up to a cent.
    const time = cents(hours), rate = cents(hourlyRate), reserve = cents(reviewReserve);
    if (time === null || rate === null || reserve === null || time > 100000 || rate > 1000000 || reserve > 100000000) return null;
    const work = Math.ceil(time * rate / 100);
    return { solver: (work / 100).toFixed(2), reserve: (reserve / 100).toFixed(2), total: ((work + reserve) / 100).toFixed(2) };
  }
  function countdown(iso, now = Date.now()) {
    const remaining = Date.parse(iso) - now;
    if (!Number.isFinite(remaining)) return "";
    if (remaining <= 0) return "Deadline passed. Update it before posting.";
    const minutes = Math.ceil(remaining / MINUTE);
    if (minutes < 60) return `${minutes} minute${minutes === 1 ? "" : "s"} remaining`;
    const hours = Math.floor(minutes / 60), rest = minutes % 60;
    if (hours < 24) return `${hours}h ${rest}m remaining`;
    return `${Math.floor(hours / 24)}d ${hours % 24}h remaining`;
  }
  function warnings({ goal = "", budget = "", deadline = null, draft = null }, now = Date.now()) {
    const output = [];
    const remaining = Date.parse(deadline) - now;
    if (remaining <= 0) output.push("The delivery deadline has passed. Choose a new calendar deadline before publishing.");
    else if (remaining < 2 * 3600000) output.push("Less than two hours remain, including wallet setup and solver work. Consider extending the deadline.");
    else if (remaining < 86400000) output.push("Less than a day remains. Wallet setup uses part of the same delivery window.");
    const total = cents(budget);
    const solver = draft ? cents(draft.solver_reward_usdc) : null;
    if (total !== null && total <= 500 && (/\b(video|animation|animate|3d|simulation)\b/i.test(goal) || draft?.acceptance_criteria?.length >= 5)) {
      output.push(`This ${((solver ?? Math.round(total * .9)) / 100).toFixed(2)} USDC solver reward accompanies substantial creative work or several checks. Consider a smaller first deliverable or a higher budget; this is scope guidance, not a market price estimate.`);
    }
    return output;
  }
  function manualDraft(brief, now = Date.now()) {
    const title = String(brief.title || "").trim(), goal = String(brief.goal || "").trim();
    const criteria = String(brief.criteria || "").split(/\r?\n/).map(s => s.trim()).filter(Boolean);
    if (!title || title.length > 200) throw new Error("Enter a title of up to 200 characters.");
    if (!goal || goal.length > 4000) throw new Error("Describe the work in up to 4,000 characters.");
    if (!criteria.length || criteria.length > 20 || criteria.some(s => s.length > 1000)) throw new Error("Add 1–20 acceptance criteria, one per line, with up to 1,000 characters each.");
    const total = cents(brief.budget_usdc), reserve = cents(brief.review_reward_usdc);
    if (total === null || total > 900000000000 || reserve === null || total - reserve < 200) throw new Error("The total must cover at least 2 USDC for the worker plus a review reward of at least 0.01 USDC.");
    const remaining = Date.parse(brief.deadline_at) - now;
    if (!Number.isFinite(remaining) || remaining <= 0 || remaining > 30 * 86400000) throw new Error("Choose a future delivery deadline within 30 days. Use a smaller milestone for longer work.");
    if (brief.review_consent !== true) throw new Error("Confirm that you will review submissions yourself.");
    return { title, goal, acceptance_criteria: criteria, solver_reward_usdc: ((total - reserve) / 100).toFixed(2), verifier_reward_usdc: (reserve / 100).toFixed(2), task_window_days: Math.max(1, Math.ceil(remaining / 86400000)), delivery_deadline: brief.deadline_at, review_mode: "creator", discovery_source: "Platform form (no AI)" };
  }
  return Object.freeze({ manualDraft, resolveDeadline, wallTime, proposedSplit, estimateBudget, countdown, warnings, cents });
});
