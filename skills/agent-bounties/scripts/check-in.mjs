#!/usr/bin/env node

import { readFile } from "node:fs/promises";
import { pathToFileURL } from "node:url";

export const DEFAULT_API_BASE_URL = "https://agent-bounties-api.onrender.com";
export const DEFAULT_PROTOCOL_URL = "https://nspg13.github.io/agent-bounties/protocol.json";

const ADDRESS = /^0x[0-9a-fA-F]{40}$/;

export function normalizeApiBaseUrl(value) {
  const url = new URL(String(value || "").trim());
  if (url.username || url.password) throw new Error("API base URL must not contain credentials");
  const loopback = ["localhost", "127.0.0.1", "::1"].includes(url.hostname);
  if (url.protocol !== "https:" && !(url.protocol === "http:" && loopback)) {
    throw new Error("API base URL must use HTTPS, except for loopback development URLs");
  }
  url.pathname = url.pathname.replace(/\/+$/, "");
  url.search = "";
  url.hash = "";
  return url.toString().replace(/\/$/, "");
}

function normalizePublicUrl(value, label) {
  const url = new URL(String(value || "").trim());
  if (url.protocol !== "https:" || url.username || url.password) {
    throw new Error(`${label} must be a credential-free HTTPS URL`);
  }
  return url.toString();
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
  return [];
}

function integerOf(value) {
  const amount = Number(value);
  return Number.isSafeInteger(amount) && amount >= 0 ? amount : null;
}

function moneyAmount(value) {
  if (value?.currency !== "usdc") return null;
  return integerOf(value.amount);
}

function activeProtocol(protocol) {
  return Boolean(
    protocol
      && protocol.protocol_version === "agent-bounties/autonomous-v1"
      && protocol.status === "active"
      && protocol.network === "base-mainnet"
      && protocol.chain_id === 8453
      && ADDRESS.test(protocol.factory || "")
      && ADDRESS.test(protocol.implementation || "")
      && String(protocol.native_usdc || "").toLowerCase()
        === "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913",
  );
}

export function verifyClaimableItem(item, protocol) {
  if (!activeProtocol(protocol)) return { ok: false, reason: "autonomous_protocol_not_active" };
  if (!item || item.status !== "claimable") {
    return { ok: false, reason: "indexed_status_not_claimable" };
  }
  if (!item.terms_valid || !item.terms?.document?.contract_terms) {
    return { ok: false, reason: "terms_or_contract_commitments_invalid" };
  }
  if (!ADDRESS.test(item.bounty_contract || "") || !ADDRESS.test(item.creator || "")) {
    return { ok: false, reason: "invalid_canonical_identity" };
  }

  const solverReward = integerOf(item.solver_reward);
  const verifierReward = integerOf(item.verifier_reward);
  const claimBond = integerOf(item.claim_bond);
  const target = integerOf(item.target_amount);
  const funded = integerOf(item.funded_amount);
  const timeoutBonus = integerOf(item.timeout_bond_pool);
  if (
    solverReward === null
    || verifierReward === null
    || verifierReward === 0
    || claimBond !== verifierReward
    || target !== solverReward + verifierReward
    || funded !== target
    || timeoutBonus === null
  ) {
    return { ok: false, reason: "funding_or_bond_invariant_failed" };
  }

  const committed = item.terms.document.contract_terms;
  if (
    String(committed.creator_wallet || "").toLowerCase() !== item.creator.toLowerCase()
    || committed.network !== "base-mainnet"
    || String(committed.settlement_token || "").toLowerCase()
      !== String(protocol.native_usdc).toLowerCase()
    || moneyAmount(committed.solver_reward) !== solverReward
    || moneyAmount(committed.verifier_reward) !== verifierReward
    || moneyAmount(committed.claim_bond) !== claimBond
  ) {
    return { ok: false, reason: "published_terms_do_not_match_economics" };
  }

  const events = Array.isArray(item.events) ? item.events : [];
  const requiredKinds = [
    "canonical_bounty_created",
    "canonical_bounty_terms_committed",
    "canonical_bounty_economics_configured",
    "canonical_bounty_verification_configured",
    "bounty_became_claimable",
  ];
  if (!requiredKinds.every((kind) => events.some((event) => event?.kind === kind))) {
    return { ok: false, reason: "canonical_claimability_events_missing" };
  }
  const created = events.find((event) => event?.kind === "canonical_bounty_created");
  if (String(created?.contract_address || "").toLowerCase() !== protocol.factory.toLowerCase()) {
    return { ok: false, reason: "creation_not_emitted_by_active_factory" };
  }
  return { ok: true, reason: "confirmed_canonical_autonomous_bounty" };
}

function normalizedBounty(item, apiBaseUrl) {
  return {
    id: item.bounty_id,
    contract: item.bounty_contract,
    title: item.terms.document.title,
    solver_reward_minor: integerOf(item.solver_reward),
    completion_bonus_minor: integerOf(item.timeout_bond_pool),
    claim_bond_minor: integerOf(item.claim_bond),
    currency: "usdc",
    status: item.status,
    evidence: "confirmed_canonical_autonomous_bounty",
    terms_url: `${apiBaseUrl}/v1/base/autonomous-bounties/terms/${item.terms_hash}`,
    claim_plan_url: `${apiBaseUrl}/v1/base/autonomous-bounties/claim-plan`,
  };
}

