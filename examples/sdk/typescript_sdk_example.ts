/**
 * Agent Bounties SDK Example — TypeScript
 * =======================================
 * Demonstrates the complete agent workflow: discover, inspect pooled bounties,
 * add funding, claim work, submit proof, and check paid status.
 *
 * Usage:
 *   npx tsx typescript_sdk_example.ts --local
 *   npx tsx typescript_sdk_example.ts --base-sepolia
 *   npx tsx typescript_sdk_example.ts --discover
 */

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface DiscoveryManifest {
  name: string;
  version: string;
  description: string;
  open_source: boolean;
  endpoints: Record<string, string>;
  agent_entrypoints: string[];
  payment_rails: PaymentRail[];
  trust_tiers: TrustTier[];
  templates: BountyTemplate[];
  proof_surfaces: ProofSurface[];
  risk_controls: RiskControl[];
  risk_policy: RiskPolicy;
}

interface PaymentRail {
  id: string;
  name: string;
  kind: "Simulated" | "StripeFiatLedger" | "BaseUsdcEscrow";
  currency: string;
  decimals: number;
}

interface TrustTier {
  id: string;
  name: string;
  max_amount: number;
  requires_operator: boolean;
}

interface BountyTemplate {
  slug: string;
  name: string;
  description: string;
  verifier_kind: string;
}

interface ProofSurface {
  id: string;
  bounty_id: string;
  proof_url: string;
  verifier_output: Record<string, unknown>;
}

interface Bounty {
  id: string;
  title: string;
  description: string;
  status: string;
  template_slug: string;
  target_amount: number;
  applied_amount: number;
  payment_rail: string;
  claimable: boolean;
}

interface FundingContribution {
  id: string;
  bounty_id: string;
  amount: number;
  rail: string;
  contributor: string;
}

interface BaseFundingPlan {
  escrow_contract: string;
  usdc_token: string;
  amount_usdc: number;
  network: string;
  unsigned_transaction?: string;
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

const LOCAL_API = "http://127.0.0.1:8080";
const BASE_SEPOLIA_API = "https://api.agentbounties.org";

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

async function discover(apiBase: string): Promise<DiscoveryManifest> {
  const resp = await fetch(`${apiBase}/.well-known/agent-bounties.json`);
  if (!resp.ok) throw new Error(`Discovery failed: ${resp.status}`);
  return resp.json();
}

async function fetchLlmsTxt(apiBase: string): Promise<string> {
  const resp = await fetch(`${apiBase}/llms.txt`);
  if (!resp.ok) throw new Error(`llms.txt failed: ${resp.status}`);
  return resp.text();
}

// ---------------------------------------------------------------------------
// Pooled Bounty Workflow
// ---------------------------------------------------------------------------

async function openPooledBounty(
  apiBase: string,
  title: string,
  description: string,
  targetAmount: number,
  templateSlug = "write-docs-for-area",
  rail = "Simulated",
): Promise<Bounty> {
  const resp = await fetch(`${apiBase}/v1/bounties/pooled`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      title,
      description,
      target_amount: targetAmount,
      template_slug: templateSlug,
      payment_rail: rail,
    }),
  });
  if (!resp.ok) throw new Error(`Open pooled bounty failed: ${resp.status}`);
  return resp.json();
}

async function getBounty(apiBase: string, bountyId: string): Promise<Bounty> {
  const resp = await fetch(`${apiBase}/v1/bounties/${bountyId}`);
  if (!resp.ok) throw new Error(`Get bounty failed: ${resp.status}`);
  return resp.json();
}

async function listClaimableBounties(apiBase: string): Promise<Bounty[]> {
  const resp = await fetch(`${apiBase}/v1/bounties/claimable`);
  if (!resp.ok) throw new Error(`List claimable failed: ${resp.status}`);
  return resp.json();
}

async function addFundingContribution(
  apiBase: string,
  bountyId: string,
  amount: number,
  rail = "Simulated",
  contributor = "local-agent",
): Promise<FundingContribution> {
  const resp = await fetch(`${apiBase}/v1/bounties/${bountyId}/funding-contributions`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      amount,
      rail,
      contributor,
      idempotency_key: crypto.randomUUID(),
    }),
  });
  if (!resp.ok) throw new Error(`Add funding failed: ${resp.status}`);
  return resp.json();
}

// ---------------------------------------------------------------------------
// Claim & Proof
// ---------------------------------------------------------------------------

async function claimBounty(
  apiBase: string,
  bountyId: string,
  solverId = "local-agent",
): Promise<{ id: string }> {
  const resp = await fetch(`${apiBase}/v1/bounties/${bountyId}/claims`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      solver_id: solverId,
      idempotency_key: crypto.randomUUID(),
    }),
  });
  if (!resp.ok) throw new Error(`Claim failed: ${resp.status}`);
  return resp.json();
}

async function submitProof(
  apiBase: string,
  bountyId: string,
  proofUrl: string,
  proofTitle: string,
  templateSlug = "write-docs-for-area",
): Promise<{ id: string }> {
  const resp = await fetch(`${apiBase}/v1/bounties/${bountyId}/proofs`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      proof_url: proofUrl,
      title: proofTitle,
      template_slug: templateSlug,
      verifier_kind: "documentation",
    }),
  });
  if (!resp.ok) throw new Error(`Submit proof failed: ${resp.status}`);
  return resp.json();
}

async function checkPaidStatus(
  apiBase: string,
  bountyId: string,
): Promise<{ status: string }> {
  const resp = await fetch(`${apiBase}/v1/bounties/${bountyId}/paid-status`);
  if (!resp.ok) throw new Error(`Check paid status failed: ${resp.status}`);
  return resp.json();
}

