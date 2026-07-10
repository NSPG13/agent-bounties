const assert = require("assert");
const fs = require("fs");
const path = require("path");
const vm = require("vm");

const repoRoot = path.resolve(__dirname, "..");
const source = fs.readFileSync(path.join(repoRoot, "site", "main.js"), "utf8");

const context = {
  console,
  document: {
    getElementById() {
      return null;
    },
  },
  URLSearchParams,
};
context.window = context;
vm.createContext(context);
vm.runInContext(source, context, { filename: "site/main.js" });

const wallet = context.window.AgentBountiesBaseWallet;
const config = wallet.baseMainnetWalletConfig;
const bountyId = "31f83d55-f388-4cc8-b384-651403b71163";
const payer = "0x0a8a657ba581f7a7c7dcd59ef637b7fbc6249850";
const termsHash = "886ef7e42d7f2968ae5b44a9f963a95b4726d8d7244a6fbe35af6f2cffbbc7af";

function clone(value) {
  return JSON.parse(JSON.stringify(value));
}

function bounty(overrides = {}) {
  return {
    id: bountyId,
    amount: { amount: 1_000_000, currency: "usdc" },
    funding_mode: "BaseUsdcEscrow",
    funding_targets: [],
    status: "Unfunded",
    terms_hash: termsHash,
    ...overrides,
  };
}

function validPlan(overrides = {}) {
  const plan = {
    network: {
      name: "Base",
      chain_id: config.chainId,
      native_usdc_token_address: config.nativeUsdc,
    },
    bounty: bounty(),
    create: {
      bounty_id: bountyId,
      payer,
      token: config.nativeUsdc,
      amount: { amount: 1_000_000, currency: "usdc" },
      terms_hash: termsHash,
    },
    funding: {
      approve: {
        from: payer,
        to: config.nativeUsdc,
        value_wei: 0,
        data: "0xapprove",
        function: "approve(address,uint256)",
      },
      create_escrow: {
        from: payer,
        to: config.escrowContract,
        value_wei: 0,
        data: "0xcreate",
        function: "createEscrow(bytes32,address,uint256,bytes32)",
      },
    },
  };
  return Object.assign(plan, overrides);
}

function statusReport({ reconciled = false } = {}) {
  return {
    bounty: bounty({ status: reconciled ? "Claimable" : "Unfunded" }),
    funding_summary: {
      applied: { amount: reconciled ? 1_000_000 : 0, currency: "usdc" },
      remaining: { amount: reconciled ? 0 : 1_000_000, currency: "usdc" },
      claimable: reconciled,
      partitions: [
        {
          rail: "BaseUsdc",
          confirmed: { amount: reconciled ? 1_000_000 : 0, currency: "usdc" },
          remaining: { amount: reconciled ? 0 : 1_000_000, currency: "usdc" },
          escrow_count: reconciled ? 1 : 0,
          claimable: reconciled,
        },
      ],
    },
    escrows: reconciled ? [{ id: "escrow-1" }] : [],
  };
}

function mockProvider(handlers) {
  const calls = [];
  return {
    calls,
    async request(request) {
      calls.push(request);
      const handler = handlers[request.method];
      if (Array.isArray(handler)) {
        const next = handler.shift();
        return typeof next === "function" ? next(request.params) : next;
      }
      if (typeof handler === "function") {
        return handler(request.params);
      }
      return handler;
    },
  };
}

function mockFetch(responses) {
  const calls = [];
  const fetchImpl = async (url, options) => {
    calls.push({ url, options });
    const response = responses.shift();
    if (!response) {
      throw new Error(`unexpected fetch ${url}`);
    }
    if (response.ok === false) {
      return { ok: false, status: response.status || 500, json: async () => response.body || {} };
    }
    return { ok: true, status: 200, json: async () => response };
  };
  fetchImpl.calls = calls;
  return fetchImpl;
}

