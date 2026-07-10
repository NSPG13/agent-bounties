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
  setTimeout,
  URLSearchParams,
};
context.window = context;
vm.createContext(context);
vm.runInContext(source, context, { filename: "site/main.js" });

const wallet = context.window.AgentBountiesBaseWallet;
const config = wallet.baseMainnetWalletConfig;
const bountyId = "31f83d55-f388-4cc8-b384-651403b71163";
const payer = "0x0a8a657ba581f7a7c7dcd59ef637b7fbc6249850";
const otherAddress = "0x1111111111111111111111111111111111111111";
const termsHash = "0x886ef7e42d7f2968ae5b44a9f963a95b4726d8d7244a6fbe35af6f2cffbbc7af";
const amountMinor = 1_000_000;
const escrowTxHash = `0x${"ab".repeat(32)}`;
const otherEscrowTxHash = `0x${"cd".repeat(32)}`;

function clone(value) {
  return JSON.parse(JSON.stringify(value));
}

function hex(value) {
  return String(value || "").replace(/^0x/i, "").toLowerCase();
}

function word(value) {
  return hex(value).padStart(64, "0");
}

function wordAddress(address) {
  return word(hex(address));
}

function wordUint(value) {
  return BigInt(value).toString(16).padStart(64, "0");
}

function uuidBytes32(value) {
  return hex(String(value).replace(/-/g, "")).padStart(64, "0");
}

function encodeApprove(spender, amount) {
  return `0x095ea7b3${wordAddress(spender)}${wordUint(amount)}`;
}

function encodeCreateEscrow(id, token, amount, hash) {
  return `0x64a20554${uuidBytes32(id)}${wordAddress(token)}${wordUint(amount)}${word(hash)}`;
}

function evidenceFixture(overrides = {}) {
  return {
    amount: String(amountMinor),
    bountyId: `0x${uuidBytes32(bountyId)}`,
    logIndex: "0x0",
    onchainEscrowId: "7",
    payer,
    termsHash,
    token: config.nativeUsdc.toLowerCase(),
    txHash: escrowTxHash,
    ...overrides,
  };
}

function escrowCreatedLog(evidence = evidenceFixture()) {
  return {
    address: config.escrowContract,
    topics: [
      config.escrowCreatedTopic,
      `0x${wordUint(evidence.onchainEscrowId)}`,
      evidence.bountyId,
      `0x${wordAddress(evidence.payer)}`,
    ],
    data: `0x${wordAddress(evidence.token)}${wordUint(evidence.amount)}${word(evidence.termsHash)}`,
    transactionHash: evidence.txHash,
    blockNumber: "0x10",
    logIndex: evidence.logIndex || "0x0",
  };
}

function escrowReceipt(evidence = evidenceFixture()) {
  return {
    status: "0x1",
    transactionHash: evidence.txHash,
    logs: [escrowCreatedLog(evidence)],
  };
}

function hostedEscrow(evidence = evidenceFixture(), overrides = {}) {
  return {
    id: `escrow-${evidence.onchainEscrowId}`,
    bounty_id: bountyId,
    rail: "BaseUsdc",
    token: config.nativeUsdc,
    amount: { amount: amountMinor, currency: "usdc" },
    status: "Funded",
    external_reference: `base:${evidence.onchainEscrowId}`,
    ...overrides,
  };
}

function hostedEscrowEvent(evidence = evidenceFixture(), overrides = {}) {
  return {
    id: `event-${evidence.onchainEscrowId}`,
    log_key: `${evidence.txHash}:${evidence.logIndex || "0x0"}`,
    tx_hash: evidence.txHash,
    block_number: 16,
    onchain_escrow_id: Number(evidence.onchainEscrowId),
    bounty_id: bountyId,
    kind: "Created",
    status: "Funded",
    token: config.nativeUsdc,
    amount: { amount: amountMinor, currency: "usdc" },
    terms_hash: termsHash,
    ...overrides,
  };
}

