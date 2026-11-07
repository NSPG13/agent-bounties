#!/usr/bin/env python3
"""Local administrator relay: only GitHub-encrypted ciphertext crosses this host.

Does not fetch tokens or plaintext keys, dispatch workflows, enable signing,
modify environment protection, or change a production deployment.
"""
import argparse
import datetime as dt
import importlib.util
import io
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import zipfile

SPEC = importlib.util.spec_from_file_location("custody", Path(__file__).with_name("github_key_custody.py"))
c = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(c)
ROOT = Path(__file__).resolve().parents[1]
PREFIX = f"repos/{c.REPOSITORY}"
ORIGINALS = {aliases[0] for _, aliases in c.SLOTS.values()}
EXPECTED = {name: {alias for environment, aliases in c.SLOTS.values() if environment == name for alias in aliases}
            for name in {environment for environment, _ in c.SLOTS.values()}}


class GitHub:
    def __init__(self, executable):
        self.executable = executable

    def api(self, path, method="GET", data=None, binary=False):
        command = [self.executable, "api", "--method", method, path]
        if data is not None:
            command += ["--input", "-"]
        result = subprocess.run(command, input=None if data is None else c.canonical(data),
                                capture_output=True, timeout=90)
        c.require(result.returncode == 0)
        if binary:
            return result.stdout
        return json.loads(result.stdout) if result.stdout.strip() else None

    def pages(self, path, field):
        result = []
        separator = "&" if "?" in path else "?"
        for page in range(1, 101):
            rows = self.api(f"{path}{separator}per_page=100&page={page}")[field]
            result.extend(rows)
            if len(rows) < 100:
                return result
        raise c.Refused()


def secret_metadata(rows):
    return {row["name"]: {key: row[key] for key in ("created_at", "updated_at")} for row in rows}


def environment_controls(api, plan, filled):
    result = {}
    for name, expected in plan["environments"].items():
        path = f"{PREFIX}/environments/{name}"
        metadata = api.api(path)
        c.require(metadata["id"] == expected["id"] and metadata["name"] == name)
        c.require(metadata["deployment_branch_policy"] == {
            "protected_branches": False, "custom_branch_policies": True})
        branches = api.pages(path + "/deployment-branch-policies", "branch_policies")
        c.require(len(branches) == 1 and branches[0]["name"] == "main" and branches[0]["type"] == "branch")
        public = api.api(path + "/secrets/public-key")
        c.require(public == {key: expected[key] for key in ("key", "key_id")})
        # Environment variables must not override repository authorization.
        variables = api.pages(path + "/variables", "variables")
        c.require(not any(row["name"].startswith("KEY_CUSTODY_") for row in variables))
        secrets = secret_metadata(api.pages(path + "/secrets", "secrets"))
        c.require(set(secrets) == EXPECTED[name] if filled else set(secrets) <= EXPECTED[name])
        result[name] = {"metadata": metadata, "branches": branches, "public_key": public, "secrets": secrets}
    return result


def held_consumers(api, revision):
    c.require(api.api(f"{PREFIX}/git/ref/heads/main")["object"]["sha"] == revision)
    # Read the exact reviewed checkout, never a candidate artifact or fork.
    head = subprocess.run(["git", "rev-parse", "HEAD"], cwd=ROOT, capture_output=True, text=True, check=True).stdout.strip()
    c.require(head == revision)
    paths = set()
    for path in (ROOT / ".github/workflows").iterdir():
        if path.suffix not in {".yml", ".yaml"} or path.name == "github-key-custody.yml":
            continue
        text = path.read_text().lower()
        if any(key.lower() in text for key in ORIGINALS | {"secrets.verifier_private_key", "secrets.signer_private_key"}):
            paths.add(path.relative_to(ROOT).as_posix())
    c.require(len(paths) >= 22)
    workflows = {row["path"]: row for row in api.pages(f"{PREFIX}/actions/workflows", "workflows")}
    c.require(paths <= set(workflows))
    consumers = [workflows[path] for path in sorted(paths)]
    c.require(all(row["state"] == "disabled_manually" for row in consumers))
    ids = {row["id"] for row in consumers}
    for status in ("queued", "in_progress", "waiting", "pending", "requested"):
        runs = api.pages(f"{PREFIX}/actions/runs?status={status}", "workflow_runs")
        c.require(not any(row["workflow_id"] in ids for row in runs))
    return [{key: row[key] for key in ("id", "path", "state")} for row in consumers]


