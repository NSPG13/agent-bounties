#!/usr/bin/env python3
"""One-time GitHub-internal key copy. Never emit or persist plaintext secrets.

The runner has no secret-write token. It can only seal for the reviewed GitHub
environment public keys, or verify that an imported value matches its source.
The administrator relays sealed ciphertext with a separate local API client.
"""
import argparse
import base64
import ctypes
import ctypes.util
import datetime as dt
import hashlib
import hmac
import json
import os
from pathlib import Path
import re
import resource
import sys

REPOSITORY = "NSPG13/agent-bounties"
WORKFLOW = ".github/workflows/github-key-custody.yml"
SLOTS = {
    "keeper": ("legacy-keeper", ("BASE_KEEPER_PRIVATE_KEY",)),
    "attester": ("legacy-keeper", ("PARTICIPANT_ATTESTER_PRIVATE_KEY",)),
    "verifier-one": ("legacy-verifier-one", (
        "REGRESSION_VERIFIER_ONE_PRIVATE_KEY", "VERIFIER_PRIVATE_KEY", "SIGNER_PRIVATE_KEY")),
    "verifier-two": ("legacy-verifier-two", (
        "REGRESSION_VERIFIER_TWO_PRIVATE_KEY", "VERIFIER_PRIVATE_KEY", "SIGNER_PRIVATE_KEY")),
}
VALUE_ENVS = ("CUSTODY_VALUE", "CUSTODY_VERIFIER_ALIAS", "CUSTODY_SIGNER_ALIAS")
ORDER = 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141


class Refused(Exception):
    """Deliberately contains no input data, including malformed secret values."""


def require(condition):
    if not condition:
        raise Refused()


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode("ascii")


def read_json(path):
    require(path.is_file() and not path.is_symlink() and path.stat().st_size <= 16384)
    return json.loads(path.read_text(encoding="ascii"))


def validate_plan(plan, now=None):
    require(set(plan) == {"schema", "repository", "nonce", "expires_at", "environments"})
    require(plan["schema"] == 1 and plan["repository"] == REPOSITORY)
    require(re.fullmatch(r"[0-9a-f]{64}", plan["nonce"]) is not None)
    expires = dt.datetime.fromisoformat(plan["expires_at"])
    require(expires.utcoffset() == dt.timedelta(0))
    require((now or dt.datetime.now(dt.timezone.utc)) < expires)
    require(set(plan["environments"]) == {v[0] for v in SLOTS.values()})
    for environment in plan["environments"].values():
        require(set(environment) == {"id", "key_id", "key"})
        require(type(environment["id"]) is int and environment["id"] > 0)
        require(re.fullmatch(r"[0-9]+", environment["key_id"]) is not None)
        require(len(base64.b64decode(environment["key"], validate=True)) == 32)
    return hashlib.sha256(canonical(plan)).hexdigest()


def runner_boundary(phase, environ):
    require(environ.get("GITHUB_REPOSITORY") == REPOSITORY)
    require(environ.get("GITHUB_REF") == "refs/heads/main")
    require(environ.get("GITHUB_EVENT_NAME") == "workflow_dispatch")
    require(environ.get("GITHUB_WORKFLOW_REF") == f"{REPOSITORY}/{WORKFLOW}@refs/heads/main")
    require(environ.get("GITHUB_ACTOR") == "NSPG13")
    require(environ.get("GITHUB_TRIGGERING_ACTOR") == "NSPG13")
    require(environ.get("GITHUB_RUN_ATTEMPT") == "1")
    require(environ.get("GITHUB_ACTIONS") == "true")
    require(environ.get("RUNNER_ENVIRONMENT") == "github-hosted")
    require(environ.get("RUNNER_OS") == "Linux")
    for name in ("ACTIONS_STEP_DEBUG", "ACTIONS_RUNNER_DEBUG", "RUNNER_DEBUG"):
        require(environ.get(name, "").lower() not in {"true", "1"})
    revision = environ.get("GITHUB_WORKFLOW_SHA", "")
    require(re.fullmatch(r"[0-9a-f]{40}", revision) is not None)
    require(revision == environ.get("GITHUB_SHA") == environ.get("CUSTODY_AUTHORIZED_REVISION"))
    require(phase == environ.get("CUSTODY_AUTHORIZED_PHASE"))
    run_id = environ.get("GITHUB_RUN_ID", "")
    require(re.fullmatch(r"[1-9][0-9]*", run_id) is not None)
    source_run = run_id if phase == "seal" else environ.get("CUSTODY_SOURCE_RUN_ID", "")
    require(re.fullmatch(r"[1-9][0-9]*", source_run) is not None)
    require(phase == "seal" or source_run != run_id)
    return revision, source_run


def scalar(value):
    # Validate the existing encoding; encrypt the ORIGINAL bytes without normalizing.
    require(isinstance(value, str) and len(value) <= 256)
    text = value.strip()
    require(re.fullmatch(r"(?:0x)?[0-9a-fA-F]{64}", text) is not None)
    raw = bytes.fromhex(text.removeprefix("0x"))
    require(0 < int.from_bytes(raw, "big") < ORDER)
    return raw