async function assertRejects(promise, pattern) {
  let rejected = false;
  try {
    await promise;
  } catch (error) {
    rejected = true;
    assert(pattern.test(error.message), `${error.message} did not match ${pattern}`);
  }
  assert(rejected, "expected promise to reject");
}

(async () => {
  await assertRejects(wallet.connectBaseWallet(null), /No injected EVM wallet/);

  await assertRejects(
    wallet.connectBaseWallet(mockProvider({
      eth_requestAccounts: () => [payer],
      eth_chainId: ["0x1"],
      wallet_switchEthereumChain: () => {
        throw new Error("User rejected network switch");
      },
    })),
    /User rejected network switch/,
  );

  await assertRejects(
    wallet.connectBaseWallet(mockProvider({
      eth_requestAccounts: () => {
        throw new Error("User rejected account request");
      },
    })),
    /User rejected account request/,
  );

  const mismatch = validPlan();
  mismatch.network.chain_id = 1;
  mismatch.create.token = "0x9999999999999999999999999999999999999999";
  mismatch.funding.approve.to = mismatch.create.token;
  mismatch.create.payer = "0x1111111111111111111111111111111111111111";
  const issues = wallet.baseWalletPlanValidationIssues(mismatch, {
    bountyId,
    connectedAddress: payer,
    hostedBounty: bounty(),
  });
  assert(issues.some((issue) => issue.includes("Base mainnet chain 8453")));
  assert(issues.some((issue) => issue.includes("connected wallet")));
  assert(issues.some((issue) => issue.includes("native USDC")));

  const pendingProvider = mockProvider({
    eth_chainId: "0x2105",
    eth_sendTransaction: ["0xapprove", "0xescrow"],
    eth_getTransactionReceipt: { status: "0x1" },
  });
  const pendingResult = await wallet.fundBaseWalletBounty({
    apiBaseUrl: "https://api.example.com",
    bountyId,
    connectedAddress: payer,
    fetchImpl: mockFetch([statusReport(), validPlan(), statusReport()]),
    provider: pendingProvider,
  });
  assert.strictEqual(pendingResult.heading, "waiting for confirmations");
  assert.strictEqual(
    pendingProvider.calls.filter((call) => call.method === "eth_sendTransaction").length,
    2,
  );
  assert(pendingResult.lines.includes("Wallet transactions or transaction hashes are not funding evidence"));

  const revertedResult = await wallet.fundBaseWalletBounty({
    apiBaseUrl: "https://api.example.com",
    bountyId,
    connectedAddress: payer,
    fetchImpl: mockFetch([statusReport(), validPlan()]),
    provider: mockProvider({
      eth_chainId: "0x2105",
      eth_sendTransaction: ["0xapprove", "0xescrow"],
      eth_getTransactionReceipt: { status: "0x0" },
    }),
  });
  assert.strictEqual(revertedResult.heading, "needs operator review");
  assert(revertedResult.lines.includes("reverted"));

  const reconciledResult = await wallet.fundBaseWalletBounty({
    apiBaseUrl: "https://api.example.com",
    bountyId,
    connectedAddress: payer,
    fetchImpl: mockFetch([statusReport(), validPlan(), statusReport({ reconciled: true })]),
    provider: mockProvider({
      eth_chainId: "0x2105",
      eth_sendTransaction: ["0xapprove", "0xescrow"],
      eth_getTransactionReceipt: { status: "0x1" },
    }),
  });
  assert.strictEqual(reconciledResult.heading, "funding reconciled");
  assert(reconciledResult.lines.includes("State: funding reconciled"));
  assert(reconciledResult.lines.includes("indexed EscrowCreated evidence"));

  const genericClaimableWithoutBaseEscrow = wallet.baseWalletStatusLines({
    bounty: bounty({ status: "Claimable" }),
    funding_summary: { claimable: true, applied: { amount: 1, currency: "usd" }, remaining: { amount: 0, currency: "usd" } },
    escrows: [],
  });
  assert(genericClaimableWithoutBaseEscrow.includes("State: waiting for confirmations"));

  console.log("base wallet flow tests passed");
})();
