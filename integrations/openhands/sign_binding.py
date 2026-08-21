#!/usr/bin/env python3
"""Operator-side tool: sign an OpenHands session binding with an Ed25519 key.

RUN THIS OUTSIDE THE SANDBOX. The private key must never enter an environment
the agent session can read; that is the entire point of the signed binding. The
session only ever sees the PUBLIC key, pinned through
`AGENT_BOUNTIES_OPERATOR_PUBKEY`.

    # once: create an operator keypair
    python3 -B integrations/openhands/sign_binding.py keygen \\
        --out ~/.agent-bounties/operator.key

    # then: sign a binding document
    python3 -B integrations/openhands/sign_binding.py sign \\
        --key ~/.agent-bounties/operator.key --binding ./binding.json

`sign` rewrites the binding in place with a `signature` envelope covering every
other field. Re-run it after ANY edit: a stale signature is an invalid one, and
the guard fails closed on an invalid signature.

WALLET SAFETY: this key authenticates a claim BINDING. It is not a wallet key,
it authorizes no transfer, and it must not be reused as one.
"""

from __future__ import annotations

import argparse
import base64
import json
import os
import stat
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from ed25519_verify import canonical_payload, verify  # noqa: E402

SIGNATURE_ALG = "ed25519"


def _require_pycryptodome():
    try:
        from Crypto.PublicKey import ECC
        from Crypto.Signature import eddsa
    except ImportError:  # pragma: no cover - operator-side dependency
        raise SystemExit(
            "signing needs PyCryptodome (already pinned by "
            "scripts/requirements-attest.txt): pip install pycryptodome\n"
            "NOTE: only SIGNING needs it. The Stop hook verifies with the "
            "standard library alone, so the sandbox needs no extra packages."
        )
    return ECC, eddsa


def keygen(out: Path) -> int:
    ECC, _ = _require_pycryptodome()
    key = ECC.generate(curve="Ed25519")
    seed = key.seed
    public = key.public_key().export_key(format="raw")
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps({
        "alg": SIGNATURE_ALG,
        "private_seed_b64": base64.b64encode(seed).decode(),
        "public_key_b64": base64.b64encode(public).decode(),
    }, indent=2) + "\n", encoding="utf-8")
    if os.name != "nt":
        out.chmod(stat.S_IRUSR | stat.S_IWUSR)
    print(f"wrote operator key to {out} (mode 0600 on POSIX)")
    print("\nPin this PUBLIC key in the session environment:")
    print(f'  AGENT_BOUNTIES_OPERATOR_PUBKEY={base64.b64encode(public).decode()}')
    print("\nKeep the private seed OUT of the agent sandbox.")
    return 0


def sign(key_path: Path, binding_path: Path) -> int:
    ECC, eddsa = _require_pycryptodome()
    material = json.loads(key_path.read_text(encoding="utf-8"))
    seed = base64.b64decode(material["private_seed_b64"])
    public = base64.b64decode(material["public_key_b64"])
    key = ECC.construct(curve="Ed25519", seed=seed)

    binding = json.loads(binding_path.read_text(encoding="utf-8"))
    if not isinstance(binding, dict):
        raise SystemExit("binding must be a JSON object")
    binding.pop("signature", None)
    payload = canonical_payload(binding)
    signature = eddsa.new(key, "rfc8032").sign(payload)

    if not verify(public, payload, signature):
        raise SystemExit("internal error: produced a signature that does not verify")

    binding["signature"] = {
        "alg": SIGNATURE_ALG,
        "public_key_b64": base64.b64encode(public).decode(),
        "signature_b64": base64.b64encode(signature).decode(),
    }
    binding_path.write_text(json.dumps(binding, indent=2) + "\n", encoding="utf-8")
    print(f"signed {binding_path} with operator key "
          f"{base64.b64encode(public).decode()[:16]}...")
    return 0


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    gen = sub.add_parser("keygen", help="create an operator Ed25519 keypair")
    gen.add_argument("--out", required=True, type=Path)
    sgn = sub.add_parser("sign", help="sign a binding document in place")
    sgn.add_argument("--key", required=True, type=Path)
    sgn.add_argument("--binding", required=True, type=Path)
    args = parser.parse_args(argv)
    if args.command == "keygen":
        return keygen(args.out)
    return sign(args.key, args.binding)


if __name__ == "__main__":
    sys.exit(main())
