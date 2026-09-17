/* Legacy autonomous-v1 recovery. Wallet confirmation is always required. */
"use strict";
(function (root, factory) {
  if (typeof module === "object" && module.exports) module.exports = factory;
  else root.AgentBountiesRecovery = factory(root.AgentBountiesEvm);
})(typeof window === "object" ? window : globalThis, function createRecovery(evm) {
  const CHAIN = "0x2105";
  const FACTORY = "0x082c52131aaf0c56e76b075f895eab6fcab6d2f9";
  const IMPLEMENTATION = "0x2fa36d2b2327642db3a6cc8cdd91544ad7484eb9";
  const TOKEN = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913";
  const CODE = `0x363d3d373d3d3d363d73${IMPLEMENTATION.slice(2)}5af43d82803e903d91602b57fd5bf3`;
  const address = value => {
    const result = String(value || "").toLowerCase();
    if (!/^0x[0-9a-f]{40}$/.test(result) || /^0x0{40}$/.test(result)) throw new Error("Invalid wallet or bounty address.");
    return result;
  };
  const hash = value => {
    if (!/^0x[0-9a-f]{64}$/i.test(String(value || ""))) throw new Error("Invalid transaction hash.");
    return value.toLowerCase();
  };
  const selector = name => evm.keccak256Hex(evm.textHex(name)).slice(0, 10);
  const topic = name => evm.keccak256Hex(evm.textHex(name));
  const uint = value => {
    if (!/^0x[0-9a-f]{64}$/i.test(String(value))) throw new Error("The chain returned an invalid value.");
    return BigInt(value);
  };
  const readAddress = value => {
    uint(value);
    if (!/^0x0{24}/i.test(value)) throw new Error("The chain returned an invalid address.");
    return address(`0x${value.slice(-40)}`);
  };
  const usdc = value => {
    const n = BigInt(value);
    return `${n / 1000000n}.${(n % 1000000n).toString().padStart(6, "0")}`;
  };
  // This transport can only read or simulate. Wallet writes never use retries.
  function createRpc(fetcher = fetch) {
    const endpoints = ["https://base-rpc.publicnode.com", "https://mainnet.base.org"];
    const allowed = new Set(["eth_chainId", "eth_getBlockByNumber", "eth_getCode", "eth_call", "eth_estimateGas", "eth_getTransactionReceipt", "eth_getTransactionByHash"]);
    let nextId = 0, tail = Promise.resolve(), preferred = 0;
    async function read(method, params) {
      if (!allowed.has(method)) throw new Error("Recovery RPC only supports reads and simulations.");
      const id = ++nextId, first = preferred;
      for (let attempt = 0; attempt < endpoints.length; attempt++) {
        const index = (first + attempt) % endpoints.length;
        let response;
        try {
          response = await fetcher(endpoints[index], { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ jsonrpc: "2.0", id, method, params }), cache: "no-store", credentials: "omit", signal: AbortSignal.timeout(12000) });
        } catch (_) { continue; }
        if (response.status === 429 || response.status >= 500) continue;
        if (!response.ok) throw new Error("Base could not verify this request. Check status before trying again.");
        const result = await response.json();
        if (result.error?.code === -32005) continue;
        if (result.jsonrpc !== "2.0" || result.id !== id || result.error || !("result" in result)) throw new Error("Base could not verify this request. Check status before trying again.");
        preferred = index;
        return result.result;
      }
      throw new Error("Base is busy or unavailable. Check status shortly; no wallet request was repeated.");
    }
    return (method, params) => {
      const pending = tail.then(() => read(method, params));
      tail = pending.catch(() => {});
      return pending;
    };
  }
  function create({ rpc, plan, storage, locks, now = () => Date.now() }) {
    async function snapshot(contract, account, tag = "safe") {
      contract = address(contract); account = account ? address(account) : null;
      if (await rpc("eth_chainId", []) !== CHAIN) throw new Error("The chain service is not Base.");
      const block = await rpc("eth_getBlockByNumber", [tag, false]);
      if (!block || !/^0x[0-9a-f]+$/i.test(block.number) || !/^0x[0-9a-f]{64}$/i.test(block.hash)) throw new Error("A confirmed Base block is unavailable.");
      const at = block.number;
      const call = (name, args = "", to = contract) => rpc("eth_call", [{ to, data: selector(name) + args }, at]);
      const [code, canonical, factory, token, creator, bountyId, status, funded, pool] = await Promise.all([
        rpc("eth_getCode", [contract, at]), call("isCanonicalBounty(address)", evm.addressWord(contract), FACTORY),
        call("factory()"), call("settlementToken()"), call("creator()"), call("bountyId()"),
        call("status()"), call("fundedAmount()"), call("timeoutBondPool()"),
      ]);
      if (code.toLowerCase() !== CODE || uint(canonical) !== 1n || readAddress(factory) !== FACTORY || readAddress(token) !== TOKEN) throw new Error("This is not a supported canonical bounty. No wallet request was sent.");
      const owner = readAddress(creator), state = Number(uint(status));
      if (state < 0 || state > 5) throw new Error("Unknown bounty state.");
      const contribution = account ? uint(await call("contributions(address)", evm.addressWord(account))) : 0n;
      const checkedBlock = await rpc("eth_getBlockByNumber", [at, false]);
      if (checkedBlock?.hash?.toLowerCase() !== block.hash.toLowerCase()) throw new Error("The Base block changed. Refresh before continuing.");
      return { contract, creator: owner, bountyId: hash(bountyId), state, funded: uint(funded).toString(), pool: uint(pool).toString(), contribution: contribution.toString(), block: at, blockHash: block.hash };
    }
    const key = (contract, account) => `agentbounties:recovery:v1:${address(contract)}:${address(account)}`;
    function saved(contract, account) {
      const raw = storage.getItem(key(contract, account));
      if (!raw) return null;
      let value;
      try { value = JSON.parse(raw); } catch (_) { throw new Error("The saved wallet request cannot be read. Stop and check wallet activity."); }
      if (value.contract !== address(contract) || value.account !== address(account) || !["cancel", "refund"].includes(value.action) || !/^0x[0-9a-f]+$/i.test(value.startBlock)) throw new Error("The saved wallet request is invalid. Stop and check wallet activity.");
      if (value.txHash) hash(value.txHash);
      return value;
    }
    function save(record) {
      const name = key(record.contract, record.account), encoded = JSON.stringify(record);
      storage.setItem(name, encoded);
      if (storage.getItem(name) !== encoded) throw new Error("This browser cannot save recovery progress. No new request can be sent safely.");
    }
    async function walletContext(provider, account) {
      const [chain, accounts] = await Promise.all([provider.request({ method: "eth_chainId" }), provider.request({ method: "eth_accounts" })]);
      if (String(chain).toLowerCase() !== CHAIN || address(accounts?.[0]) !== address(account)) throw new Error("Your wallet or network changed. Reconnect the original wallet on Base.");
    }
    function eligible(view, account, action) {
      if (address(account) !== view.creator) throw new Error("Connect the wallet that created this bounty.");
      if (action === "cancel" && ![0, 1].includes(view.state)) throw new Error("This bounty cannot be cancelled while claimed, under review, settled, or already cancelled.");
      if (action === "refund" && (view.state !== 5 || BigInt(view.contribution) <= 0n)) throw new Error("A confirmed cancellation and a remaining contribution are required for a refund.");
    }
    async function request({ contract, account, action, provider }) {
      contract = address(contract); account = address(account);
      if (!["cancel", "refund"].includes(action)) throw new Error("Unsupported recovery action.");
      if (!locks?.request) throw new Error("This browser cannot safely coordinate wallet requests. Use a supported browser.");
      return locks.request(key(contract, account), { ifAvailable: true }, async lock => {
        if (!lock) throw new Error("A recovery request is already open in another tab.");
        if (saved(contract, account)) throw new Error("Check the previous wallet request before sending another.");
        await walletContext(provider, account);
        const view = await snapshot(contract, account);
        eligible(view, account, action);
        const expected = selector(action === "cancel" ? "cancel()" : "withdrawRefund()");
        const intent = await plan(action, contract, account);
        if (address(intent.from) !== account || address(intent.to) !== contract || BigInt(intent.value_wei) !== 0n || intent.data?.toLowerCase() !== expected) throw new Error("The planned wallet call does not match the action you reviewed.");
        const latest = await snapshot(contract, account, "latest");
        eligible(latest, account, action);
        if (latest.bountyId !== view.bountyId || latest.funded !== view.funded || latest.pool !== view.pool || latest.contribution !== view.contribution) throw new Error("The bounty balance changed. Refresh and review it again.");
        const tx = { from: account, to: contract, value: "0x0", data: expected, chainId: CHAIN };
        await rpc("eth_estimateGas", [tx]);
        await walletContext(provider, account);
        const record = { contract, account, action, startBlock: view.block, bountyId: view.bountyId, createdAt: now(), txHash: null };
        save(record); // Before opening the wallet: a lost response must not cause a repeat.
        try {
          record.txHash = hash(await provider.request({ method: "eth_sendTransaction", params: [tx] }));
          save(record);
          return record;
        } catch (error) {
          if (error?.code === 4001) storage.removeItem(key(contract, account));
          throw error;
        }
      });
    }
    async function reconcile(contract, account) {
      const record = saved(contract, account);
      if (!record) return { status: "idle" };
      if (!record.txHash) return { status: "unknown", message: "The wallet response was lost. Check wallet activity; do not send another request." };
      const [receipt, safe] = await Promise.all([
        rpc("eth_getTransactionReceipt", [record.txHash]), rpc("eth_getBlockByNumber", ["safe", false]),
      ]);
      if (!receipt || !safe || BigInt(receipt.blockNumber) > BigInt(safe.number)) return { status: "pending", txHash: record.txHash };
      const [block, transaction] = await Promise.all([
        rpc("eth_getBlockByNumber", [receipt.blockNumber, false]), rpc("eth_getTransactionByHash", [record.txHash]),
      ]);
      const expectedData = selector(record.action === "cancel" ? "cancel()" : "withdrawRefund()");
      if (!block || block.hash?.toLowerCase() !== receipt.blockHash?.toLowerCase()) return { status: "pending", txHash: record.txHash };
      if (!transaction || hash(receipt.transactionHash) !== record.txHash || hash(transaction.hash) !== record.txHash || address(transaction.from) !== record.account || address(transaction.to) !== record.contract || transaction.input?.toLowerCase() !== expectedData || BigInt(transaction.value) !== 0n) throw new Error("The transaction does not match the saved recovery request.");
      if (BigInt(receipt.status) === 0n) {
        storage.removeItem(key(contract, account));
        return { status: "failed", txHash: record.txHash };
      }
      if (BigInt(receipt.status) !== 1n) throw new Error("Unknown transaction result.");
      const eventTopic = topic(record.action === "cancel" ? "BountyCancelled(bytes32,uint256)" : "RefundWithdrawn(bytes32,address,uint256,uint256,uint256)");
      const matches = (receipt.logs || []).filter(log => !log.removed && log.address?.toLowerCase() === record.contract && log.topics?.[0]?.toLowerCase() === eventTopic && log.topics?.[1]?.toLowerCase() === record.bountyId);
      if (matches.length !== 1) throw new Error("The expected confirmed recovery event is missing.");
      const event = matches[0];
      if (record.action === "refund" && (event.topics.length !== 3 || event.topics[2]?.toLowerCase() !== `0x${evm.addressWord(record.account)}` || !/^0x[0-9a-f]{192}$/i.test(event.data))) throw new Error("The refund event does not belong to this wallet.");
      if (record.action === "cancel" && (event.topics.length !== 2 || !/^0x[0-9a-f]{64}$/i.test(event.data))) throw new Error("The cancellation event is invalid.");
      const amount = record.action === "refund" ? uint(`0x${event.data.slice(-64)}`).toString() : null;
      const result = { status: "confirmed", action: record.action, txHash: record.txHash, amount };
      storage.setItem(`${key(contract, account)}:confirmed`, JSON.stringify(result));
      storage.removeItem(key(contract, account));
      return result;
    }
    return { snapshot, request, reconcile, saved };
  }
  return Object.freeze({ create, createRpc, address, usdc, CHAIN, FACTORY, IMPLEMENTATION, TOKEN, CODE, selector, topic });
});
