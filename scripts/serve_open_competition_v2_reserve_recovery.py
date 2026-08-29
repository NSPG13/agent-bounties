#!/usr/bin/env python3
"""Serve a fail-closed two-step owner flow for recovering uncommitted V2 USDC."""

from __future__ import annotations

import argparse
import hashlib
import json
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import re
import threading
import time
from typing import Any
from urllib.parse import parse_qs, urlsplit

from eth_abi import decode, encode
from eth_utils import keccak, to_checksum_address
from web3 import Web3

from inspect_open_competition_v2_reward_policy import call_raw, inspect_state
from serve_open_competition_v2_reward_policy_confirmation import (
    base_rpc_call,
    lower_hex,
    normalized_rpc_urls,
    store_result,
)


CHAIN_ID = 8453
OWNER = "0x884834e884d6e93462655a2820140ad03e6747bc"
RESERVE = "0x7b0ae568f76d11aa4025e2aa05865a566bbcfc8d"
USDC = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913"
SCHEMA = "agent-bounties/open-competition-v2-reserve-recovery-v1"
REVOKE_CALLDATA = "0x9eba3667"
RECOVER_CALLDATA = "0xb0e11ec4"
PRODUCTION_RELEASE_HASH = "0x46008fb819726a43209e55ee7e58c92700a8fc3435f76be282bfdef710ced594"
PRODUCTION_COMPETITION_FACTORY = "0x29d0e39e0c03797c690633535722e6b34a69a78a"
PRODUCTION_RESERVE_FACTORY = "0xad0765eac772ff6cf696f2416751269d97a5419f"
PRODUCTION_RESERVE_IMPLEMENTATION = "0x9c62e1ab727909a18a830744eb244645ee91b0eb"
PRODUCTION_RESERVE_CLONE_HASH = "0x8c962c39a7247d0543873d0e469f1cd5a1caa9ce74d310a09f0402d24f6f99e3"
PRODUCTION_COMPETITION_FACTORY_HASH = "0x6874d03d64442358d84f5dcf3bf779e3a2e8b73fdbe573a3f880245757fa39da"
PRODUCTION_RESERVE_FACTORY_HASH = "0xe2afb624e1eb12eb75532b072ee7e81685ecd1076be2907672dbcaeb70cfca06"
PRODUCTION_RESERVE_IMPLEMENTATION_HASH = "0xf20849ca4183d3f4d04eae8ca35238c575c5356791915d8a3420588128c64a96"
TRANSACTION_HASH = re.compile(r"^0x[0-9a-fA-F]{64}$")
TRANSFER_TOPIC = "0x" + keccak(text="Transfer(address,address,uint256)").hex()
POLL_SECONDS = 2
POLL_ATTEMPTS = 180


