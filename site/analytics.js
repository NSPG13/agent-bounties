(function () {
  "use strict";

  const API_URL = "https://api.agentbounties.app/v1/analytics/events";
  const VISITOR_KEY = "bountyboard.analytics.visitor.v1";
  const SESSION_KEY = "bountyboard.analytics.session.v1";
  const ATTRIBUTION_KEY = "bountyboard.analytics.attribution.v1";
  const CURRENT_ATTRIBUTION_KEY = "bountyboard.analytics.current-attribution.v2";
  const EXPOSURE_KEY = "bountyboard.analytics.exposures.v2";
  const OPT_OUT_KEY = "bountyboard.analytics.disabled.v1";
  const GOOGLE_CONSENT_KEY = "agent-bounties.analytics.google-consent.v1";
  const VISITOR_TTL_MS = 90 * 24 * 60 * 60 * 1000;
  const EXPOSURE_DURATION_MS = 1000;
  const MAX_EXPOSURE_KEYS = 512;
  const HOSTS = new Set([
    "agentbounties.app",
    "www.agentbounties.app",
    "bountyboard.global",
    "www.bountyboard.global",
  ]);
  const EVENTS = new Set([
    "page_view",
    "market_view",
    "opportunity_exposed",
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

  function storeValue(target, key, value) {
    if (!target) return false;
    try {
      target.setItem(key, value);
      return true;
    } catch (_error) {
      return false;
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
      storeValue(local, OPT_OUT_KEY, "true");
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
    storeValue(local, GOOGLE_CONSENT_KEY, value ? "granted" : "denied");
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
    return storeValue(local, VISITOR_KEY, JSON.stringify(next)) ? next.id : null;
  }

  function sessionId() {
    const session = storage("session");
    if (!session) return null;
    let id = session.getItem(SESSION_KEY);
    if (!id) {
      id = randomId();
      if (!storeValue(session, SESSION_KEY, id)) return null;
    }
    return id;
  }

  function observedAttribution() {
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
    return {
      source: utmSource || sharedFrom || safeToken(referrerHost) || "direct",
      campaign,
      referrer_host: referrerHost,
      has_touch: Boolean(utmSource || sharedFrom || campaign || referrerHost),
    };
  }

  function firstAttribution() {
    const local = storage("local");
    if (!local) return { source: "direct", campaign: null, referrer_host: null };
    try {
      const existing = JSON.parse(local.getItem(ATTRIBUTION_KEY) || "null");
      if (existing && existing.expires_at > Date.now()) return existing;
    } catch (_error) {
      // Replace corrupt first-touch state below.
    }

    const observed = observedAttribution();
    const attribution = {
      source: observed.source,
      campaign: observed.campaign,
      referrer_host: observed.referrer_host,
      expires_at: Date.now() + VISITOR_TTL_MS,
    };
    storeValue(local, ATTRIBUTION_KEY, JSON.stringify(attribution));
    return attribution;
  }

  function currentAttribution() {
    const session = storage("session");
    const observed = observedAttribution();
    if (!session) {
      return {
        source: observed.source,
        campaign: observed.campaign,
        referrer_host: observed.referrer_host,
      };
    }
    if (!observed.has_touch) {
      try {
        const existing = JSON.parse(session.getItem(CURRENT_ATTRIBUTION_KEY) || "null");
        if (existing && safeToken(existing.source)) return existing;
      } catch (_error) {
        // Replace corrupt session-touch state below.
      }
    }
    const attribution = {
      source: observed.source,
      campaign: observed.campaign,
      referrer_host: observed.referrer_host,
    };
    storeValue(session, CURRENT_ATTRIBUTION_KEY, JSON.stringify(attribution));
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
    const attribution = firstAttribution();
    const current = currentAttribution();
    const detail = details || {};
    const opportunityId = validDetail(detail.opportunity_id, /^[A-Za-z0-9:._-]+$/, 200);
    const bountyContract = validDetail(detail.bounty_contract, /^0x[0-9a-fA-F]{40}$/, 42);
    if (eventName === "opportunity_exposed" && !opportunityId && !bountyContract) return false;
    // Do not send site_host: the API derives its allowlisted value from Origin.
    const event = {
      event_id: randomId(),
      visitor_id: visitorId,
      session_id: currentSessionId,
      event_name: eventName,
      page_path: window.location.pathname.slice(0, 160) || "/",
      source: safeToken(attribution.source) || "direct",
      campaign: safeToken(attribution.campaign),
      referrer_host: validDetail(attribution.referrer_host, /^[a-z0-9.-]+$/, 253),
      opportunity_id: opportunityId,
      bounty_contract: bountyContract ? bountyContract.toLowerCase() : null,
      placement: safeToken(detail.placement),
      variant: safeToken(detail.variant),
      opportunity_class: safeToken(detail.opportunity_class),
      current_source: safeToken(current.source) || "direct",
      current_campaign: safeToken(current.campaign),
      current_referrer_host: validDetail(
        current.referrer_host,
        /^[a-z0-9.-]+$/,
        253
      ),
      occurred_at: new Date().toISOString(),
    };
    try {
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
    } catch (_error) {
      // Analytics must never block or alter a bounty action.
    }
    if (window.gtag && googleConsent() === "granted" && eventName !== "page_view") {
      window.gtag("event", eventName, { page_path: event.page_path });
    }
    return true;
  }

  function exposureDetails(element) {
    return {
      opportunity_id: element.dataset.analyticsOpportunityId,
      bounty_contract: element.dataset.analyticsBountyContract,
      placement: element.dataset.analyticsPlacement,
      variant: element.dataset.analyticsVariant,
      opportunity_class: element.dataset.analyticsOpportunityClass,
    };
  }

  function exposureIdentity(element) {
    const detail = exposureDetails(element);
    const opportunityId = validDetail(detail.opportunity_id, /^[A-Za-z0-9:._-]+$/, 200);
    const bountyContract = validDetail(detail.bounty_contract, /^0x[0-9a-fA-F]{40}$/, 42);
    if (!opportunityId && !bountyContract) return null;
    return JSON.stringify([
      window.location.pathname.slice(0, 160) || "/",
      opportunityId,
      bountyContract ? bountyContract.toLowerCase() : null,
      safeToken(detail.placement),
      safeToken(detail.variant),
      safeToken(detail.opportunity_class),
    ]);
  }

  function storedExposureKeys() {
    const session = storage("session");
    if (!session) return [];
    try {
      const values = JSON.parse(session.getItem(EXPOSURE_KEY) || "[]");
      return Array.isArray(values)
        ? values.filter((value) => typeof value === "string").slice(-MAX_EXPOSURE_KEYS)
        : [];
    } catch (_error) {
      return [];
    }
  }

  const exposureKeys = new Set(storedExposureKeys());
  const exposureStates = new Map();
  let exposureObserver = null;

  function persistExposureKeys() {
    const session = storage("session");
    if (!session) return;
    const values = Array.from(exposureKeys).slice(-MAX_EXPOSURE_KEYS);
    exposureKeys.clear();
    values.forEach((value) => exposureKeys.add(value));
    try {
      session.setItem(EXPOSURE_KEY, JSON.stringify(values));
    } catch (_error) {
      // Storage pressure must not affect the product or analytics delivery.
    }
  }

  function visiblyRendered(element) {
    if (!element.isConnected || document.visibilityState !== "visible") return false;
    const style = window.getComputedStyle(element);
    const opacity = Number.parseFloat(style.opacity);
    return (
      style.display !== "none" &&
      style.visibility !== "hidden" &&
      (!Number.isFinite(opacity) || opacity > 0)
    );
  }

  function cancelExposureTimer(state) {
    if (state?.timer) window.clearTimeout(state.timer);
    if (state) state.timer = null;
  }

  function scheduleExposure(element) {
    const state = exposureStates.get(element);
    if (!state || !state.intersecting || state.timer || !enabled()) return;
    const key = exposureIdentity(element);
    if (!key || exposureKeys.has(key) || !visiblyRendered(element)) return;
    state.timer = window.setTimeout(function () {
      state.timer = null;
      if (!state.intersecting || !visiblyRendered(element)) return;
      const currentKey = exposureIdentity(element);
      if (!currentKey || exposureKeys.has(currentKey)) return;
      if (track("opportunity_exposed", exposureDetails(element))) {
        exposureKeys.add(currentKey);
        persistExposureKeys();
        exposureObserver?.unobserve(element);
        exposureStates.delete(element);
      }
    }, EXPOSURE_DURATION_MS);
  }

  function registerExposure(element) {
    if (
      !exposureObserver ||
      exposureStates.has(element) ||
      element.dataset.analyticsExposure !== "opportunity_exposed" ||
      !exposureIdentity(element)
    ) {
      return;
    }
    exposureStates.set(element, { intersecting: false, timer: null });
    exposureObserver.observe(element);
  }

  function registerExposures(root) {
    if (!(root instanceof Element)) return;
    if (root.matches('[data-analytics-exposure="opportunity_exposed"]')) {
      registerExposure(root);
    }
    root
      .querySelectorAll('[data-analytics-exposure="opportunity_exposed"]')
      .forEach(registerExposure);
  }

  function unregisterExposures(root) {
    if (!(root instanceof Element)) return;
    const candidates = [];
    if (exposureStates.has(root)) candidates.push(root);
    root
      .querySelectorAll('[data-analytics-exposure="opportunity_exposed"]')
      .forEach((element) => candidates.push(element));
    candidates.forEach(function (element) {
      const state = exposureStates.get(element);
      cancelExposureTimer(state);
      exposureObserver?.unobserve(element);
      exposureStates.delete(element);
    });
  }

  function refreshExposureTimers() {
    exposureStates.forEach(function (state, element) {
      cancelExposureTimer(state);
      if (state.intersecting) scheduleExposure(element);
    });
  }

  function startExposureTracking() {
    if (!document.body || !("IntersectionObserver" in window)) return;
    exposureObserver = new IntersectionObserver(
      function (entries) {
        entries.forEach(function (entry) {
          const state = exposureStates.get(entry.target);
          if (!state) return;
          state.intersecting = entry.isIntersecting && entry.intersectionRatio >= 0.5;
          cancelExposureTimer(state);
          if (state.intersecting) scheduleExposure(entry.target);
        });
      },
      { threshold: [0, 0.5, 1] }
    );
    registerExposures(document.body);
    const mutations = new MutationObserver(function (records) {
      records.forEach(function (record) {
        if (record.type === "attributes") {
          unregisterExposures(record.target);
          registerExposure(record.target);
          return;
        }
        record.removedNodes.forEach(unregisterExposures);
        record.addedNodes.forEach(registerExposures);
      });
    });
    mutations.observe(document.body, {
      childList: true,
      subtree: true,
      attributes: true,
      attributeFilter: [
        "data-analytics-exposure",
        "data-analytics-opportunity-id",
        "data-analytics-bounty-contract",
        "data-analytics-placement",
        "data-analytics-variant",
        "data-analytics-opportunity-class",
      ],
    });
    document.addEventListener("visibilitychange", refreshExposureTimers);
  }

  function optOut() {
    const local = storage("local");
    const session = storage("session");
    if (local) {
      storeValue(local, OPT_OUT_KEY, "true");
      storeValue(local, GOOGLE_CONSENT_KEY, "denied");
      local.removeItem(VISITOR_KEY);
      local.removeItem(ATTRIBUTION_KEY);
    }
    const measurementId = googleMeasurementId();
    if (measurementId) window[`ga-disable-${measurementId}`] = true;
    if (session) {
      session.removeItem(SESSION_KEY);
      session.removeItem(CURRENT_ATTRIBUTION_KEY);
      session.removeItem(EXPOSURE_KEY);
    }
    return status();
  }

  function optIn() {
    const local = storage("local");
    if (local) local.removeItem(OPT_OUT_KEY);
    refreshExposureTimers();
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
      placement: target.dataset.analyticsPlacement,
      variant: target.dataset.analyticsVariant,
      opportunity_class: target.dataset.analyticsOpportunityClass,
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
  if (document.body) {
    startExposureTracking();
  } else {
    document.addEventListener("DOMContentLoaded", startExposureTracking, { once: true });
  }
  track("page_view");
})();
