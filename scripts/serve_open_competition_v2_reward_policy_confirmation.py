#!/usr/bin/env python3
"""Serve and reconcile the exact two-step MetaMask policy rotation."""

from __future__ import annotations

import argparse
import json
import os
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import re
import threading
import time
from typing import Any

from eth_abi import decode, encode
from eth_utils import to_checksum_address
from web3 import Web3
from web3.exceptions import TransactionNotFound

from build_open_competition_v2_reward_policy import (
    EXPECTED_LIFETIME_SPENT,
    EXPECTED_RESERVE_BALANCE,
    OWNER,
    SCHEMA,
    USDC,
)
from inspect_open_competition_v2_reward_policy import (
    POLICY_OUTPUTS,
    call_one,
    call_raw,
)


TRANSACTION_HASH = re.compile(r"^0x[0-9a-fA-F]{64}$")
HEX_DATA = re.compile(r"^0x(?:[0-9a-fA-F]{2})+$")


HTML = r"""<!doctype html>
<html lang="en">
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>Authorize the 6-USDC GMV comparison cohort</title>
<style>
  :root { color-scheme: dark; font-family: Inter, ui-sans-serif, system-ui, sans-serif; }
  body { margin: 0; background: #07111f; color: #eef5ff; }
  main { max-width: 800px; margin: 4vh auto; padding: 28px; }
  article { background: #0d1d31; border: 1px solid #274563; border-radius: 20px; padding: 28px; box-shadow: 0 24px 80px #0008; }
  h1 { margin: 0 0 12px; font-size: clamp(28px, 5vw, 48px); line-height: 1.05; }
  p { color: #b8cbe0; line-height: 1.55; }
  dl { display: grid; grid-template-columns: 1fr 2fr; gap: 12px 18px; margin: 26px 0; }
  dt { color: #8da8c4; } dd { margin: 0; overflow-wrap: anywhere; font-weight: 650; }
  .good { color: #7ff5b0; } .warn { color: #ffd580; }
  .wallet-picker { display: grid; gap: 8px; margin: 22px 0 14px; color: #b8cbe0; }
  select { width: 100%; border: 1px solid #466987; border-radius: 12px; padding: 13px 14px; background: #07111f; color: #eef5ff; font: inherit; }
  button { width: 100%; border: 0; border-radius: 14px; padding: 16px 20px; font: inherit; font-weight: 800; background: #67f0a5; color: #06120b; cursor: pointer; }
  button:disabled { opacity: .55; cursor: wait; }
  #status { min-height: 48px; }
  code { font-family: ui-monospace, monospace; font-size: .9em; }
</style>
<main><article>
  <div class="good">Base mainnet · exact zero-value owner transaction</div>
  <h1>Safely authorize five 6-USDC GMV competitions</h1>
  <p>This first revokes the old delegate policy, verifies the unchanged reserve at a Base safe block, and only then installs the treatment policy. Both owner transactions send <strong>0 USDC</strong> and <strong>0 ETH</strong>. After canonical confirmation, the existing delegate may create only the five displayed preapproved Open Competition V2 contracts. The owner keeps revocation and recovery of uncommitted USDC.</p>
  <dl id="facts"></dl>
  <p class="warn">MetaMask will request two zero-value confirmations. This closes the race in which the old delegate could spend before a one-step replacement was mined. The five later creations may spend 30.20 USDC from the reserve; they do not happen in either owner transaction.</p>
  <label class="wallet-picker" for="wallet-provider">Wallet
    <select id="wallet-provider"><option value="">Discovering installed wallets…</option></select>
  </label>
  <button id="authorize">Start safe two-step policy update</button>
  <p id="status" aria-live="polite"></p>
</article></main>
<script>
(async () => {
  const wallets = [];
  const seenProviders = new Set();
  const walletSelect = document.querySelector('#wallet-provider');
  const status = document.querySelector('#status');
  const button = document.querySelector('#authorize');
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

  const bundle = await fetch('/bundle', {cache: 'no-store'}).then(response => response.json());
  const summary = bundle.confirmation_summary;
  const facts = [
    ['Owner wallet', summary.owner],
    ['Reserve', summary.reserve],
    ['This transaction moves', `${summary.usdc_moved_by_confirmation}; ${summary.transaction_value}`],
    ['Owner confirmations', String(summary.transaction_count)],
    ['Policy action', summary.action],
    ['Each later competition', summary.per_competition],
    ['Five-competition treatment', summary.treatment_total],
    ['Reserve after treatment', summary.reserve_after_treatment],
    ['Reserved for later five-bounty floor', summary.later_floor_reserved],
    ['UTC-day cap', summary.maximum_utc_day],
    ['Lifetime cap', summary.maximum_lifetime],
  ];
  document.querySelector('#facts').innerHTML = facts.map(([key, value]) => `<dt>${key}</dt><dd><code>${value}</code></dd>`).join('');

  async function ensureBase(provider) {
    let chain = String(await provider.request({method: 'eth_chainId'})).toLowerCase();
    if (chain !== '0x2105') {
      await provider.request({method: 'wallet_switchEthereumChain', params: [{chainId: '0x2105'}]});
      chain = String(await provider.request({method: 'eth_chainId'})).toLowerCase();
    }
    if (chain !== '0x2105') throw new Error('MetaMask is not on Base mainnet.');
  }

  async function pollCanonical(targetStatus) {
    for (let attempt = 0; attempt < 80; attempt += 1) {
      const result = await fetch('/status', {cache: 'no-store'}).then(response => response.json());
      if (result.status === targetStatus) return result;
      if (result.status === 'failed') throw new Error(result.error || 'Canonical policy reconciliation failed.');
      status.textContent = result.message || 'Waiting for the transaction and Base safe-block reconciliation…';
      await new Promise(resolve => setTimeout(resolve, 3000));
    }
    throw new Error('The transaction is still pending canonical reconciliation. Leave this page open and retry status shortly.');
  }

  function walletTransaction(template, account) {
    const transaction = {...template, from: account, value: '0x0'};
    delete transaction.value_wei;
    delete transaction.function;
    return transaction;
  }

  async function submitStage(stage, provider, account) {
    const transaction = walletTransaction(bundle.owner_transactions[stage], account);
    await provider.request({method: 'eth_call', params: [transaction, 'latest']});
    const transactionHash = await provider.request({method: 'eth_sendTransaction', params: [transaction]});
    const submitted = await fetch('/submitted', {
      method: 'POST',
      headers: {'content-type': 'application/json'},
      body: JSON.stringify({stage, transaction_hash: transactionHash}),
    });
    const accepted = await submitted.json();
    if (!submitted.ok) throw new Error(accepted.error || 'The local verifier rejected the transaction hash.');
    return transactionHash;
  }

  button.addEventListener('click', async () => {
    button.disabled = true;
    try {
      if (Math.floor(Date.now() / 1000) >= bundle.confirmation_deadline) throw new Error('This matched cohort has expired. Rebuild it without the first window.');
      const selected = wallets[Number(walletSelect.value)];
      if (!selected) throw new Error('No injected wallet is available. Enable MetaMask for this page, then retry.');
      await ensureBase(selected.provider);
      const accounts = await selected.provider.request({method: 'eth_requestAccounts'});
      const account = String(accounts[0] || '').toLowerCase();
      if (account !== bundle.owner.toLowerCase()) throw new Error(`Select ${bundle.owner} in MetaMask, then retry.`);
      const current = await fetch('/status', {cache: 'no-store'}).then(response => response.json());
      if (current.status === 'confirmed') {
        status.textContent = `Policy version ${current.policy_version} is already canonically confirmed at safe block ${current.safe_block}. No USDC moved.`;
        status.className = 'good';
        button.textContent = 'Two-step policy update canonically confirmed';
        return;
      }
      if (current.status !== 'revoked') {
        status.textContent = 'Simulating the exact old-policy revocation…';
        await submitStage('revoke', selected.provider, account);
        status.textContent = 'Revocation submitted. Waiting for safe-block proof that no USDC or lifetime authority changed…';
        await pollCanonical('revoked');
      }
      status.textContent = 'Old policy is safely revoked. Review the second zero-value treatment-policy transaction in MetaMask.';
      await submitStage('configure', selected.provider, account);
      status.textContent = 'Treatment policy submitted. Waiting for exact receipt and Base safe-block policy evidence…';
      const result = await pollCanonical('confirmed');
      status.textContent = `Policy version ${result.policy_version} is canonically confirmed at safe block ${result.safe_block}. No USDC moved.`;
      status.className = 'good';
      button.textContent = 'Two-step policy update canonically confirmed';
    } catch (error) {
      status.textContent = error?.message || String(error);
      status.className = 'warn';
      button.disabled = false;
    }
  });
})().catch(error => { document.querySelector('#status').textContent = String(error); });
</script>
</html>"""