// ---------------------------------------------------------------------------
// Base Sepolia (hosted testnet rail)
// ---------------------------------------------------------------------------

async function planBaseFunding(
  apiBase: string,
  bountyId: string,
  amountUsdc: number,
  escrowContract: string,
  usdcToken: string,
  network = "base-sepolia",
): Promise<BaseFundingPlan> {
  const resp = await fetch(`${apiBase}/v1/bounties/${bountyId}/base-funding-plan`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      amount_usdc: amountUsdc,
      escrow_contract: escrowContract,
      usdc_token: usdcToken,
      network,
    }),
  });
  if (!resp.ok) throw new Error(`Plan base funding failed: ${resp.status}`);
  return resp.json();
}

// ---------------------------------------------------------------------------
// Simulated Demo
// ---------------------------------------------------------------------------

async function runSimulatedDemo() {
  console.log("=== Agent Bounties SDK Demo (TypeScript) ===\n");

  // 1. Discover
  console.log("1. Discovering service...");
  const manifest = await discover(LOCAL_API);
  console.log(`   API: ${manifest.name} v${manifest.version}`);
  console.log(`   Endpoints: ${Object.keys(manifest.endpoints).length} discovered`);
  console.log(`   Payment rails: ${manifest.payment_rails.map((r) => r.name).join(", ")}\n`);

  // 2. Open pooled bounty
  console.log("2. Opening pooled bounty...");
  const bounty = await openPooledBounty(
    LOCAL_API,
    "[bounty]: Add TypeScript SDK example",
    "Demonstrate the complete agent workflow in TypeScript",
    10,
  );
  console.log(`   Created: ${bounty.id} (status: ${bounty.status})\n`);

  // 3. Add funding
  console.log("3. Adding funding...");
  const contrib = await addFundingContribution(LOCAL_API, bounty.id, 10);
  console.log(`   Contribution: ${contrib.id} applied\n`);

  // 4. Verify claimable
  console.log("4. Checking claimable...");
  const claimable = await listClaimableBounties(LOCAL_API);
  const target = claimable.find((b) => b.id === bounty.id);
  console.log(`   Claimable: ${!!target}\n`);

  // 5. Claim
  if (target) {
    console.log("5. Claiming...");
    const claim = await claimBounty(LOCAL_API, bounty.id);
    console.log(`   Claimed: ${claim.id}\n`);
  }

  // 6. Submit proof
  console.log("6. Submitting proof...");
  const proof = await submitProof(
    LOCAL_API,
    bounty.id,
    "https://github.com/qilu13/agent-bounties/pull/23",
    "docs: agent contribution starter guide",
  );
  console.log(`   Proof: ${proof.id}\n`);

  // 7. Check paid status
  console.log("7. Checking paid status...");
  const paid = await checkPaidStatus(LOCAL_API, bounty.id);
  console.log(`   Status: ${paid.status}\n`);

  console.log("=== Demo complete ===");
  console.log("Note: Simulated mode does not settle real payments.");
  console.log("For real USDC on Base Sepolia, use --base-sepolia with a funded escrow.");
  console.log(
    `ETH: 0x0b86d893a652408cdaaf2021152db33b74fd0d25\nSOL: H7ZQ8wKvTbN7aCntSfCb5MBVX3NFUBwwBLNy2rEw8JZH`,
  );
}

async function runBaseSepoliaDemo() {
  console.log("=== Agent Bounties SDK Demo — Base Sepolia (TypeScript) ===\n");
  console.log("⚠️  This requires a funded USDC escrow contract.\n");

  const manifest = await discover(BASE_SEPOLIA_API);
  const escrowContract =
    process.env.BASE_SEPOLIA_ESCROW || "0x1111111111111111111111111111111111111111";
  const usdcToken =
    process.env.BASE_SEPOLIA_USDC || "0x3333333333333333333333333333333333333333";

  console.log(`   Escrow: ${escrowContract}`);
  console.log(`   USDC: ${usdcToken}`);
  console.log(`   Network: base-sepolia\n`);

  const bounty = await openPooledBounty(
    BASE_SEPOLIA_API,
    "[bounty]: TypeScript SDK proof",
    "Automated agent contribution in TypeScript",
    20_000_000, // 20 USDC in 6-decimal
    "write-docs-for-area",
    "BaseUsdcEscrow",
  );
  console.log(`   Created: ${bounty.id}\n`);

  const plan = await planBaseFunding(
    BASE_SEPOLIA_API,
    bounty.id,
    20,
    escrowContract,
    usdcToken,
  );
  console.log(`   Funding plan: ${JSON.stringify(plan, null, 2).slice(0, 300)}\n`);
  console.log("⚠️  Sign, broadcast, wait for EscrowCreated index, then claim.");
  console.log(
    `ETH: 0x0b86d893a652408cdaaf2021152db33b74fd0d25\nSOL: H7ZQ8wKvTbN7aCntSfCb5MBVX3NFUBwwBLNy2rEw8JZH`,
  );
}

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

async function main() {
  const args = process.argv.slice(2);
  const api = args.includes("--api") ? args[args.indexOf("--api") + 1] : LOCAL_API;

  if (args.includes("--discover")) {
    const manifest = await discover(api);
    console.log(JSON.stringify(manifest, null, 2));
    return;
  }

  if (args.includes("--base-sepolia")) {
    await runBaseSepoliaDemo();
  } else {
    await runSimulatedDemo();
  }
}

main().catch(console.error);