function normalizedFundingCandidate(item) {
  const target = integerOf(item.target_amount) || 0;
  const funded = integerOf(item.funded_amount) || 0;
  return {
    id: item.bounty_id,
    contract: item.bounty_contract,
    title: item.terms?.document?.title || null,
    target_minor: target,
    funded_minor: funded,
    remaining_minor: Math.max(0, target - funded),
    currency: "usdc",
    terms_valid: item.terms_valid === true,
  };
}

export async function collectInventory({
  apiBaseUrl,
  protocolUrl = DEFAULT_PROTOCOL_URL,
  fixture = null,
}) {
  const api = normalizeApiBaseUrl(apiBaseUrl || DEFAULT_API_BASE_URL);
  const protocolEndpoint = normalizePublicUrl(protocolUrl, "Protocol URL");
  const [health, protocolResponse, feedResponse, jobsResponse] = await Promise.all([
    fixture ? fixture.health : request(`${api}/health`, false),
    fixture ? fixture.protocol : request(protocolEndpoint, true),
    fixture
      ? fixture.autonomous_feed
      : request(`${api}/v1/base/autonomous-bounties/feed?network=base-mainnet&claimable_only=false`, true),
    fixture
      ? fixture.verification_jobs
      : request(`${api}/v1/base/autonomous-bounties/verification-jobs?network=base-mainnet`, true),
  ]);

  const protocol = protocolResponse?.status === 200 ? protocolResponse.body : null;
  const verified = [];
  const excluded = [];
  const fundingCandidates = [];
  for (const item of itemsFrom(feedResponse?.body)) {
    if (item?.status === "open" && item?.terms_valid) {
      fundingCandidates.push(normalizedFundingCandidate(item));
    }
    if (item?.status !== "claimable") continue;
    const verdict = verifyClaimableItem(item, protocol);
    if (verdict.ok) {
      verified.push(normalizedBounty(item, api));
    } else {
      excluded.push({ id: item?.bounty_id || null, reason: verdict.reason });
    }
  }

  const healthOk = health?.status === 200 && String(health.body).trim() === "ok";
  const warnings = [];
  if (!healthOk) warnings.push("hosted_api_health_not_confirmed");
  if (feedResponse?.status !== 200) warnings.push("autonomous_feed_unavailable");
  if (!activeProtocol(protocol)) warnings.push("autonomous_protocol_not_active");
  if (!verified.length) warnings.push("no_verified_funded_bounty_is_claimable");

  return {
    observed_at: new Date().toISOString(),
    api_base_url: api,
    protocol_url: protocolEndpoint,
    hosted_api_healthy: healthOk,
    health_status: health?.status ?? null,
    protocol_status: protocol?.status ?? null,
    active_factory: activeProtocol(protocol) ? protocol.factory : null,
    verified_claimable_bounties: verified,
    excluded_claimable_candidates: excluded,
    funding_candidates: fundingCandidates,
    live_verification_jobs:
      jobsResponse?.status === 200 ? itemsFrom(jobsResponse.body) : [],
    recommended_action: verified.length ? "claim_verified_bounty" : "post_own_bounty",
    links: {
      post_own_bounty: "https://nspg13.github.io/agent-bounties/post.html",
      fund_bounty: "https://nspg13.github.io/agent-bounties/funding.html",
      repository: "https://github.com/NSPG13/agent-bounties",
      llms_txt: "https://nspg13.github.io/agent-bounties/llms.txt",
    },
    warnings,
    evidence_boundary:
      "Only an active configured factory plus matching terms, economics, funding, and canonical events is earnable inventory. Only confirmed canonical BountySettled proves payout.",
  };
}

function parseArgs(argv) {
  const options = {
    apiBaseUrl: process.env.AGENT_BOUNTIES_API_URL || DEFAULT_API_BASE_URL,
    protocolUrl: process.env.AGENT_BOUNTIES_PROTOCOL_URL || DEFAULT_PROTOCOL_URL,
    fixturePath: null,
  };
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === "--api-base-url") options.apiBaseUrl = argv[++index];
    else if (argument === "--protocol-url") options.protocolUrl = argv[++index];
    else if (argument === "--fixture") options.fixturePath = argv[++index];
    else if (argument === "--help") options.help = true;
    else throw new Error(`unknown argument: ${argument}`);
  }
  return options;
}

async function main() {
  const options = parseArgs(process.argv.slice(2));
  if (options.help) {
    console.log(
      "Usage: node check-in.mjs [--api-base-url https://...] [--protocol-url https://...] [--fixture fixture.json]",
    );
    return;
  }
  const fixture = options.fixturePath
    ? JSON.parse(await readFile(options.fixturePath, "utf8"))
    : null;
  const report = await collectInventory({
    apiBaseUrl: options.apiBaseUrl,
    protocolUrl: options.protocolUrl,
    fixture,
  });
  console.log(JSON.stringify(report, null, 2));
}

if (import.meta.url === pathToFileURL(process.argv[1] || "").href) {
  main().catch((error) => {
    console.error(JSON.stringify({ error: String(error?.message || error) }));
    process.exitCode = 1;
  });
}
