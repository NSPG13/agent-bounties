"""Offline, replayable tests for the direct-bounty evidence checklist.

Run:
    python3 test_direct_evidence_checklist.py          # exit 0 on success
    python3 -m pytest test_direct_evidence_checklist.py -q

No network, no clock, no randomness. Same inputs -> same results, every run.
"""

from __future__ import annotations

import copy
import sys

from direct_evidence_checklist import (
    CHECKLIST_VERSION,
    REQUIRED_FIELDS,
    ChecklistError,
    build_checklist,
)

REPO = "https://github.com/NSPG13/agent-bounties"
PR = "https://github.com/NSPG13/agent-bounties/pull/9001"
COMMIT = "94b49dcf1e2ac0f3b7d5a6c8e90f1a2b3c4d5e6f"
DIGEST = "e" * 64
ARTIFACT = "https://example.org/artifacts/bounty-686.tar.gz"
RUNS = [
    "https://github.com/NSPG13/agent-bounties/actions/runs/1001",
    "https://github.com/NSPG13/agent-bounties/actions/runs/1002",
]

# Only a LIVE-CANONICAL receipt upgrades payment. A local `settled: true` does not.
CANONICAL_RECEIPT = {
    "provenance": "canonical_live",
    "tx_hash": "0x" + "a" * 64,
    "log_key": "base:8453:log:17",
    "block_number": 39_221_004,
}


def good_evidence() -> dict:
    return {
        "repository": REPO,
        "source_commit": COMMIT,
        "subdirectory": "crates/sdk-python",
        "pull_request_url": PR,
        "check_run_urls": list(RUNS),
        "artifact_url": ARTIFACT,
        "artifact_sha256": DIGEST,
    }


def test_passing_evidence_is_complete() -> None:
    out = build_checklist(good_evidence())
    assert out["complete"] is True, out["problems"]
    assert out["problems"] == []
    assert out["checklist_version"] == CHECKLIST_VERSION
    assert out["submission"]["source_commit"] == COMMIT
    assert len(out["verification"]["check_run_urls"]) == 2
    assert len(out["binding_digest"]) == 64


def test_missing_field_is_reported() -> None:
    for field in REQUIRED_FIELDS:
        ev = good_evidence()
        del ev[field]
        out = build_checklist(ev)
        assert out["complete"] is False, field
        assert any(field in p for p in out["problems"]), (field, out["problems"])


def test_empty_value_counts_as_missing() -> None:
    for field in ("repository", "source_commit", "artifact_url", "artifact_sha256"):
        ev = good_evidence()
        ev[field] = "   "
        out = build_checklist(ev)
        assert out["complete"] is False, field
        assert any(field in p for p in out["problems"]), (field, out["problems"])


def test_mutable_artifact_is_rejected() -> None:
    for bad in (
        "https://github.com/NSPG13/agent-bounties/blob/main/evidence.json",
        "https://example.org/artifacts/latest.tar.gz",
        "https://example.org/a.tar.gz?ref=main",
    ):
        ev = good_evidence()
        ev["artifact_url"] = bad
        out = build_checklist(ev)
        assert out["complete"] is False, bad
        assert any("mutable" in p for p in out["problems"]), (bad, out["problems"])


def test_non_https_artifact_is_rejected() -> None:
    for bad in (
        "http://example.org/a.tar.gz",
        "file:///tmp/a.tar.gz",
        "git://github.com/o/r",
        "ftp://example.org/a.tar.gz",
    ):
        ev = good_evidence()
        ev["artifact_url"] = bad
        out = build_checklist(ev)
        assert out["complete"] is False, bad
        assert any("https://" in p for p in out["problems"]), (bad, out["problems"])


def test_branch_as_commit_is_rejected() -> None:
    for bad in ("main", "master", "HEAD", "latest"):
        ev = good_evidence()
        ev["source_commit"] = bad
        out = build_checklist(ev)
        assert out["complete"] is False, bad
        assert any("commit" in p for p in out["problems"]), (bad, out["problems"])


def test_short_commit_is_rejected() -> None:
    ev = good_evidence()
    ev["source_commit"] = "94b49dc"
    out = build_checklist(ev)
    assert out["complete"] is False
    assert any("hex" in p for p in out["problems"])


def test_bad_pull_request_url_is_rejected() -> None:
    for bad in (
        "https://github.com/NSPG13/agent-bounties/issues/686",
        "github.com/NSPG13/agent-bounties/pull/9001",
        "https://github.com/NSPG13/agent-bounties/pull/",
    ):
        ev = good_evidence()
        ev["pull_request_url"] = bad
        out = build_checklist(ev)
        assert out["complete"] is False, bad