function bounty(overrides = {}) {
  return {
    id: bountyId,
    title: "Wallet funding test bounty",
    amount: { amount: amountMinor, currency: "usdc" },
    funding_mode: "BaseUsdcEscrow",
    funding_targets: [],
    status: "Unfunded",
    terms_hash: termsHash,
    ...overrides,
  };
}

function validPlan() {
  return {
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
      amount: { amount: amountMinor, currency: "usdc" },
      terms_hash: termsHash,
    },
    funding: {
      approve: {
        from: payer,
        to: config.nativeUsdc,
        value_wei: 0,
        data: encodeApprove(config.escrowContract, amountMinor),
        function: "approve(address,uint256)",
      },
      create_escrow: {
        from: payer,
        to: config.escrowContract,
        value_wei: 0,
        data: encodeCreateEscrow(bountyId, config.nativeUsdc, amountMinor, termsHash),
        function: "createEscrow(bytes32,address,uint256,bytes32)",
      },
    },
  };
}

function statusReport({ reconciled = false, evidence = evidenceFixture(), otherEscrow = false } = {}) {
  const activeEvidence = reconciled ? evidence : null;
  const raceEvidence = otherEscrow ? evidenceFixture({
    onchainEscrowId: "8",
    payer: otherAddress,
    txHash: otherEscrowTxHash,
  }) : null;
  const escrows = [
    ...(activeEvidence ? [hostedEscrow(activeEvidence)] : []),
    ...(raceEvidence ? [hostedEscrow(raceEvidence)] : []),
  ];
  const baseEscrowEvents = [
    ...(activeEvidence ? [hostedEscrowEvent(activeEvidence)] : []),
    ...(raceEvidence ? [hostedEscrowEvent(raceEvidence)] : []),
  ];
  const claimable = escrows.length > 0;
  return {
    bounty: bounty({ status: claimable ? "Claimable" : "Unfunded" }),
    funding_summary: {
      applied: { amount: claimable ? amountMinor : 0, currency: "usdc" },
      remaining: { amount: claimable ? 0 : amountMinor, currency: "usdc" },
      claimable,
      partitions: [
        {
          rail: "BaseUsdc",
          confirmed: { amount: claimable ? amountMinor : 0, currency: "usdc" },
          remaining: { amount: claimable ? 0 : amountMinor, currency: "usdc" },
          escrow_count: escrows.length,
          claimable,
        },
      ],
    },
    escrows,
    base_escrow_events: baseEscrowEvents,
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
        assert(handler.length > 0, `unexpected provider call ${request.method}`);
        const next = handler.shift();
        return typeof next === "function" ? next(request.params) : next;
      }
      if (typeof handler === "function") {
        return handler(request.params);
      }
      if (handler !== undefined) {
        return handler;
      }
      throw new Error(`unexpected provider call ${request.method}`);
    },
  };
}

