(function () {
  "use strict";

  const API_URL = "https://api.agentbounties.app/v1/analytics/events";
  const VISITOR_KEY = "bountyboard.analytics.visitor.v1";
  const SESSION_KEY = "bountyboard.analytics.session.v1";
  const ATTRIBUTION_KEY = "bountyboard.analytics.attribution.v1";
  const OPT_OUT_KEY = "bountyboard.analytics.disabled.v1";
  const GOOGLE_CONSENT_KEY = "agent-bounties.analytics.google-consent.v1";
  const VISITOR_TTL_MS = 90 * 24 * 60 * 60 * 1000;
  const HOSTS = new Set([
    "agentbounties.app",
    "www.agentbounties.app",
    "bountyboard.global",
    "www.bountyboard.global",
  ]);
  const EVENTS = new Set([
    "page_view",
    "market_view",
    "funded_bounty_click",
    "unfunded_post_started",
    "unfunded_post_completed",
    "funding_started",
    "claim_started",
    "claim_confirmed",
    "canonical_post_started",
    "canonical_post_confirmed",
  ]);

  function storage(kind) {
    try {
      return kind === "session" ? window.sessionStorage : window.localStorage;
    } catch (_error) {
      return null;
    }
  }

  function randomId() {
    if (window.crypto && typeof window.crypto.randomUUID === "function") {
      return window.crypto.randomUUID();
    }
    const bytes = new Uint8Array(16);
    window.crypto.getRandomValues(bytes);
    bytes[6] = (bytes[6] & 15) | 64;
    bytes[8] = (bytes[8] & 63) | 128;
    const hex = Array.from(bytes, (value) => value.toString(16).padStart(2, "0"));
    return `${hex.slice(0, 4).join("")}-${hex.slice(4, 6).join("")}-${hex
      .slice(6, 8)
      .join("")}-${hex.slice(8, 10).join("")}-${hex.slice(10).join("")}`;
  }

  function safeToken(value) {
    const normalized = String(value || "")
      .trim()
      .toLowerCase()
      .replace(/[^a-z0-9._-]+/g, "-")
      .replace(/^-+|-+$/g, "")
      .slice(0, 64);
    return /^[a-z0-9][a-z0-9._-]*$/.test(normalized) ? normalized : null;
  }

  function privacySignalEnabled() {
    const dnt = navigator.doNotTrack || window.doNotTrack || navigator.msDoNotTrack;
    return navigator.globalPrivacyControl === true || dnt === "1" || dnt === "yes";
  }

  function explicitOptOut() {
    const local = storage("local");
    const params = new URLSearchParams(window.location.search);
    if (params.get("analytics") === "off" && local) {
      local.setItem(OPT_OUT_KEY, "true");
    }
    return local ? local.getItem(OPT_OUT_KEY) === "true" : true;
  }

  function enabled() {
    return HOSTS.has(window.location.hostname) && !privacySignalEnabled() && !explicitOptOut();
  }

  function googleMeasurementId() {
    const value = window.agentBountiesAnalyticsConfig?.googleMeasurementId;
    return /^G-[A-Z0-9]+$/.test(value || "") ? value : null;
  }

  function googleConsent() {
    const local = storage("local");
    return local ? local.getItem(GOOGLE_CONSENT_KEY) : null;
  }

  function setGoogleConsent(value) {
    const local = storage("local");
    if (local) local.setItem(GOOGLE_CONSENT_KEY, value ? "granted" : "denied");
  }

  function loadGoogleAnalytics() {
    const measurementId = googleMeasurementId();
    if (!measurementId || privacySignalEnabled() || explicitOptOut()) return false;
    if (document.querySelector(`script[data-google-tag="${measurementId}"]`)) return true;
    window.dataLayer = window.dataLayer || [];
    window.gtag = function () {
      window.dataLayer.push(arguments);
    };
    window.gtag("js", new Date());
    window.gtag("config", measurementId, {
      allow_google_signals: false,
      allow_ad_personalization_signals: false,
    });
    const script = document.createElement("script");
    script.async = true;
    script.dataset.googleTag = measurementId;
    script.src = `https://www.googletagmanager.com/gtag/js?id=${encodeURIComponent(
      measurementId
    )}`;
    document.head.appendChild(script);
    return true;
  }

  function offerGoogleAnalytics() {
    if (!googleMeasurementId() || privacySignalEnabled() || explicitOptOut()) return;
    const consent = googleConsent();
    if (consent === "granted") {
      loadGoogleAnalytics();
      return;
    }
    if (consent === "denied" || document.querySelector("[data-google-analytics-consent]")) return;
    const notice = document.createElement("aside");
    notice.className = "analytics-consent";
    notice.dataset.googleAnalyticsConsent = "";
    notice.setAttribute("aria-label", "Optional analytics choice");
    notice.innerHTML =
      '<p><strong>Help improve Agent Bounties?</strong> Allow Google Analytics for anonymous traffic and page-use measurement. Wallets, bounty evidence, and payment data are not sent.</p>' +
      '<div class="actions"><button class="button primary" type="button" data-google-analytics-allow>Allow</button><button class="button secondary" type="button" data-google-analytics-deny>No thanks</button><a href="privacy.html">Privacy</a></div>';
    document.body.appendChild(notice);
    notice.querySelector("[data-google-analytics-allow]").addEventListener("click", function () {
      setGoogleConsent(true);
      notice.remove();
      loadGoogleAnalytics();
    });
    notice.querySelector("[data-google-analytics-deny]").addEventListener("click", function () {
      setGoogleConsent(false);
      notice.remove();
    });
  }

  function browserId() {
    const local = storage("local");
    if (!local) return null;
    const now = Date.now();
    try {
      const existing = JSON.parse(local.getItem(VISITOR_KEY) || "null");
      if (existing && existing.id && existing.expires_at > now) {
        return existing.id;
      }
    } catch (_error) {
      // Replace corrupt first-party state with a new opaque identifier.
    }
    const next = { id: randomId(), expires_at: now + VISITOR_TTL_MS };
    local.setItem(VISITOR_KEY, JSON.stringify(next));
    return next.id;
  }

  function sessionId() {
    const session = storage("session");
    if (!session) return null;
    let id = session.getItem(SESSION_KEY);
    if (!id) {
      id = randomId();
      session.setItem(SESSION_KEY, id);
    }
    return id;
  }

  function currentAttribution() {
    const local = storage("local");
    if (!local) return { source: "direct", campaign: null, referrer_host: null };
    try {
      const existing = JSON.parse(local.getItem(ATTRIBUTION_KEY) || "null");
      if (existing && existing.expires_at > Date.now()) return existing;
    } catch (_error) {
      // Replace corrupt first-touch state below.
    }

    const params = new URLSearchParams(window.location.search);
    const utmSource = safeToken(params.get("utm_source"));
    const sharedFrom = safeToken(params.get("from"));
    const campaign = safeToken(params.get("utm_campaign"));
    let referrerHost = null;
    try {
      const candidate = document.referrer ? new URL(document.referrer).hostname.toLowerCase() : null;
      if (candidate && !HOSTS.has(candidate) && /^[a-z0-9.-]+$/.test(candidate)) {
        referrerHost = candidate.slice(0, 253);
      }
    } catch (_error) {
      referrerHost = null;
    }
    const attribution = {
      source: utmSource || sharedFrom || safeToken(referrerHost) || "direct",
      campaign,
      referrer_host: referrerHost,
      expires_at: Date.now() + VISITOR_TTL_MS,
    };
    local.setItem(ATTRIBUTION_KEY, JSON.stringify(attribution));
    return attribution;
  }

  function validDetail(value, pattern, maxLength) {
    if (!value) return null;
    const normalized = String(value).trim().slice(0, maxLength);
    return pattern.test(normalized) ? normalized : null;
  }

  function track(eventName, details) {
    if (!enabled() || !EVENTS.has(eventName)) return false;
    const visitorId = browserId();
    const currentSessionId = sessionId();
    if (!visitorId || !currentSessionId) return false;
    const attribution = currentAttribution();
    const detail = details || {};
    const opportunityId = validDetail(detail.opportunity_id, /^[A-Za-z0-9:._-]+$/, 200);
    const bountyContract = validDetail(detail.bounty_contract, /^0x[0-9a-fA-F]{40}$/, 42);
    const event = {
      event_id: randomId(),
      visitor_id: visitorId,
      session_id: currentSessionId,
      event_name: eventName,
      page_path: window.location.pathname.slice(0, 160) || "/",
      source: safeToken(attribution.source) || "direct",
      campaign: safeToken(attribution.campaign),
      referrer_host: attribution.referrer_host,
      opportunity_id: opportunityId,
      bounty_contract: bountyContract ? bountyContract.toLowerCase() : null,
      occurred_at: new Date().toISOString(),
    };
    window.fetch(API_URL, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(event),
      credentials: "omit",
      referrerPolicy: "no-referrer",
      keepalive: true,
    }).catch(function () {
      // Analytics must never block or alter a bounty action.
    });
    if (window.gtag && googleConsent() === "granted" && eventName !== "page_view") {
      window.gtag("event", eventName, { page_path: event.page_path });
    }
    return true;
  }

  function optOut() {
    const local = storage("local");
    const session = storage("session");
    if (local) {
      local.setItem(OPT_OUT_KEY, "true");
      local.setItem(GOOGLE_CONSENT_KEY, "denied");
      local.removeItem(VISITOR_KEY);
      local.removeItem(ATTRIBUTION_KEY);
    }
    const measurementId = googleMeasurementId();
    if (measurementId) window[`ga-disable-${measurementId}`] = true;
    if (session) session.removeItem(SESSION_KEY);
    return status();
  }

  function optIn() {
    const local = storage("local");
    if (local) local.removeItem(OPT_OUT_KEY);
    return status();
  }

  function status() {
    return {
      enabled: enabled(),
      explicit_opt_out: explicitOptOut(),
      privacy_signal: privacySignalEnabled(),
      google_analytics: googleConsent(),
    };
  }

  const analytics = { track, optOut, optIn, status };
  window.agentBountiesAnalytics = analytics;
  window.bountyBoardAnalytics = analytics;

  document.addEventListener("click", function (event) {
    const target = event.target.closest("[data-analytics-event]");
    if (!target) return;
    track(target.dataset.analyticsEvent, {
      opportunity_id: target.dataset.analyticsOpportunityId,
      bounty_contract: target.dataset.analyticsBountyContract,
    });
  });

  document.addEventListener("click", function (event) {
    const optOutControl = event.target.closest("[data-analytics-opt-out]");
    if (!optOutControl) return;
    optOut();
    document.querySelector("[data-google-analytics-consent]")?.remove();
    optOutControl.textContent = "Analytics disabled on this browser";
    optOutControl.setAttribute("aria-pressed", "true");
  });

  offerGoogleAnalytics();
  track("page_view");
})();
