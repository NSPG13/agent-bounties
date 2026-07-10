#!/usr/bin/env node

import { readFile } from "node:fs/promises";
import { pathToFileURL } from "node:url";

export const DEFAULT_API_BASE_URL = "https://agent-bounties-api.onrender.com";

export function normalizeApiBaseUrl(value) {
  const url = new URL(String(value || "").trim());
  if (url.username || url.password) {
    throw new Error("API base URL must not contain credentials");
  }
  const loopback = ["localhost", "127.0.0.1", "::1"].includes(url.hostname);
  if (url.protocol !== "https:" && !(url.protocol === "http:" && loopback)) {
    throw new Error("API base URL must use HTTPS, except for loopback development URLs");
  }
  url.pathname = url.pathname.replace(/\/+$/, "");
  url.search = "";
  url.hash = "";
  return url.toString().replace(/\/$/, "");
}

async function request(url, parseJson) {
  try {
    const response = await fetch(url, {
      headers: { accept: parseJson ? "application/json" : "text/plain" },
      signal: AbortSignal.timeout(10_000),
    });
    const text = await response.text();
    let body = text;
    if (parseJson && text) {
      try {
        body = JSON.parse(text);
      } catch {
        return { status: response.status, body: null, error: "invalid_json" };
      }
    }
    return { status: response.status, body, error: null };
  } catch (error) {
    return { status: null, body: null, error: String(error?.message || error) };
  }
}

function itemsFrom(body) {
  if (Array.isArray(body)) return body;
  if (Array.isArray(body?.items)) return body.items;
  if (Array.isArray(body?.bounties)) return body.bounties;
  return [];
}

function amountOf(money) {
  const amount = Number(money?.amount);
  return Number.isSafeInteger(amount) ? amount : null;
}

function hasBaseEvidence(status) {
  const fundedEscrow = (status.escrows || []).some(
    (escrow) =>
      escrow?.rail === "BaseUsdc" &&
      escrow?.status === "Funded" &&
      typeof escrow?.external_reference === "string" &&
      escrow.external_reference.length > 0,
  );
  const indexedCreated = (status.base_escrow_events || []).some(
    (event) => event?.kind === "Created" && event?.status === "Funded",
  );
  return fundedEscrow || indexedCreated;
}

function hasStripeEvidence(status) {
  const appliedContribution = (status.funding_contributions || []).some(
    (contribution) =>
      contribution?.rail === "StripeFiat" && contribution?.status === "Applied",
  );
  const appliedIntent = (status.funding_intents || []).some(
    (intent) => intent?.rail === "StripeFiat" && intent?.status === "Applied",
  );
  return appliedContribution || appliedIntent;
}

export function verifyClaimableStatus(status) {
  const bounty = status?.bounty;
  const summary = status?.funding_summary;
  if (!bounty || bounty.status !== "Claimable") {
    return { ok: false, reason: "scoped_status_not_claimable" };
  }
  if (status.claims?.length) {
    return { ok: false, reason: "already_claimed" };
  }
  const target = amountOf(summary?.target);
  const applied = amountOf(summary?.applied);
  if (!summary?.claimable || target === null || applied === null || applied < target) {
    return { ok: false, reason: "funding_summary_not_claimable" };
  }

  const baseEvidence = hasBaseEvidence(status);
  const stripeEvidence = hasStripeEvidence(status);
  switch (bounty.funding_mode) {
    case "BaseUsdcEscrow":
      return baseEvidence
        ? { ok: true, reason: "indexed_base_funding" }
        : { ok: false, reason: "missing_indexed_base_funding" };
    case "StripeFiatLedger":
      return stripeEvidence
        ? { ok: true, reason: "verified_stripe_funding" }
        : { ok: false, reason: "missing_verified_stripe_funding" };
    case "MixedRails": {
      const requiredRails = (summary.partitions || [])
        .filter((partition) => (amountOf(partition?.target) || 0) > 0)
        .map((partition) => partition.rail);
      const railsSatisfied = requiredRails.every(
        (rail) =>
          (rail === "BaseUsdc" && baseEvidence) ||
          (rail === "StripeFiat" && stripeEvidence),
      );
      return railsSatisfied && requiredRails.length > 0
        ? { ok: true, reason: "reconciled_mixed_funding" }
        : { ok: false, reason: "missing_mixed_rail_evidence" };
    }
    case "Simulated":
      return { ok: false, reason: "simulated_value_is_not_earnable_money" };
    default:
      return { ok: false, reason: "unsupported_funding_mode" };
  }
}

function normalizedBounty(status, apiBaseUrl) {
  const bounty = status.bounty;
  return {
    id: bounty.id,
    title: bounty.title,
    template_slug: bounty.template_slug,
    amount_minor: bounty.amount?.amount ?? null,
    currency: bounty.amount?.currency ?? null,
    funding_mode: bounty.funding_mode,
    status: bounty.status,
    verifier_evidence_required: true,
    status_url: `${apiBaseUrl}/v1/bounties/${bounty.id}`,
    public_url: `${apiBaseUrl}/public/bounties/${bounty.id}`,
  };
}

