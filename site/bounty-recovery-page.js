"use strict";
(async () => {
  const core = window.AgentBountiesRecovery;
  const ui = Object.fromEntries(["contract", "owner", "balance", "wallet", "contribution", "connect", "send", "refresh", "status", "receipt", "next"].map(name => [name, document.getElementById(`recovery-${name}`)]));
  const api = "https://api.agentbounties.app";
  let contract, provider, account, view, busy = false, walletWaiting = false, refreshing = false, timer;
  const message = text => { ui.status.textContent = text; };
  async function json(url, init) {
    const response = await fetch(url, { ...init, cache: "no-store", credentials: "omit", signal: AbortSignal.timeout(20000) });
    if (!response.ok) throw new Error(response.status === 409 ? "The service is still reconciling this bounty. Check status and try again shortly." : `The recovery service is unavailable (${response.status}). No new transaction was sent.`);
    return response.json();
  }
  const engine = core.create({
    storage: localStorage, locks: navigator.locks,
    rpc: core.createRpc(),
    plan: (action, bounty, caller) => json(`${api}/v1/base/autonomous-bounties/${action === "cancel" ? "cancel-plan" : "refund-withdrawal-plan"}`, {
      method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ network: "base-mainnet", bounty_contract: bounty, caller }),
    }),
  });
  function render() {
    ui.connect.disabled = busy || refreshing;
    ui.send.disabled = true;
    ui.refresh.disabled = refreshing || (busy && !walletWaiting);
    ui.wallet.textContent = account || "Not connected";
    if (!view) { ui.contribution.textContent = account ? "Check status to update" : "Connect to check"; return; }
    ui.owner.textContent = view.creator;
    ui.balance.textContent = `${core.usdc(view.funded)} USDC`;
    ui.contribution.textContent = account ? `${core.usdc(view.contribution)} USDC` : "Connect to check";
    ui.send.textContent = view.state === 5 ? "Return my funds" : "Cancel old bounty";
    if (account && provider && account === view.creator && !busy && !refreshing && !engine.saved(contract, account)) {
      ui.send.disabled = !([0, 1].includes(view.state) || (view.state === 5 && BigInt(view.contribution) > 0n));
    }
  }
  async function refresh() {
    clearTimeout(timer);
    const connected = account;
    let result = connected ? await engine.reconcile(contract, connected) : { status: "idle" };
    const nextView = await engine.snapshot(contract, connected);
    if (connected !== account) return;
    view = nextView;
    ui.next.hidden = !(result.status === "confirmed" && result.action === "refund");
    if (result.txHash) {
      ui.receipt.href = `https://basescan.org/tx/${result.txHash}`;
      ui.receipt.hidden = false;
    }
    if (result.status === "confirmed") message(result.action === "cancel" ? "Cancellation confirmed. Choose Return my funds for the separate refund request." : `${core.usdc(result.amount)} USDC returned to your wallet. The refund is complete.`);
    else if (result.status === "failed") message("The transaction failed. No recovery step was completed. Review the current state before trying again.");
    else if (result.status === "pending") message("Your transaction was sent. Waiting for confirmation on Base; do not send it again.");
    else if (result.status === "unknown") message(walletWaiting ? "Open your wallet to finish the refund or cancellation request. If you already confirmed it, wait for the receipt. Do not send it again." : result.message);
    else if (!account) message("Connect the wallet that created this bounty. Connecting does not cancel or spend anything.");
    else if (account !== view.creator) message("This is not the creator wallet. Connect the creator address shown above.");
    else if ([2, 3].includes(view.state)) message("There is an active claim or submission. This page cannot cancel it or interrupt the solver.");
    else if (view.state === 4) message("This bounty has settled and cannot be cancelled.");
    else if (view.state === 5 && BigInt(view.contribution) === 0n) message("This bounty is cancelled. Your wallet has no funds left to withdraw from it.");
    else message(view.state === 5 ? "Cancellation confirmed on Base. Review Return my funds; the refund goes back to this wallet." : "Ready to cancel. This closes the bounty; your funds stay in it until the separate refund step.");
    render();
    if (result.status === "pending") timer = setTimeout(() => refresh().catch(error => { view = null; message(error.message); render(); }), 10000);
  }
  ui.connect.addEventListener("click", async () => {
    if (busy) return;
    busy = true; render();
    try {
      const choice = await window.AgentBountiesWalletLink.select({ purpose: "recovery" });
      if (choice.provider?.agentBountiesCapabilities?.directTransactions === false) throw new Error("Choose MetaMask or another wallet that supports direct contract calls.");
      message(choice.kind === "phone" ? "Open your phone wallet to connect. This does not move money." : `Open ${choice.label} in your browser. Unlock it and approve the connection if asked.`);
      const selected = await core.connectWallet(choice.provider);
      message("Checking your wallet's network…");
      if (String(await core.walletRead(choice.provider, "eth_chainId")).toLowerCase() !== core.CHAIN) {
        message("Open your wallet and switch to Base. This does not move money.");
        await core.switchToBase(choice.provider);
        if (String(await core.walletRead(choice.provider, "eth_chainId")).toLowerCase() !== core.CHAIN) throw new Error("Your wallet is still on another network. Switch to Base, then connect again.");
      }
      provider = choice.provider; account = selected; view = null;
      const changed = () => { if (provider !== choice.provider) return; provider = null; account = null; clearTimeout(timer); ui.next.hidden = true; ui.send.disabled = true; message("Your wallet changed. Connect it again before continuing."); render(); };
      provider.on?.("accountsChanged", changed); provider.on?.("chainChanged", changed); provider.on?.("disconnect", changed);
      render(); ui.contribution.textContent = "Checking…"; message("Wallet connected. Checking your refundable balance…");
      await refresh();
    } catch (error) { view = null; message(error.code === -32002 ? "A connection request is already open in your wallet. Finish it there, then connect again." : error.code === 4001 ? "Connection cancelled. Choose Connect my wallet when you are ready." : error.message); }
    finally { busy = false; render(); }
  });
  ui.send.addEventListener("click", async (event) => {
    if (!event.isTrusted || busy || refreshing || !provider || !account || !view) return;
    busy = true; render();
    try {
      const action = view.state === 5 ? "refund" : "cancel";
      await engine.request({ contract, account, action, provider, onProgress: stage => {
        walletWaiting = stage === "wallet_approval";
        message(stage === "wallet_check" ? "Checking that your wallet is connected…" : stage === "bounty_check" ? "Checking the bounty and network fee. No wallet approval has been requested yet."
          : `Open your wallet to confirm the ${action === "refund" ? "refund to your own address" : "cancellation"}. Your wallet shows the Base network fee. Do not send another request.`);
        render();
      } });
      walletWaiting = false;
      await refresh();
    } catch (error) { message(error.code === 4001 ? "You declined the wallet request. No transaction was sent." : `${error.message}\nCheck wallet activity before retrying if a request was opened.`); }
    finally { busy = false; walletWaiting = false; render(); }
  });
  ui.refresh.addEventListener("click", async () => {
    if (refreshing || (busy && !walletWaiting)) return;
    refreshing = true; render();
    try { await refresh(); } catch (error) { view = null; message(error.message); }
    finally { refreshing = false; render(); }
  });
  try {
    contract = core.address(new URLSearchParams(location.search).get("bountyContract"));
    ui.contract.textContent = contract;
    await refresh();
  } catch (error) { view = null; message(error.message); render(); }
})();
