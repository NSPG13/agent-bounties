/** Offline structural review only. No RPC, signatures, wallet calls or eligibility decisions. */
import { readFileSync, statSync } from "node:fs";
import { pathToFileURL } from "node:url";

const ADDRESS = /^0x[0-9a-f]{40}$/i, HASH = /^0x[0-9a-f]{64}$/i;
const nonzero = (value, pattern) => typeof value === "string" && pattern.test(value) && !/^0x0+$/.test(value);
const amount = value => typeof value === "string" && /^(0|[1-9][0-9]{0,77})$/.test(value) && BigInt(value) < (1n << 256n);
const uint = value => Number.isSafeInteger(value) && value >= 0;
const lower = value => typeof value === "string" ? value.toLowerCase() : "";
const object = value => value !== null && typeof value === "object" && !Array.isArray(value);

export function preflight(payload, policy) {
  const errors = [];
  const check = (condition, message) => { if (!condition) errors.push(message); };
  if (!object(payload) || !object(policy)) errors.push("manifest and policy must be objects");
  else {
    check(payload.schema === "https://agentbounties.app/schemas/gmv-settlement-attribution.v1.json", "schema mismatch");
    check(payload.network === "base-mainnet" && payload.chain_id === 8453, "Base mainnet required");
    for (const [field, format] of [["competition", ADDRESS], ["bounty_id", HASH], ["epoch_id", HASH], ["verification_policy_hash", HASH]]) {
      check(nonzero(policy[field], format) && nonzero(payload[field], format) && lower(payload[field]) === lower(policy[field]), `${field} must match the selected policy`);
    }
    const windowValid = uint(policy.window_start) && uint(policy.window_end) && policy.window_start < policy.window_end;
    check(windowValid, "policy scoring window must use ordered integer seconds");
    for (const field of ["excluded_wallets", "excluded_bounty_contracts"]) {
      check(Array.isArray(policy[field]) && policy[field].every(x => nonzero(x, ADDRESS)), `${field} must be an explicit address list`);
    }
    const excludedWallets = new Set((Array.isArray(policy.excluded_wallets) ? policy.excluded_wallets : []).map(lower));
    const excludedContracts = new Set((Array.isArray(policy.excluded_bounty_contracts) ? policy.excluded_bounty_contracts : []).map(lower));
    check(nonzero(payload.entrant, ADDRESS) && !excludedWallets.has(lower(payload.entrant)), "entrant must be a valid non-excluded address");
    const settlements = payload.settlements ?? (payload.settlement ? [payload.settlement] : null);
    check(Array.isArray(settlements) && settlements.length > 0 && settlements.length <= 1000, "provide 1–1000 settlement references");
    const seen = new Set();
    for (const [i, event] of (Array.isArray(settlements) ? settlements.slice(0, 1000) : []).entries()) {
      if (!object(event)) { errors.push(`settlement ${i} must be an object`); continue; }
      for (const field of ["creator", "solver", "funder", "child_bounty_contract"]) check(nonzero(event[field], ADDRESS), `settlement ${i}: invalid ${field}`);
      const creator = lower(event.creator), solver = lower(event.solver), funder = lower(event.funder);
      check(funder === lower(payload.entrant) && funder !== solver && creator !== solver, `settlement ${i}: entrant must fund work completed by a different solver`);
      check(![creator, solver, funder].some(x => excludedWallets.has(x)) && !excludedContracts.has(lower(event.child_bounty_contract)), `settlement ${i}: excluded wallet or contract`);
      check(windowValid && uint(event.settled_at) && event.settled_at >= policy.window_start && event.settled_at < policy.window_end, `settlement ${i}: outside scoring window`);
      check(amount(event.gmv_base_units) && BigInt(event.gmv_base_units) > 0n, `settlement ${i}: GMV must be a positive uint256 decimal string`);
      check(nonzero(event.tx_hash, HASH) && uint(event.log_index), `settlement ${i}: transaction and log index required`);
      const identity = `8453:${lower(event.child_bounty_contract)}:${lower(event.tx_hash)}:${event.log_index}`;
      check(!seen.has(identity), `settlement ${i}: duplicate event identity`); seen.add(identity);
    }
  }
  return { structure_valid: errors.length === 0, ready: false, eligible: false, status: "settlement_unverified", errors,
    evidence_boundary: "Caller-provided JSON and the selected policy are untrusted. Independently verify canonical events, immutable policy, entrant attribution, exclusions and the committed snapshot quorum before determining eligibility. This check never proves funding, delivery, settlement or completeness." };
}

function readBounded(path) {
  if (!path || !statSync(path).isFile() || statSync(path).size > 1_000_000) throw new Error("provide regular JSON files of at most 1 MB");
  return JSON.parse(readFileSync(path, "utf8"));
}
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    if (process.argv.length !== 4) throw new Error("usage: node scripts/gmv-manifest-preflight.mjs manifest.json policy.json");
    const result = preflight(readBounded(process.argv[2]), readBounded(process.argv[3]));
    console.log(JSON.stringify(result)); process.exitCode = result.structure_valid ? 0 : 1;
  } catch (error) {
    console.log(JSON.stringify({ structure_valid: false, ready: false, eligible: false, status: "settlement_unverified", errors: [error.message] })); process.exitCode = 2;
  }
}
