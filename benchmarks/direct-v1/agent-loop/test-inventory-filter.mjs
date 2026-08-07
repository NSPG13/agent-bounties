// Regression test for ready-to-earn inventory filter (#683)
// Standalone test - validates the canonical filter logic directly.

const h = (d) => "0x" + d.repeat(64);
const a = (d) => "0x" + d.repeat(40);
const solver = "0x1111111111111111111111111111111111111111";

function bounty(overrides) {
  return Object.assign({
    bounty_id: h("a"), bounty_contract: a("a"), creator: a("f"),
    status: "claimable", solver_reward: "2000000", verifier_reward: "200000",
    claim_bond: "200000", target_amount: "2200000", funded_amount: "2200000",
    terms_hash: h("b"), terms_valid: true, verification_ready: true,
    validation_errors: []
  }, overrides || {});
}

// The canonical filter logic from select-funded-bounty
function filterAndRank(feed, wallet) {
  const eligible = feed.filter(function(item) {
    return item.status === "claimable"
      && item.terms_valid
      && item.verification_ready
      && item.validation_errors.length === 0
      && BigInt(item.solver_reward) > 0n
      && BigInt(item.claim_bond) > 0n
      && BigInt(item.funded_amount) >= BigInt(item.target_amount)
      && item.creator !== wallet;
  });
  eligible.sort(function(l, r) {
    var rw = BigInt(r.solver_reward) - BigInt(l.solver_reward);
    if (rw !== 0n) return rw > 0n ? 1 : -1;
    var bd = BigInt(l.claim_bond) - BigInt(r.claim_bond);
    if (bd !== 0n) return bd > 0n ? 1 : -1;
    return l.bounty_id.localeCompare(r.bounty_id);
  });
  return eligible;
}

var passed = 0, failed = 0;

function expectEmpty(feed, label) {
  var result = filterAndRank(feed, solver);
  if (result.length === 0) { passed++; console.log("PASS: " + label); }
  else { failed++; console.log("FAIL: " + label + " (got " + result.length + " items)"); }
}

function expectFirst(feed, expectedContract, label) {
  var result = filterAndRank(feed, solver);
  if (result.length > 0 && result[0].bounty_contract === expectedContract) {
    passed++; console.log("PASS: " + label);
  } else {
    failed++; console.log("FAIL: " + label + " (expected " + expectedContract + ", got " + (result[0] ? result[0].bounty_contract : "none") + ")");
  }
}

console.log("=== Inventory Filter Regression Tests (#683) ===");

// Must filter non-claimable
expectEmpty([bounty({status:"funded"}), bounty({status:"draft"})], "filter-non-claimable");

// Must filter invalid terms
expectEmpty([bounty({terms_valid:false})], "filter-invalid-terms");

// Must filter not verification-ready
expectEmpty([bounty({verification_ready:false})], "filter-not-ready");

// Must filter bounties with validation errors
expectEmpty([bounty({validation_errors:["schema_err"]})], "filter-has-errors");

// Must filter zero reward
expectEmpty([bounty({solver_reward:"0"})], "filter-zero-reward");

// Must filter zero bond
expectEmpty([bounty({claim_bond:"0"})], "filter-zero-bond");

// Must filter underfunded
expectEmpty([bounty({funded_amount:"1000000",target_amount:"2200000"})], "filter-underfunded");

// Must filter creator-owned
expectEmpty([bounty({creator:solver})], "filter-creator-owned");

// Must select valid bounty from mixed feed
var good = bounty({bounty_id:h("g"),bounty_contract:a("g"),solver_reward:"5000000",terms_hash:h("g")});
expectFirst([bounty({status:"draft"}),bounty({terms_valid:false}),good,bounty({verification_ready:false})], a("g"), "select-valid-from-mixed");

// Must rank by highest reward
var best = bounty({bounty_id:h("h"),bounty_contract:a("h"),solver_reward:"10000000",terms_hash:h("h")});
expectFirst([bounty({solver_reward:"1",bounty_id:h("l"),bounty_contract:a("l"),terms_hash:h("l")}),best,bounty({solver_reward:"5",bounty_id:h("m"),bounty_contract:a("m"),terms_hash:h("m")})], a("h"), "rank-highest-reward");

// Tie-break: same reward, lower bond wins
var lb = bounty({bounty_id:h("x"),bounty_contract:a("x"),solver_reward:"3000000",claim_bond:"100000",terms_hash:h("x")});
expectFirst([bounty({solver_reward:"3000000",claim_bond:"300000",bounty_id:h("y"),bounty_contract:a("y"),terms_hash:h("y")}),lb], a("x"), "tie-break-lower-bond");

console.log("\n=== " + passed + " passed, " + failed + " failed ===");
if (failed > 0) process.exit(1);
