"""Build failure-case mock fixtures from success.json."""

from __future__ import annotations

import copy
import json
from pathlib import Path

from Crypto.Hash import keccak

ROOT = Path(__file__).resolve().parent / "fixtures" / "base_attest"
SUCCESS = ROOT / "success.json"


def selector(signature: str) -> str:
    return keccak.new(digest_bits=256, data=signature.encode()).hexdigest()[:8]


def find_key(responses: dict[str, object], selector_name: str) -> str:
    sel = selector(selector_name)
    for key in responses:
        if sel in key:
            return key
    raise KeyError(selector_name)


def write(name: str, mutate) -> None:
    data = copy.deepcopy(json.loads(SUCCESS.read_text(encoding="utf-8")))
    mutate(data)
    (ROOT / name).write_text(json.dumps(data, indent=2), encoding="utf-8")


def main() -> None:
    base = json.loads(SUCCESS.read_text(encoding="utf-8"))
    owner_key = find_key(base["responses"], "owner()")
    signer_key = find_key(base["responses"], "settlementSigner()")
    paused_key = find_key(base["responses"], "paused()")
    code_key = next(key for key in base["responses"] if key.startswith("eth_getCode:"))
    receipt_key = next(key for key in base["responses"] if key.startswith("eth_getTransactionReceipt:"))

    write("missing_code.json", lambda d: d["responses"].update({code_key: {"jsonrpc": "2.0", "id": 1, "result": "0x"}}))
    write(
        "failed_receipt.json",
        lambda d: d["responses"].update(
            {
                receipt_key: {
                    **d["responses"][receipt_key],
                    "result": {**d["responses"][receipt_key]["result"], "status": "0x0"},
                }
            }
        ),
    )
    write(
        "wrong_owner.json",
        lambda d: d["responses"].update(
            {
                owner_key: {
                    "jsonrpc": "2.0",
                    "id": 1,
                    "result": "0x0000000000000000000000001111111111111111111111111111111111111111",
                }
            }
        ),
    )
    write(
        "wrong_signer.json",
        lambda d: d["responses"].update(
            {
                signer_key: {
                    "jsonrpc": "2.0",
                    "id": 1,
                    "result": "0x0000000000000000000000002222222222222222222222222222222222222222",
                }
            }
        ),
    )
    write(
        "paused_contract.json",
        lambda d: d["responses"].update(
            {
                paused_key: {
                    "jsonrpc": "2.0",
                    "id": 1,
                    "result": "0x0000000000000000000000000000000000000000000000000000000000000001",
                }
            }
        ),
    )
    write(
        "code_hash_mismatch.json",
        lambda d: d["responses"].update({code_key: {"jsonrpc": "2.0", "id": 1, "result": "0x60006000"}}),
    )
    write(
        "invalid_hex.json",
        lambda d: d["responses"].update({code_key: {"jsonrpc": "2.0", "id": 1, "result": "0xZZ"}}),
    )
    write(
        "malformed_response.json",
        lambda d: d.setdefault("raw_bodies", {}).update({"eth_chainId:[]": "{not-json"}),
    )
    write(
        "rpc_provider_error.json",
        lambda d: d["responses"].update(
            {
                "eth_chainId:[]": {
                    "jsonrpc": "2.0",
                    "id": 1,
                    "error": {"code": -32603, "message": "provider failure at https://rpc.example/?apikey=SECRET"},
                }
            }
        ),
    )
    print(f"wrote failure fixtures to {ROOT}")


if __name__ == "__main__":
    main()
