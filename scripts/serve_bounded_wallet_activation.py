#!/usr/bin/env python3
"""Serve the locked activation UI and relay only bundle-pinned transactions."""

from __future__ import annotations

import argparse
import json
import re
import shutil
import subprocess
import time
import urllib.request
from http import HTTPStatus
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_BUNDLE = ROOT / "deployments" / "bounded-wallet-base-activation.json"
KEY_DIR = ROOT / "target" / "bounded-wallet-activation" / "keys"
POLICY_TYPE = "(address,uint64,uint64,uint64,uint256,uint256,uint256,uint8,uint8)"
CREATE_WITH_AUTHORIZATION = (
    f"createWalletWithAuthorization(address,{POLICY_TYPE},bytes32,uint256,uint256,"
    "uint256,bytes32,uint8,bytes32,bytes32)"
)
PERMIT = "permit(address,address,uint256,uint256,uint8,bytes32,bytes32)"
HEX_32 = re.compile(r"^0x[0-9a-fA-F]{64}$")


def executable(name: str) -> str:
    found = shutil.which(name)
    if found:
        return found
    candidate = ROOT / ".tools" / "foundry" / f"{name}.exe"
    if candidate.exists():
        return str(candidate)
    raise SystemExit(f"{name} is required; install Foundry or use .tools/foundry")


CAST = executable("cast")


def run(*args: str) -> str:
    result = subprocess.run(
        [CAST, *args],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
    )
    return result.stdout.strip()


def rpc(url: str, method: str, params: list):
    request = urllib.request.Request(
        url,
        data=json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode(),
        headers={
            "content-type": "application/json",
            "user-agent": "agent-bounties-bounded-wallet-activation/1",
        },
    )
    with urllib.request.urlopen(request, timeout=45) as response:
        body = json.loads(response.read().decode())
    if "error" in body:
        raise RuntimeError(f"RPC {method} failed: {body['error']}")
    return body["result"]


def wait_receipt(url: str, transaction_hash: str, timeout: int = 240) -> dict:
    deadline = time.time() + timeout
    while time.time() < deadline:
        receipt = rpc(url, "eth_getTransactionReceipt", [transaction_hash])
        if receipt:
            if int(receipt["status"], 16) != 1:
                raise RuntimeError(f"transaction reverted: {transaction_hash}")
            return receipt
        time.sleep(1.5)
    raise TimeoutError(f"transaction receipt timed out: {transaction_hash}")


def relayer_address() -> str:
    return run(
        "wallet",
        "address",
        "--keystore",
        str(KEY_DIR / "relayer.keystore"),
        "--password-file",
        str(KEY_DIR / "relayer.password"),
    ).lower()


def send_raw(network: dict, to: str, data: str) -> tuple[str, dict]:
    transaction_hash = run(
        "send",
        to,
        "--data",
        data,
        "--rpc-url",
        network["rpc_url"],
        "--keystore",
        str(KEY_DIR / "relayer.keystore"),
        "--password-file",
        str(KEY_DIR / "relayer.password"),
        "--async",
    ).splitlines()[-1]
    return transaction_hash, wait_receipt(network["rpc_url"], transaction_hash)


def send_call(network: dict, to: str, signature: str, *args: str) -> tuple[str, dict]:
    transaction_hash = run(
        "send",
        to,
        signature,
        *args,
        "--rpc-url",
        network["rpc_url"],
        "--keystore",
        str(KEY_DIR / "relayer.keystore"),
        "--password-file",
        str(KEY_DIR / "relayer.password"),
        "--async",
    ).splitlines()[-1]
    return transaction_hash, wait_receipt(network["rpc_url"], transaction_hash)


def call_int(network: dict, contract: str, signature: str, *args: str) -> int:
    value = run("call", contract, signature, *args, "--rpc-url", network["rpc_url"])
    return int(value.split()[0], 0)


def wait_call_int(
    network: dict,
    contract: str,
    signature: str,
    args: tuple[str, ...],
    expected: int,
    timeout: int = 30,
) -> int:
    deadline = time.time() + timeout
    observed = call_int(network, contract, signature, *args)
    while observed != expected and time.time() < deadline:
        time.sleep(1)
        observed = call_int(network, contract, signature, *args)
    return observed


