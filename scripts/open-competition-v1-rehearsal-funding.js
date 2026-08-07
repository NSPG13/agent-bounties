(() => {
  "use strict";
  const ADMIN = "0x884834e884d6e93462655a2820140ad03e6747bc";
  const CHAIN = "0x14a34";
  const USDC = "0x036cbd53842c5426634e7929541ec2318f3dcf7e";
  const EXPECTED_ETH = 500000000000000n;
  const EXPECTED_USDC = 500000n;
  const READ_RPC_URLS = ["https://sepolia.base.org", "https://base-sepolia-rpc.publicnode.com"];
  const announced = [];
  let request;
  let provider;
  let account;
  const $ = (id) => document.getElementById(id);
  const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

  function remember(event) {
    const item = event && event.detail;
    if (item && item.provider && typeof item.provider.request === "function"
      && !announced.some((candidate) => candidate.provider === item.provider)) announced.push(item);
  }
  window.addEventListener("eip6963:announceProvider", remember);

  function fail(message) {
    $("status").className = "status bad";
    $("status").textContent = message;
    $("reviewed").disabled = true;
    $("execute").disabled = true;
  }
  function note(message) { $("status").className = "status"; $("status").textContent = message; }
  async function wallet(method, params = []) {
    try { return await provider.request({ method, params }); }
    catch (error) { throw new Error(`${method} failed${error && error.code !== undefined ? ` (${error.code})` : ""}: ${error.message || error}`); }
  }
  async function rpc(method, params = []) {
    const failures = [];
    for (const url of READ_RPC_URLS) {
      try {
        const response = await fetch(url, {
          method: "POST", headers: { "content-type": "application/json" },
          body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params }),
        });
        const payload = await response.json();
        if (!response.ok || payload.error) throw new Error(payload.error ? payload.error.message : `HTTP ${response.status}`);
        return payload.result;
      } catch (error) {
        failures.push(`${url}: ${error.message}`);
      }
    }
    throw new Error(`${method} read failed on every Base Sepolia RPC: ${failures.join("; ")}`);
  }
  async function receipt(hash) {
    for (let attempt = 0; attempt < 180; attempt += 1) {
      const value = await rpc("eth_getTransactionReceipt", [hash]);
      if (value) return value;
      await sleep(1000);
    }
    throw new Error(`Receipt timed out: ${hash}`);
  }
  function transferData(to, amount) {
    return `0xa9059cbb${to.toLowerCase().slice(2).padStart(64, "0")}${amount.toString(16).padStart(64, "0")}`;
  }
  async function load() {
    const response = await fetch("/target/open-competition-v1/base-sepolia-rehearsal-funding.json", { cache: "no-store" });
    if (!response.ok) throw new Error(`Funding request returned HTTP ${response.status}.`);
    request = await response.json();
    if (request.schema_version !== "agent-bounties/open-competition-v1-rehearsal-funding-v1"
      || request.network !== "base-sepolia" || Number(request.chain_id) !== 84532
      || request.from.toLowerCase() !== ADMIN || request.usdc_token.toLowerCase() !== USDC
      || BigInt(request.eth_wei) !== EXPECTED_ETH || BigInt(request.usdc_base_units) !== EXPECTED_USDC
      || !/^0x[0-9a-fA-F]{40}$/.test(request.recipient)) throw new Error("Funding request failed exact validation.");
    $("facts").innerHTML = [
      ["Sender", request.from], ["Recipient", request.recipient],
      ["Base Sepolia ETH", `${request.eth_wei} wei (0.0005 ETH)`],
      ["Test USDC", `${request.usdc_base_units} base units (0.5 USDC)`],
    ].map(([term, value]) => `<dt>${term}</dt><dd>${value}</dd>`).join("");
    $("connect").disabled = false;
    note("Exact local funding request loaded and validated.");
  }
  async function connect() {
    window.dispatchEvent(new Event("eip6963:requestProvider"));
    await sleep(300);
    const selected = announced.find((item) => String(item.info && item.info.rdns).toLowerCase() === "io.metamask")
      || announced.find((item) => item.provider.isMetaMask && !item.provider.isBraveWallet);
    if (!selected) throw new Error("MetaMask EIP-6963 provider not found.");
    provider = selected.provider;
    let chain = String(await wallet("eth_chainId")).toLowerCase();
    if (chain !== CHAIN) { await wallet("wallet_switchEthereumChain", [{ chainId: CHAIN }]); chain = String(await wallet("eth_chainId")).toLowerCase(); }
    if (chain !== CHAIN) throw new Error("MetaMask is not on Base Sepolia.");
    const accounts = await wallet("eth_requestAccounts");
    account = String(accounts[0] || "").toLowerCase();
    if (account !== ADMIN) throw new Error(`Connected account ${account || "(none)"} is not the admin.`);
    $("reviewed").disabled = false;
    note("Correct admin and Base Sepolia are connected. Review the exact bounded funding request.");
  }
  async function execute() {
    $("execute").disabled = true; $("reviewed").disabled = true;
    note("Requesting the exact 0.0005 Base Sepolia ETH transfer...");
    const ethHash = await wallet("eth_sendTransaction", [{ from: account, to: request.recipient, value: `0x${EXPECTED_ETH.toString(16)}` }]);
    const ethReceipt = await receipt(ethHash);
    if (Number.parseInt(ethReceipt.status, 16) !== 1) throw new Error("ETH funding reverted.");
    note("Requesting the exact 0.5 test USDC transfer...");
    const usdcHash = await wallet("eth_sendTransaction", [{ from: account, to: USDC, value: "0x0", data: transferData(request.recipient, EXPECTED_USDC) }]);
    const usdcReceipt = await receipt(usdcHash);
    if (Number.parseInt(usdcReceipt.status, 16) !== 1) throw new Error("USDC funding reverted.");
    const eth = BigInt(await rpc("eth_getBalance", [request.recipient, "latest"]));
    const balanceCall = `0x70a08231${request.recipient.toLowerCase().slice(2).padStart(64, "0")}`;
    const usdc = BigInt(await rpc("eth_call", [{ to: USDC, data: balanceCall }, "latest"]));
    if (eth !== EXPECTED_ETH || usdc !== EXPECTED_USDC) throw new Error("Canonical funding balances do not exactly match.");
    note(`Both exact testnet transfers are confirmed. ETH ${ethHash}; USDC ${usdcHash}.`);
  }
  $("connect").addEventListener("click", () => connect().catch((error) => fail(error.message)));
  $("reviewed").addEventListener("change", () => { $("execute").disabled = !$("reviewed").checked || account !== ADMIN; });
  $("execute").addEventListener("click", () => execute().catch((error) => fail(error.message)));
  load().catch((error) => fail(error.message));
})();
