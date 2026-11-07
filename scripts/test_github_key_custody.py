#!/usr/bin/env python3
"""Synthetic custody rehearsal; never reads GitHub or real signing keys."""
import base64
import copy
import ctypes
import datetime as dt
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

PATH = Path(__file__).with_name("github_key_custody.py")
SPEC = importlib.util.spec_from_file_location("custody", PATH)
c = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(c)
REVISION = "a" * 40
VALUE = "0x" + "ab" * 32  # Synthetic, unfunded fixture, never a production key.


def keypair():
    lib = c.sodium()
    lib.crypto_box_keypair.argtypes = [ctypes.c_void_p, ctypes.c_void_p]
    lib.crypto_box_keypair.restype = ctypes.c_int
    public, private = ctypes.create_string_buffer(32), ctypes.create_string_buffer(32)
    if lib.crypto_box_keypair(public, private) != 0:
        raise AssertionError("synthetic key generation failed")
    return public.raw, private.raw


def decrypt(ciphertext, public, private):
    lib = c.sodium()
    lib.crypto_box_seal_open.argtypes = [ctypes.c_void_p, ctypes.c_void_p,
        ctypes.c_ulonglong, ctypes.c_void_p, ctypes.c_void_p]
    lib.crypto_box_seal_open.restype = ctypes.c_int
    cipher = base64.b64decode(ciphertext)
    output = ctypes.create_string_buffer(len(cipher) - 48)
    if lib.crypto_box_seal_open(output, cipher, len(cipher), public, private) != 0:
        raise ValueError("synthetic recipient rejected ciphertext")
    return output.raw