HTML = r"""<!doctype html>
<html lang="en">
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>Recover uncommitted Open Competition V2 reserve</title>
<style>
  :root { color-scheme: dark; font-family: Inter, ui-sans-serif, system-ui, sans-serif; }
  body { margin: 0; background: #07111f; color: #eef5ff; }
  main { max-width: 860px; margin: 3vh auto; padding: 24px; }
  article { background: #0d1d31; border: 1px solid #274563; border-radius: 20px; padding: 28px; box-shadow: 0 24px 80px #0008; }
  h1 { margin: 0 0 12px; font-size: clamp(28px, 5vw, 46px); line-height: 1.05; }
  p { color: #b8cbe0; line-height: 1.55; }
  dl { display: grid; grid-template-columns: minmax(140px, 1fr) 2fr; gap: 12px 18px; margin: 24px 0; }
  dt { color: #8da8c4; } dd { margin: 0; overflow-wrap: anywhere; font-weight: 650; }
  .good { color: #7ff5b0; } .warn { color: #ffd580; }
  .step { border-left: 4px solid #67f0a5; background: #0a1728; padding: 14px 18px; margin: 14px 0; }
  .wallet-picker { display: grid; gap: 8px; margin: 22px 0 14px; color: #b8cbe0; }
  select { width: 100%; border: 1px solid #466987; border-radius: 12px; padding: 13px 14px; background: #07111f; color: #eef5ff; font: inherit; }
  button { width: 100%; border: 0; border-radius: 14px; padding: 16px 20px; font: inherit; font-weight: 800; background: #67f0a5; color: #06120b; cursor: pointer; }
  button:disabled { opacity: .55; cursor: wait; }
  #status { min-height: 48px; }
  code { font-family: ui-monospace, monospace; font-size: .9em; }
  @media (max-width: 580px) { dl { grid-template-columns: 1fr; gap: 5px; } dd { margin-bottom: 10px; } article { padding: 20px; } }
</style>
<main><article>
  <div class="good">Base mainnet · owner-only · two explicit confirmations</div>
  <h1>Return the unused V2 reserve to its owner</h1>
  <p>This page can only revoke the reserve's delegate policy and return the reserve's current uncommitted USDC to the owner. It cannot cancel, settle, enter, or withdraw from any active competition.</p>
  <dl id="facts"></dl>
  <section class="step"><strong>Step 1 — revoke policy</strong><p>From the exact owner to the exact reserve, with 0 ETH and 0 USDC moved. This stops new delegate spending until the owner installs another policy.</p></section>
  <section class="step"><strong>Step 2 — recover uncommitted USDC</strong><p>From the same owner to the same reserve, with 0 ETH. The reserve contract transfers its full current USDC balance to the owner. The server simulates the exact call again immediately before MetaMask opens.</p></section>
  <p class="warn">MetaMask will ask twice. Read each prompt. Active competition escrow is outside this reserve and remains governed by its immutable deadlines and settlement rules.</p>
  <label class="wallet-picker" for="wallet-provider">Wallet
    <select id="wallet-provider"><option value="">Discovering installed wallets…</option></select>
  </label>
  <button id="recover">Start the two-step recovery</button>
  <p id="status" aria-live="polite"></p>
</article></main>
<script>
(async () => {
  const wallets = [];
  const seenProviders = new Set();
  const walletSelect = document.querySelector('#wallet-provider');
  const status = document.querySelector('#status');
  const button = document.querySelector('#recover');
  let userSelectedWallet = false;
  walletSelect.addEventListener('change', () => { userSelectedWallet = true; });

  function addWallet(provider, info = {}) {
    if (!provider || seenProviders.has(provider)) return;
    seenProviders.add(provider);
    const rdns = String(info.rdns || '').toLowerCase();
    const isCoinbase = rdns.includes('coinbase') || Boolean(provider.isCoinbaseWallet);
    const isBrave = rdns.includes('brave') || Boolean(provider.isBraveWallet);
    const isMetaMask = !isCoinbase && !isBrave && (rdns === 'io.metamask' || Boolean(provider.isMetaMask));
    const name = String(info.name || (isMetaMask ? 'MetaMask' : isCoinbase ? 'Coinbase Wallet' : isBrave ? 'Brave Wallet' : 'Injected wallet'));
    wallets.push({provider, name, isMetaMask});
    renderWallets();
  }

  function renderWallets() {
    const previous = userSelectedWallet ? wallets[Number(walletSelect.value)]?.provider : null;
    wallets.sort((a, b) => Number(b.isMetaMask) - Number(a.isMetaMask) || a.name.localeCompare(b.name));
    walletSelect.replaceChildren();
    if (!wallets.length) {
      const option = document.createElement('option');
      option.value = '';
      option.textContent = 'No injected wallet detected';
      walletSelect.append(option);
    }
    wallets.forEach((wallet, index) => {
      const option = document.createElement('option');
      option.value = String(index);
      option.textContent = `${wallet.name}${wallet.isMetaMask ? ' (recommended)' : ''}`;
      walletSelect.append(option);
    });
    const previousIndex = wallets.findIndex(wallet => wallet.provider === previous);
    const metaMaskIndex = wallets.findIndex(wallet => wallet.isMetaMask);
    walletSelect.value = String(previousIndex >= 0 ? previousIndex : metaMaskIndex >= 0 ? metaMaskIndex : 0);
  }

  window.addEventListener('eip6963:announceProvider', event => addWallet(event.detail?.provider, event.detail?.info));
  window.dispatchEvent(new Event('eip6963:requestProvider'));
  const legacy = Array.isArray(window.ethereum?.providers) ? window.ethereum.providers : window.ethereum ? [window.ethereum] : [];
  legacy.forEach(provider => addWallet(provider));

  const plan = await fetch('/plan', {cache: 'no-store'}).then(response => response.json());
  const facts = [
    ['Network', 'Base mainnet (chain 8453)'],
    ['Owner / sender', plan.owner],
    ['Reserve / destination', plan.reserve],
    ['Observed unused reserve', plan.display.reserve_balance],
    ['Lifetime already committed', plan.display.lifetime_spent],
    ['Step 1 value', '0 ETH; 0 USDC moved'],
    ['Step 1 calldata', plan.transactions.revoke.data],
    ['Step 2 value', `0 ETH; currently ${plan.display.reserve_balance} returned to owner`],
    ['Step 2 calldata', plan.transactions.recover.data],
    ['Active competition escrow', 'Not touched by either transaction'],
  ];
  const factsElement = document.querySelector('#facts');
  facts.forEach(([key, value]) => {
    const term = document.createElement('dt');
    const detail = document.createElement('dd');
    const code = document.createElement('code');
    term.textContent = key;
    code.textContent = value;
    detail.append(code);
    factsElement.append(term, detail);
  });

  async function ensureBase(provider) {
    let chain = String(await provider.request({method: 'eth_chainId'})).toLowerCase();
    if (chain !== '0x2105') {
      await provider.request({method: 'wallet_switchEthereumChain', params: [{chainId: '0x2105'}]});
      chain = String(await provider.request({method: 'eth_chainId'})).toLowerCase();
    }
    if (chain !== '0x2105') throw new Error('The selected wallet is not on Base mainnet.');
  }

  async function pollCanonical(targetStatus) {
    for (let attempt = 0; attempt < 120; attempt += 1) {
      const result = await fetch('/status', {cache: 'no-store'}).then(response => response.json());
      if (result.status === targetStatus) return result;
      if (result.status === 'failed') throw new Error(result.error || 'Canonical recovery reconciliation failed.');
      status.textContent = result.message || 'Waiting for the transaction and Base safe-block reconciliation…';
      await new Promise(resolve => setTimeout(resolve, 3000));
    }
    throw new Error('The transaction is still pending. Its hash is saved; restart this server to resume without rebroadcasting.');
  }

  function walletTransaction(template, account) {
    return {from: account, to: template.to, data: template.data, value: '0x0'};
  }

  async function submitStage(stage, provider, account) {
    const simulation = await fetch(`/simulate?stage=${encodeURIComponent(stage)}`, {cache: 'no-store'});
    const simulationResult = await simulation.json();
    if (!simulation.ok) throw new Error(simulationResult.error || `${stage} server simulation failed.`);
    if (stage === 'recover') status.textContent = `Exact simulation: ${simulationResult.display_amount} will return to the owner. Review the MetaMask prompt.`;
    const transaction = walletTransaction(plan.transactions[stage], account);
    await provider.request({method: 'eth_call', params: [transaction, 'latest']});
    const transactionHash = await provider.request({method: 'eth_sendTransaction', params: [transaction]});
    const submitted = await fetch('/submitted', {
      method: 'POST',
      headers: {'content-type': 'application/json'},
      body: JSON.stringify({stage, transaction_hash: transactionHash}),
    });
    const accepted = await submitted.json();
    if (!submitted.ok) throw new Error(accepted.error || 'The local verifier rejected the transaction hash.');
  }

  button.addEventListener('click', async () => {
    button.disabled = true;
    try {
      const selected = wallets[Number(walletSelect.value)];
      if (!selected) throw new Error('No injected wallet is available. Enable MetaMask for this page, then retry.');
      await ensureBase(selected.provider);
      const accounts = await selected.provider.request({method: 'eth_requestAccounts'});
      const account = String(accounts[0] || '').toLowerCase();
      if (account !== plan.owner.toLowerCase()) throw new Error(`Select owner ${plan.owner} in the wallet, then retry.`);
      let current = await fetch('/status', {cache: 'no-store'}).then(response => response.json());
      if (current.status === 'confirmed') {
        status.textContent = `${current.display_amount} was already canonically recovered in ${current.recover_transaction_hash}.`;
        status.className = 'good';
        button.textContent = 'Recovery canonically confirmed';
        return;
      }
      if (current.status !== 'revoked') {
        status.textContent = 'Simulating the exact zero-value policy revocation…';
        await submitStage('revoke', selected.provider, account);
        status.textContent = 'Revocation submitted. Waiting for safe-block proof that no USDC moved…';
        await pollCanonical('revoked');
      }
      status.textContent = 'Policy safely revoked. Simulating the owner-only reserve recovery…';
      await submitStage('recover', selected.provider, account);
      status.textContent = 'Recovery submitted. Waiting for the exact receipt, USDC Transfer event, and Base safe block…';
      current = await pollCanonical('confirmed');
      status.textContent = `${current.display_amount} was canonically returned to the owner in ${current.recover_transaction_hash}.`;
      status.className = 'good';
      button.textContent = 'Recovery canonically confirmed';
    } catch (error) {
      status.textContent = error?.message || String(error);
      status.className = 'warn';
      button.disabled = false;
    }
  });
})().catch(error => { document.querySelector('#status').textContent = String(error); });
</script>
</html>"""


