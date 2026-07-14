from __future__ import annotations

import argparse
import json
import os
import re
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Callable


SCHEMA = "agent-bounties/render-deploy-evidence-v1"
REPOSITORY = "github.com/nspg13/agent-bounties"
PROTOCOL = "agent-bounties/autonomous-v1"
ACTIVE_STATUSES = {
    "created",
    "queued",
    "build_in_progress",
    "pre_deploy_in_progress",
    "update_in_progress",
}
FAILED_STATUSES = {
    "build_failed",
    "pre_deploy_failed",
    "update_failed",
    "canceled",
    "deactivated",
}
TRANSIENT_HTTP_STATUSES = {429, 500, 503}


class RecoveryError(RuntimeError):
    pass


class RenderHttpError(RecoveryError):
    def __init__(self, status: int, body: str) -> None:
        self.status = status
        super().__init__(f"Render API returned HTTP {status}: {redact(body)}")


class RenderTransportError(RecoveryError):
    pass


@dataclass(frozen=True)
class ServiceSpec:
    name: str
    service_type: str
    health_url: str | None


SERVICE_SPECS = (
    ServiceSpec(
        "agent-bounties-api",
        "web_service",
        "https://agent-bounties-api.onrender.com/health",
    ),
    ServiceSpec(
        "agent-bounties-mcp",
        "web_service",
        "https://agent-bounties-mcp.onrender.com/health",
    ),
    ServiceSpec("agent-bounties-base-indexer", "background_worker", None),
)


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def redact(value: str) -> str:
    value = re.sub(r"(?i)(authorization:\s*bearer\s+)[^\s]+", r"\1[redacted]", value)
    value = re.sub(r"(?i)([?&](?:key|token)=)[^&\s]+", r"\1[redacted]", value)
    value = re.sub(r"(?i)(render_api_key\s*[=:]\s*)[^\s,;]+", r"\1[redacted]", value)
    return value[:1000]


def validate_revision(revision: str) -> str:
    normalized = revision.strip().lower()
    if not re.fullmatch(r"[0-9a-f]{40}", normalized):
        raise RecoveryError("revision must be an exact 40-character Git SHA")
    return normalized


def normalize_repo(value: object) -> str:
    if not isinstance(value, str):
        return ""
    normalized = value.strip().lower().removesuffix(".git")
    normalized = re.sub(r"^(?:https?://|git@)", "", normalized)
    return normalized.replace(":", "/", 1) if normalized.startswith("github.com:") else normalized


def unwrap_service_entries(payload: object) -> list[dict[str, Any]]:
    if not isinstance(payload, list):
        raise RecoveryError("Render service-list response must be an array")
    services: list[dict[str, Any]] = []
    for entry in payload:
        if not isinstance(entry, dict):
            continue
        service = entry.get("service", entry)
        if isinstance(service, dict):
            services.append(service)
    return services


def select_service(spec: ServiceSpec, payload: object) -> dict[str, Any]:
    matches = [
        service
        for service in unwrap_service_entries(payload)
        if service.get("name") == spec.name
    ]
    if len(matches) != 1:
        raise RecoveryError(
            f"expected exactly one Render service named {spec.name}; found {len(matches)}"
        )
    service = matches[0]
    if service.get("type") != spec.service_type:
        raise RecoveryError(f"{spec.name} has unexpected Render service type")
    if service.get("branch") != "main":
        raise RecoveryError(f"{spec.name} is not connected to the main branch")
    if normalize_repo(service.get("repo")) != REPOSITORY:
        raise RecoveryError(f"{spec.name} is connected to an unexpected repository")
    service_id = service.get("id")
    if not isinstance(service_id, str) or not re.fullmatch(r"srv-[0-9a-z]+", service_id):
        raise RecoveryError(f"{spec.name} has an invalid Render service id")
    return service


def unwrap_deploy(payload: object) -> dict[str, Any]:
    if not isinstance(payload, dict):
        raise RecoveryError("Render deploy response must be an object")
    deploy = payload.get("deploy", payload)
    if not isinstance(deploy, dict):
        raise RecoveryError("Render deploy response is missing deploy metadata")
    return deploy


def deploy_commit(deploy: dict[str, Any]) -> str | None:
    commit = deploy.get("commit")
    if not isinstance(commit, dict):
        return None
    commit_id = commit.get("id")
    return commit_id.lower() if isinstance(commit_id, str) else None


