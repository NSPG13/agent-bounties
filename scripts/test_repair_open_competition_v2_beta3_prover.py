from pathlib import Path
import unittest

import yaml


ROOT = Path(__file__).resolve().parents[1]


class ProductionProverRepairWorkflowTests(unittest.TestCase):
    def test_repair_is_protected_local_and_rehearses_without_money(self) -> None:
        path = ROOT / ".github/workflows/repair-open-competition-v2-beta3-prover.yml"
        text = path.read_text(encoding="utf-8")
        workflow = yaml.safe_load(text)
        job = workflow["jobs"]["repair-and-rehearse"]
        self.assertEqual(job["environment"], "v2-beta2-mainnet")
        self.assertIn("self-hosted", job["runs-on"])
        self.assertIn("ram-256gb", job["runs-on"])
        self.assertIn("install_open_competition_v2_prover_assets.py", text)
        self.assertIn("rehearse_open_competition_v2_beta3_prover_service.py", text)
        self.assertIn(".money_moved == false", text)

    def test_service_pins_local_image_and_required_group(self) -> None:
        text = (ROOT / "ops/open-competition-v2-prover.service").read_text(encoding="utf-8")
        self.assertIn("SupplementaryGroups=docker", text)
        self.assertIn("Environment=TMPDIR=/var/lib/agent-bounties-prover/tmp", text)
        self.assertIn(
            "ExecStartPre=/usr/bin/test -f /var/lib/agent-bounties-prover/circuits/groth16/agent-bounties-sp1-safe-v5/.complete",
            text,
        )
        self.assertIn(
            "ExecStartPre=/usr/bin/docker image inspect ghcr.io/succinctlabs/sp1-gnark:agent-bounties-sp1-safe-v5",
            text,
        )

    def test_repair_uses_private_writable_verified_circuit_cache(self) -> None:
        text = (
            ROOT / ".github/workflows/repair-open-competition-v2-beta3-prover.yml"
        ).read_text(encoding="utf-8")
        self.assertIn(
            "--install-root /var/lib/agent-bounties-prover/circuits", text
        )
        self.assertIn("sudo chmod -R u=rwX,go=", text)
        self.assertNotIn("--install-root /opt/agent-bounties/circuits", text)


if __name__ == "__main__":
    unittest.main()
