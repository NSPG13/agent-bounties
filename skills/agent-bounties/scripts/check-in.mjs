#!/usr/bin/env node

import { readFile } from "node:fs/promises";
import { pathToFileURL } from "node:url";

export const DEFAULT_API_BASE_URL = "https://agent-bounties-api.onrender.com";
export const DEFAULT_PROTOCOL_URL = "https://nspg13.github.io/agent-bounties/protocol.json";
export const DEFAULT_BASE_RPC_URL = "https://mainnet.base.org";

const ADDRESS = /^0x[0-9a-fA-F]{40}$/;
const HASH = /^0x[0-9a-fA-F]{64}$/;
const EMPTY_CODE_HASH = "0xc5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470";
const TERMS_SOURCE_COMMIT = "eea06e72dbdc1f647ad4aa3dac2a9f5ed93f67c8";
export const STANDING_META_BOUNTY = Object.freeze({
  schemaVersion: "agent-bounties/standing-meta-bounty-v1",
  inventoryClass: "post_bounty_third_party_completion",
  verifierProtocol: "agent-bounties/canonical-child-v1",
  verifierModule: "0x40adac5a1d00a725f77682f8940b893eaed31ecf",
  verifierRuntimeCodeHash: "0xbb6d6df11b85f59b5010aa61f4caf499fb27b94a0f5978aff85fa97ed2bbd2c3",
  acceptanceCriteriaHash: "0xa103c2c907f96e03a2f2b0e6b2209e0a3ca53686f7e9f79d89d7bfa1f8e314de",
  acceptanceCriteria: Object.freeze([
    "Post a canonical autonomous-v1 child bounty whose creator is the active solver.",
    "Fully fund the child to at least the parent solver reward; pooled contributors are allowed.",
    "Bind the child benchmark to the parent bounty ID and round and use an explicit deterministic verifier.",
    "Have a different wallet complete the child and receive canonical settlement before the parent verification deadline.",
  ]),
});
const CHAIN_MANIFEST_URL = new URL("../fixtures/base-mainnet-canaries.json", import.meta.url);
const SELECTOR = Object.freeze({
  acceptanceCriteriaHash: "0x8a2b02be",
  allowance: "0xdd62ed3e",
  approve: "0x095ea7b3",
  balanceOf: "0x70a08231",
  benchmarkHash: "0x13e4873c",
  bountyId: "0xc17bd75e",
  claim: "0x4e71d92d",
  creator: "0x02d05d3f",
  evidenceSchemaHash: "0x858c99ee",
  factory: "0xc45a0155",
  factoryImplementation: "0x5c60da1b",
  factoryProtocolVersion: "0xc6532cbe",
  fundedAmount: "0x820a5f50",
  isCanonicalBounty: "0xdb021126",
  policyHash: "0x098fb624",
  protocolVersion: "0x2ae9c600",
  settlementToken: "0x7b9e618d",
  solverReward: "0x798d5bef",
  status: "0x200d2ed2",
  targetAmount: "0x953b8fb8",
  termsHash: "0xb311d9fd",
  timeoutBondPool: "0xca279823",
  verifierReward: "0xb49e80f4",
  verificationMode: "0x402c51b8",
  verifierModule: "0x41506fc1",
  verifierSetHash: "0xae8e71a6",
});

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