def read_run(api, run_id, revision):
    c.require(re.fullmatch(r"[1-9][0-9]*", run_id) is not None)
    run = api.api(f"{PREFIX}/actions/runs/{run_id}")
    c.require(str(run["id"]) == run_id and run["head_sha"] == revision and run["head_branch"] == "main")
    c.require(run["event"] == "workflow_dispatch" and run["run_attempt"] == 1)
    c.require(run["status"] == "completed" and run["conclusion"] == "success")
    c.require(run["path"] == c.WORKFLOW and run["repository"]["full_name"] == c.REPOSITORY)
    c.require(run["actor"]["login"] == "NSPG13" and run["triggering_actor"]["login"] == "NSPG13")
    jobs = api.pages(f"{PREFIX}/actions/runs/{run_id}/jobs", "jobs")
    c.require(len(jobs) == 4 and {job["name"] for job in jobs} == set(c.SLOTS))
    c.require(all(job["conclusion"] == "success" for job in jobs))
    return run


def receipts(api, run_id, verified=False):
    artifacts = api.pages(f"{PREFIX}/actions/runs/{run_id}/artifacts", "artifacts")
    expected = {f"key-custody-{slot}-{run_id}": slot for slot in c.SLOTS}
    c.require(len(artifacts) == 4 and {row["name"] for row in artifacts} == set(expected))
    result = {}
    for row in artifacts:
        c.require(not row["expired"] and row["size_in_bytes"] <= 32768)
        slot = expected[row["name"]]
        filename = slot + ("-verified" if verified else "") + ".json"
        raw = api.api(f"{PREFIX}/actions/artifacts/{row['id']}/zip", binary=True)
        c.require(len(raw) <= 32768)
        with zipfile.ZipFile(io.BytesIO(raw)) as archive:
            c.require(archive.namelist() == [filename])
            c.require(archive.getinfo(filename).file_size <= 16384)
            result[slot] = json.loads(archive.read(filename).decode("ascii"))
    return result, artifacts


def source_envelopes(api, plan, revision, source_run):
    run = read_run(api, source_run, revision)
    envelopes, artifacts = receipts(api, source_run)
    for slot, envelope in envelopes.items():
        c.validate_envelope(plan, slot, revision, source_run, envelope)
    c.require(envelopes["verifier-one"]["scalar_commitment"] != envelopes["verifier-two"]["scalar_commitment"])
    return envelopes, artifacts, run


def verify_receipts(api, plan, revision, source_run, verify_run):
    c.require(verify_run != source_run)
    run = read_run(api, verify_run, revision)
    rows, artifacts = receipts(api, verify_run, verified=True)
    for slot, row in rows.items():
        expected = {**c.binding(plan, slot, revision, source_run), "verified_aliases": list(c.SLOTS[slot][1]),
                    "verified": True, "verification_run_id": verify_run}
        c.require(row == expected)
    return rows, artifacts, run