def policy_tuple(wallet: dict) -> str:
    policy = wallet["policy"]
    return (
        f"({wallet['delegate']},{policy['valid_after']},{policy['valid_until']},"
        f"{policy['period_seconds']},{policy['max_per_action']},{policy['max_per_period']},"
        f"{policy['max_lifetime_spend']},{policy['allowed_actions']},"
        f"{policy['allowed_verification_modes']})"
    )


class ActivationServer(ThreadingHTTPServer):
    bundle: dict
    enable_mainnet: bool


class ActivationHandler(SimpleHTTPRequestHandler):
    server: ActivationServer

    def __init__(self, *args, **kwargs):
        super().__init__(*args, directory=str(ROOT), **kwargs)

    def end_headers(self) -> None:
        self.send_header("cache-control", "no-store")
        super().end_headers()

    def _json(self, status: HTTPStatus, value: dict) -> None:
        body = json.dumps(value).encode()
        self.send_response(status)
        self.send_header("content-type", "application/json")
        self.send_header("cache-control", "no-store")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def _network(self, name: str) -> dict:
        if name == "base-mainnet" and not self.server.enable_mainnet:
            raise ValueError("mainnet activation is disabled for this server")
        network = self.server.bundle["networks"].get(name)
        if not network:
            raise ValueError("unsupported activation network")
        return network

    def _body(self) -> dict:
        if self.headers.get_content_type() != "application/json":
            raise ValueError("activation requests require application/json")
        host = self.headers.get("host", "")
        if host not in {f"127.0.0.1:{self.server.server_port}", f"localhost:{self.server.server_port}"}:
            raise ValueError("activation host is not local")
        origin = self.headers.get("origin")
        if origin not in {
            f"http://127.0.0.1:{self.server.server_port}",
            f"http://localhost:{self.server.server_port}",
        }:
            raise ValueError("activation origin is not the local console")
        length = int(self.headers.get("content-length", "0"))
        if length <= 0 or length > 32_768:
            raise ValueError("invalid request body size")
        value = json.loads(self.rfile.read(length))
        if not isinstance(value, dict):
            raise ValueError("request body must be an object")
        return value

    def do_POST(self) -> None:  # noqa: N802
        try:
            body = self._body()
            network = self._network(str(body.get("network", "")))
            if relayer_address() != network["pilot"]["relayer"]:
                raise ValueError("local relayer does not match the activation bundle")
            if int(rpc(network["rpc_url"], "eth_getBalance", [network["pilot"]["relayer"], "latest"]), 16) == 0:
                raise ValueError("the pinned relayer has no gas")
            if self.path == "/activation/deploy-components":
                result = self._deploy_components(network)
            elif self.path == "/activation/fund-policy-wallets":
                result = self._fund_policy_wallets(network, body)
            else:
                self._json(HTTPStatus.NOT_FOUND, {"error": "unknown activation endpoint"})
                return
            self._json(HTTPStatus.OK, result)
        except (ValueError, KeyError, TypeError, json.JSONDecodeError) as error:
            self._json(HTTPStatus.BAD_REQUEST, {"error": str(error)})
        except Exception as error:
            self._json(HTTPStatus.INTERNAL_SERVER_ERROR, {"error": str(error)})

    def _deploy_components(self, network: dict) -> dict:
        transactions = []
        deployer = self.server.bundle["deterministic_deployer"]["contract"]
        for component in network["deployments"]:
            observed = rpc(network["rpc_url"], "eth_getCode", [component["expected_contract"], "latest"]).lower()
            if observed == component["runtime_code"]:
                continue
            if observed != "0x":
                raise ValueError(f"{component['name']} has unexpected runtime bytecode")
            transaction_hash, receipt = send_raw(network, deployer, component["deployment_transaction"])
            transactions.append(
                {"component": component["name"], "hash": transaction_hash, "block": receipt["blockNumber"]}
            )
            deployed = rpc(network["rpc_url"], "eth_getCode", [component["expected_contract"], "latest"]).lower()
            if deployed != component["runtime_code"]:
                raise RuntimeError(f"{component['name']} runtime bytecode does not match the bundle")
        return {"network": network["network"], "transactions": transactions}

    def _fund_policy_wallets(self, network: dict, body: dict) -> dict:
        received = body.get("authorizations")
        if not isinstance(received, list):
            raise ValueError("authorizations must be an array")
        by_role = {str(item.get("role")): item for item in received if isinstance(item, dict)}
        if len(by_role) != len(received):
            raise ValueError("authorization roles must be unique")
        now = int(time.time())
        maximum = int(network["pilot"]["funding_authorization"]["max_validity_seconds"])
        transactions = []
        for wallet in network["pilot"]["wallets"]:
            observed = rpc(network["rpc_url"], "eth_getCode", [wallet["expected_contract"], "latest"])
            if observed != "0x":
                continue
            authorization = by_role.get(wallet["role"])
            if not authorization:
                raise ValueError(f"missing {wallet['role']} authorization")
            valid_after = int(authorization.get("valid_after", -1))
            valid_before = int(authorization.get("valid_before", -1))
            if valid_after != 0 or valid_before <= now or valid_before > now + maximum:
                raise ValueError(f"{wallet['role']} authorization validity is outside the bundle cap")
            if str(authorization.get("nonce", "")).lower() != wallet["funding_authorization_nonce"]:
                raise ValueError(f"{wallet['role']} authorization nonce is not bundle-pinned")
            v = int(authorization.get("v", -1))
            r = str(authorization.get("r", ""))
            s = str(authorization.get("s", ""))
            if v not in (27, 28) or not HEX_32.fullmatch(r) or not HEX_32.fullmatch(s):
                raise ValueError(f"{wallet['role']} authorization signature is invalid")
            transaction_hash, receipt = send_call(
                network,
                network["wallet_factory"],
                CREATE_WITH_AUTHORIZATION,
                wallet["owner"],
                policy_tuple(wallet),
                wallet["user_salt"],
                wallet["initial_funding"],
                str(valid_after),
                str(valid_before),
                wallet["funding_authorization_nonce"],
                str(v),
                r,
                s,
            )
            transactions.append(
                {"role": wallet["role"], "wallet": wallet["expected_contract"], "hash": transaction_hash, "block": receipt["blockNumber"]}
            )
        allowance = call_int(
            network,
            network["native_usdc"],
            "allowance(address,address)(uint256)",
            network["pilot"]["owner"],
            network["wallet_factory"],
        )
        revocation = None
        if allowance > 0:
            permit = body.get("allowance_revoke")
            if not isinstance(permit, dict):
                raise ValueError("residual factory allowance requires a zero-value permit")
            deadline = int(permit.get("deadline", -1))
            if deadline <= now or deadline > now + maximum:
                raise ValueError("allowance-revocation deadline is outside the bundle cap")
            nonce = call_int(
                network,
                network["native_usdc"],
                "nonces(address)(uint256)",
                network["pilot"]["owner"],
            )
            if int(permit.get("nonce", -1)) != nonce:
                raise ValueError("allowance-revocation nonce does not match USDC")
            v = int(permit.get("v", -1))
            r = str(permit.get("r", ""))
            s = str(permit.get("s", ""))
            if v not in (27, 28) or not HEX_32.fullmatch(r) or not HEX_32.fullmatch(s):
                raise ValueError("allowance-revocation signature is invalid")
            transaction_hash, receipt = send_call(
                network,
                network["native_usdc"],
                PERMIT,
                network["pilot"]["owner"],
                network["wallet_factory"],
                "0",
                str(deadline),
                str(v),
                r,
                s,
            )
            if wait_call_int(
                network,
                network["native_usdc"],
                "allowance(address,address)(uint256)",
                (network["pilot"]["owner"], network["wallet_factory"]),
                0,
            ) != 0:
                raise RuntimeError("wallet-factory USDC allowance did not reset to zero")
            revocation = {"hash": transaction_hash, "block": receipt["blockNumber"]}
        return {
            "network": network["network"],
            "transactions": transactions,
            "allowance_revocation": revocation,
        }

    def log_message(self, format: str, *args) -> None:
        if self.path.startswith("/activation/"):
            return
        super().log_message(format, *args)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--bundle", type=Path, default=DEFAULT_BUNDLE)
    parser.add_argument("--port", type=int, default=8877)
    parser.add_argument("--enable-mainnet-canary", action="store_true")
    args = parser.parse_args()
    for filename in ("relayer.keystore", "relayer.password"):
        if not (KEY_DIR / filename).exists():
            raise SystemExit(f"missing encrypted local signer: {filename}")
    server = ActivationServer(("127.0.0.1", args.port), ActivationHandler)
    server.bundle = json.loads(args.bundle.read_text(encoding="utf-8"))
    server.enable_mainnet = args.enable_mainnet_canary
    print(f"http://127.0.0.1:{args.port}/tools/bounded-wallet-activation.html")
    server.serve_forever()


if __name__ == "__main__":
    main()