class RecoveryError(ValueError):
    pass


def canonical_json(value: Any) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode()


def plan_identifier(plan_without_id: dict[str, Any]) -> str:
    return "sha256:" + hashlib.sha256(canonical_json(plan_without_id)).hexdigest()


def display_usdc(amount: int) -> str:
    whole, fraction = divmod(amount, 1_000_000)
    return f"{whole}.{fraction:06d} USDC"


def require_address(value: object, label: str) -> str:
    try:
        return str(to_checksum_address(str(value))).lower()
    except (TypeError, ValueError) as error:
        raise RecoveryError(f"{label} is not an address") from error


def deployment_manifest_hash(manifest: dict[str, Any]) -> str:
    return "0x" + hashlib.sha256(canonical_json(manifest)).hexdigest()


def validate_manifest(manifest: dict[str, Any]) -> None:
    if manifest.get("schema") != "agent-bounties/bounded-open-competition-v2-wallet-deployment-v1":
        raise RecoveryError("reserve deployment manifest schema is invalid")
    if manifest.get("network") != "base-mainnet" or int(manifest.get("chain_id", 0)) != CHAIN_ID:
        raise RecoveryError("reserve deployment manifest is not Base mainnet")
    canonical = manifest.get("canonical", {})
    reserve_factory = manifest.get("reserve_factory", {})
    required = (
        canonical.get("competition_factory"),
        canonical.get("settlement_token"),
        reserve_factory.get("address"),
        reserve_factory.get("clone_runtime_code_hash"),
    )
    if not all(required):
        raise RecoveryError("reserve deployment manifest is incomplete")

    exact = (
        (str(canonical["competition_factory"]).lower(), PRODUCTION_COMPETITION_FACTORY, "competition factory"),
        (str(canonical["settlement_token"]).lower(), USDC, "settlement token"),
        (str(canonical.get("release_hash", "")).lower(), PRODUCTION_RELEASE_HASH, "release hash"),
        (str(reserve_factory["address"]).lower(), PRODUCTION_RESERVE_FACTORY, "reserve factory"),
        (str(reserve_factory.get("implementation", "")).lower(), PRODUCTION_RESERVE_IMPLEMENTATION, "reserve implementation"),
        (str(reserve_factory["clone_runtime_code_hash"]).lower(), PRODUCTION_RESERVE_CLONE_HASH, "reserve clone runtime hash"),
    )
    for actual, expected, label in exact:
        if actual != expected:
            raise RecoveryError(f"deployment manifest {label} is not the reviewed production release")