class CustodyTest(unittest.TestCase):
    def setUp(self):
        self.keys = {name: keypair() for name in ("legacy-keeper", "legacy-verifier-one", "legacy-verifier-two")}
        self.plan = {"schema": 1, "repository": c.REPOSITORY, "nonce": "b" * 64,
            "expires_at": "2099-01-01T00:00:00+00:00", "environments": {
                name: {"id": index + 1, "key_id": "123", "key": base64.b64encode(pair[0]).decode()}
                for index, (name, pair) in enumerate(self.keys.items())}}
        self.env = {"GITHUB_REPOSITORY": c.REPOSITORY, "GITHUB_REF": "refs/heads/main",
            "GITHUB_EVENT_NAME": "workflow_dispatch", "GITHUB_ACTOR": "NSPG13",
            "GITHUB_TRIGGERING_ACTOR": "NSPG13", "GITHUB_RUN_ATTEMPT": "1",
            "GITHUB_WORKFLOW_REF": f"{c.REPOSITORY}/{c.WORKFLOW}@refs/heads/main",
            "GITHUB_ACTIONS": "true", "RUNNER_ENVIRONMENT": "github-hosted", "RUNNER_OS": "Linux",
            "GITHUB_WORKFLOW_SHA": REVISION, "GITHUB_SHA": REVISION, "GITHUB_RUN_ID": "1234",
            "CUSTODY_AUTHORIZED_REVISION": REVISION, "CUSTODY_AUTHORIZED_PHASE": "seal"}

    def test_real_sealed_box_round_trip_and_every_alias(self):
        for slot, (environment, aliases) in c.SLOTS.items():
            with self.subTest(slot=slot):
                envelope = c.seal(self.plan, slot, REVISION, "1234", VALUE)
                restored = decrypt(envelope["encrypted_value"], *self.keys[environment]).decode()
                self.assertEqual(restored, VALUE)
                result = c.verify(self.plan, slot, REVISION, "1234", envelope, [restored] * len(aliases))
                self.assertEqual(result["verified_aliases"], list(aliases))
                self.assertTrue(result["verified"])
                self.assertNotIn(VALUE, json.dumps(envelope))
                self.assertNotIn(VALUE, json.dumps(result))

    def test_wrong_recipient_and_ciphertext_corruption(self):
        envelope = c.seal(self.plan, "keeper", REVISION, "1234", VALUE)
        with self.assertRaises(ValueError):
            decrypt(envelope["encrypted_value"], *self.keys["legacy-verifier-one"])
        raw = bytearray(base64.b64decode(envelope["encrypted_value"]))
        raw[-1] ^= 1
        with self.assertRaises(ValueError):
            decrypt(base64.b64encode(raw), *self.keys["legacy-keeper"])

    def test_byte_identity_preserves_whitespace_and_case(self):
        value = "  " + VALUE.upper().replace("0X", "0x") + "\n"
        envelope = c.seal(self.plan, "keeper", REVISION, "1234", value)
        self.assertEqual(decrypt(envelope["encrypted_value"], *self.keys["legacy-keeper"]), value.encode())
        with self.assertRaises(c.Refused):
            c.verify(self.plan, "keeper", REVISION, "1234", envelope, [VALUE])
        self.assertEqual(c.commitment(value, self.plan["nonce"], True), c.commitment(VALUE, self.plan["nonce"], True))

    def test_every_alias_is_checked_and_empty_fallback_fails(self):
        envelope = c.seal(self.plan, "verifier-one", REVISION, "1234", VALUE)
        for index in range(3):
            for bad in ("", "0x" + "11" * 32):
                values = [VALUE] * 3
                values[index] = bad
                with self.subTest(index=index, empty=not bad), self.assertRaises(c.Refused):
                    c.verify(self.plan, "verifier-one", REVISION, "1234", envelope, values)
        with self.assertRaises(c.Refused):
            c.verify(self.plan, "verifier-one", REVISION, "1234", envelope, [VALUE])

    def test_rejects_malformed_or_invalid_scalars(self):
        for value in ("", "junk", "0" * 64, f"{c.ORDER:064x}", "f" * 64, "ab" * 31, VALUE + " suffix"):
            with self.subTest(length=len(value)), self.assertRaises(c.Refused):
                c.seal(self.plan, "keeper", REVISION, "1234", value)

    def test_commitment_is_challenge_bound_and_distinguishes_verifiers(self):
        first = c.commitment(VALUE, "a" * 64, True)
        self.assertNotEqual(first, c.commitment(VALUE, "b" * 64, True))
        self.assertNotEqual(first, c.commitment("11" * 32, "a" * 64, True))

    def test_envelope_replay_and_destination_substitution_fail(self):
        envelope = c.seal(self.plan, "keeper", REVISION, "1234", VALUE)
        for field, bad in {"environment": "legacy-verifier-one", "aliases": ["UNRELATED_KEY"],
                "environment_id": 42, "key_id": "7", "plan_sha256": "c" * 64,
                "source_run_id": "9", "revision": "b" * 40, "slot": "attester",
                "repository": "attacker/repository", "schema": 2,
                "encrypted_value": "bad", "bytes_commitment": "bad"}.items():
            changed = {**envelope, field: bad}
            with self.subTest(field=field), self.assertRaises(Exception):
                c.validate_envelope(self.plan, "keeper", REVISION, "1234", changed)
        with self.assertRaises(c.Refused):
            c.validate_envelope(self.plan, "keeper", REVISION, "1234", {**envelope, "extra": True})

    def test_plan_refuses_wrong_scope_expiry_and_recipient(self):
        c.validate_plan(self.plan)
        for field, bad in {"repository": "elsewhere/repo", "schema": 2, "nonce": "tiny",
                "expires_at": "2000-01-01T00:00:00+00:00", "environments": {}}.items():
            with self.subTest(field=field), self.assertRaises(Exception):
                c.validate_plan({**self.plan, field: bad})
        changed = copy.deepcopy(self.plan)
        changed["environments"]["legacy-keeper"]["key"] = "bad"
        with self.assertRaises(Exception):
            c.validate_plan(changed)

    def test_runner_rejects_untrusted_context_and_debug(self):
        self.assertEqual(c.runner_boundary("seal", self.env), (REVISION, "1234"))
        for field, bad in {"GITHUB_REPOSITORY": "elsewhere/repo", "GITHUB_REF": "refs/tags/main",
                "GITHUB_EVENT_NAME": "pull_request_target", "GITHUB_ACTOR": "contributor",
                "GITHUB_TRIGGERING_ACTOR": "contributor", "GITHUB_RUN_ATTEMPT": "2",
                "GITHUB_WORKFLOW_REF": "other", "GITHUB_ACTIONS": "false", "RUNNER_OS": "Windows",
                "RUNNER_ENVIRONMENT": "self-hosted", "GITHUB_SHA": "b" * 40,
                "GITHUB_WORKFLOW_SHA": "b" * 40, "GITHUB_RUN_ID": "0",
                "CUSTODY_AUTHORIZED_REVISION": "", "CUSTODY_AUTHORIZED_PHASE": "verify",
                "ACTIONS_STEP_DEBUG": "true", "ACTIONS_RUNNER_DEBUG": "true", "RUNNER_DEBUG": "1"}.items():
            with self.subTest(field=field), self.assertRaises(c.Refused):
                c.runner_boundary("seal", {**self.env, field: bad})

    def test_verification_requires_different_pinned_source_run(self):
        env = {**self.env, "CUSTODY_AUTHORIZED_PHASE": "verify"}
        for value in ("", "0", "1234"):
            with self.assertRaises(c.Refused):
                c.runner_boundary("verify", {**env, "CUSTODY_SOURCE_RUN_ID": value})
        self.assertEqual(c.runner_boundary("verify", {**env, "CUSTODY_SOURCE_RUN_ID": "12"}), (REVISION, "12"))

    def test_cli_round_trip_leaks_no_plaintext_and_fails_closed(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            plan = root / "plan.json"
            plan.write_text(json.dumps(self.plan))
            env = {"PATH": os.environ["PATH"], **self.env, "CUSTODY_VALUE": VALUE}
            command = [sys.executable, "-I", str(PATH), "seal", "--slot", "keeper",
                       "--plan", str(plan), "--directory", str(root / "out")]
            first = subprocess.run(command, env=env, capture_output=True, text=True)
            self.assertEqual(first.returncode, 0, first.stderr)
            command[3] = "verify"
            env.update(CUSTODY_AUTHORIZED_PHASE="verify", CUSTODY_SOURCE_RUN_ID="1234", GITHUB_RUN_ID="1235")
            second = subprocess.run(command, env=env, capture_output=True, text=True)
            self.assertEqual(second.returncode, 0, second.stderr)
            for result in (first, second):
                self.assertNotIn(VALUE, result.stdout + result.stderr)
            for path in root.rglob("*"):
                if path.is_file():
                    self.assertNotIn(VALUE, path.read_text())
            # Missing alias, denied context, invalid key and output collision are all opaque.
            for changes in ({"CUSTODY_VALUE": ""}, {"GITHUB_REF": "refs/heads/attacker"},
                            {"CUSTODY_VALUE": VALUE + " secret malformed"}, {}):
                failed = subprocess.run(command, env={**env, **changes}, capture_output=True, text=True)
                self.assertEqual(failed.returncode, 1)
                self.assertEqual(failed.stderr, "Key custody operation refused; no plaintext was written.\n")
                self.assertNotIn(VALUE, failed.stdout + failed.stderr)

    def test_refuse_symlink_or_large_envelope_and_existing_output(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            output = root / "receipt.json"
            c.write_json(output, {"verified": True})
            with self.assertRaises(FileExistsError):
                c.write_json(output, {"verified": False})
            link = root / "link.json"
            link.symlink_to(output)
            with self.assertRaises(c.Refused):
                c.read_json(link)
            output.write_text(" " * 16385)
            with self.assertRaises(c.Refused):
                c.read_json(output)

    def test_migration_obeys_the_existing_shared_keeper_lock_guard(self):
        spec = importlib.util.spec_from_file_location("keeper_guard", PATH.with_name("regression_verifier_source_guard.py"))
        guard = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(guard)
        root = PATH.resolve().parents[1]
        self.assertIn("github-key-custody.yml", guard.validate_keeper_workflow_locks(root))
        workflow = (root / c.WORKFLOW).read_text()
        lock = "    concurrency:\n      group: agent-bounties-shared-base-keeper\n      cancel-in-progress: false\n"
        self.assertIn(lock, workflow)
        with self.assertRaises(guard.GuardError):
            guard._validate_yaml_keeper_locks(workflow.replace(lock, "", 1), "github-key-custody.yml")


if __name__ == "__main__":
    unittest.main()
