import time
from pathlib import Path
import tempfile
import unittest

import open_competition_v2_prover_service as service


def request() -> dict:
    return {
        "schema_version": service.REQUEST_SCHEMA,
        "idempotency_key": "job:one",
        "proof_job_id": "11111111-1111-1111-1111-111111111111",
        "proof_system": "groth16",
        "program_input": {"cases": []},
        "expected_public_values": "0x" + "11" * 640,
        "proof_sla_deadline": int(time.time()) + 300,
    }


class ProverServiceTests(unittest.TestCase):
    def test_request_binds_header_system_and_exact_journal(self) -> None:
        value = request()
        self.assertEqual(service.validate_request(value, "job:one", int(time.time())), value)
        with self.assertRaisesRegex(ValueError, "idempotency"):
            service.validate_request(value, "job:two", int(time.time()))
        value["proof_system"] = "network"
        with self.assertRaisesRegex(ValueError, "proof_system"):
            service.validate_request(value, "job:one", int(time.time()))

    def test_response_has_only_broker_contract_fields(self) -> None:
        value = {
            "status": "pending",
            "provider_job_id": "beta2-1",
            "proof": None,
            "public_values": None,
            "failure_code": None,
            "failure_message": None,
            "request": request(),
        }
        self.assertEqual(
            set(service.response_for(value)),
            {
                "status",
                "provider_job_id",
                "proof",
                "public_values",
                "failure_code",
                "failure_message",
            },
        )

    def test_queue_capacity_rejects_before_persisting_a_job(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            jobs = service.ProverJobs(
                Path(directory),
                {"public-vector-metric-v1": Path("missing")},
                60,
                1,
            )
            jobs.queued.add("already-running")
            with self.assertRaisesRegex(service.QueueFullError, "bounded capacity"):
                jobs.submit(request())
            self.assertIsNone(jobs.read("job:one"))
            jobs.executor.shutdown(wait=False, cancel_futures=True)


if __name__ == "__main__":
    unittest.main()