def validate_deployment_evidence(evidence: dict[str, Any], manifest: dict[str, Any]) -> None:
    if evidence.get("schema_version") != "agent-bounties/bounded-open-competition-v2-wallet-deployment-evidence-v1":
        raise RecoveryError("reserve deployment evidence schema is invalid")
    if evidence.get("network") != "base-mainnet" or int(evidence.get("chain_id", 0)) != CHAIN_ID or evidence.get("complete") is not True:
        raise RecoveryError("reserve deployment evidence is not complete Base mainnet evidence")
    exact = (
        (str(evidence.get("manifest_hash", "")).lower(), deployment_manifest_hash(manifest), "manifest hash"),
        (str(evidence.get("release_hash", "")).lower(), PRODUCTION_RELEASE_HASH, "release hash"),
        (str(evidence.get("competition_factory", "")).lower(), PRODUCTION_COMPETITION_FACTORY, "competition factory"),
        (str(evidence.get("reserve_factory", "")).lower(), PRODUCTION_RESERVE_FACTORY, "reserve factory"),
        (str(evidence.get("reserve_implementation", "")).lower(), PRODUCTION_RESERVE_IMPLEMENTATION, "reserve implementation"),
    )
    for actual, expected, label in exact:
        if actual != expected:
            raise RecoveryError(f"deployment evidence {label} differs from the reviewed production release")
    runtime_hashes = {str(key).lower(): str(value).lower() for key, value in evidence.get("runtime_hashes", {}).items()}
    expected_hashes = {
        PRODUCTION_COMPETITION_FACTORY: PRODUCTION_COMPETITION_FACTORY_HASH,
        PRODUCTION_RESERVE_FACTORY: PRODUCTION_RESERVE_FACTORY_HASH,
        PRODUCTION_RESERVE_IMPLEMENTATION: PRODUCTION_RESERVE_IMPLEMENTATION_HASH,
    }
    if any(runtime_hashes.get(address) != code_hash for address, code_hash in expected_hashes.items()):
        raise RecoveryError("deployment evidence runtime hashes differ from the reviewed production release")


def validate_deployment_state(
    state: dict[str, Any], manifest: dict[str, Any], evidence: dict[str, Any]
) -> None:
    validate_manifest(manifest)
    validate_deployment_evidence(evidence, manifest)
    canonical = manifest["canonical"]
    reserve_factory = manifest["reserve_factory"]
    comparisons = (
        (state.get("network"), "base-mainnet", "network"),
        (int(state.get("chain_id", 0)), CHAIN_ID, "chain"),
        (require_address(state.get("reserve_wallet"), "reserve"), RESERVE, "reserve"),
        (require_address(state.get("owner"), "owner"), OWNER, "owner"),
        (require_address(state.get("settlement_token"), "settlement token"), require_address(canonical["settlement_token"], "manifest settlement token"), "settlement token"),
        (require_address(state.get("competition_factory"), "competition factory"), require_address(canonical["competition_factory"], "manifest competition factory"), "competition factory"),
        (require_address(state.get("deployment_factory"), "deployment factory"), require_address(reserve_factory["address"], "manifest reserve factory"), "deployment factory"),
        (str(state.get("reserve_runtime_code_hash", "")).lower(), str(reserve_factory["clone_runtime_code_hash"]).lower(), "reserve runtime code hash"),
        (str(state.get("competition_factory_runtime_code_hash", "")).lower(), PRODUCTION_COMPETITION_FACTORY_HASH, "competition factory runtime code hash"),
        (str(state.get("deployment_factory_runtime_code_hash", "")).lower(), PRODUCTION_RESERVE_FACTORY_HASH, "reserve factory runtime code hash"),
        (require_address(state.get("deployment_implementation"), "reserve implementation"), PRODUCTION_RESERVE_IMPLEMENTATION, "reserve implementation"),
        (str(state.get("deployment_implementation_runtime_code_hash", "")).lower(), PRODUCTION_RESERVE_IMPLEMENTATION_HASH, "reserve implementation runtime code hash"),
    )
    for actual, expected, label in comparisons:
        if actual != expected:
            raise RecoveryError(f"live {label} differs from the reviewed deployment")


def build_plan(
    state: dict[str, Any],
    manifest: dict[str, Any],
    evidence: dict[str, Any],
    expected_balance: int,
    expected_lifetime_spent: int,
) -> dict[str, Any]:
    validate_deployment_state(state, manifest, evidence)
    if bool(state.get("revoked")):
        raise RecoveryError("reserve policy is already revoked; resume from the saved recovery plan")
    balance = int(state.get("reserve_balance", -1))
    lifetime_spent = int(state.get("lifetime_spent", -1))
    if balance <= 0 or balance != expected_balance:
        raise RecoveryError("safe-block reserve balance differs from the explicitly expected recovery amount")
    if lifetime_spent != expected_lifetime_spent:
        raise RecoveryError("safe-block lifetime spend differs from the explicitly expected amount")
    if int(state.get("policy_version", 0)) <= 0 or not state.get("active_policy_hash"):
        raise RecoveryError("safe-block policy evidence is incomplete")
    owner_balance = int(state.get("owner_balance", -1))
    if owner_balance < 0:
        raise RecoveryError("safe-block owner USDC balance is missing")
    plan = {
        "schema": SCHEMA,
        "chain_id": CHAIN_ID,
        "network": "base-mainnet",
        "owner": OWNER,
        "reserve": RESERVE,
        "settlement_token": USDC,
        "snapshot": {
            "safe_block": int(state["safe_block"]),
            "safe_block_hash": str(state["safe_block_hash"]).lower(),
            "policy_version": int(state["policy_version"]),
            "active_policy_hash": str(state["active_policy_hash"]).lower(),
            "lifetime_spent": lifetime_spent,
            "reserve_balance": balance,
            "owner_balance": owner_balance,
            "revoked": False,
        },
        "display": {
            "reserve_balance": display_usdc(balance),
            "lifetime_spent": display_usdc(lifetime_spent),
        },
        "transactions": {
            "revoke": {"from": OWNER, "to": RESERVE, "value_wei": 0, "data": REVOKE_CALLDATA, "function": "revokePolicy()"},
            "recover": {"from": OWNER, "to": RESERVE, "value_wei": 0, "data": RECOVER_CALLDATA, "function": "recoverUncommitted()"},
        },
        "boundary": "Revokes future delegate authority and returns only the reserve's current USDC balance. It does not touch active competition escrow, prove GMV, or prove a payout.",
    }
    plan["plan_id"] = plan_identifier(plan)
    return plan