class ConfirmationError(ValueError):
    pass


def validate_bundle(bundle: dict[str, Any]) -> None:
    if bundle.get("schema_version") != SCHEMA or bundle.get("chain_id") != 8453:
        raise ConfirmationError("policy bundle schema or chain is invalid")
    if str(bundle.get("owner")).lower() != OWNER:
        raise ConfirmationError("policy bundle owner is invalid")
    transactions = bundle.get("owner_transactions")
    if not isinstance(transactions, dict) or set(transactions) != {
        "revoke",
        "configure",
    }:
        raise ConfirmationError(
            "exact revoke and configure owner transactions are required"
        )
    for stage, transaction in transactions.items():
        if not isinstance(transaction, dict):
            raise ConfirmationError(f"{stage} owner transaction is missing")
        if str(transaction.get("from")).lower() != OWNER:
            raise ConfirmationError(f"{stage} owner transaction sender is invalid")
        if (
            str(transaction.get("to")).lower()
            != str(bundle.get("reserve_wallet")).lower()
        ):
            raise ConfirmationError(f"{stage} owner transaction destination is invalid")
        if int(transaction.get("value_wei", -1)) != 0 or not HEX_DATA.fullmatch(
            str(transaction.get("data") or "")
        ):
            raise ConfirmationError(
                f"{stage} owner transaction must be exact nonempty zero-value calldata"
            )
    if transactions["revoke"]["data"].lower() != "0x9eba3667":
        raise ConfirmationError("revoke transaction selector is invalid")
    if (
        transactions["revoke"]["data"].lower()
        == transactions["configure"]["data"].lower()
    ):
        raise ConfirmationError("revoke and configure calldata must differ")
    if (
        bundle.get("execution_boundary", {}).get("policy_change_moves_usdc")
        is not False
    ):
        raise ConfirmationError("bundle does not prove a zero-USDC policy update")
    if (
        int(bundle.get("current_policy", {}).get("lifetime_spent", -1))
        != EXPECTED_LIFETIME_SPENT
    ):
        raise ConfirmationError("bundle lifetime spend changed")
    if (
        int(bundle.get("current_policy", {}).get("reserve_balance", -1))
        != EXPECTED_RESERVE_BALANCE
    ):
        raise ConfirmationError("bundle reserve balance changed")
    if int(bundle.get("next_policy", {}).get("version", 0)) != 2:
        raise ConfirmationError("bundle is not the exact version-2 policy rotation")
    if len(bundle.get("approved_creation_commitments", [])) != 5:
        raise ConfirmationError("bundle must approve exactly five creations")


