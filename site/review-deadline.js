/* Calendar reminders are convenience only; the canonical deadline is authoritative. */
(function (root, factory) {
  const api = factory();
  if (typeof module === "object" && module.exports) module.exports = api;
  if (root) root.AgentBountiesReviewDeadline = api;
})(typeof window === "undefined" ? globalThis : window, function () {
  "use strict";
  const escape = value => String(value).replace(/\\/g, "\\\\").replace(/[\r\n]+/g, "\\n").replace(/;/g, "\\;").replace(/,/g, "\\,");
  const fold = line => {
    const parts = []; let width = 75;
    while (line.length > width) { parts.push(line.slice(0, width)); line = line.slice(width); width = 74; }
    parts.push(line); return parts.join("\r\n ");
  };
  function calendar({ bounty_contract, round, verification_expires_at, protocol = "autonomous-v1", network = "base-mainnet" }, now = Date.now()) {
    if (!/^0x[0-9a-f]{40}$/i.test(bounty_contract) || !Number.isSafeInteger(round) || round <= 0
      || !Number.isSafeInteger(verification_expires_at) || verification_expires_at * 1000 <= now
      || verification_expires_at > 253402214400) throw new Error("A current canonical review deadline is required.");
    if (!["autonomous-v1", "creator-open-v1"].includes(protocol) || !["base-mainnet", "base-sepolia"].includes(network) || (protocol === "autonomous-v1" && network !== "base-mainnet")) throw new Error("Unsupported review calendar network or protocol.");
    const date = seconds => new Date(seconds * 1000).toISOString().replace(/[-:]/g, "").replace(/\.\d{3}Z$/, "Z");
    const url = protocol === "creator-open-v1" ? `https://agentbounties.app/creator-open.html?network=${network}&bounty=${bounty_contract.toLowerCase()}&entry=${round}` : `https://agentbounties.app/participate.html?bountyContract=${bounty_contract.toLowerCase()}&network=base-mainnet`;
    return ["BEGIN:VCALENDAR", "VERSION:2.0", "PRODID:-//Agent Bounties//Creator Review//EN", "CALSCALE:GREGORIAN", "BEGIN:VEVENT",
      `UID:${protocol === "creator-open-v1" ? `${protocol}-${network}-` : ""}${bounty_contract.toLowerCase()}-${round}-${verification_expires_at}@agentbounties.app`, `DTSTAMP:${date(Math.floor(now / 1000))}`,
      `DTSTART:${date(Math.max(Math.floor(now / 1000), verification_expires_at - 1800))}`, `DTEND:${date(verification_expires_at)}`,
      "SUMMARY:Review your Agent Bounties submission", `DESCRIPTION:${escape(`Confirm your verdict before ${new Date(verification_expires_at * 1000).toISOString()}. Missing the review window does not pay the worker. This reminder is not payment evidence.`)}`,
      `URL:${url}`, "BEGIN:VALARM", "TRIGGER:-PT1H", "ACTION:DISPLAY", "DESCRIPTION:Review your bounty before its deadline", "END:VALARM", "END:VEVENT", "END:VCALENDAR", ""].map(fold).join("\r\n");
  }
  return Object.freeze({ calendar });
});