export function normalizeRpcUrl(value) {
  const url = new URL(String(value || "").trim());
  if (url.username || url.password) throw new Error("Base RPC URL must not contain credentials");
  const loopback = ["localhost", "127.0.0.1", "::1"].includes(url.hostname);
  if (url.protocol !== "https:" && !(url.protocol === "http:" && loopback)) {
    throw new Error("Base RPC URL must use HTTPS, except for loopback development URLs");
  }
  url.hash = "";
  return url.toString();
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

async function rpcBatchTransport(rpcUrl, calls) {
  const payload = calls.map((call, index) => ({
    jsonrpc: "2.0",
    id: index + 1,
    method: call.method,
    params: call.params,
  }));
  for (let attempt = 0; attempt < 4; attempt += 1) {
    const response = await fetch(rpcUrl, {
      method: "POST",
      headers: { accept: "application/json", "content-type": "application/json" },
      body: JSON.stringify(payload),
      signal: AbortSignal.timeout(15_000),
    });
    if (!response.ok) {
      if (attempt < 3 && response.status === 429) {
        await new Promise((resolve) => setTimeout(resolve, 250 * (2 ** attempt)));
        continue;
      }
      throw new Error(`Base RPC returned HTTP ${response.status}`);
    }
    const body = await response.json();
    if (!Array.isArray(body)) throw new Error("Base RPC did not return a batch response");
    const byId = new Map(body.map((item) => [item?.id, item]));
    const rateLimited = calls.some((_, index) => byId.get(index + 1)?.error?.code === -32016);
    if (rateLimited && attempt < 3) {
      await new Promise((resolve) => setTimeout(resolve, 250 * (2 ** attempt)));
      continue;
    }
    const results = new Map();
    for (let index = 0; index < calls.length; index += 1) {
      const item = byId.get(index + 1);
      if (!item || item.error || item.result === undefined) {
        throw new Error(`Base RPC failed ${calls[index].key}`);
      }
      results.set(calls[index].key, item.result);
    }
    return results;
  }
  throw new Error("Base RPC retry budget exhausted");
}

function addressWord(value) {
  if (!ADDRESS.test(value || "")) throw new Error("invalid address in chain inventory");
  return value.toLowerCase().slice(2).padStart(64, "0");
}

function uintWord(value) {
  const amount = typeof value === "bigint" ? value : BigInt(value);
  if (amount < 0n || amount >= (1n << 256n)) throw new Error("invalid uint256 in chain inventory");
  return amount.toString(16).padStart(64, "0");
}

function calldata(selector, ...words) {
  return `${selector}${words.join("")}`;
}

function decodedWord(value, label) {
  const normalized = String(value || "").toLowerCase();
  if (!/^0x[0-9a-f]{64}$/.test(normalized)) throw new Error(`${label} is not one ABI word`);
  return normalized;
}

function decodedHash(value, label) {
  const word = decodedWord(value, label);
  if (!HASH.test(word)) throw new Error(`${label} is not bytes32`);
  return word;
}

function decodedAddress(value, label) {
  return `0x${decodedWord(value, label).slice(-40)}`;
}

function decodedUint(value, label) {
  return BigInt(decodedWord(value, label));
}

function safeNumber(value, label) {
  if (value < 0n || value > BigInt(Number.MAX_SAFE_INTEGER)) {
    throw new Error(`${label} exceeds safe integer range`);
  }
  return Number(value);
}

function proofCodeHash(value, label) {
  const hash = String(value?.codeHash || "").toLowerCase();
  if (!HASH.test(hash)) throw new Error(`${label} proof has no code hash`);
  return hash;
}

function callRequest(key, to, data, blockNumber) {
  return { key, method: "eth_call", params: [{ to, data }, blockNumber] };
}

function proofRequest(key, address, blockNumber) {
  return { key, method: "eth_getProof", params: [address, [], blockNumber] };
}

function validateChainManifest(manifest) {
  if (
    manifest?.schema_version !== "agent-bounties/chain-inventory-v1"
    || manifest.protocol_version !== "agent-bounties/autonomous-v1"
    || manifest.network !== "base-mainnet"
    || manifest.chain_id !== 8453
    || !HASH.test(manifest.protocol_hash || "")
    || !ADDRESS.test(manifest.native_usdc || "")
    || !ADDRESS.test(manifest.factory || "")
    || !HASH.test(manifest.factory_runtime_code_hash || "")
    || !ADDRESS.test(manifest.implementation || "")
    || !HASH.test(manifest.implementation_runtime_code_hash || "")
    || !HASH.test(manifest.bounty_proxy_runtime_code_hash || "")
    || !HASH.test(manifest.verifier_set_hash || "")
    || !Array.isArray(manifest.bounties)
    || manifest.bounties.length === 0
    || manifest.bounties.length > 100
  ) {
    throw new Error("invalid direct-chain inventory manifest");
  }
  const ids = new Set();
  const contracts = new Set();
  for (const bounty of manifest.bounties) {
    const amounts = [
      bounty?.solver_reward_minor,
      bounty?.verifier_reward_minor,
      bounty?.claim_bond_minor,
      bounty?.target_minor,
    ];
    if (
      !Number.isSafeInteger(bounty?.issue)
      || bounty.issue <= 0
      || typeof bounty.title !== "string"
      || !normalizePublicUrl(bounty.source_url, "Bounty source URL")
      || !/^fixtures\/terms\/[0-9]+\.json$/.test(bounty.terms_path || "")
      || !HASH.test(bounty.terms_hash || "")
      || !HASH.test(bounty.policy_hash || "")
      || !HASH.test(bounty.acceptance_criteria_hash || "")
      || !HASH.test(bounty.benchmark_hash || "")
      || !HASH.test(bounty.evidence_schema_hash || "")
      || !HASH.test(bounty.bounty_id || "")
      || !ADDRESS.test(bounty.contract || "")
      || !ADDRESS.test(bounty.creator || "")
      || !["deterministic_module", "signed_quorum", "ai_judge_quorum"]
        .includes(bounty.verification_mode)
      || (bounty.verification_mode === "deterministic_module"
        && (
          !ADDRESS.test(bounty.verifier_module || "")
          || !HASH.test(bounty.verifier_runtime_code_hash || "")
        ))
      || amounts.some((amount) => !Number.isSafeInteger(amount) || amount <= 0)
      || bounty.claim_bond_minor !== bounty.verifier_reward_minor
      || bounty.target_minor !== bounty.solver_reward_minor + bounty.verifier_reward_minor
    ) {
      throw new Error(`invalid direct-chain bounty manifest entry #${bounty?.issue || "unknown"}`);
    }
    const id = bounty.bounty_id.toLowerCase();
    const contract = bounty.contract.toLowerCase();
    if (ids.has(id) || contracts.has(contract)) throw new Error("duplicate direct-chain bounty identity");
    ids.add(id);
    contracts.add(contract);
  }
  return manifest;
}

function directClaimPlan(manifest, bounty, solverWallet, solverBalance, allowance) {
  if (!solverWallet) return { ready: false, reason: "solver_wallet_not_supplied", wallet_calls: [] };
  if (solverWallet.toLowerCase() === bounty.creator.toLowerCase()) {
    return { ready: false, reason: "creator_cannot_claim", wallet_calls: [] };
  }
  const bond = BigInt(bounty.claim_bond_minor);
  if (solverBalance < bond) {
    return { ready: false, reason: "insufficient_usdc_for_claim_bond", wallet_calls: [] };
  }
  const calls = [];
  if (allowance < bond) {
    calls.push({
      from: solverWallet.toLowerCase(),
      to: manifest.native_usdc.toLowerCase(),
      value_wei: 0,
      data: calldata(SELECTOR.approve, addressWord(bounty.contract), uintWord(bond)),
      function: "approve(address,uint256)",
    });
  }
  calls.push({
    from: solverWallet.toLowerCase(),
    to: bounty.contract.toLowerCase(),
    value_wei: 0,
    data: SELECTOR.claim,
    function: "claim()",
  });
  return {
    ready: true,
    reason: "safe_chain_state_and_solver_bond_confirmed",
    solver_usdc_balance_minor: safeNumber(solverBalance, "solver USDC balance"),
    current_allowance_minor: safeNumber(allowance, "solver allowance"),
    wallet_calls: calls,
    evidence_boundary: "This is unsigned calldata, not a claim. Re-read chain state and obtain bounded wallet authorization before broadcast.",
  };
}

function normalizedDirectBounty(manifest, bounty, observedBlock, timeoutBonus, claimPlan) {
  const normalized = {
    id: bounty.bounty_id.toLowerCase(),
    contract: bounty.contract.toLowerCase(),
    issue: bounty.issue,
    title: bounty.title,
    solver_reward_minor: bounty.solver_reward_minor,
    completion_bonus_minor: safeNumber(timeoutBonus, "timeout bond pool"),
    claim_bond_minor: bounty.claim_bond_minor,
    currency: "usdc",
    status: "claimable",
    evidence: "confirmed_canonical_autonomous_bounty",
    evidence_source: "direct_safe_chain",
    observed_block_number: observedBlock.number,
    observed_block_hash: observedBlock.hash,
    terms_hash: bounty.terms_hash.toLowerCase(),
    terms_path: bounty.terms_path,
    terms_url: `https://github.com/NSPG13/agent-bounties/blob/${TERMS_SOURCE_COMMIT}/bounties/autonomous-v1/${bounty.issue}.json`,
    source_url: bounty.source_url,
    claim_plan_url: null,
    claim_plan: claimPlan,
    claim_contract: bounty.contract.toLowerCase(),
    verification_mode: bounty.verification_mode,
    verifier_module: bounty.verifier_module?.toLowerCase() || null,
    verification_ready: true,
  };
  const standingMeta = standingMetaDescriptor({
    verifierModule: bounty.verifier_module,
    verifierRuntimeCodeHash: bounty.verifier_runtime_code_hash,
    acceptanceCriteriaHash: bounty.acceptance_criteria_hash,
    observedBlock,
  });
  if (standingMeta) normalized.standing_meta_bounty = standingMeta;
  return normalized;
}

export async function verifyDirectChainInventory({
  manifest,
  rpcUrl = DEFAULT_BASE_RPC_URL,
  rpcTransport = rpcBatchTransport,
  solverWallet = null,
}) {
  try {
    const checked = validateChainManifest(manifest);
    const rpc = normalizeRpcUrl(rpcUrl);
    const solver = solverWallet ? String(solverWallet).toLowerCase() : null;
    if (solver && !ADDRESS.test(solver)) throw new Error("solver wallet is not an address");

    const blockResults = await rpcTransport(rpc, [
      { key: "safe_block", method: "eth_getBlockByNumber", params: ["safe", false] },
    ]);
    const block = blockResults.get("safe_block");
    const blockNumber = String(block?.number || "").toLowerCase();
    const blockHash = String(block?.hash || "").toLowerCase();
    if (!/^0x[0-9a-f]+$/.test(blockNumber) || !HASH.test(blockHash)) {
      throw new Error("Base RPC did not return a safe block identity");
    }
    const observedBlock = {
      number: safeNumber(BigInt(blockNumber), "safe block number"),
      hash: blockHash,
      tag: "safe",
    };

    const factoryProof = await rpcTransport(rpc, [
      proofRequest("factory_proof", checked.factory, blockNumber),
    ]);
    const factoryCodeHash = proofCodeHash(factoryProof.get("factory_proof"), "factory");
    if (factoryCodeHash === EMPTY_CODE_HASH) {
      return {
        status: "not_deployed",
        observed_block: observedBlock,
        protocol: null,
        verified: [],
        excluded: [],
        warning: "canonical_factory_not_deployed_at_safe_block",
      };
    }
    const implementationProof = await rpcTransport(rpc, [
      proofRequest("implementation_proof", checked.implementation, blockNumber),
    ]);
    const globalResults = await rpcTransport(rpc, [
      callRequest("factory_protocol", checked.factory, SELECTOR.factoryProtocolVersion, blockNumber),
      callRequest("factory_implementation", checked.factory, SELECTOR.factoryImplementation, blockNumber),
      callRequest("factory_token", checked.factory, SELECTOR.settlementToken, blockNumber),
    ]);
    if (
      factoryCodeHash !== checked.factory_runtime_code_hash.toLowerCase()
      || proofCodeHash(implementationProof.get("implementation_proof"), "implementation")
        !== checked.implementation_runtime_code_hash.toLowerCase()
      || decodedHash(globalResults.get("factory_protocol"), "factory protocol")
        !== checked.protocol_hash.toLowerCase()
      || decodedAddress(globalResults.get("factory_implementation"), "factory implementation")
        !== checked.implementation.toLowerCase()
      || decodedAddress(globalResults.get("factory_token"), "factory token")
        !== checked.native_usdc.toLowerCase()
    ) {
      throw new Error("canonical factory code or configuration mismatch");
    }

    const requests = [];
    for (const bounty of checked.bounties) {
      const prefix = `bounty_${bounty.issue}`;
      requests.push(
        callRequest(`${prefix}_canonical`, checked.factory, calldata(SELECTOR.isCanonicalBounty, addressWord(bounty.contract)), blockNumber),
        callRequest(`${prefix}_protocol`, bounty.contract, SELECTOR.protocolVersion, blockNumber),
        callRequest(`${prefix}_id`, bounty.contract, SELECTOR.bountyId, blockNumber),
        callRequest(`${prefix}_creator`, bounty.contract, SELECTOR.creator, blockNumber),
        callRequest(`${prefix}_factory`, bounty.contract, SELECTOR.factory, blockNumber),
        callRequest(`${prefix}_token`, bounty.contract, SELECTOR.settlementToken, blockNumber),
        callRequest(`${prefix}_solver_reward`, bounty.contract, SELECTOR.solverReward, blockNumber),
        callRequest(`${prefix}_verifier_reward`, bounty.contract, SELECTOR.verifierReward, blockNumber),
        callRequest(`${prefix}_target`, bounty.contract, SELECTOR.targetAmount, blockNumber),
        callRequest(`${prefix}_funded`, bounty.contract, SELECTOR.fundedAmount, blockNumber),
        callRequest(`${prefix}_status`, bounty.contract, SELECTOR.status, blockNumber),
        callRequest(`${prefix}_timeout_bonus`, bounty.contract, SELECTOR.timeoutBondPool, blockNumber),
        callRequest(`${prefix}_terms`, bounty.contract, SELECTOR.termsHash, blockNumber),
        callRequest(`${prefix}_policy`, bounty.contract, SELECTOR.policyHash, blockNumber),
        callRequest(`${prefix}_acceptance`, bounty.contract, SELECTOR.acceptanceCriteriaHash, blockNumber),
        callRequest(`${prefix}_benchmark`, bounty.contract, SELECTOR.benchmarkHash, blockNumber),
        callRequest(`${prefix}_evidence`, bounty.contract, SELECTOR.evidenceSchemaHash, blockNumber),
        callRequest(`${prefix}_verifier_set`, bounty.contract, SELECTOR.verifierSetHash, blockNumber),
        callRequest(`${prefix}_verification_mode`, bounty.contract, SELECTOR.verificationMode, blockNumber),
        callRequest(`${prefix}_verifier_module`, bounty.contract, SELECTOR.verifierModule, blockNumber),
        callRequest(`${prefix}_token_balance`, checked.native_usdc, calldata(SELECTOR.balanceOf, addressWord(bounty.contract)), blockNumber),
      );
      if (solver) {
        requests.push(
          callRequest(`${prefix}_solver_balance`, checked.native_usdc, calldata(SELECTOR.balanceOf, addressWord(solver)), blockNumber),
          callRequest(`${prefix}_allowance`, checked.native_usdc, calldata(SELECTOR.allowance, addressWord(solver), addressWord(bounty.contract)), blockNumber),
        );
      }
    }
    const results = await rpcTransport(rpc, requests);
    for (const bounty of checked.bounties) {
      const key = `bounty_${bounty.issue}_proof`;
      const proofRequests = [proofRequest(key, bounty.contract, blockNumber)];
      if (bounty.verification_mode === "deterministic_module") {
        proofRequests.push(proofRequest(
          `bounty_${bounty.issue}_verifier_proof`,
          bounty.verifier_module,
          blockNumber,
        ));
      }
      const proofs = await rpcTransport(rpc, proofRequests);
      for (const request of proofRequests) results.set(request.key, proofs.get(request.key));
    }
    const verified = [];
    const excluded = [];
    for (const bounty of checked.bounties) {
      const prefix = `bounty_${bounty.issue}`;
      try {
        const timeoutBonus = decodedUint(results.get(`${prefix}_timeout_bonus`), "timeout bond pool");
        const funded = decodedUint(results.get(`${prefix}_funded`), "funded amount");
        const tokenBalance = decodedUint(results.get(`${prefix}_token_balance`), "bounty token balance");
        const expectedMode = {
          deterministic_module: 0n,
          signed_quorum: 1n,
          ai_judge_quorum: 2n,
        }[bounty.verification_mode];
        const expectedModule = bounty.verifier_module?.toLowerCase()
          || "0x0000000000000000000000000000000000000000";
        const matches = [
          proofCodeHash(results.get(`${prefix}_proof`), `bounty #${bounty.issue}`)
            === checked.bounty_proxy_runtime_code_hash.toLowerCase(),
          decodedUint(results.get(`${prefix}_canonical`), "canonical registration") === 1n,
          decodedHash(results.get(`${prefix}_protocol`), "bounty protocol") === checked.protocol_hash.toLowerCase(),
          decodedHash(results.get(`${prefix}_id`), "bounty id") === bounty.bounty_id.toLowerCase(),
          decodedAddress(results.get(`${prefix}_creator`), "bounty creator") === bounty.creator.toLowerCase(),
          decodedAddress(results.get(`${prefix}_factory`), "bounty factory") === checked.factory.toLowerCase(),
          decodedAddress(results.get(`${prefix}_token`), "bounty token") === checked.native_usdc.toLowerCase(),
          decodedUint(results.get(`${prefix}_solver_reward`), "solver reward") === BigInt(bounty.solver_reward_minor),
          decodedUint(results.get(`${prefix}_verifier_reward`), "verifier reward") === BigInt(bounty.verifier_reward_minor),
          decodedUint(results.get(`${prefix}_target`), "target amount") === BigInt(bounty.target_minor),
          funded === BigInt(bounty.target_minor),
          decodedUint(results.get(`${prefix}_status`), "bounty status") === 1n,
          decodedHash(results.get(`${prefix}_terms`), "terms hash") === bounty.terms_hash.toLowerCase(),
          decodedHash(results.get(`${prefix}_policy`), "policy hash") === bounty.policy_hash.toLowerCase(),
          decodedHash(results.get(`${prefix}_acceptance`), "acceptance hash") === bounty.acceptance_criteria_hash.toLowerCase(),
          decodedHash(results.get(`${prefix}_benchmark`), "benchmark hash") === bounty.benchmark_hash.toLowerCase(),
          decodedHash(results.get(`${prefix}_evidence`), "evidence schema hash") === bounty.evidence_schema_hash.toLowerCase(),
          decodedHash(results.get(`${prefix}_verifier_set`), "verifier set hash") === checked.verifier_set_hash.toLowerCase(),
          decodedUint(results.get(`${prefix}_verification_mode`), "verification mode") === expectedMode,
          decodedAddress(results.get(`${prefix}_verifier_module`), "verifier module") === expectedModule,
          tokenBalance >= funded + timeoutBonus,
        ];
        if (bounty.verification_mode === "deterministic_module") {
          matches.push(
            proofCodeHash(results.get(`${prefix}_verifier_proof`), `bounty #${bounty.issue} verifier`)
              === bounty.verifier_runtime_code_hash.toLowerCase(),
          );
        }
        if (!matches.every(Boolean)) throw new Error("safe chain state does not match committed bounty");
        if (bounty.verification_mode !== "deterministic_module") {
          excluded.push({
            id: bounty.bounty_id.toLowerCase(),
            reason: "quorum_verifier_service_not_attested",
            detail: "direct earning inventory requires a permissionless deterministic module",
          });
          continue;
        }
        const solverBalance = solver
          ? decodedUint(results.get(`${prefix}_solver_balance`), "solver USDC balance")
          : 0n;
        const allowance = solver
          ? decodedUint(results.get(`${prefix}_allowance`), "solver allowance")
          : 0n;
        verified.push(normalizedDirectBounty(
          checked,
          bounty,
          observedBlock,
          timeoutBonus,
          directClaimPlan(checked, bounty, solver, solverBalance, allowance),
        ));
      } catch (error) {
        excluded.push({
          id: bounty.bounty_id.toLowerCase(),
          reason: "direct_safe_chain_invariant_failed",
          detail: String(error?.message || error),
        });
      }
    }
    return {
      status: verified.length ? "verified" : "no_claimable_bounties",
      observed_block: observedBlock,
      protocol: {
        protocol_version: checked.protocol_version,
        status: "active",
        network: checked.network,
        chain_id: checked.chain_id,
        native_usdc: checked.native_usdc.toLowerCase(),
        factory: checked.factory.toLowerCase(),
        implementation: checked.implementation.toLowerCase(),
      },
      verified,
      excluded,
      warning: verified.length ? null : "no_direct_safe_chain_bounty_is_claimable",
    };
  } catch (error) {
    return {
      status: "verification_failed",
      observed_block: null,
      protocol: null,
      verified: [],
      excluded: [],
      warning: "direct_safe_chain_verification_failed",
      error: String(error?.message || error),
    };
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

function exactStrings(actual, expected) {
  return Array.isArray(actual)
    && actual.length === expected.length
    && actual.every((value, index) => value === expected[index]);
}

function hostedStandingMetaCandidate(item) {
  const document = item?.terms?.document;
  const policy = document?.verification_policy;
  const benchmark = document?.benchmark;
  const requiredEvidence = document?.evidence_schema?.required;
  return Boolean(
    item?.verification_mode === "deterministic_module"
      && String(item?.verifier_module || "").toLowerCase() === STANDING_META_BOUNTY.verifierModule
      && exactStrings(document?.acceptance_criteria, STANDING_META_BOUNTY.acceptanceCriteria)
      && policy?.mechanism === "deterministic_module"
      && String(policy?.verifier_module || "").toLowerCase() === STANDING_META_BOUNTY.verifierModule
      && policy?.threshold === 1
      && benchmark?.engine === "canonical_child_loop_v1"
      && benchmark?.required_child_status === "settled"
      && String(benchmark?.verifier_module || "").toLowerCase() === STANDING_META_BOUNTY.verifierModule
      && Array.isArray(requiredEvidence)
      && requiredEvidence.includes("child_bounty_contract"),
  );
}

function standingMetaDescriptor({
  verifierModule,
  verifierRuntimeCodeHash,
  acceptanceCriteriaHash,
  observedBlock,
}) {
  if (
    String(verifierModule || "").toLowerCase() !== STANDING_META_BOUNTY.verifierModule
    || String(verifierRuntimeCodeHash || "").toLowerCase()
      !== STANDING_META_BOUNTY.verifierRuntimeCodeHash
    || String(acceptanceCriteriaHash || "").toLowerCase()
      !== STANDING_META_BOUNTY.acceptanceCriteriaHash
    || !Number.isSafeInteger(observedBlock?.number)
    || observedBlock.number <= 0
    || !HASH.test(observedBlock?.hash || "")
  ) return null;
  return {
    schema_version: STANDING_META_BOUNTY.schemaVersion,
    inventory_class: STANDING_META_BOUNTY.inventoryClass,
    verifier_protocol: STANDING_META_BOUNTY.verifierProtocol,
    verifier_module: STANDING_META_BOUNTY.verifierModule,
    verifier_runtime_code_hash: STANDING_META_BOUNTY.verifierRuntimeCodeHash,
    acceptance_criteria_hash: STANDING_META_BOUNTY.acceptanceCriteriaHash,
    requires_funded_canonical_child: true,
    requires_different_solver_wallet: true,
    required_child_status: "settled",
    observed_block_number: observedBlock.number,
    observed_block_hash: observedBlock.hash.toLowerCase(),
  };
}

async function attestStandingMetaVerifier(rpcUrl, rpcTransport) {
  try {
    const rpc = normalizeRpcUrl(rpcUrl);
    const blocks = await rpcTransport(rpc, [
      { key: "standing_meta_safe_block", method: "eth_getBlockByNumber", params: ["safe", false] },
    ]);
    const block = blocks.get("standing_meta_safe_block");
    const blockNumber = String(block?.number || "").toLowerCase();
    const blockHash = String(block?.hash || "").toLowerCase();
    if (!/^0x[0-9a-f]+$/.test(blockNumber) || !HASH.test(blockHash)) {
      throw new Error("Base RPC did not return a safe block identity");
    }
    const proofs = await rpcTransport(rpc, [
      proofRequest("standing_meta_verifier_proof", STANDING_META_BOUNTY.verifierModule, blockNumber),
    ]);
    const codeHash = proofCodeHash(
      proofs.get("standing_meta_verifier_proof"),
      "standing meta verifier",
    );
    const observedBlock = {
      tag: "safe",
      number: safeNumber(BigInt(blockNumber), "safe block number"),
      hash: blockHash,
    };
    return {
      ready: codeHash === STANDING_META_BOUNTY.verifierRuntimeCodeHash,
      codeHash,
      observedBlock,
      warning: codeHash === STANDING_META_BOUNTY.verifierRuntimeCodeHash
        ? null
        : "standing_meta_verifier_code_mismatch",
    };
  } catch (error) {
    return {
      ready: false,
      codeHash: null,
      observedBlock: null,
      warning: "standing_meta_verifier_attestation_failed",
      detail: error instanceof Error ? error.message : String(error),
    };
  }
}

export function verifyClaimableItem(item, protocol) {
  if (!activeProtocol(protocol)) return { ok: false, reason: "autonomous_protocol_not_active" };
  if (!item || item.status !== "claimable") {
    return { ok: false, reason: "indexed_status_not_claimable" };
  }
  if (!item.terms_valid || !item.terms?.document?.contract_terms) {
    return { ok: false, reason: "terms_or_contract_commitments_invalid" };
  }
  if (
    item.verification_ready !== true
    || item.verification_mode !== "deterministic_module"
    || !ADDRESS.test(item.verifier_module || "")
  ) {
    return { ok: false, reason: "verification_path_not_executable" };
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

function normalizedBounty(item, apiBaseUrl, standingMetaAttestation = null) {
  const normalized = {
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
    verification_mode: item.verification_mode,
    verifier_module: item.verifier_module?.toLowerCase() || null,
    verification_ready: item.verification_ready === true,
  };
  if (hostedStandingMetaCandidate(item) && standingMetaAttestation?.ready) {
    normalized.standing_meta_bounty = standingMetaDescriptor({
      verifierModule: item.verifier_module,
      verifierRuntimeCodeHash: standingMetaAttestation.codeHash,
      acceptanceCriteriaHash: STANDING_META_BOUNTY.acceptanceCriteriaHash,
      observedBlock: standingMetaAttestation.observedBlock,
    });
  }
  return normalized;
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
  baseRpcUrl = DEFAULT_BASE_RPC_URL,
  chainManifest = null,
  rpcTransport = rpcBatchTransport,
  solverWallet = null,
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
  const hostedVerified = [];
  const excluded = [];
  const fundingCandidates = [];
  for (const item of itemsFrom(feedResponse?.body)) {
    if (item?.status === "open" && item?.terms_valid) {
      fundingCandidates.push(normalizedFundingCandidate(item));
    }
    if (item?.status !== "claimable") continue;
    const verdict = verifyClaimableItem(item, protocol);
    if (verdict.ok) {
      hostedVerified.push(item);
    } else {
      excluded.push({ id: item?.bounty_id || null, reason: verdict.reason });
    }
  }

  const hasStandingMetaCandidate = hostedVerified.some(hostedStandingMetaCandidate);
  const standingMetaAttestation = hasStandingMetaCandidate
    ? await attestStandingMetaVerifier(baseRpcUrl, rpcTransport)
    : null;
  for (const item of hostedVerified) {
    verified.push(normalizedBounty(item, api, standingMetaAttestation));
  }

  let direct = {
    status: "not_checked",
    observed_block: null,
    protocol: null,
    verified: [],
    excluded: [],
    warning: null,
  };
  if (verified.length === 0 && (!fixture || chainManifest)) {
    const manifest = chainManifest || JSON.parse(await readFile(CHAIN_MANIFEST_URL, "utf8"));
    direct = await verifyDirectChainInventory({
      manifest,
      rpcUrl: baseRpcUrl,
      rpcTransport,
      solverWallet,
    });
    const existingIds = new Set(verified.map((item) => item.id.toLowerCase()));
    for (const item of direct.verified) {
      if (!existingIds.has(item.id.toLowerCase())) {
        verified.push(item);
        existingIds.add(item.id.toLowerCase());
      }
    }
    excluded.push(...direct.excluded);
  }

  const healthOk = health?.status === 200 && String(health.body).trim() === "ok";
  const hostedProtocolActive = activeProtocol(protocol);
  const directProtocolActive = activeProtocol(direct.protocol);
  const effectiveProtocol = hostedProtocolActive ? protocol : (directProtocolActive ? direct.protocol : null);
  const warnings = [];
  if (!healthOk) warnings.push("hosted_api_health_not_confirmed");
  if (feedResponse?.status !== 200) warnings.push("autonomous_feed_unavailable");
  if (!effectiveProtocol) warnings.push("autonomous_protocol_not_active");
  if (direct.warning) warnings.push(direct.warning);
  if (standingMetaAttestation?.warning) warnings.push(standingMetaAttestation.warning);
  if (!verified.length) warnings.push("no_verified_funded_bounty_is_claimable");

  return {
    observed_at: new Date().toISOString(),
    api_base_url: api,
    protocol_url: protocolEndpoint,
    hosted_api_healthy: healthOk,
    health_status: health?.status ?? null,
    protocol_status: effectiveProtocol?.status ?? protocol?.status ?? null,
    protocol_source: hostedProtocolActive
      ? "hosted_indexed_feed"
      : (directProtocolActive ? "direct_safe_chain" : "unavailable"),
    active_factory: effectiveProtocol?.factory ?? null,
    direct_chain_status: direct.status,
    direct_chain_observed_block: direct.observed_block,
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
      "Only an active exact-code factory plus matching terms, economics, funding, and canonical registration at a Base safe block, or the equivalent indexed canonical events, is earnable inventory. Only confirmed canonical BountySettled proves payout.",
  };
}

function parseArgs(argv) {
  const options = {
    apiBaseUrl: process.env.AGENT_BOUNTIES_API_URL || DEFAULT_API_BASE_URL,
    protocolUrl: process.env.AGENT_BOUNTIES_PROTOCOL_URL || DEFAULT_PROTOCOL_URL,
    baseRpcUrl: process.env.AGENT_BOUNTIES_BASE_RPC_URL || DEFAULT_BASE_RPC_URL,
    solverWallet: process.env.AGENT_BOUNTIES_SOLVER_WALLET || null,
    fixturePath: null,
  };
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === "--api-base-url") options.apiBaseUrl = argv[++index];
    else if (argument === "--protocol-url") options.protocolUrl = argv[++index];
    else if (argument === "--base-rpc-url") options.baseRpcUrl = argv[++index];
    else if (argument === "--solver-wallet") options.solverWallet = argv[++index];
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
      "Usage: node check-in.mjs [--api-base-url https://...] [--protocol-url https://...] [--base-rpc-url https://...] [--solver-wallet 0x...] [--fixture fixture.json]",
    );
    return;
  }
  const fixture = options.fixturePath
    ? JSON.parse(await readFile(options.fixturePath, "utf8"))
    : null;
  const report = await collectInventory({
    apiBaseUrl: options.apiBaseUrl,
    protocolUrl: options.protocolUrl,
    baseRpcUrl: options.baseRpcUrl,
    solverWallet: options.solverWallet,
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