def transaction_matches(bundle: dict[str, Any], stage: str, transaction: Any) -> None:
    if stage not in {"revoke", "configure"}:
        raise ConfirmationError("transaction stage is invalid")
    expected = bundle["owner_transactions"][stage]
    if lower_hex(transaction.get("from")) != lower_hex(expected["from"]):
        raise ConfirmationError("confirmed transaction sender differs")
    if lower_hex(transaction.get("to")) != lower_hex(expected["to"]):
        raise ConfirmationError("confirmed transaction destination differs")
    if int(transaction.get("value", 0)) != 0:
        raise ConfirmationError("confirmed transaction value is not zero")
    actual_input = transaction.get("input", transaction.get("data", ""))
    if lower_hex(actual_input) != lower_hex(expected["data"]):
        raise ConfirmationError("confirmed transaction calldata differs")


def lower_hex(value: object) -> str:
    if isinstance(value, bytes):
        return "0x" + value.hex()
    if hasattr(value, "hex") and not isinstance(value, str):
        return str(value.hex()).lower()
    return str(value or "").lower()


def store_result(path: Path, value: dict[str, Any]) -> None:
    payload = (json.dumps(value, indent=2) + "\n").encode()
    path = path.resolve()
    path.parent.mkdir(parents=True, exist_ok=True)
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_BINARY", 0)
    descriptor = os.open(path, flags, 0o600)
    try:
        view = memoryview(payload)
        while view:
            written = os.write(descriptor, view)
            if written <= 0:
                raise OSError("result output write made no progress")
            view = view[written:]
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def revocation_result_path(result_output: Path) -> Path:
    return result_output.with_name(f"{result_output.name}.revoked")