def validate_plan(plan: dict[str, Any]) -> None:
    if plan.get("schema") != SCHEMA or int(plan.get("chain_id", 0)) != CHAIN_ID:
        raise RecoveryError("saved recovery plan schema or chain is invalid")
    if require_address(plan.get("owner"), "plan owner") != OWNER or require_address(plan.get("reserve"), "plan reserve") != RESERVE:
        raise RecoveryError("saved recovery plan owner or reserve differs")
    if require_address(plan.get("settlement_token"), "plan settlement token") != USDC:
        raise RecoveryError("saved recovery plan token differs")
    transactions = plan.get("transactions")
    if not isinstance(transactions, dict) or set(transactions) != {"revoke", "recover"}:
        raise RecoveryError("saved recovery plan transactions are incomplete")
    for stage, calldata in (("revoke", REVOKE_CALLDATA), ("recover", RECOVER_CALLDATA)):
        transaction = transactions[stage]
        if require_address(transaction.get("from"), f"{stage} sender") != OWNER:
            raise RecoveryError(f"{stage} sender differs")
        if require_address(transaction.get("to"), f"{stage} destination") != RESERVE:
            raise RecoveryError(f"{stage} destination differs")
        if int(transaction.get("value_wei", -1)) != 0 or lower_hex(transaction.get("data")) != calldata:
            raise RecoveryError(f"{stage} value or calldata differs")
    supplied_id = str(plan.get("plan_id") or "")
    unsigned = dict(plan)
    unsigned.pop("plan_id", None)
    if supplied_id != plan_identifier(unsigned):
        raise RecoveryError("saved recovery plan integrity check failed")


def add_owner_balance(state: dict[str, Any], w3: Web3) -> dict[str, Any]:
    enriched = dict(state)
    arguments = encode(["address"], [to_checksum_address(OWNER)])
    enriched["owner_balance"] = int(
        decode(
            ["uint256"],
            call_raw(w3, USDC, "balanceOf(address)", int(state["safe_block"]), arguments),
        )[0]
    )
    return enriched


def inspect_live(
    w3: Web3, manifest: dict[str, Any], evidence: dict[str, Any]
) -> dict[str, Any]:
    state = add_owner_balance(inspect_state(w3, RESERVE), w3)
    safe_block = int(state["safe_block"])
    deployment_factory = require_address(state["deployment_factory"], "deployment factory")
    deployment_implementation = require_address(
        decode(
            ["address"],
            call_raw(w3, deployment_factory, "implementation()", safe_block),
        )[0],
        "reserve implementation",
    )
    factory_code = bytes(
        w3.eth.get_code(to_checksum_address(deployment_factory), block_identifier=safe_block)
    )
    implementation_code = bytes(
        w3.eth.get_code(
            to_checksum_address(deployment_implementation), block_identifier=safe_block
        )
    )
    if not factory_code or not implementation_code:
        raise RecoveryError("reserve deployment dependency has no code at the safe block")
    state["deployment_factory_runtime_code_hash"] = "0x" + keccak(factory_code).hex()
    state["deployment_implementation"] = deployment_implementation
    state["deployment_implementation_runtime_code_hash"] = (
        "0x" + keccak(implementation_code).hex()
    )
    validate_deployment_state(state, manifest, evidence)
    return state


def transaction_matches(plan: dict[str, Any], stage: str, transaction: Any) -> None:
    if stage not in {"revoke", "recover"}:
        raise RecoveryError("transaction stage is invalid")
    expected = plan["transactions"][stage]
    if lower_hex(transaction.get("from")) != lower_hex(expected["from"]):
        raise RecoveryError("confirmed transaction sender differs")
    if lower_hex(transaction.get("to")) != lower_hex(expected["to"]):
        raise RecoveryError("confirmed transaction destination differs")
    if int(transaction.get("value", 0)) != 0:
        raise RecoveryError("confirmed transaction ETH value is not zero")
    actual_input = transaction.get("input", transaction.get("data", ""))
    if lower_hex(actual_input) != lower_hex(expected["data"]):
        raise RecoveryError("confirmed transaction calldata differs")


def topic_address(value: object) -> str:
    raw = lower_hex(value)
    if not re.fullmatch(r"0x[0-9a-f]{64}", raw):
        raise RecoveryError("USDC Transfer topic is malformed")
    return "0x" + raw[-40:]


def extract_recovery_transfer(receipt: Any, plan: dict[str, Any]) -> int:
    matches: list[int] = []
    for log in receipt.get("logs", []):
        if lower_hex(log.get("address")) != USDC:
            continue
        topics = log.get("topics", [])
        if len(topics) != 3 or lower_hex(topics[0]) != TRANSFER_TOPIC:
            continue
        if topic_address(topics[1]) != RESERVE or topic_address(topics[2]) != OWNER:
            continue
        data = lower_hex(log.get("data"))
        if not re.fullmatch(r"0x[0-9a-f]{64}", data):
            raise RecoveryError("USDC Transfer amount is malformed")
        matches.append(int(data, 16))
    if len(matches) != 1 or matches[0] <= 0:
        raise RecoveryError("receipt does not contain one positive reserve-to-owner USDC Transfer")
    if matches[0] < int(plan["snapshot"]["reserve_balance"]):
        raise RecoveryError("canonical recovery transferred less than the approved safe-block balance")
    return matches[0]


def validate_revoke_evidence(plan: dict[str, Any], state: dict[str, Any]) -> None:
    if not bool(state.get("revoked")):
        raise RecoveryError("safe-block reserve policy is not revoked")
    if int(state.get("lifetime_spent", -1)) != int(plan["snapshot"]["lifetime_spent"]):
        raise RecoveryError("revocation unexpectedly changed lifetime spend")
    if int(state.get("reserve_balance", -1)) < int(plan["snapshot"]["reserve_balance"]):
        raise RecoveryError("delegate spending reduced the reserve before revocation became canonical")