function mockFetch(responses) {
  const calls = [];
  const fetchImpl = async (url, options) => {
    calls.push({ url, options });
    assert(responses.length > 0, `unexpected fetch ${url}`);
    const response = responses.shift();
    if (response.ok === false) {
      return { ok: false, status: response.status || 500, json: async () => response.body || {} };
    }
    return { ok: true, status: 200, json: async () => clone(response) };
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

function sendCount(provider) {
  return provider.calls.filter((call) => call.method === "eth_sendTransaction").length;
}

async function reviewValidPlan(provider = mockProvider({
  eth_chainId: "0x2105",
  eth_accounts: () => [payer],
})) {
  return wallet.prepareBaseWalletFundingPlan({
    apiBaseUrl: "https://api.example.com",
    bountyId,
    connectedAddress: payer,
    fetchImpl: mockFetch([statusReport(), validPlan()]),
    provider,
  });
}

async function assertBadPlanDoesNotSend(label, mutatePlan, pattern) {
  const plan = validPlan();
  mutatePlan(plan);
  const provider = mockProvider({
    eth_chainId: "0x2105",
    eth_accounts: () => [payer],
    eth_sendTransaction: () => {
      throw new Error(`unexpected send for ${label}`);
    },
  });
  await assertRejects(
    wallet.prepareBaseWalletFundingPlan({
      apiBaseUrl: "https://api.example.com",
      bountyId,
      connectedAddress: payer,
      fetchImpl: mockFetch([statusReport(), plan]),
      provider,
    }),
    pattern,
  );
  assert.strictEqual(sendCount(provider), 0, `${label} sent a transaction`);
}

(async () => {
  assert(wallet, "wallet helper export missing");
  assert.strictEqual(
    wallet.decodeBaseWalletPlanCalldata(validPlan()).approve.spender,
    config.escrowContract.toLowerCase(),
  );

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

  const review = await reviewValidPlan();
  assert.strictEqual(review.heading, "ready for human wallet review");
  assert(review.lines.includes("Approval spender decoded from calldata"));
  assert(review.lines.includes("Create escrow calldata"));

  await assertBadPlanDoesNotSend(
    "approval spender word mismatch",
    (plan) => {
      plan.funding.approve.data = encodeApprove(otherAddress, amountMinor);
    },
    /approval calldata spender/,
  );
  await assertBadPlanDoesNotSend(
    "approval amount word mismatch",
    (plan) => {
      plan.funding.approve.data = encodeApprove(config.escrowContract, amountMinor + 1);
    },
    /calldata amount/,
  );
  await assertBadPlanDoesNotSend(
    "createEscrow bounty id word mismatch",
    (plan) => {
      plan.funding.create_escrow.data = encodeCreateEscrow(
        "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
        config.nativeUsdc,
        amountMinor,
        termsHash,
      );
    },
    /calldata bounty id/,
  );
  await assertBadPlanDoesNotSend(
    "createEscrow token word mismatch",
    (plan) => {
      plan.funding.create_escrow.data = encodeCreateEscrow(bountyId, otherAddress, amountMinor, termsHash);
    },
    /calldata token/,
  );
  await assertBadPlanDoesNotSend(
    "createEscrow amount word mismatch",
    (plan) => {
      plan.funding.create_escrow.data = encodeCreateEscrow(bountyId, config.nativeUsdc, amountMinor + 1, termsHash);
    },
    /calldata amount/,
  );
  await assertBadPlanDoesNotSend(
    "createEscrow terms hash word mismatch",
    (plan) => {
      plan.funding.create_escrow.data = encodeCreateEscrow(
        bountyId,
        config.nativeUsdc,
        amountMinor,
        `0x${"2".repeat(64)}`,
      );
    },
    /calldata terms hash/,
  );
  await assertBadPlanDoesNotSend(
    "escrow target mismatch",
    (plan) => {
      plan.funding.create_escrow.to = otherAddress;
    },
    /escrow target/,
  );
  await assertBadPlanDoesNotSend(
    "nonzero ETH value",
    (plan) => {
      plan.funding.approve.value_wei = 1;
    },
    /zero ETH value/,
  );
  await assertBadPlanDoesNotSend(
    "missing terms hash",
    (plan) => {
      plan.create.terms_hash = "";
    },
    /terms hash/,
  );

  const approvalProvider = mockProvider({
    eth_chainId: "0x2105",
    eth_accounts: () => [payer],
    eth_sendTransaction: ["0xapprovehash"],
    eth_getTransactionReceipt: [{ status: "0x1" }],
  });
  const approval = await wallet.sendBaseWalletApproval({
    provider: approvalProvider,
    review,
    receiptDelayMs: 0,
  });
  assert.strictEqual(approval.heading, "approval confirmed");
  assert.strictEqual(sendCount(approvalProvider), 1);

  const changedChainProvider = mockProvider({
    eth_chainId: "0x1",
    eth_accounts: () => [payer],
  });
  await assertRejects(
    wallet.sendBaseWalletApproval({ provider: changedChainProvider, review, receiptDelayMs: 0 }),
    /Base mainnet/,
  );
  assert.strictEqual(sendCount(changedChainProvider), 0);

  const changedAccountProvider = mockProvider({
    eth_chainId: "0x2105",
    eth_accounts: () => [otherAddress],
  });
  await assertRejects(
    wallet.sendBaseWalletApproval({ provider: changedAccountProvider, review, receiptDelayMs: 0 }),
    /account changed/,
  );
  assert.strictEqual(sendCount(changedAccountProvider), 0);

  const approvalRejectProvider = mockProvider({
    eth_chainId: "0x2105",
    eth_accounts: () => [payer],
    eth_sendTransaction: () => {
      throw new Error("User rejected approval");
    },
  });
  await assertRejects(
    wallet.sendBaseWalletApproval({ provider: approvalRejectProvider, review, receiptDelayMs: 0 }),
    /User rejected approval/,
  );
  assert.strictEqual(sendCount(approvalRejectProvider), 1);

  const approvalRevertProvider = mockProvider({
    eth_chainId: "0x2105",
    eth_accounts: () => [payer],
    eth_sendTransaction: ["0xapprovehash"],
    eth_getTransactionReceipt: [{ status: "0x0" }],
  });
  await assertRejects(
    wallet.sendBaseWalletApproval({ provider: approvalRevertProvider, review, receiptDelayMs: 0 }),
    /reverted/,
  );
  assert.strictEqual(sendCount(approvalRevertProvider), 1);

  const betweenSendProvider = mockProvider({
    eth_chainId: ["0x2105", "0x1"],
    eth_accounts: [[payer], [payer]],
    eth_sendTransaction: ["0xapprovehash"],
    eth_getTransactionReceipt: [{ status: "0x1" }],
  });
  await wallet.sendBaseWalletApproval({ provider: betweenSendProvider, review, receiptDelayMs: 0 });
  await assertRejects(
    wallet.sendBaseWalletEscrow({
      fetchImpl: mockFetch([]),
      provider: betweenSendProvider,
      review,
      pollDelayMs: 0,
    }),
    /Base mainnet/,
  );
  assert.strictEqual(sendCount(betweenSendProvider), 1);

  const escrowRejectProvider = mockProvider({
    eth_chainId: "0x2105",
    eth_accounts: () => [payer],
    eth_sendTransaction: () => {
      throw new Error("User rejected escrow");
    },
  });
  await assertRejects(
    wallet.sendBaseWalletEscrow({
      fetchImpl: mockFetch([]),
      provider: escrowRejectProvider,
      review,
      pollDelayMs: 0,
    }),
    /User rejected escrow/,
  );
  assert.strictEqual(sendCount(escrowRejectProvider), 1);

  const escrowRevert = await wallet.sendBaseWalletEscrow({
    fetchImpl: mockFetch([]),
    provider: mockProvider({
      eth_chainId: "0x2105",
      eth_accounts: () => [payer],
      eth_sendTransaction: [escrowTxHash],
      eth_getTransactionReceipt: { status: "0x0" },
    }),
    review,
    pollDelayMs: 0,
  });
  assert.strictEqual(escrowRevert.heading, "needs operator review");
  assert(escrowRevert.lines.includes("reverted"));

  const pendingFetch = mockFetch([statusReport(), statusReport()]);
  const expectedEvidence = evidenceFixture({ txHash: escrowTxHash });
  const pending = await wallet.sendBaseWalletEscrow({
    fetchImpl: pendingFetch,
    pollAttempts: 2,
    pollDelayMs: 0,
    provider: mockProvider({
      eth_chainId: "0x2105",
      eth_accounts: () => [payer],
      eth_sendTransaction: [escrowTxHash],
      eth_getTransactionReceipt: escrowReceipt(expectedEvidence),
    }),
    review,
    shareUrl: "https://example.com/share",
  });
  assert.strictEqual(pending.heading, "waiting for confirmations");
  assert.strictEqual(pendingFetch.calls.length, 2);
  assert(!pending.lines.includes("Share link"));

  const raceFetch = mockFetch([statusReport({ otherEscrow: true })]);
  const race = await wallet.sendBaseWalletEscrow({
    fetchImpl: raceFetch,
    pollAttempts: 1,
    pollDelayMs: 0,
    provider: mockProvider({
      eth_chainId: "0x2105",
      eth_accounts: () => [payer],
      eth_sendTransaction: [escrowTxHash],
      eth_getTransactionReceipt: escrowReceipt(expectedEvidence),
    }),
    review,
    shareUrl: "https://example.com/share",
  });
  assert.strictEqual(race.heading, "waiting for confirmations");
  assert(race.lines.includes("Whole bounty claimable: yes"));
  assert(!race.lines.includes("Share link"));

  const pendingRaceFetch = mockFetch([statusReport({ otherEscrow: true })]);
  await assertRejects(
    wallet.sendBaseWalletEscrow({
      fetchImpl: pendingRaceFetch,
      pollDelayMs: 0,
      provider: mockProvider({
        eth_chainId: "0x2105",
        eth_accounts: () => [payer],
        eth_sendTransaction: [escrowTxHash],
        eth_getTransactionReceipt: [null, null],
      }),
      receiptAttempts: 2,
      receiptDelayMs: 0,
      review,
      shareUrl: "https://example.com/share",
    }),
    /did not produce a successful receipt/,
  );
  assert.strictEqual(pendingRaceFetch.calls.length, 0);

  const reconciledFetch = mockFetch([
    statusReport(),
    statusReport(),
    statusReport({ reconciled: true, evidence: expectedEvidence }),
  ]);
  const reconciled = await wallet.sendBaseWalletEscrow({
    fetchImpl: reconciledFetch,
    pollAttempts: 3,
    pollDelayMs: 0,
    provider: mockProvider({
      eth_chainId: "0x2105",
      eth_accounts: () => [payer],
      eth_sendTransaction: [escrowTxHash],
      eth_getTransactionReceipt: escrowReceipt(expectedEvidence),
    }),
    review,
    shareUrl: "https://example.com/share",
  });
  assert.strictEqual(reconciled.heading, "funding reconciled");
  assert.strictEqual(reconciledFetch.calls.length, 3);
  assert.strictEqual(reconciled.escrowEvidence.txHash, escrowTxHash);
  assert(reconciled.lines.includes("Share link: https://example.com/share"));
  assert(reconciled.lines.includes("Default CTA: Post your own bounty."));

  const genericClaimableWithoutBaseEscrow = wallet.baseWalletStatusLines({
    bounty: bounty({ status: "Claimable" }),
    funding_summary: {
      claimable: true,
      applied: { amount: 1, currency: "usd" },
      remaining: { amount: 0, currency: "usd" },
    },
    escrows: [],
  }, { shareUrl: "https://example.com/share" });
  assert(genericClaimableWithoutBaseEscrow.includes("State: waiting for confirmations"));
  assert(!genericClaimableWithoutBaseEscrow.includes("Share link"));

  const busyState = { busy: false };
  let releaseBusy;
  const firstAction = wallet.runExclusiveWalletAction(
    busyState,
    () => new Promise((resolve) => {
      releaseBusy = resolve;
    }),
  );
  await Promise.resolve();
  await assertRejects(
    wallet.runExclusiveWalletAction(busyState, async () => "second"),
    /already in progress/,
  );
  releaseBusy("first");
  assert.strictEqual(await firstAction, "first");
  assert.strictEqual(busyState.busy, false);

  console.log("base wallet flow tests passed");
})();