def commitment(value, nonce, identity=False):
    key = scalar(value) if identity else value.encode("utf-8")
    domain = b"key-custody-scalar-v1:" if identity else b"key-custody-bytes-v1:"
    return hmac.new(key, domain + bytes.fromhex(nonce), hashlib.sha256).hexdigest()


def sodium():
    name = ctypes.util.find_library("sodium")
    require(name is not None)
    lib = ctypes.CDLL(name)
    lib.sodium_init.argtypes = []
    lib.sodium_init.restype = ctypes.c_int
    require(lib.sodium_init() >= 0)
    lib.crypto_box_seal.argtypes = [ctypes.c_void_p, ctypes.c_void_p,
                                    ctypes.c_ulonglong, ctypes.c_void_p]
    lib.crypto_box_seal.restype = ctypes.c_int
    return lib


def seal_bytes(raw, public_key):
    require(len(public_key) == 32)
    output = ctypes.create_string_buffer(len(raw) + 48)
    require(sodium().crypto_box_seal(output, raw, len(raw), public_key) == 0)
    return base64.b64encode(output.raw).decode("ascii")


def binding(plan, slot, revision, source_run):
    environment, aliases = SLOTS[slot]
    return {"schema": 1, "repository": REPOSITORY, "slot": slot,
            "environment": environment, "aliases": list(aliases),
            "environment_id": plan["environments"][environment]["id"],
            "key_id": plan["environments"][environment]["key_id"],
            "plan_sha256": hashlib.sha256(canonical(plan)).hexdigest(),
            "revision": revision, "source_run_id": source_run}


def seal(plan, slot, revision, source_run, value):
    scalar(value)
    result = binding(plan, slot, revision, source_run)
    public_key = base64.b64decode(plan["environments"][result["environment"]]["key"], validate=True)
    result.update(encrypted_value=seal_bytes(value.encode("utf-8"), public_key),
                  bytes_commitment=commitment(value, plan["nonce"]),
                  scalar_commitment=commitment(value, plan["nonce"], identity=True))
    return result


def validate_envelope(plan, slot, revision, source_run, envelope):
    expected = binding(plan, slot, revision, source_run)
    require(set(envelope) == set(expected) | {
        "encrypted_value", "bytes_commitment", "scalar_commitment"})
    require(all(envelope[k] == v for k, v in expected.items()))
    for name in ("bytes_commitment", "scalar_commitment"):
        require(re.fullmatch(r"[0-9a-f]{64}", envelope[name]) is not None)
    cipher = base64.b64decode(envelope["encrypted_value"], validate=True)
    require(112 <= len(cipher) <= 304)


def verify(plan, slot, revision, source_run, envelope, values):
    validate_envelope(plan, slot, revision, source_run, envelope)
    require(len(values) == len(SLOTS[slot][1]))
    for value in values:
        scalar(value)
        require(hmac.compare_digest(commitment(value, plan["nonce"]), envelope["bytes_commitment"]))
        require(hmac.compare_digest(commitment(value, plan["nonce"], identity=True), envelope["scalar_commitment"]))
    return {**binding(plan, slot, revision, source_run), "verified_aliases": list(SLOTS[slot][1]),
            "verified": True}


def write_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    # Fresh file only; a partial/replayed invocation must not overwrite evidence.
    with path.open("x", encoding="ascii") as handle:
        handle.write(canonical(value).decode("ascii") + "\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("phase", choices=("check", "seal", "verify"))
    parser.add_argument("--slot", choices=tuple(SLOTS), required=True)
    parser.add_argument("--plan", type=Path, required=True)
    parser.add_argument("--directory", type=Path, required=True)
    args = parser.parse_args()
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
    os.umask(0o077)
    try:
        plan = read_json(args.plan)
        validate_plan(plan)
        phase = os.environ.get("CUSTODY_AUTHORIZED_PHASE", "") if args.phase == "check" else args.phase
        require(phase in {"seal", "verify"})
        revision, source_run = runner_boundary(phase, os.environ)
        if args.phase == "check":
            require(not any(os.environ.get(name) for name in VALUE_ENVS))
            print("Authorized GitHub runner and unexpired migration plan verified.")
            return 0
        # Remove secret environment entries before any native library lookup.
        values = [os.environ.pop(name, "") for name in VALUE_ENVS]
        path = args.directory / (args.slot + ".json")
        if args.phase == "seal":
            require(not any(values[1:]))
            write_json(path, seal(plan, args.slot, revision, source_run, values[0]))
        else:
            count = len(SLOTS[args.slot][1])
            require(not any(values[count:]))
            result = verify(plan, args.slot, revision, source_run, read_json(path), values[:count])
            result["verification_run_id"] = os.environ["GITHUB_RUN_ID"]
            write_json(args.directory / (args.slot + "-verified.json"), result)
    except Exception:
        # Never interpolate exception details, environment, args, or key values.
        print("Key custody operation refused; no plaintext was written.", file=sys.stderr)
        return 1
    print("Key custody operation completed; signing remains disabled.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