def verify_metadata_age(controls, run):
    started = dt.datetime.fromisoformat(run["created_at"].replace("Z", "+00:00"))
    for environment in controls.values():
        for secret in environment["secrets"].values():
            c.require(dt.datetime.fromisoformat(secret["updated_at"].replace("Z", "+00:00")) <= started)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("operation", choices=("inspect", "import", "verify", "retire", "complete"))
    parser.add_argument("--gh", required=True)
    parser.add_argument("--revision", required=True)
    parser.add_argument("--source-run", required=True)
    parser.add_argument("--verification-run")
    parser.add_argument("--before", type=Path, required=True, help="Original metadata snapshot, never key values")
    parser.add_argument("--evidence", type=Path, required=True)
    args = parser.parse_args()
    os.umask(0o077)
    try:
        c.require(re.fullmatch(r"[0-9a-f]{40}", args.revision) is not None)
        plan = c.read_json(ROOT / "ops/key-custody-migration-20261107.json")
        c.validate_plan(plan)
        api = GitHub(args.gh)
        args.evidence.mkdir(mode=0o700, parents=True, exist_ok=True)
        # Reject duplicate completed operations BEFORE any external side effect.
        c.require(not (args.evidence / (args.operation + ".json")).exists())
        variables = {row["name"]: row["value"] for row in api.pages(f"{PREFIX}/actions/variables", "variables")}
        c.require(variables.get("KEY_CUSTODY_MIGRATION_REVISION") == args.revision)
        c.require(variables.get("KEY_CUSTODY_MIGRATION_PHASE") == ("seal" if args.operation in {"inspect", "import"} else "verify"))
        if args.operation not in {"inspect", "import"}:
            c.require(variables.get("KEY_CUSTODY_SOURCE_RUN_ID") == args.source_run)
        holds = held_consumers(api, args.revision)
        controls = environment_controls(api, plan, filled=args.operation not in {"inspect", "import"})
        repository_secrets = secret_metadata(api.pages(f"{PREFIX}/actions/secrets", "secrets"))
        before = json.loads(args.before.read_text())
        originals = secret_metadata(before["repository_secrets"]["secrets"])
        retire_intent_path = args.evidence / "retire-intent.json"
        retire_intent = json.loads(retire_intent_path.read_text()) if retire_intent_path.exists() else None
        if args.operation == "retire" and retire_intent:
            c.require(retire_intent["revision"] == args.revision and retire_intent["source_run"] == args.source_run
                      and retire_intent["verification_run"] == args.verification_run)
            c.require(all(repository_secrets[name] == originals[name] for name in ORIGINALS & set(repository_secrets)))
        elif args.operation != "complete":
            c.require(all(repository_secrets.get(name) == originals[name] for name in ORIGINALS))
        else:
            c.require(not (ORIGINALS & set(repository_secrets)))
        envelopes, source_artifacts, source = source_envelopes(api, plan, args.revision, args.source_run)
        evidence = {"operation": args.operation, "revision": args.revision, "source_run": args.source_run,
                    "holds": holds, "environments": controls, "repository_secrets": repository_secrets,
                    "source_artifact_ids": [row["id"] for row in source_artifacts]}
        if args.operation == "import":
            # Empty at seal time; existing entries can only be a partial import
            # after the completed source run, never a silent environment fallback.
            completed = dt.datetime.fromisoformat(source["updated_at"].replace("Z", "+00:00"))
            for environment in controls.values():
                for secret in environment["secrets"].values():
                    c.require(dt.datetime.fromisoformat(secret["created_at"].replace("Z", "+00:00")) > completed)
            # No key bytes here: GitHub alone can decrypt encrypted_value.
            # PUT is idempotent, so interrupted imports can safely resume while held.
            for slot, envelope in envelopes.items():
                for alias in c.SLOTS[slot][1]:
                    api.api(f"{PREFIX}/environments/{envelope['environment']}/secrets/{alias}", "PUT",
                            {key: envelope[key] for key in ("encrypted_value", "key_id")})
            evidence["after_environments"] = environment_controls(api, plan, filled=True)
            # Authorization is advanced only after all eight entries are present.
            for name, value in (("KEY_CUSTODY_SOURCE_RUN_ID", args.source_run), ("KEY_CUSTODY_MIGRATION_PHASE", "verify")):
                if name in variables:
                    api.api(f"{PREFIX}/actions/variables/{name}", "PATCH", {"name": name, "value": value})
                else:
                    api.api(f"{PREFIX}/actions/variables", "POST", {"name": name, "value": value})
        if args.operation in {"verify", "retire", "complete"}:
            c.require(args.verification_run is not None)
            rows, verify_artifacts, run = verify_receipts(api, plan, args.revision, args.source_run, args.verification_run)
            verify_metadata_age(controls, run)
            evidence.update(verification_run=args.verification_run, verified_aliases=rows,
                            verification_artifact_ids=[row["id"] for row in verify_artifacts])
            if args.operation == "retire":
                # Recheck after receipt validation, immediately before deleting originals.
                held_consumers(api, args.revision)
                controls = environment_controls(api, plan, filled=True)
                verify_metadata_age(controls, run)
                current = secret_metadata(api.pages(f"{PREFIX}/actions/secrets", "secrets"))
                c.require(all(current[name] == originals[name] for name in ORIGINALS & set(current)))
                if retire_intent is None:
                    c.require(ORIGINALS <= set(current))
                    c.write_json(retire_intent_path, {"revision": args.revision, "source_run": args.source_run,
                        "verification_run": args.verification_run, "environments": controls})
                else:
                    c.require(controls == retire_intent["environments"])
                for name in sorted(ORIGINALS):
                    if name in current:
                        api.api(f"{PREFIX}/actions/secrets/{name}", "DELETE")
                remaining = secret_metadata(api.pages(f"{PREFIX}/actions/secrets", "secrets"))
                c.require(not (ORIGINALS & set(remaining)))
                evidence["remaining_repository_secrets"] = remaining
            if args.operation == "complete":
                retired = json.loads((args.evidence / "retire.json").read_text())
                c.require(retired["revision"] == args.revision and retired["source_run"] == args.source_run)
                c.require(args.verification_run != retired["verification_run"])
                c.require(dt.datetime.fromisoformat(run["created_at"].replace("Z", "+00:00")) >
                          dt.datetime.fromisoformat(retired["observed_at"]))
                # Durable final proof precedes cleanup. A failed cleanup needs
                # only metadata/authorization removal, never another key export.
                evidence["observed_at"] = dt.datetime.now(dt.timezone.utc).isoformat()
                c.write_json(args.evidence / "complete-before-cleanup.json", evidence)
                # Close one-time access; never reactivate a consumer.
                api.api(f"{PREFIX}/actions/workflows/{source['workflow_id']}/disable", "PUT")
                for name in ("KEY_CUSTODY_MIGRATION_REVISION", "KEY_CUSTODY_MIGRATION_PHASE", "KEY_CUSTODY_SOURCE_RUN_ID"):
                    api.api(f"{PREFIX}/actions/variables/{name}", "DELETE")
                # Remove every temporary migration artifact, including the first verification.
                ids = {row["id"] for row in source_artifacts + verify_artifacts}
                _, earlier, _ = verify_receipts(api, plan, args.revision, args.source_run, retired["verification_run"])
                ids.update(row["id"] for row in earlier)
                for artifact_id in sorted(ids):
                    api.api(f"{PREFIX}/actions/artifacts/{artifact_id}", "DELETE")
                evidence["deleted_artifact_ids"] = sorted(ids)
                c.require(api.api(f"{PREFIX}/actions/workflows/{source['workflow_id']}")["state"] == "disabled_manually")
                remaining_variables = api.pages(f"{PREFIX}/actions/variables", "variables")
                c.require(not any(row["name"].startswith("KEY_CUSTODY_") for row in remaining_variables))
                for run_id in (args.source_run, retired["verification_run"], args.verification_run):
                    c.require(not api.pages(f"{PREFIX}/actions/runs/{run_id}/artifacts", "artifacts"))
                evidence["final_holds"] = held_consumers(api, args.revision)
                evidence["final_environments"] = environment_controls(api, plan, filled=True)
                verify_metadata_age(evidence["final_environments"], run)
                c.require(not (ORIGINALS & set(secret_metadata(api.pages(f"{PREFIX}/actions/secrets", "secrets")))))
        evidence["observed_at"] = dt.datetime.now(dt.timezone.utc).isoformat()
        c.write_json(args.evidence / (args.operation + ".json"), evidence)
    except Exception:
        print("Custody relay stopped. Preserve holds and existing evidence; inspect metadata before retrying.", file=sys.stderr)
        return 1
    print(f"Custody {args.operation} verified. Signing remains disabled.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