def validate_recovery_evidence(plan: dict[str, Any], state: dict[str, Any], amount: int) -> None:
    if not bool(state.get("revoked")):
        raise RecoveryError("safe-block reserve policy is no longer revoked")
    if int(state.get("lifetime_spent", -1)) != int(plan["snapshot"]["lifetime_spent"]):
        raise RecoveryError("recovery unexpectedly changed lifetime spend")
    if int(state.get("reserve_balance", -1)) != 0:
        raise RecoveryError("reserve still has USDC after canonical recovery")
    if amount < int(plan["snapshot"]["reserve_balance"]):
        raise RecoveryError("recovered amount is below the approved safe-block balance")


def plan_path(result_output: Path) -> Path:
    return result_output.with_name(f"{result_output.name}.plan")


def stage_result_path(result_output: Path, stage: str) -> Path:
    if stage not in {"revoke", "recover"}:
        raise RecoveryError("transaction stage is invalid")
    return result_output.with_name(f"{result_output.name}.{stage}")


def submission_path(result_output: Path, stage: str) -> Path:
    return result_output.with_name(f"{result_output.name}.submitted-{stage}")


def load_json(path: Path) -> dict[str, Any] | None:
    if not path.exists():
        return None
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise RecoveryError(f"{path.name} is not an object")
    return value


def store_submission(result_output: Path, plan: dict[str, Any], stage: str, transaction_hash: str) -> dict[str, Any]:
    if stage not in {"revoke", "recover"}:
        raise RecoveryError("transaction stage is invalid")
    if not TRANSACTION_HASH.fullmatch(transaction_hash):
        raise RecoveryError("transaction hash must be exact bytes32")
    path = submission_path(result_output, stage)
    value = {"schema": SCHEMA, "plan_id": plan["plan_id"], "stage": stage, "transaction_hash": transaction_hash.lower()}
    existing = load_json(path)
    if existing is not None:
        if existing != value:
            raise RecoveryError(f"a different {stage} transaction is already durably recorded")
        return existing
    store_result(path, value)
    return value


def load_submission(result_output: Path, plan: dict[str, Any], stage: str) -> dict[str, Any] | None:
    value = load_json(submission_path(result_output, stage))
    if value is None:
        return None
    if value.get("schema") != SCHEMA or value.get("plan_id") != plan["plan_id"] or value.get("stage") != stage:
        raise RecoveryError(f"saved {stage} submission differs from this plan")
    if not TRANSACTION_HASH.fullmatch(str(value.get("transaction_hash") or "")):
        raise RecoveryError(f"saved {stage} transaction hash is invalid")
    return value


