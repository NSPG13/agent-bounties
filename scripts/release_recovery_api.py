#!/usr/bin/env python3
"""Release only the replacement API after exact protected-main CI and Containers.

No service settings, environment, readers, credentials or payment state writes.
The only mutation is one exact-commit deploy request to the fixed API service.
"""
import argparse
import json
import os
from pathlib import Path
import re
import time
import urllib.error
import urllib.parse
import urllib.request

REPOSITORY = "NSPG13/agent-bounties"
SERVICE = "srv-dbp7j78m7kps73a35vm0"
OWNER = "tea-d4tk87f5r7bs73au55lg"
HEALTH = "https://api.agentizable.ai/health"
ACTIVE = {"created", "build_in_progress", "pre_deploy_in_progress", "update_in_progress", "queued"}
FAILED = {"build_failed", "pre_deploy_failed", "update_failed", "canceled", "deactivated"}
SHA = re.compile(r"[0-9a-f]{40}")
DEPLOY_ID = re.compile(r"dep-[a-z0-9]{20}")


class ReleaseError(Exception):
    """Only fixed, credential-free error codes may leave this controller."""


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


class Client:
    def __init__(self, base, token):
        if not token:
            raise ReleaseError("missing_api_credential")
        self.base, self.token = base, token

    def request(self, method, path, payload=None):
        body = None if payload is None else json.dumps(payload).encode()
        request = urllib.request.Request(self.base + path, data=body, method=method,
            headers={"Authorization": "Bearer " + self.token, "Accept": "application/json",
                     "Content-Type": "application/json", "User-Agent": "agentizable-api-release/1"})
        try:
            with urllib.request.build_opener(NoRedirect).open(request, timeout=20) as response:
                data = response.read(1_048_577)
            if len(data) > 1_048_576:
                raise ReleaseError("oversized_api_response")
            return json.loads(data)
        except (urllib.error.URLError, TimeoutError, OSError, ValueError):
            # Never log provider bodies, URLs, credentials or ambiguous POSTs.
            raise ReleaseError("api_request_failed_no_mutation_retry") from None


def trigger_revision(event):
    run = event.get("workflow_run", {})
    revision = run.get("head_sha", "")
    if (event.get("repository", {}).get("full_name") != REPOSITORY
        or run.get("head_repository", {}).get("full_name") != REPOSITORY
        or run.get("head_branch") != "main" or run.get("event") != "push"
        or run.get("name") not in {"CI", "Containers"}
        or run.get("conclusion") != "success" or run.get("status") != "completed"
        or not isinstance(revision, str) or not SHA.fullmatch(revision)):
        raise ReleaseError("invalid_workflow_trigger")
    return revision


def current_main(github, revision):
    branch = github.request("GET", f"/repos/{REPOSITORY}/branches/main")
    if branch.get("name") != "main" or branch.get("protected") is not True:
        raise ReleaseError("main_must_be_protected")
    return branch.get("commit", {}).get("sha") == revision


def qualify(github, revision):
    if not current_main(github, revision):
        return "stale_main"
    for filename, name in (("ci.yml", "CI"), ("containers.yml", "Containers")):
        query = urllib.parse.urlencode({"head_sha": revision, "branch": "main", "event": "push", "per_page": 100})
        payload = github.request("GET", f"/repos/{REPOSITORY}/actions/workflows/{filename}/runs?{query}")
        if not isinstance(payload.get("workflow_runs"), list) or payload.get("total_count", 0) > 100:
            raise ReleaseError("invalid_workflow_runs")
        runs = [run for run in payload["workflow_runs"] if run.get("head_sha") == revision
                and run.get("head_branch") == "main" and run.get("event") == "push"
                and run.get("head_repository", {}).get("full_name") == REPOSITORY]
        if not runs:
            return "awaiting_exact_sha_checks"
        latest = max(runs, key=lambda run: (int(run["id"]), int(run.get("run_attempt", 1))))
        if latest.get("name") != name:
            raise ReleaseError("unexpected_workflow_identity")
        if latest.get("status") != "completed" or latest.get("conclusion") != "success":
            return "awaiting_exact_sha_checks"
    return "qualified" if current_main(github, revision) else "stale_main"


def validate_service(service):
    expected = {"id": SERVICE, "name": "agentizable-api-recovery-20261109", "ownerId": OWNER,
        "repo": "https://github.com/NSPG13/agent-bounties", "branch": "main", "rootDir": "",
        "suspended": "not_suspended", "type": "web_service", "autoDeploy": "no", "autoDeployTrigger": "off"}
    details = service.get("serviceDetails", {})
    docker = details.get("envSpecificDetails", {})
    if (any(service.get(key) != value for key, value in expected.items())
        or details.get("runtime") != "docker" or details.get("healthCheckPath") != "/health"
        or docker.get("dockerfilePath") != "./Dockerfile.recovery-api"
        or docker.get("dockerContext") != "." or docker.get("dockerCommand") != ""
        or docker.get("preDeployCommand") != "/usr/local/bin/check-recovery-revision --built-revision"):
        raise ReleaseError("replacement_api_binding_mismatch")


