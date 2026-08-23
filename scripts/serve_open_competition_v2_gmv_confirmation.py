#!/usr/bin/env python3
"""Serve a local, exact EIP-3009 confirmation page and verify its signature."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

from eth_account import Account
from eth_account.messages import encode_typed_data


SIGNATURE = re.compile(r"^0x[0-9a-fA-F]{130}$")


HTML = r"""<!doctype html>
<html lang="en">
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>Authorize GMV meta-bounty reserve</title>
<style>
  :root { color-scheme: dark; font-family: Inter, ui-sans-serif, system-ui, sans-serif; }
  body { margin: 0; background: #07111f; color: #eef5ff; }
  main { max-width: 760px; margin: 5vh auto; padding: 28px; }
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
  #status { min-height: 24px; }
  code { font-family: ui-monospace, monospace; font-size: .9em; }
</style>
<main><article>
  <div class="good">Base mainnet · exact one-time authorization</div>
  <h1>Fund competitive GMV meta-bounties</h1>
  <p>This authorizes native USDC from the displayed owner wallet into an owner-recoverable bounded reserve. The delegate cannot transfer or withdraw USDC; it can only create the twenty preapproved Open Competition V2 highest-GMV contests under the caps below.</p>
  <dl id="facts"></dl>
  <p class="warn">This signature authorizes 77.668098 USDC. It is not an email login and does not transfer funds to the delegate wallet.</p>
  <label class="wallet-picker" for="wallet-provider">Wallet
    <select id="wallet-provider"><option value="">Discovering installed wallets…</option></select>
  </label>
  <button id="authorize">Authorize exact amount with selected wallet</button>
  <p id="status" aria-live="polite"></p>
</article></main>
<script>
(async () => {
  const wallets = [];
  const seenProviders = new Set();
  const walletSelect = document.querySelector('#wallet-provider');
  let userSelectedWallet = false;
  walletSelect.addEventListener('change', () => { userSelectedWallet = true; });

  function addWallet(provider, info = {}) {
    if (!provider || seenProviders.has(provider)) return;
    seenProviders.add(provider);
    const rdns = String(info.rdns || '').toLowerCase();
    const isCoinbase = rdns.includes('coinbase') || Boolean(provider.isCoinbaseWallet);
    const isBrave = rdns.includes('brave') || Boolean(provider.isBraveWallet);
    const isMetaMask = !isCoinbase && !isBrave &&
      (rdns === 'io.metamask' || Boolean(provider.isMetaMask));
    const name = String(info.name || (isMetaMask ? 'MetaMask' :
      isCoinbase ? 'Coinbase Wallet' : isBrave ? 'Brave Wallet' : 'Injected wallet'));
    wallets.push({provider, name, rdns, isMetaMask, isCoinbase, isBrave});
    renderWallets();
  }

  function renderWallets() {
    const previouslySelected = userSelectedWallet ? wallets[Number(walletSelect.value)]?.provider : null;
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
    const previousIndex = wallets.findIndex(wallet => wallet.provider === previouslySelected);
    const metaMaskIndex = wallets.findIndex(wallet => wallet.isMetaMask);
    walletSelect.value = String(previousIndex >= 0 ? previousIndex : metaMaskIndex >= 0 ? metaMaskIndex : 0);
  }

  window.addEventListener('eip6963:announceProvider', event => {
    addWallet(event.detail && event.detail.provider, event.detail && event.detail.info);
  });
  window.dispatchEvent(new Event('eip6963:requestProvider'));
  const legacyProviders = Array.isArray(window.ethereum && window.ethereum.providers)
    ? window.ethereum.providers : window.ethereum ? [window.ethereum] : [];
  legacyProviders.forEach(provider => addWallet(provider));

  async function assertBaseMainnetWhenQueryable(provider) {
    try {
      const chain = await provider.request({method: 'eth_chainId'});
      if (String(chain).toLowerCase() !== '0x2105') {
        throw new Error('Switch the wallet to Base mainnet, then retry.');
      }
      return true;
    } catch (error) {
      const code = Number(error && error.code);
      const message = String(error && error.message ? error.message : error).toLowerCase();
      const methodUnavailable = code === -32601 || code === 4200 ||
        message.includes('method not found') || message.includes('method is not supported') ||
        message.includes('unsupported method') ||
        (message.includes('request method') && message.includes('is not supported'));
      if (!methodUnavailable) throw error;
      return false;
    }
  }

  const bundle = await fetch('/bundle', {cache: 'no-store'}).then(r => r.json());
  const summary = bundle.confirmation_summary;
  const facts = [
    ['Owner wallet', summary.wallet],
    ['Amount', summary.amount],
    ['Destination', summary.destination],
    ['Destination type', summary.destination_kind],
    ['Per competition cap', summary.maximum_single_competition],
    ['UTC-day cap', summary.maximum_utc_day],
    ['Lifetime cap', summary.maximum_lifetime],
    ['Objective', summary.objective],
  ];
  document.querySelector('#facts').innerHTML = facts.map(([k,v]) => `<dt>${k}</dt><dd><code>${v}</code></dd>`).join('');
  const button = document.querySelector('#authorize');
  const status = document.querySelector('#status');
  button.addEventListener('click', async () => {
    button.disabled = true;
    try {
      const selection = wallets[Number(walletSelect.value)];
      if (!selection) throw new Error('No injected wallet is available. Enable MetaMask for this site, then retry.');
      const provider = selection.provider;
      const chainWasChecked = await assertBaseMainnetWhenQueryable(provider);
      if (!chainWasChecked) {
        status.textContent = 'This wallet cannot report its selected chain. The signature request itself remains locked to Base mainnet.';
      }
      const accounts = await provider.request({method: 'eth_requestAccounts'});
      const account = String(accounts[0] || '').toLowerCase();
      if (account !== bundle.owner.toLowerCase()) throw new Error(`Select ${bundle.owner} in the wallet, then retry.`);
      status.textContent = 'Review the exact 77.668098-USDC authorization in your wallet.';
      const signature = await provider.request({
        method: 'eth_signTypedData_v4',
        params: [account, JSON.stringify(bundle.owner_authorization.typed_data)],
      });
      const response = await fetch('/signature', {
        method: 'POST', headers: {'content-type': 'application/json'}, body: JSON.stringify({signature}),
      });
      const result = await response.json();
      if (!response.ok) throw new Error(result.error || 'The local verifier rejected the signature.');
      status.textContent = 'Signature verified locally. The relayer can now submit the exact reserve funding call.';
      status.className = 'good';
      button.textContent = 'Authorization verified';
    } catch (error) {
      status.textContent = error && error.message ? error.message : String(error);
      status.className = 'warn';
      button.disabled = false;
    }
  });
})().catch(error => { document.querySelector('#status').textContent = String(error); });
</script>
</html>"""


def store_verified_signature(path: Path, owner: str, signature: str) -> str:
    """Persist the verified signature once without exposing it in process output."""
    path = path.resolve()
    path.parent.mkdir(parents=True, exist_ok=True)
    payload = (json.dumps({"owner": owner, "signature": signature}) + "\n").encode()
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_BINARY", 0)
    descriptor = os.open(path, flags, 0o600)
    try:
        view = memoryview(payload)
        while view:
            written = os.write(descriptor, view)
            if written <= 0:
                raise OSError("signature output write made no progress")
            view = view[written:]
        os.fsync(descriptor)
    finally:
        os.close(descriptor)
    return "0x" + hashlib.sha256(signature.lower().encode()).hexdigest()


class ConfirmationServer(ThreadingHTTPServer):
    def __init__(self, address: tuple[str, int], bundle: dict, signature_output: Path) -> None:
        super().__init__(address, ConfirmationHandler)
        self.bundle = bundle
        self.signature_output = signature_output


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
        elif self.path == "/bundle":
            public = {
                key: self.server.bundle[key]
                for key in (
                    "owner",
                    "reserve_wallet",
                    "confirmation_summary",
                    "owner_authorization",
                )
            }
            self.send(200, "application/json", json.dumps(public).encode())
        else:
            self.send(404, "application/json", b'{"error":"not found"}')

    def do_POST(self) -> None:  # noqa: N802
        if self.path != "/signature":
            self.send(404, "application/json", b'{"error":"not found"}')
            return
        try:
            length = int(self.headers.get("content-length", "0"))
            if length <= 0 or length > 1_024:
                raise ValueError("signature request size is invalid")
            value = json.loads(self.rfile.read(length))
            signature = str(value.get("signature") or "")
            if not SIGNATURE.fullmatch(signature):
                raise ValueError("signature must be 65-byte hex")
            typed_data = self.server.bundle["owner_authorization"]["typed_data"]
            recovered = Account.recover_message(
                encode_typed_data(full_message=typed_data), signature=signature
            ).lower()
            expected = str(self.server.bundle["owner"]).lower()
            if recovered != expected:
                raise ValueError(f"signature recovered {recovered}, expected {expected}")
        except (ValueError, KeyError, json.JSONDecodeError) as error:
            self.send(400, "application/json", json.dumps({"error": str(error)}).encode())
            return
        try:
            signature_hash = store_verified_signature(
                self.server.signature_output, recovered, signature
            )
        except OSError as error:
            self.send(409, "application/json", json.dumps({"error": str(error)}).encode())
            return
        print(
            json.dumps(
                {
                    "status": "signature_verified",
                    "owner": recovered,
                    "signature_sha256": signature_hash,
                }
            ),
            flush=True,
        )
        self.send(200, "application/json", b'{"status":"signature_verified"}')


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bundle", type=Path, required=True)
    parser.add_argument("--signature-output", type=Path, required=True)
    parser.add_argument("--port", type=int, default=8787)
    args = parser.parse_args()
    bundle = json.loads(args.bundle.read_text(encoding="utf-8-sig"))
    server = ConfirmationServer(("127.0.0.1", args.port), bundle, args.signature_output)
    print(f"confirmation_url=http://127.0.0.1:{args.port}/", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