def load_revocation_result(
    result_output: Path, bundle: dict[str, Any]
) -> dict[str, Any] | None:
    path = revocation_result_path(result_output)
    if not path.exists():
        return None
    value = json.loads(path.read_text(encoding="utf-8-sig"))
    current = bundle["current_policy"]
    if (
        value.get("status") != "revoked"
        or not TRANSACTION_HASH.fullmatch(
            str(value.get("revoke_transaction_hash") or "")
        )
        or int(value.get("policy_version", -1)) != int(current["version"])
        or str(value.get("policy_hash") or "").lower() != str(current["hash"]).lower()
        or int(value.get("lifetime_spent", -1)) != EXPECTED_LIFETIME_SPENT
        or int(value.get("reserve_balance", -1)) != EXPECTED_RESERVE_BALANCE
    ):
        raise ConfirmationError("stored revocation result differs from the bundle")
    return value


def ensure_confirmation_open(bundle: dict[str, Any]) -> None:
    if int(time.time()) >= int(bundle.get("confirmation_deadline", 0)):
        raise ConfirmationError("policy confirmation deadline has passed")


class ConfirmationServer(ThreadingHTTPServer):
    def __init__(
        self,
        address: tuple[str, int],
        bundle: dict[str, Any],
        rpc_url: str,
        result_output: Path,
    ) -> None:
        super().__init__(address, ConfirmationHandler)
        self.bundle = bundle
        self.rpc_url = rpc_url
        self.result_output = result_output
        self.lock = threading.Lock()
        self.transaction_hashes: dict[str, str] = {}
        prior_revocation = load_revocation_result(result_output, bundle)
        if prior_revocation:
            self.status = prior_revocation
            self.transaction_hashes["revoke"] = str(
                prior_revocation["revoke_transaction_hash"]
            )
        else:
            self.status = {
                "status": "ready",
                "message": "Ready for the exact owner confirmation.",
            }

    def begin_reconciliation(self, stage: str, transaction_hash: str) -> None:
        if stage not in {"revoke", "configure"}:
            raise ConfirmationError("transaction stage is invalid")
        ensure_confirmation_open(self.bundle)
        with self.lock:
            if stage == "configure" and self.status.get("status") != "revoked":
                raise ConfirmationError(
                    "the old policy is not safely revoked and reconciled"
                )
            existing = self.transaction_hashes.get(stage)
            if existing and existing.lower() != transaction_hash.lower():
                raise ConfirmationError(
                    f"a different {stage} transaction is already being reconciled"
                )
            if existing:
                return
            self.transaction_hashes[stage] = transaction_hash
            self.status = {
                "status": "pending",
                "stage": stage,
                "message": f"Waiting for the exact {stage} Base receipt…",
            }
        threading.Thread(
            target=self.reconcile, args=(stage, transaction_hash), daemon=True
        ).start()

    def reconcile(self, stage: str, transaction_hash: str) -> None:
        try:
            w3 = Web3(Web3.HTTPProvider(self.rpc_url))
            if not w3.is_connected() or int(w3.eth.chain_id) != 8453:
                raise ConfirmationError("canonical Base RPC is unavailable")
            receipt = None
            for _ in range(120):
                try:
                    receipt = w3.eth.get_transaction_receipt(transaction_hash)
                except TransactionNotFound:
                    receipt = None
                if receipt is not None:
                    break
                time.sleep(2)
            if receipt is None:
                raise ConfirmationError("transaction receipt did not arrive")
            if int(receipt["status"]) != 1:
                raise ConfirmationError(f"{stage} transaction reverted")
            transaction = w3.eth.get_transaction(transaction_hash)
            transaction_matches(self.bundle, stage, transaction)
            receipt_block = int(receipt["blockNumber"])
            safe_block = 0
            for _ in range(120):
                response = w3.provider.make_request(
                    "eth_getBlockByNumber", ["safe", False]
                )
                value = response.get("result") if isinstance(response, dict) else None
                safe_block = (
                    int(str(value.get("number")), 16)
                    if isinstance(value, dict) and value.get("number")
                    else 0
                )
                if safe_block >= receipt_block:
                    break
                with self.lock:
                    self.status = {
                        "status": "pending",
                        "stage": stage,
                        "message": f"{stage.title()} receipt confirmed; waiting for Base safe-block inclusion…",
                    }
                time.sleep(2)
            if safe_block < receipt_block:
                raise ConfirmationError("transaction did not reach the Base safe block")
            reserve = str(self.bundle["reserve_wallet"])
            version = int(
                call_one(w3, reserve, "policyVersion()", "uint64", safe_block)
            )
            policy_hash = (
                "0x"
                + call_one(
                    w3, reserve, "activePolicyHash()", "bytes32", safe_block
                ).hex()
            )
            lifetime_spent = int(
                call_one(w3, reserve, "lifetimeSpent()", "uint256", safe_block)
            )
            balance_arguments = encode(["address"], [to_checksum_address(reserve)])
            balance = int(
                decode(
                    ["uint256"],
                    call_raw(
                        w3, USDC, "balanceOf(address)", safe_block, balance_arguments
                    ),
                )[0]
            )
            policy = decode(
                POLICY_OUTPUTS, call_raw(w3, reserve, "policy()", safe_block)
            )
            if (
                lifetime_spent != EXPECTED_LIFETIME_SPENT
                or balance != EXPECTED_RESERVE_BALANCE
            ):
                raise ConfirmationError(
                    f"{stage} confirmation unexpectedly moved or spent USDC"
                )
            revoked = bool(call_one(w3, reserve, "revoked()", "bool", safe_block))
            if stage == "revoke":
                current = self.bundle["current_policy"]
                if (
                    version != int(current["version"])
                    or policy_hash.lower() != str(current["hash"]).lower()
                    or not revoked
                ):
                    raise ConfirmationError(
                        "safe-block revocation state, version, or policy hash differs"
                    )
                result = {
                    "status": "revoked",
                    "revoke_transaction_hash": transaction_hash,
                    "transaction_block": receipt_block,
                    "safe_block": safe_block,
                    "policy_version": version,
                    "policy_hash": policy_hash,
                    "lifetime_spent": lifetime_spent,
                    "reserve_balance": balance,
                    "message": "Old policy safely revoked with unchanged USDC and lifetime spend.",
                }
                store_result(revocation_result_path(self.result_output), result)
                with self.lock:
                    self.status = result
                return
            expected = self.bundle["next_policy"]
            if (
                version != int(expected["version"])
                or policy_hash.lower() != str(expected["hash"]).lower()
                or revoked
            ):
                raise ConfirmationError(
                    "safe-block policy version, hash, or active state differs"
                )
            if int(policy[4]) != int(expected["solver_reward"]) or int(
                policy[6]
            ) != int(expected["exact_funding_per_competition"]):
                raise ConfirmationError("safe-block policy economics differ")
            result = {
                "status": "confirmed",
                "revoke_transaction_hash": self.transaction_hashes["revoke"],
                "configure_transaction_hash": transaction_hash,
                "transaction_block": receipt_block,
                "safe_block": safe_block,
                "policy_version": version,
                "policy_hash": policy_hash,
                "lifetime_spent": lifetime_spent,
                "reserve_balance": balance,
                "usdc_moved_by_confirmation": 0,
                "evidence_boundary": "This proves only the safely sequenced owner revocation and policy rotation. It is not competition activation, GMV, entry, payout, or settlement evidence.",
            }
            store_result(self.result_output, result)
            with self.lock:
                self.status = result
        except (
            Exception
        ) as error:  # reconciliation must surface every fail-closed reason
            with self.lock:
                self.status = {"status": "failed", "error": str(error)}