def deploy_record(payload):
    record = payload.get("deploy", payload) if isinstance(payload, dict) else {}
    if not DEPLOY_ID.fullmatch(record.get("id", "")):
        raise ReleaseError("invalid_deploy_record")
    return record


def deploy_revision(record):
    return record.get("commit", {}).get("id")


def verify_health(revision):
    request = urllib.request.Request(HEALTH, headers={"Cache-Control": "no-cache"})
    try:
        with urllib.request.build_opener(NoRedirect).open(request, timeout=20) as response:
            valid = (response.status == 200 and response.read(100) == b"ok"
                     and response.headers.get("x-agent-bounties-revision") == revision)
    except (urllib.error.URLError, TimeoutError, OSError):
        raise ReleaseError("api_health_unavailable") from None
    if not valid:
        raise ReleaseError("api_health_revision_mismatch")


def release(github, render, revision, *, sleep=time.sleep, clock=time.monotonic, health=verify_health):
    reason = qualify(github, revision)
    if reason != "qualified":
        return {"outcome": reason}
    validate_service(render.request("GET", f"/services/{SERVICE}"))
    entries = render.request("GET", f"/services/{SERVICE}/deploys?limit=100")
    if not isinstance(entries, list) or not entries:
        raise ReleaseError("missing_deploy_history")
    records = [deploy_record(entry) for entry in entries]
    if any(record.get("status") not in ACTIVE | FAILED | {"live"} for record in records):
        raise ReleaseError("unknown_deploy_status")
    active = [record for record in records if record.get("status") in ACTIVE]
    if len(active) > 1:
        return {"outcome": "service_busy"}
    record = None
    if active:
        if deploy_revision(active[0]) != revision:
            return {"outcome": "service_busy"}
        record = active[0]
    else:
        live = next((item for item in records if item.get("status") == "live"), None)
        if live and deploy_revision(live) == revision:
            health(revision)
            return {"outcome": "already_live", "deploy_id": live["id"]}
        if any(deploy_revision(item) == revision for item in records):
            raise ReleaseError("previous_attempt_failed_no_automatic_retry")
    if record is None:
        # Recheck after provider discovery; new pushes/reruns revoke admission.
        reason = qualify(github, revision)
        if reason != "qualified":
            return {"outcome": reason}
        record = deploy_record(render.request("POST", f"/services/{SERVICE}/deploys",
                                              {"commitId": revision, "clearCache": "do_not_clear"}))
    deploy_id = record["id"]
    deadline = clock() + 1800
    while True:
        if deploy_revision(record) != revision:
            raise ReleaseError("deployed_commit_mismatch")
        status = record.get("status")
        if status == "live":
            validate_service(render.request("GET", f"/services/{SERVICE}"))
            health(revision)
            return {"outcome": "live", "deploy_id": deploy_id}
        if status in FAILED:
            raise ReleaseError("deploy_failed")
        if status not in ACTIVE:
            raise ReleaseError("unknown_deploy_status")
        if clock() >= deadline:
            raise ReleaseError("deploy_monitor_timeout")
        sleep(10)
        record = deploy_record(render.request("GET", f"/services/{SERVICE}/deploys/{deploy_id}"))
        if record["id"] != deploy_id:
            raise ReleaseError("deploy_identity_mismatch")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=["qualify", "deploy"])
    parser.add_argument("--evidence", type=Path, required=True)
    args = parser.parse_args()
    evidence = {"schema": "agentizable/recovery-api-main-release-v1", "service_id": SERVICE}
    result = 0
    try:
        revision = trigger_revision(json.loads(Path(os.environ["GITHUB_EVENT_PATH"]).read_text()))
        evidence["revision"] = revision
        github = Client("https://api.github.com", os.environ.get("GH_TOKEN", ""))
        reason = qualify(github, revision)
        evidence["outcome"] = reason
        if args.mode == "deploy" and reason == "qualified":
            render = Client("https://api.render.com/v1", os.environ.get("RENDER_API_KEY", ""))
            evidence.update(release(github, render, revision))
        if args.mode == "qualify" and os.environ.get("GITHUB_OUTPUT"):
            with open(os.environ["GITHUB_OUTPUT"], "a") as output:
                output.write("deploy=" + str(reason == "qualified").lower() + "\n")
    except (ReleaseError, AttributeError, KeyError, TypeError, ValueError, OSError) as error:
        evidence["outcome"] = "failed"
        evidence["error"] = str(error) if isinstance(error, ReleaseError) else "invalid_release_input"
        result = 1
    args.evidence.parent.mkdir(parents=True, exist_ok=True)
    args.evidence.write_text(json.dumps(evidence, indent=2) + "\n")
    print(json.dumps(evidence))
    return result


if __name__ == "__main__":
    raise SystemExit(main())
