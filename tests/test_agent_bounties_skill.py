import os
import json
import tempfile
import shutil
from pathlib import Path

import pytest

# Import the skill module
from .agents.skills.agent_bounties.skill import stop, run, _determine_next_action

# Helper to create dummy verification files
def create_verification_files(tmp_dir):
    verification_dir = Path(tmp_dir) / ".openhands" / "verification"
    verification_dir.mkdir(parents=True, exist_ok=True)
    (verification_dir / "focused_checks.json").write_text(json.dumps({"checks": []}))
    (verification_dir / "evidence.json").write_text(json.dumps({"evidence": []}))

@pytest.fixture
def tmp_verification_dir(tmp_path):
    create_verification_files(tmp_path)
    return tmp_path

def test_stop_hook(tmp_verification_dir):
    # Temporarily change working directory to the temp path
    cwd = os.getcwd()
    os.chdir(tmp_verification_dir)
    try:
        assert stop() is True
    finally:
        os.chdir(cwd)

def test_stop_hook_missing_files(tmp_path):
    # No verification files present
    cwd = os.getcwd()
    os.chdir(tmp_path)
    try:
        assert stop() is False
    finally:
        os.chdir(cwd)

def test_determine_next_action():
    # Unfunded state
    state = {"is_funded": False, "is_verifier_ready": False, "is_claimable": False, "is_submitted": False, "is_paid": False}
    assert _determine_next_action(state) == "wait for funding"

    # Funded but verifier not ready
    state = {"is_funded": True, "is_verifier_ready": False, "is_claimable": False, "is_submitted": False, "is_paid": False}
    assert _determine_next_action(state) == "wait for verifier readiness"

    # Claimable
    state = {"is_funded": True, "is_verifier_ready": True, "is_claimable": True, "is_submitted": False, "is_paid": False}
    assert _determine_next_action(state) == "claim"

    # Submitted but not paid
    state = {"is_funded": True, "is_verifier_ready": True, "is_claimable": False, "is_submitted": True, "is_paid": False}
    assert _determine_next_action(state) == "wait for payment"

    # Paid
    state = {"is_funded": True, "is_verifier_ready": True, "is_claimable": False, "is_submitted": True, "is_paid": True}
    assert _determine_next_action(state) == "done"