class ConfirmationHandler(BaseHTTPRequestHandler):
    server: ConfirmationServer

    def log_message(self, format: str, *args: object) -> None:
        return

    def send(self, status: int, content_type: str, body: bytes) -> None:
        self.send_response(status)
        self.send_header("content-type", content_type)
        self.send_header("cache-control", "no-store")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self) -> None:  # noqa: N802
        if self.path == "/":
            self.send(200, "text/html; charset=utf-8", HTML.encode())
            return
        if self.path == "/bundle":
            public = {
                key: self.server.bundle[key]
                for key in (
                    "owner",
                    "reserve_wallet",
                    "confirmation_deadline",
                    "confirmation_summary",
                    "owner_transactions",
                )
            }
            self.send(200, "application/json", json.dumps(public).encode())
            return
        if self.path == "/status":
            with self.server.lock:
                value = dict(self.server.status)
            self.send(200, "application/json", json.dumps(value).encode())
            return
        self.send(404, "application/json", b'{"error":"not found"}')

    def do_POST(self) -> None:  # noqa: N802
        if self.path != "/submitted":
            self.send(404, "application/json", b'{"error":"not found"}')
            return
        try:
            length = int(self.headers.get("content-length", "0"))
            if length <= 0 or length > 512:
                raise ConfirmationError("transaction request size is invalid")
            value = json.loads(self.rfile.read(length))
            stage = str(value.get("stage") or "")
            transaction_hash = str(value.get("transaction_hash") or "")
            if not TRANSACTION_HASH.fullmatch(transaction_hash):
                raise ConfirmationError("transaction hash must be exact bytes32")
            self.server.begin_reconciliation(stage, transaction_hash)
        except (ConfirmationError, ValueError, json.JSONDecodeError) as error:
            self.send(
                400, "application/json", json.dumps({"error": str(error)}).encode()
            )
            return
        self.send(202, "application/json", b'{"status":"pending"}')


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bundle", type=Path, required=True)
    parser.add_argument("--rpc-url", default="https://base-rpc.publicnode.com")
    parser.add_argument("--result-output", type=Path, required=True)
    parser.add_argument("--port", type=int, default=8791)
    args = parser.parse_args()
    try:
        bundle = json.loads(args.bundle.read_text(encoding="utf-8-sig"))
        validate_bundle(bundle)
        ensure_confirmation_open(bundle)
        if args.result_output.exists():
            raise ConfirmationError("result output already exists")
    except (
        OSError,
        json.JSONDecodeError,
        ConfirmationError,
        TypeError,
        ValueError,
    ) as error:
        print(f"confirmation server blocked: {error}")
        return 2
    server = ConfirmationServer(
        ("127.0.0.1", args.port), bundle, args.rpc_url, args.result_output
    )
    print(f"confirmation_url=http://127.0.0.1:{args.port}/", flush=True)
    server.serve_forever()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