function normalizedFundingCandidate(item) {
  return {
    id: item.bounty_id || item.id || null,
    title: item.title || null,
    amount_minor: item.funding_target_minor ?? item.amount?.amount ?? null,
    currency: item.currency || item.amount?.currency || null,
    remaining_minor: item.funding_remaining_minor ?? null,
    public_url: item.public_url || null,
  };
}

export async function collectInventory({ apiBaseUrl, fixture = null }) {
  const api = normalizeApiBaseUrl(apiBaseUrl || DEFAULT_API_BASE_URL);
  const healthPromise = fixture
    ? fixture.health
    : request(`${api}/health`, false);
  const readinessPromise = fixture
    ? fixture.readiness
    : request(`${api}/v1/readiness/live-money?network=base-mainnet`, true);
  const claimablePromise = fixture
    ? fixture.claimable_feed
    : request(`${api}/v1/bounties/claimable`, true);
  const fundingPromise = fixture
    ? fixture.funding_feed
    : request(`${api}/v1/bounties/funding-feed`, true);
  const [health, readiness, claimableFeed, fundingFeed] = await Promise.all([
    healthPromise,
    readinessPromise,
    claimablePromise,
    fundingPromise,
  ]);

  const verified = [];
  const excluded = [];
  for (const candidate of itemsFrom(claimableFeed?.body)) {
    const bountyId = candidate?.id || candidate?.bounty_id;
    if (!bountyId) {
      excluded.push({ id: null, reason: "missing_bounty_id" });
      continue;
    }
    const statusResponse = fixture
      ? fixture.statuses?.[bountyId] || { status: 404, body: null, error: null }
      : await request(`${api}/v1/bounties/${encodeURIComponent(bountyId)}`, true);
    if (statusResponse?.status !== 200 || !statusResponse.body) {
      excluded.push({ id: bountyId, reason: "scoped_status_unavailable" });
      continue;
    }
    const verdict = verifyClaimableStatus(statusResponse.body);
    if (verdict.ok) {
      verified.push({
        ...normalizedBounty(statusResponse.body, api),
        evidence: verdict.reason,
      });
    } else {
      excluded.push({ id: bountyId, reason: verdict.reason });
    }
  }

  const healthOk = health?.status === 200 && String(health.body).trim() === "ok";
  const warnings = [];
  if (!healthOk) warnings.push("hosted_api_health_not_confirmed");
  if (claimableFeed?.status !== 200) warnings.push("claimable_feed_unavailable");
  if (!verified.length) warnings.push("no_verified_funded_bounty_is_claimable");

  return {
    observed_at: new Date().toISOString(),
    api_base_url: api,
    hosted_api_healthy: healthOk,
    health_status: health?.status ?? null,
    readiness_status: readiness?.status ?? null,
    live_money_ready: readiness?.body?.live_money_ready === true,
    verified_claimable_bounties: verified,
    excluded_claimable_candidates: excluded,
    funding_candidates: itemsFrom(fundingFeed?.body).map(normalizedFundingCandidate),
    recommended_action: verified.length ? "claim_verified_bounty" : "post_own_bounty",
    links: {
      post_own_bounty: "https://nspg13.github.io/agent-bounties/post.html",
      fund_bounty: "https://nspg13.github.io/agent-bounties/funding.html",
      repository: "https://github.com/NSPG13/agent-bounties",
      llms_txt: "https://nspg13.github.io/agent-bounties/llms.txt",
    },
    warnings,
    evidence_boundary:
      "Only scoped claimable status plus reconciled real-rail funding evidence is earnable inventory. Verification is not payout; paid requires reconciled settlement evidence.",
  };
}

function parseArgs(argv) {
  const options = {
    apiBaseUrl: process.env.AGENT_BOUNTIES_API_URL || DEFAULT_API_BASE_URL,
    fixturePath: null,
  };
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === "--api-base-url") options.apiBaseUrl = argv[++index];
    else if (argument === "--fixture") options.fixturePath = argv[++index];
    else if (argument === "--help") options.help = true;
    else throw new Error(`unknown argument: ${argument}`);
  }
  if (!options.apiBaseUrl) throw new Error("--api-base-url requires a value");
  return options;
}

async function main() {
  const options = parseArgs(process.argv.slice(2));
  if (options.help) {
    console.log(
      "Usage: node check-in.mjs [--api-base-url https://...] [--fixture fixture.json]",
    );
    return;
  }
  const fixture = options.fixturePath
    ? JSON.parse(await readFile(options.fixturePath, "utf8"))
    : null;
  const report = await collectInventory({ apiBaseUrl: options.apiBaseUrl, fixture });
  console.log(JSON.stringify(report, null, 2));
}

if (import.meta.url === pathToFileURL(process.argv[1] || "").href) {
  main().catch((error) => {
    console.error(JSON.stringify({ error: String(error?.message || error) }));
    process.exitCode = 1;
  });
}