class RecoveryServer(ThreadingHTTPServer):
    def __init__(self, address: tuple[str, int], plan: dict[str, Any], manifest: dict[str, Any], evidence: dict[str, Any], rpc_urls: tuple[str, ...], result_output: Path) -> None:
        self.plan = plan
        self.manifest = manifest
        self.evidence = evidence
        self.rpc_urls = rpc_urls
        self.result_output = result_output
        self.lock = threading.Lock()
        self.reconciling: set[str] = set()
        final = load_json(result_output)
        revoked = load_json(stage_result_path(result_output, "revoke"))
        if final is not None:
            if final.get("plan_id") != plan["plan_id"] or final.get("status") != "confirmed":
                raise RecoveryError("saved final recovery result differs from this plan")
            self.status = final
        elif revoked is not None:
            if revoked.get("plan_id") != plan["plan_id"] or revoked.get("status") != "revoked":
                raise RecoveryError("saved revocation result differs from this plan")
            self.status = revoked
        else:
            self.status = {"status": "ready", "message": "Exact deployment and safe-block recovery plan verified."}
        super().__init__(address, RecoveryHandler)
        recover_submission = load_submission(result_output, plan, "recover")
        revoke_submission = load_submission(result_output, plan, "revoke")
        if recover_submission and final is None:
            self._launch("recover", recover_submission["transaction_hash"])
        elif revoke_submission and final is None and revoked is None:
            self._launch("revoke", revoke_submission["transaction_hash"])

    def _launch(self, stage: str, transaction_hash: str) -> None:
        with self.lock:
            if stage in self.reconciling:
                return
            self.reconciling.add(stage)
            self.status = {"status": "pending", "stage": stage, "message": f"Reconciling the exact {stage} transaction…"}
        threading.Thread(target=self._reconcile, args=(stage, transaction_hash), daemon=True).start()

    def begin_reconciliation(self, stage: str, transaction_hash: str) -> None:
        if stage not in {"revoke", "recover"}:
            raise RecoveryError("transaction stage is invalid")
        with self.lock:
            current = str(self.status.get("status"))
        if stage == "recover" and current != "revoked":
            raise RecoveryError("canonical revocation evidence is required before recovery")
        if stage == "revoke" and current not in {"ready", "pending"}:
            raise RecoveryError("revocation stage is already complete")
        store_submission(self.result_output, self.plan, stage, transaction_hash)
        self._launch(stage, transaction_hash)

    def simulate(self, stage: str) -> dict[str, Any]:
        if stage not in {"revoke", "recover"}:
            raise RecoveryError("simulation stage is invalid")
        with self.lock:
            current = str(self.status.get("status"))
        if stage == "recover" and current != "revoked":
            raise RecoveryError("safe-block revocation is required before recovery simulation")

        def operation(w3: Web3) -> dict[str, Any]:
            state = inspect_live(w3, self.manifest, self.evidence)
            if stage == "revoke":
                if bool(state["revoked"]):
                    raise RecoveryError("reserve policy is already revoked")
            elif not bool(state["revoked"]):
                raise RecoveryError("reserve policy is not revoked")
            transaction = {"from": to_checksum_address(OWNER), "to": to_checksum_address(RESERVE), "data": self.plan["transactions"][stage]["data"], "value": 0}
            raw = bytes(w3.eth.call(transaction, block_identifier="latest"))
            amount = 0
            if stage == "recover":
                if len(raw) != 32:
                    raise RecoveryError("recovery simulation returned an invalid amount")
                amount = int(decode(["uint256"], raw)[0])
                if amount != int(state["reserve_balance"]) or amount <= 0:
                    raise RecoveryError("recovery simulation differs from the live reserve balance")
            return {"status": "simulated", "stage": stage, "amount": amount, "display_amount": display_usdc(amount)}

        return base_rpc_call(self.rpc_urls, operation, f"{stage} simulation")

    def _poll_transaction(self, transaction_hash: str) -> tuple[Any, Any]:
        for _ in range(POLL_ATTEMPTS):
            transaction = base_rpc_call(
                self.rpc_urls,
                lambda w3: w3.eth.get_transaction(transaction_hash),
                "submitted transaction",
                transaction_not_found_is_none=True,
            )
            receipt = base_rpc_call(
                self.rpc_urls,
                lambda w3: w3.eth.get_transaction_receipt(transaction_hash),
                "submitted transaction receipt",
                transaction_not_found_is_none=True,
            )
            if transaction is not None and receipt is not None:
                return transaction, receipt
            time.sleep(POLL_SECONDS)
        raise RecoveryError("submitted transaction was not mined before the reconciliation timeout")

    def _wait_safe_block(self, receipt_block: int) -> tuple[int, dict[str, Any]]:
        for _ in range(POLL_ATTEMPTS):
            def operation(w3: Web3) -> tuple[int, dict[str, Any]]:
                state = inspect_live(w3, self.manifest, self.evidence)
                return int(state["safe_block"]), state

            safe_block, state = base_rpc_call(self.rpc_urls, operation, "Base safe-block recovery evidence")
            if safe_block >= receipt_block:
                return safe_block, state
            time.sleep(POLL_SECONDS)
        raise RecoveryError("transaction did not reach a Base safe block")

    def _reconcile(self, stage: str, transaction_hash: str) -> None:
        try:
            transaction, receipt = self._poll_transaction(transaction_hash)
            transaction_matches(self.plan, stage, transaction)
            if int(receipt.get("status", 0)) != 1:
                raise RecoveryError(f"{stage} transaction reverted")
            receipt_block = int(receipt["blockNumber"])
            with self.lock:
                self.status = {"status": "pending", "stage": stage, "message": f"{stage.title()} mined; waiting for Base safe-block inclusion…"}
            safe_block, state = self._wait_safe_block(receipt_block)
            if stage == "revoke":
                validate_revoke_evidence(self.plan, state)
                result = {
                    "schema": SCHEMA,
                    "plan_id": self.plan["plan_id"],
                    "status": "revoked",
                    "revoke_transaction_hash": transaction_hash.lower(),
                    "transaction_block": receipt_block,
                    "safe_block": safe_block,
                    "reserve_balance": int(state["reserve_balance"]),
                    "lifetime_spent": int(state["lifetime_spent"]),
                    "message": "Policy revocation is safe-block confirmed; no USDC moved.",
                }
                existing = load_json(stage_result_path(self.result_output, "revoke"))
                if existing is None:
                    store_result(stage_result_path(self.result_output, "revoke"), result)
                elif existing != result:
                    raise RecoveryError("saved revocation evidence differs")
            else:
                amount = extract_recovery_transfer(receipt, self.plan)
                validate_recovery_evidence(self.plan, state, amount)
                revoke_submission = load_submission(self.result_output, self.plan, "revoke")
                if revoke_submission is None:
                    raise RecoveryError("saved revocation transaction is missing")
                result = {
                    "schema": SCHEMA,
                    "plan_id": self.plan["plan_id"],
                    "status": "confirmed",
                    "revoke_transaction_hash": revoke_submission["transaction_hash"],
                    "recover_transaction_hash": transaction_hash.lower(),
                    "transaction_block": receipt_block,
                    "safe_block": safe_block,
                    "recovered_amount": amount,
                    "display_amount": display_usdc(amount),
                    "reserve_balance": 0,
                    "owner_balance_at_safe_block": int(state["owner_balance"]),
                    "lifetime_spent": int(state["lifetime_spent"]),
                    "canonical_evidence": "Exact owner transaction plus canonical USDC Transfer event from reserve to owner, reconciled at a Base safe block.",
                    "boundary": "This proves reserve recovery only. It is not competition cancellation, GMV, bounty payout, or settlement evidence.",
                }
                existing = load_json(self.result_output)
                if existing is None:
                    store_result(self.result_output, result)
                elif existing != result:
                    raise RecoveryError("saved recovery evidence differs")
            with self.lock:
                self.status = result
        except Exception as error:  # every reconciliation failure must remain visible and restartable
            with self.lock:
                self.status = {
                    "status": "failed",
                    "stage": stage,
                    "error": str(error),
                    "message": "The exact transaction hash is saved. Restart the server to reconcile it again without rebroadcasting.",
                }
        finally:
            with self.lock:
                self.reconciling.discard(stage)


