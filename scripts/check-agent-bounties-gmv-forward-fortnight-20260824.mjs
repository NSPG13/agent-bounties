import { readFileSync } from "node:fs";

/**
 * Write failure payload to standard output and exit with status code.
 * @param {number} status Process exit code
 * @param {string[]} errors Array of error codes
 */
function fail(status, errors) {
  process.stdout.write(JSON.stringify({ ready: false, errors }) + "\n");
  process.exit(status);
}

const EXCLUDED_WALLETS = new Set([
  "0x1eaa1c68772cf76bc5f4e4174766076e33ace662",
  "0x6fe4d6da2a4371d82b4a7ff94810a94091fb4c35",
  "0x7b0ae568f76d11aa4025e2aa05865a566bbcfc8d",
  "0x884834e884d6e93462655a2820140ad03e6747bc",
  "0xb358898d34c5e907877a1cd7540b234f6851f61b",
  "0xfb58949365e3a30fd62e86edb0daffccf4ef7477",
  "0xfd7be4c69541ab297aece2a674fc1418b898cc0a",
]);

const EXCLUDED_BOUNTY_CONTRACTS = new Set([
  "0x3e052b933628b960d61654a68fca23d869d8989f",
  "0x5f884d4a4cc2727ddbc22382efd776274bc3e7aa",
  "0xaa4a9300bb1c90f93b4048fd83298da6c6145734",
  "0xf8c8897e748e4057d52182c27beb4025f4d49d68",
]);

const CANONICAL_COMPETITION = "0x6f635dfd07085aa48ec8b11767eeb48936969f5c";
const CANONICAL_BOUNTY_ID = "0x650ab64c02cba5dd8d3d4f3efcc1bf4c2c8125d2fea0e13b69e8e326f1b8b81f";
const CANONICAL_EPOCH_ID = "0xf4954bbd1a48ec059078bfbb84a24cd68d35e8837631a562704dbe83edde5a9d";
const CANONICAL_POLICY_HASH = "0xcd88233c773ba4804318df8423e0313bbf733edd48cd0117d87100750fb36f76";
const WINDOW_START = 1787529600;
const WINDOW_END = 1788739200;

/**
 * Validate forward GMV settlement attribution file against fortnight August 24-September 7 competition requirements.
 */
function main() {
  if (process.argv.length !== 3) {
    fail(2, ["manifest_path_required"]);
  }

  const manifestPath = process.argv[2];
  let text = "";
  try {
    text = readFileSync(manifestPath, "utf8");
  } catch {
    fail(2, ["manifest_unreadable"]);
  }

  let payload = null;
  try {
    payload = JSON.parse(text);
  } catch {
    fail(2, ["manifest_invalid_json"]);
  }

  if (payload === null || Array.isArray(payload) || typeof payload !== "object") {
    fail(2, ["manifest_root_object_required"]);
  }

  const errors = [];
  if (payload.schema !== "https://agentbounties.app/schemas/gmv-settlement-attribution.v1.json") {
    errors.push("schema_mismatch");
  }
  if (payload.network !== "base-mainnet") {
    errors.push("network_mismatch");
  }
  if (payload.chain_id !== 8453) {
    errors.push("chain_id_mismatch");
  }
  if (String(payload.competition ?? "").toLowerCase() !== CANONICAL_COMPETITION) {
    errors.push("competition_mismatch");
  }
  if (String(payload.bounty_id ?? "").toLowerCase() !== CANONICAL_BOUNTY_ID) {
    errors.push("bounty_id_mismatch");
  }
  if (String(payload.epoch_id ?? "").toLowerCase() !== CANONICAL_EPOCH_ID) {
    errors.push("epoch_id_mismatch");
  }
  if (String(payload.verification_policy_hash ?? "").toLowerCase() !== CANONICAL_POLICY_HASH) {
    errors.push("verification_policy_mismatch");
  }

  const settlement = payload.settlement && typeof payload.settlement === "object" ? payload.settlement : null;
  if (!settlement) {
    errors.push("settlement_record_missing");
    fail(1, errors);
  }

  const settledAt = Number(settlement.settled_at ?? 0);
  if (!Number.isFinite(settledAt) || settledAt < WINDOW_START || settledAt >= WINDOW_END) {
    errors.push("settlement_outside_scoring_window");
  }

  const creator = String(settlement.creator ?? "").toLowerCase();
  const solver = String(settlement.solver ?? "").toLowerCase();
  const funder = String(settlement.funder ?? "").toLowerCase();
  const contract = String(settlement.child_bounty_contract ?? "").toLowerCase();

  if (creator.length === 0 || solver.length === 0 || creator === solver) {
    errors.push("self_dealing_disallowed");
  }

  if (EXCLUDED_WALLETS.has(creator) || EXCLUDED_WALLETS.has(solver) || EXCLUDED_WALLETS.has(funder)) {
    errors.push("excluded_wallet_disallowed");
  }

  if (EXCLUDED_BOUNTY_CONTRACTS.has(contract)) {
    errors.push("excluded_contract_disallowed");
  }

  const gmv = Number(settlement.gmv_base_units ?? 0);
  if (!Number.isInteger(gmv) || gmv < 1) {
    errors.push("insufficient_gmv_base_units");
  }

  if (errors.length > 0) {
    fail(1, errors);
  }

  const successPayload = {
    ready: true,
    network: "base-mainnet",
    competition: CANONICAL_COMPETITION,
    bounty_id: CANONICAL_BOUNTY_ID,
    gmv_base_units: gmv,
    status: "eligible",
  };
  process.stdout.write(JSON.stringify(successPayload) + "\n");
  process.exit(0);
}

main();