def existing_deploy(payload: object, revision: str) -> dict[str, Any] | None:
    if not isinstance(payload, list):
        raise RecoveryError("Render deploy-list response must be an array")
    deploys: list[dict[str, Any]] = []
    for entry in payload:
        try:
            deploy = unwrap_deploy(entry)
        except RecoveryError:
            continue
        deploys.append(deploy)
        if deploy_commit(deploy) == revision and deploy.get("status") in ACTIVE_STATUSES:
            return deploy
    if deploys and deploy_commit(deploys[0]) == revision and deploys[0].get("status") == "live":
        return deploys[0]
    return None


class RenderClient:
    def __init__(
        self,
        token: str,
        *,
        api_base: str = "https://api.render.com/v1",
        timeout_seconds: float = 20,
        sleeper: Callable[[float], None] = time.sleep,
    ) -> None:
        if not token.strip():
            raise RecoveryError("RENDER_API_KEY is required")
        self._token = token.strip()
        self._api_base = api_base.rstrip("/")
        self._timeout_seconds = timeout_seconds
        self._sleep = sleeper

    def _request_json(
        self,
        method: str,
        path: str,
        payload: dict[str, Any] | None = None,
    ) -> Any:
        body = None if payload is None else json.dumps(payload).encode("utf-8")
        request = urllib.request.Request(
            f"{self._api_base}{path}",
            data=body,
            method=method,
            headers={
                "Accept": "application/json",
                "Authorization": f"Bearer {self._token}",
                "Content-Type": "application/json",
                "User-Agent": "agent-bounties-render-recovery/1",
            },
        )
        try:
            with urllib.request.urlopen(request, timeout=self._timeout_seconds) as response:
                response_body = response.read().decode("utf-8")
        except urllib.error.HTTPError as error:
            error_body = error.read().decode("utf-8", errors="replace")
            raise RenderHttpError(error.code, error_body) from None
        except (urllib.error.URLError, TimeoutError) as error:
            raise RenderTransportError(
                f"Render API transport failed: {redact(str(error))}"
            ) from None
        try:
            return json.loads(response_body) if response_body else {}
        except json.JSONDecodeError as error:
            raise RecoveryError(f"Render API returned invalid JSON: {error}") from None

    def _read_with_retry(self, path: str, attempts: int = 3) -> Any:
        for attempt in range(1, attempts + 1):
            try:
                return self._request_json("GET", path)
            except RenderHttpError as error:
                if error.status not in TRANSIENT_HTTP_STATUSES or attempt == attempts:
                    raise
            except RenderTransportError:
                if attempt == attempts:
                    raise
            self._sleep(float(attempt * 2))
        raise AssertionError("unreachable")

    def resolve_service(self, spec: ServiceSpec) -> dict[str, Any]:
        query = urllib.parse.urlencode(
            {"name": spec.name, "includePreviews": "false", "limit": "20"}
        )
        return select_service(spec, self._read_with_retry(f"/services?{query}"))

    def disable_native_auto_deploy(self, service: dict[str, Any]) -> None:
        if service.get("autoDeploy") is False:
            return
        service_id = service["id"]
        updated = None
        for attempt in range(1, 4):
            try:
                updated = self._request_json(
                    "PATCH", f"/services/{service_id}", {"autoDeploy": "no"}
                )
                break
            except RenderHttpError as error:
                if error.status not in TRANSIENT_HTTP_STATUSES or attempt == 3:
                    raise
            except RenderTransportError:
                if attempt == 3:
                    raise
            self._sleep(float(attempt * 2))
        updated_service = updated.get("service", updated) if isinstance(updated, dict) else None
        if not isinstance(updated_service, dict) or updated_service.get("autoDeploy") is not False:
            raise RecoveryError(f"Render did not disable native auto-deploy for {service['name']}")

    def list_deploys(self, service_id: str) -> Any:
        return self._read_with_retry(f"/services/{service_id}/deploys?limit=20")

    def get_deploy(self, service_id: str, deploy_id: str) -> dict[str, Any]:
        return unwrap_deploy(
            self._read_with_retry(f"/services/{service_id}/deploys/{deploy_id}")
        )

    def ensure_deploy(self, service: dict[str, Any], revision: str) -> dict[str, Any]:
        service_id = service["id"]
        matched = existing_deploy(self.list_deploys(service_id), revision)
        if matched is not None:
            return matched

        for attempt in range(1, 4):
            try:
                return unwrap_deploy(
                    self._request_json(
                        "POST",
                        f"/services/{service_id}/deploys",
                        {"clearCache": "do_not_clear", "commitId": revision},
                    )
                )
            except RenderHttpError as error:
                if error.status not in TRANSIENT_HTTP_STATUSES or attempt == 3:
                    raise
            except RenderTransportError:
                if attempt == 3:
                    raise
            self._sleep(float(attempt * 2))
            matched = existing_deploy(self.list_deploys(service_id), revision)
            if matched is not None:
                return matched
        raise AssertionError("unreachable")


