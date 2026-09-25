"use strict";
(async () => {
  const core = window.AgentBountiesRecovery;
  const ui = Object.fromEntries(["contract", "owner", "balance", "wallet", "contribution", "connect", "send", "refresh", "status", "receipt", "next"].map(name => [name, document.getElementById(`recovery-${name}`)]));
  const api = "https://api.agentbounties.app";
  const params = new URLSearchParams(location.search), recoveryAction = params.get("action"), requestedRound = params.get("round");
  const timeoutMode = recoveryAction !== null;
  const timeoutReady = () => timeoutMode && view?.round === requestedRound && core.canExpire(view, recoveryAction);
  let contract, provider, account, view, busy = false, timer;
  const message = text => { ui.status.textContent = text; };
  async function json(url, init) {
    const response = await fetch(url, { ...init, cache: "no-store", credentials: "omit", signal: AbortSignal.timeout(20000) });
    if (!response.ok) throw new Error(response.status === 409 ? "The service is still reconciling this bounty. Check status and try again shortly." : `The recovery service is unavailable (${response.status}). No new transaction was sent.`);
    return response.json();
  }
  const engine = core.create({
    storage: localStorage, locks: navigator.locks,
    rpc: core.createRpc(),
    plan: (action, bounty, caller) => json(`${api}/v1/base/autonomous-bounties/${core.ACTIONS[action].endpoint}`, {
      method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ network: "base-mainnet", bounty_contract: bounty, caller }),
    }),
  });
  function render() {
    ui.connect.disabled = busy || !view;
    ui.send.disabled = true;
    ui.refresh.disabled = busy;
    if (!view) return;
    ui.owner.textContent = view.creator;
    ui.balance.textContent = `${core.usdc(view.funded)} USDC`;
    ui.wallet.textContent = account || "Not connected";
    ui.contribution.textContent = account ? `${core.usdc(view.contribution)} USDC` : "Connect to check";
    if (timeoutMode) {
      ui.send.textContent = "Reopen bounty";
      ui.send.disabled = !account || !provider || busy || !timeoutReady() || Boolean(engine.saved(contract, account));
      document.getElementById("recovery-bond").textContent = view.round === requestedRound
        ? `Round ${view.round} · ${core.usdc(view.activeBond)} USDC bond · solver ${view.solver}` : `The displayed round ${requestedRound} is no longer active.`;
      return;
    }
    ui.send.textContent = view.state === 5 ? "Return my funds" : "Cancel old bounty";
    if (account && provider && account === view.creator && !busy && !engine.saved(contract, account)) {
      ui.send.disabled = !([0, 1].includes(view.state) || (view.state === 5 && BigInt(view.contribution) > 0n));
    }
  }
  async function refresh() {
    clearTimeout(timer);
    const connected = account;
    let result = connected ? await engine.reconcile(contract, connected, timeoutMode ? { action: recoveryAction, round: requestedRound } : undefined) : { status: "idle" };
    const nextView = await engine.snapshot(contract, connected);
    if (connected !== account) return;
    view = nextView;
    ui.next.hidden = !(result.status === "confirmed" && result.action === "refund");
    if (result.txHash) {
      ui.receipt.href = `https://basescan.org/tx/${result.txHash}`;
      ui.receipt.hidden = false;
    }
    if (timeoutMode && view.round && view.round !== requestedRound) message(`Round ${view.round} is now active. This link is for round ${requestedRound}. Return to the bounty and refresh.`);
    else if (result.status === "confirmed" && ["expire_claim", "expire_submission"].includes(result.action)) message(result.action === "expire_submission"
      ? `Submission ${result.round} released. The ${core.usdc(result.bond)} USDC bond was returned to the original solver. The bounty stays funded; no reward was paid.`
      : `Claim ${result.round} released. Its ${core.usdc(result.bond)} USDC bond went to the bounty bonus pool. The bounty stays funded; no reward was paid.`);
    else if (result.status === "confirmed") message(result.action === "cancel" ? "Cancellation confirmed. Choose Return my funds for the separate refund request." : `${core.usdc(result.amount)} USDC returned to your wallet. Your replacement still needs its own funding approval.`);
    else if (result.status === "failed") message("The transaction failed. No recovery step was completed. Review the current state before trying again.");
    else if (result.status === "pending") message("Your transaction was sent. Waiting for confirmation on Base; do not send it again.");
    else if (result.status === "unknown") message(result.message);
    else if (timeoutMode) message(!timeoutReady() ? "This exact round cannot be released now. Its state or deadline changed. Return to the bounty and refresh."
      : !account ? "Connect a wallet on Base to review this expiry. Any wallet can pay the gas; the bond always goes to its recorded destination."
      : recoveryAction === "expire_submission" ? "Ready to release this expired submission. The bond returns to the original solver; the bounty stays funded. Your wallet will ask you to confirm."
      : "Ready to release this expired claim. The bond goes to the bounty bonus pool; the bounty stays funded. Your wallet will ask you to confirm.");
    else if (!account) message("Connect the wallet that created this bounty. Connecting does not cancel or spend anything.");
    else if (account !== view.creator) message("This is not the creator wallet. Connect the creator address shown above.");
    else if ([2, 3].includes(view.state)) message("There is an active claim or submission. This page cannot cancel it or interrupt the solver.");
    else if (view.state === 4) message("This bounty has settled and cannot be cancelled.");
    else if (view.state === 5 && BigInt(view.contribution) === 0n) message("The bounty is cancelled and this wallet has no remaining contribution to withdraw. Check the refund receipt before funding a replacement.");
    else message(view.state === 5 ? "Cancellation confirmed on Base. Review Return my funds; the refund goes back to this wallet." : "Ready to cancel. This closes the bounty; your funds stay in it until the separate refund step.");
    render();
    if (result.status === "pending") timer = setTimeout(() => refresh().catch(error => { message(error.message); ui.send.disabled = true; }), 10000);
  }
  ui.connect.addEventListener("click", async () => {
    if (busy) return;
    busy = true; render();
    try {
      const choice = await window.AgentBountiesWalletLink.select();
      if (choice.provider?.agentBountiesCapabilities?.directTransactions === false) throw new Error("Choose MetaMask or another wallet that supports direct contract calls.");
      const accounts = await choice.provider.request({ method: "eth_requestAccounts" });
      const selected = core.address(accounts?.[0]);
      if (String(await choice.provider.request({ method: "eth_chainId" })).toLowerCase() !== core.CHAIN) await choice.provider.request({ method: "wallet_switchEthereumChain", params: [{ chainId: core.CHAIN }] });
      provider = choice.provider; account = selected;
      const changed = () => { if (provider !== choice.provider) return; provider = null; account = null; clearTimeout(timer); ui.next.hidden = true; ui.send.disabled = true; message("Your wallet changed. Connect it again before continuing."); render(); };
      provider.on?.("accountsChanged", changed); provider.on?.("chainChanged", changed); provider.on?.("disconnect", changed);
      await refresh();
    } catch (error) { message(error.message); }
    finally { busy = false; render(); }
  });
  ui.send.addEventListener("click", async (event) => {
    if (!event.isTrusted || busy || !provider || !account || !view) return;
    busy = true; render();
    try {
      message("Checking the exact call. MetaMask will ask you to confirm; wait for that request.");
      await engine.request({ contract, account, action: timeoutMode ? recoveryAction : view.state === 5 ? "refund" : "cancel", provider, reviewed: view });
      await refresh();
    } catch (error) { message(error.code === 4001 ? "You declined the wallet request. No transaction was sent." : `${error.message}\nCheck wallet activity before retrying if a request was opened.`); }
    finally { busy = false; render(); }
  });
  ui.refresh.addEventListener("click", async () => {
    if (busy) return;
    busy = true; render();
    try { await refresh(); } catch (error) { view = null; message(error.message); }
    finally { busy = false; render(); }
  });
  try {
    if (params.has("network") && params.get("network") !== "base-mainnet") throw new Error("Open this recovery on Base mainnet.");
    if (timeoutMode) {
      if (!["expire_claim", "expire_submission"].includes(recoveryAction) || !/^[1-9]\d*$/.test(requestedRound || "") || BigInt(requestedRound) > 0xffffffffffffffffn) throw new Error("Open the exact recovery link from the bounty page.");
      document.getElementById("recovery-title").textContent = "Reopen bounty";
      document.title = "Reopen bounty | AgentBounties.app";
      ui.contribution.hidden = true;
      document.getElementById("recovery-contribution-label").hidden = true;
      document.getElementById("recovery-intro").textContent = recoveryAction === "expire_submission"
        ? "The review time ended. Release this submission and return its bond to the original solver. The bounty stays funded so work can continue."
        : "The work time ended. Release this claim and move its bond to the bounty bonus pool. The bounty stays funded so another solver can work.";
      document.getElementById("recovery-cancel-steps").hidden = true;
      document.getElementById("recovery-bond").hidden = false;
      document.getElementById("recovery-disclosure").textContent = "This sends 0 ETH to the bounty. You pay the Base network fee. It does not cancel the bounty or pay a reward.";
    }
    contract = core.address(params.get("bountyContract"));
    ui.contract.textContent = contract;
    await refresh();
  } catch (error) { message(error.message); ui.send.disabled = true; }
})();