def test_check_runs_must_be_non_empty_and_https() -> None:
    for bad in ([], (), "https://github.com/o/r/actions/runs/1"):
        ev = good_evidence()
        ev["check_run_urls"] = bad
        out = build_checklist(ev)
        assert out["complete"] is False, bad

    ev = good_evidence()
    ev["check_run_urls"] = ["http://github.com/o/r/actions/runs/1"]
    out = build_checklist(ev)
    assert out["complete"] is False

    ev = good_evidence()
    ev["check_run_urls"] = [RUNS[0], RUNS[0]]
    out = build_checklist(ev)
    assert out["complete"] is False
    assert any("duplicate" in p for p in out["problems"])


def test_subdirectory_traversal_is_rejected() -> None:
    for bad in ("../../etc", "/abs/path", ""):
        ev = good_evidence()
        ev["subdirectory"] = bad
        out = build_checklist(ev)
        assert out["complete"] is False, bad


def test_local_settled_flag_stays_unverified() -> None:
    """The whole point: local input can never assert payment."""
    ev = good_evidence()
    ev["settled"] = True
    out = build_checklist(ev)
    assert out["complete"] is True          # deliverable is still fully bound
    assert out["payment"]["status"] == "unverified"
    assert "canonical settlement receipt" in out["payment"]["reason"]


def test_tx_hash_without_provenance_stays_unverified() -> None:
    ev = good_evidence()
    ev["settlement"] = copy.deepcopy(CANONICAL_RECEIPT)
    ev["settlement"]["provenance"] = "self_reported"
    out = build_checklist(ev)
    assert out["payment"]["status"] == "unverified"
    assert "canonical_live" in out["payment"]["reason"]


def test_malformed_receipt_stays_unverified() -> None:
    variants = [
        {"tx_hash": "0xabc", "log_key": "k", "block_number": 1},
        {"provenance": "canonical_live", "log_key": "k", "block_number": 1},
        {"provenance": "canonical_live", "tx_hash": "0x" + "a" * 64, "block_number": 1},
        {"provenance": "canonical_live", "tx_hash": "0x" + "a" * 64,
         "log_key": "k", "block_number": 0},
        {"provenance": "canonical_live", "tx_hash": "0x" + "a" * 64,
         "log_key": "k", "block_number": "39221004"},
    ]
    for i, settlement in enumerate(variants):
        ev = good_evidence()
        ev["settlement"] = settlement
        out = build_checklist(ev)
        assert out["payment"]["status"] == "unverified", i


def test_canonical_receipt_settles() -> None:
    ev = good_evidence()
    ev["settlement"] = copy.deepcopy(CANONICAL_RECEIPT)
    out = build_checklist(ev)
    assert out["payment"]["status"] == "settled"
    assert out["payment"]["block_number"] == 39_221_004


def test_secrets_and_personal_data_are_scrubbed() -> None:
    ev = good_evidence()
    ev["api_key"] = "sk-should-never-appear"
    ev["private_key"] = "0xnope"
    ev["author_email"] = "someone@example.org"
    ev["wallet_address"] = "0xdeadbeef"
    blob = repr(build_checklist(ev))
    for leak in ("sk-should-never-appear", "0xnope", "someone@example.org", "0xdeadbeef"):
        assert leak not in blob, leak


def test_output_is_deterministic() -> None:
    a = build_checklist(good_evidence())
    b = build_checklist(good_evidence())
    assert a == b
    assert a["binding_digest"] == b["binding_digest"]


def test_binding_digest_covers_all_submission_fields() -> None:
    base = build_checklist(good_evidence())["binding_digest"]
    ev = good_evidence()
    ev["source_commit"] = "f" * 40
    assert build_checklist(ev)["binding_digest"] != base


def test_non_mapping_input_raises() -> None:
    for bad in ("not a dict", 42, None, ["a"]):
        try:
            build_checklist(bad)
        except ChecklistError:
            pass
        else:
            raise AssertionError(f"expected ChecklistError for {bad!r}")


def main() -> int:
    tests = [(n, o) for n, o in sorted(globals().items())
             if n.startswith("test_") and callable(o)]
    failed = []
    for name, fn in tests:
        try:
            fn()
            print(f"  PASS  {name}")
        except AssertionError as exc:
            failed.append(name)
            print(f"  FAIL  {name}: {exc}")
        except Exception as exc:  # noqa: BLE001 - report, do not hide
            failed.append(name)
            print(f"  ERROR {name}: {type(exc).__name__}: {exc}")
    print(f"\n{len(tests) - len(failed)}/{len(tests)} passed")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