def validate_deploy(deploy: dict[str, Any], revision: str, service_name: str) -> tuple[str, str]:
    deploy_id = deploy.get("id")
    status = deploy.get("status")
    if not isinstance(deploy_id, str) or not deploy_id.startswith("dep-"):
        raise RecoveryError(f"{service_name} deploy is missing an id")
    if deploy_commit(deploy) != revision:
        raise RecoveryError(f"{service_name} deploy does not attest the requested revision")
    if not isinstance(status, str):
        raise RecoveryError(f"{service_name} deploy is missing status")
    return deploy_id, status


def poll_deploys(
    client: RenderClient,
    pending: dict[str, tuple[str, str]],
    revision: str,
    *,
    timeout_seconds: float,
    poll_seconds: float,
    clock: Callable[[], float] = time.monotonic,
    sleeper: Callable[[float], None] = time.sleep,
) -> dict[str, dict[str, Any]]:
    deadline = clock() + timeout_seconds
    completed: dict[str, dict[str, Any]] = {}
    while pending:
        for service_name, (service_id, deploy_id) in list(pending.items()):
            deploy = client.get_deploy(service_id, deploy_id)
            _, status = validate_deploy(deploy, revision, service_name)
            if status == "live":
                completed[service_name] = deploy
                pending.pop(service_name)
            elif status in FAILED_STATUSES:
                raise RecoveryError(f"{service_name} deploy ended with status {status}")
            elif status not in ACTIVE_STATUSES:
                raise RecoveryError(f"{service_name} deploy has unknown status {status}")
        if not pending:
            break
        if clock() >= deadline:
            names = ", ".join(sorted(pending))
            raise RecoveryError(f"timed out waiting for Render deploys: {names}")
        sleeper(poll_seconds)
    return completed


def fetch_health(url: str, timeout_seconds: float) -> tuple[int, str, dict[str, str]]:
    request = urllib.request.Request(
        url,
        method="GET",
        headers={"Accept": "text/plain", "User-Agent": "agent-bounties-render-recovery/1"},
    )
    try:
        with urllib.request.urlopen(request, timeout=timeout_seconds) as response:
            return (
                response.status,
                response.read().decode("utf-8", errors="replace"),
                {key.lower(): value for key, value in response.headers.items()},
            )
    except urllib.error.HTTPError as error:
        return error.code, error.read().decode("utf-8", errors="replace"), {}
    except (urllib.error.URLError, TimeoutError) as error:
        raise RecoveryError(f"health probe transport failed: {redact(str(error))}") from None


def validate_health(
    service_name: str,
    revision: str,
    response: tuple[int, str, dict[str, str]],
) -> dict[str, Any]:
    status, body, headers = response
    observed_revision = headers.get("x-agent-bounties-revision")
    observed_protocol = headers.get("x-agent-bounties-protocol")
    if status != 200 or body.strip() != "ok":
        raise RecoveryError(f"{service_name} health contract is not ready")
    if observed_revision != revision:
        raise RecoveryError(f"{service_name} health reports a different revision")
    if observed_protocol != PROTOCOL:
        raise RecoveryError(f"{service_name} health reports a different protocol")
    return {
        "service": service_name,
        "status": status,
        "body": "ok",
        "revision": observed_revision,
        "protocol": observed_protocol,
    }


def wait_for_health(
    spec: ServiceSpec,
    revision: str,
    *,
    timeout_seconds: float,
    poll_seconds: float,
    probe: Callable[[str, float], tuple[int, str, dict[str, str]]] = fetch_health,
    clock: Callable[[], float] = time.monotonic,
    sleeper: Callable[[float], None] = time.sleep,
) -> dict[str, Any]:
    if spec.health_url is None:
        raise RecoveryError(f"{spec.name} has no public health contract")
    deadline = clock() + timeout_seconds
    last_error = "health did not become ready"
    while True:
        try:
            return validate_health(spec.name, revision, probe(spec.health_url, 10))
        except RecoveryError as error:
            last_error = str(error)
        if clock() >= deadline:
            raise RecoveryError(f"{spec.name} health verification timed out: {last_error}")
        sleeper(poll_seconds)


