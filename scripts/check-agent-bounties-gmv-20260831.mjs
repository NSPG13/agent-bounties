import { readFileSync } from "node:fs";

/**
 * Write failure payload to standard output and exit with status code.
 * @param {number} status
 * @param {string[]} errors
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
  "0x1692fecdb678537eb4cc0093e9e4e99dbded9806",
  "0x2f1d2b24105596b153e473032256569fe544a44f",
  "0x36e45ec89b61f551724558ed799ad4e07c288175",
  "0x3e052b933628b960d61654a68fca23d869d8989f",
  "0x441e8bded29917999fc0e8330c83553262fa2f08",
  "0x5817b7742b085d333c7e7831daa62a490c493b56",
  "0x5f884d4a4cc2727ddbc22382efd776274bc3e7aa",
  "0x6a791b05333d9ca7d28a052003ced4818a372c7f",
  "0x6f635dfd07085aa48ec8b11767eeb48936969f5c",
  "0x8c494466711c1de316c7e7599f8b0641a30a0c98",
  "0x8c990ddf5360c00ee0b2090000e3a3a6f90a6a9d",
  "0x979782be0bfefb78ac0459d4695d619932ecdda4",
  "0xaa4a9300bb1c90f93b4048fd83298da6c6145734",
  "0xf8c8897e748e4057d52182c27beb4025f4d49d68",
]);

const CANONICAL_COMPETITION = "0xdc1bbcbcb149b07262565c8b9caa1ae5e2058f76";
const CANONICAL_BOUNTY_ID = "0x46a5a34d8596f6f54efae2487e4ef7906ff3940583a97b9d266ef15c45c3df67";
const CANONICAL_EPOCH_ID = "0xb2f9f8ba04bfcbcf4858ccf32338f9f807a5fa57e37d31d3fd0d46b60a8fef3f";
const CANONICAL_POLICY_HASH = "0xbf60c6e630dc123f02115765623bb882f661a9b8594448678b39e774b2c9831a";
const WINDOW_START = 1788134400;
const WINDOW_END = 1788739200;

/**
 * Validate forward GMV settlement attribution file against competition requirements.
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

  const settlement = (payload.settlement && typeof payload.settlement === "object") ? payload.settlement : null;
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