class RecoveryHandler(BaseHTTPRequestHandler):
    server: RecoveryServer

    def log_message(self, format: str, *args: object) -> None:
        return

    def trusted_request(self) -> bool:
        host = self.headers.get("host", "").split(":", 1)[0].lower()
        return host in {"127.0.0.1", "localhost"}

    def send(self, status: int, content_type: str, body: bytes) -> None:
        self.send_response(status)
        self.send_header("content-type", content_type)
        self.send_header("cache-control", "no-store")
        self.send_header("content-security-policy", "default-src 'self'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; connect-src 'self'")
        self.send_header("referrer-policy", "no-referrer")
        self.send_header("x-content-type-options", "nosniff")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self) -> None:  # noqa: N802
        if not self.trusted_request():
            self.send(421, "application/json", b'{"error":"untrusted host"}')
            return
        parsed = urlsplit(self.path)
        try:
            if parsed.path == "/":
                self.send(200, "text/html; charset=utf-8", HTML.encode())
                return
            if parsed.path == "/plan":
                public = {key: self.server.plan[key] for key in ("schema", "plan_id", "chain_id", "owner", "reserve", "settlement_token", "display", "transactions", "boundary")}
                self.send(200, "application/json", json.dumps(public).encode())
                return
            if parsed.path == "/status":
                with self.server.lock:
                    value = dict(self.server.status)
                self.send(200, "application/json", json.dumps(value).encode())
                return
            if parsed.path == "/simulate":
                stage = str(parse_qs(parsed.query).get("stage", [""])[0])
                value = self.server.simulate(stage)
                self.send(200, "application/json", json.dumps(value).encode())
                return
        except Exception as error:
            self.send(409, "application/json", json.dumps({"error": str(error)}).encode())
            return
        self.send(404, "application/json", b'{"error":"not found"}')

    def do_POST(self) -> None:  # noqa: N802
        if not self.trusted_request():
            self.send(421, "application/json", b'{"error":"untrusted host"}')
            return
        expected_origin = f"http://127.0.0.1:{self.server.server_port}"
        if self.headers.get("origin") != expected_origin or self.path != "/submitted":
            self.send(403, "application/json", b'{"error":"untrusted origin or path"}')
            return
        try:
            length = int(self.headers.get("content-length", "0"))
            if length <= 0 or length > 512:
                raise RecoveryError("transaction request size is invalid")
            value = json.loads(self.rfile.read(length))
            stage = str(value.get("stage") or "")
            transaction_hash = str(value.get("transaction_hash") or "")
            if not TRANSACTION_HASH.fullmatch(transaction_hash):
                raise RecoveryError("transaction hash must be exact bytes32")
            self.server.begin_reconciliation(stage, transaction_hash)
        except (RecoveryError, ValueError, json.JSONDecodeError) as error:
            self.send(400, "application/json", json.dumps({"error": str(error)}).encode())
            return
        self.send(202, "application/json", b'{"status":"pending"}')


def load_or_create_plan(
    result_output: Path,
    manifest: dict[str, Any],
    evidence: dict[str, Any],
    rpc_urls: tuple[str, ...],
    expected_balance: int,
    expected_lifetime_spent: int,
) -> dict[str, Any]:
    path = plan_path(result_output)
    existing = load_json(path)
    if existing is not None:
        validate_plan(existing)
        return existing
    state = base_rpc_call(
        rpc_urls,
        lambda w3: inspect_live(w3, manifest, evidence),
        "safe-block reserve inspection",
    )
    plan = build_plan(
        state, manifest, evidence, expected_balance, expected_lifetime_spent
    )
    store_result(path, plan)
    return plan


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--deployment", type=Path, required=True)
    parser.add_argument("--deployment-evidence", type=Path, required=True)
    parser.add_argument("--expected-balance", type=int, required=True, help="exact safe-block reserve USDC base units")
    parser.add_argument("--expected-lifetime-spent", type=int, required=True, help="exact safe-block lifetime-spent USDC base units")
    parser.add_argument("--rpc-url", action="append", dest="rpc_urls", help="credential-free Base RPC URL; repeat for failover")
    parser.add_argument("--result-output", type=Path, required=True)
    parser.add_argument("--port", type=int, default=8792)
    args = parser.parse_args()
    try:
        manifest = json.loads(args.deployment.read_text(encoding="utf-8-sig"))
        evidence = json.loads(
            args.deployment_evidence.read_text(encoding="utf-8-sig")
        )
        validate_manifest(manifest)
        validate_deployment_evidence(evidence, manifest)
        rpc_urls = normalized_rpc_urls(args.rpc_urls)
        plan = load_or_create_plan(
            args.result_output,
            manifest,
            evidence,
            rpc_urls,
            args.expected_balance,
            args.expected_lifetime_spent,
        )
        validate_plan(plan)
    except (OSError, json.JSONDecodeError, RecoveryError, TypeError, ValueError) as error:
        print(f"reserve recovery server blocked: {error}")
        return 2
    try:
        server = RecoveryServer(
            ("127.0.0.1", args.port),
            plan,
            manifest,
            evidence,
            rpc_urls,
            args.result_output,
        )
    except (OSError, json.JSONDecodeError, RecoveryError, TypeError, ValueError) as error:
        print(f"reserve recovery server blocked: {error}")
        return 2
    print(f"recovery_url=http://127.0.0.1:{args.port}/", flush=True)
    print(f"owner={OWNER}", flush=True)
    print(f"destination={RESERVE}", flush=True)
    print("chain=Base mainnet (8453); value=0 ETH for both transactions", flush=True)
    print(f"step_1={REVOKE_CALLDATA} revokePolicy(); step_2={RECOVER_CALLDATA} recoverUncommitted()", flush=True)
    print(f"approved_safe_block_amount={plan['display']['reserve_balance']}", flush=True)
    server.serve_forever()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