def write_evidence(path: Path, evidence: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(evidence, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def deploy(
    client: RenderClient,
    revision: str,
    *,
    specs: tuple[ServiceSpec, ...] = SERVICE_SPECS,
    deploy_timeout_seconds: float,
    health_timeout_seconds: float,
    poll_seconds: float,
) -> dict[str, Any]:
    services: list[tuple[ServiceSpec, dict[str, Any]]] = []
    pending: dict[str, tuple[str, str]] = {}
    initial: dict[str, dict[str, Any]] = {}

    for spec in specs:
        services.append((spec, client.resolve_service(spec)))

    for spec, service in services:
        client.disable_native_auto_deploy(service)

    for spec, service in services:
        created = client.ensure_deploy(service, revision)
        deploy_id, status = validate_deploy(created, revision, spec.name)
        initial[spec.name] = created
        if status == "live":
            continue
        if status in FAILED_STATUSES:
            raise RecoveryError(f"{spec.name} deploy ended with status {status}")
        if status not in ACTIVE_STATUSES:
            raise RecoveryError(f"{spec.name} deploy has unknown status {status}")
        pending[spec.name] = (service["id"], deploy_id)

    completed = {
        name: deploy_record
        for name, deploy_record in initial.items()
        if deploy_record.get("status") == "live"
    }
    completed.update(
        poll_deploys(
            client,
            pending,
            revision,
            timeout_seconds=deploy_timeout_seconds,
            poll_seconds=poll_seconds,
        )
    )

    health = [
        wait_for_health(
            spec,
            revision,
            timeout_seconds=health_timeout_seconds,
            poll_seconds=poll_seconds,
        )
        for spec, _ in services
        if spec.health_url is not None
    ]
    service_evidence = []
    for spec, service in services:
        deployed = completed[spec.name]
        deploy_id, status = validate_deploy(deployed, revision, spec.name)
        service_evidence.append(
            {
                "name": spec.name,
                "service_id": service["id"],
                "service_type": spec.service_type,
                "deploy_id": deploy_id,
                "status": status,
                "commit": revision,
                "trigger": deployed.get("trigger"),
                "native_auto_deploy": "disabled",
            }
        )
    return {"services": service_evidence, "health": health}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Deploy and attest an exact reviewed main revision on Render."
    )
    parser.add_argument("--revision", required=True)
    parser.add_argument(
        "--api-url",
        default="https://agent-bounties-api.onrender.com/health",
    )
    parser.add_argument(
        "--mcp-url",
        default="https://agent-bounties-mcp.onrender.com/health",
    )
    parser.add_argument("--deploy-timeout-seconds", type=float, default=2400)
    parser.add_argument("--health-timeout-seconds", type=float, default=300)
    parser.add_argument("--poll-seconds", type=float, default=10)
    parser.add_argument(
        "--output",
        type=Path,
        default=Path("target/operations/render-deploy.json"),
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    evidence: dict[str, Any] = {
        "schema": SCHEMA,
        "started_at": utc_now(),
        "completed_at": None,
        "revision": args.revision,
        "success": False,
        "services": [],
        "health": [],
        "error": None,
    }
    try:
        revision = validate_revision(args.revision)
        specs = list(SERVICE_SPECS)
        specs[0] = ServiceSpec(specs[0].name, specs[0].service_type, args.api_url)
        specs[1] = ServiceSpec(specs[1].name, specs[1].service_type, args.mcp_url)
        client = RenderClient(os.environ.get("RENDER_API_KEY", ""))
        result = deploy(
            client,
            revision,
            specs=tuple(specs),
            deploy_timeout_seconds=args.deploy_timeout_seconds,
            health_timeout_seconds=args.health_timeout_seconds,
            poll_seconds=args.poll_seconds,
        )
        evidence.update(result)
        evidence["revision"] = revision
        evidence["success"] = True
    except (RecoveryError, ValueError) as error:
        evidence["error"] = redact(str(error))
    finally:
        evidence["completed_at"] = utc_now()
        write_evidence(args.output, evidence)

    if not evidence["success"]:
        print(f"render deploy recovery failed: {evidence['error']}", file=sys.stderr)
        return 1
    print(f"render deploy recovery verified {evidence['revision']}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
