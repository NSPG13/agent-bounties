#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
project_name="${PRODUCTION_COMPOSE_PROJECT:-agent-bounties-prod-smoke}"
env_file="${PRODUCTION_COMPOSE_ENV_FILE:-.env.example}"
api_port="${PRODUCTION_COMPOSE_API_PORT:-18080}"
mcp_port="${PRODUCTION_COMPOSE_MCP_PORT:-18090}"
require_eval_history="false"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --project-name)
      project_name="${2:-}"
      shift 2
      ;;
    --env-file)
      env_file="${2:-}"
      shift 2
      ;;
    --api-port)
      api_port="${2:-}"
      shift 2
      ;;
    --mcp-port)
      mcp_port="${2:-}"
      shift 2
      ;;
    --require-eval-history)
      require_eval_history="true"
      shift
      ;;
    *)
      echo "unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

cd "$repo_root"
docker_uses_windows_cli="false"
runtime_env_file=".production-compose.$$.env"
grep -v -E '^(API_PORT|MCP_PORT|PUBLIC_BASE_URL|MCP_BASE_URL)=' "$env_file" > "$runtime_env_file" || true
{
  echo "API_PORT=$api_port"
  echo "MCP_PORT=$mcp_port"
  echo "PUBLIC_BASE_URL=http://127.0.0.1:$api_port"
  echo "MCP_BASE_URL=http://127.0.0.1:$mcp_port"
} >> "$runtime_env_file"
compose_args=(--env-file "$runtime_env_file" -p "$project_name" -f docker-compose.production.yml)

if ! docker version >/dev/null 2>&1 && command -v docker.exe >/dev/null 2>&1; then
  docker() { docker.exe "$@"; }
  docker_uses_windows_cli="true"
fi

cleanup() {
  docker compose "${compose_args[@]}" down -v
  rm -f "$runtime_env_file"
}
trap cleanup EXIT

wait_http_ok() {
  local url="$1"
  for _ in $(seq 1 60); do
    if [[ "$docker_uses_windows_cli" == "true" ]] && command -v powershell.exe >/dev/null 2>&1; then
      if powershell.exe -NoProfile -Command "try { Invoke-WebRequest -UseBasicParsing -Uri '$url' -TimeoutSec 2 | Out-Null; exit 0 } catch { exit 1 }" >/dev/null 2>&1; then
        return 0
      fi
    elif curl -fsS "$url" >/dev/null 2>&1; then
      return 0
    fi
    sleep 2
  done
  echo "$url did not become healthy within 120 seconds" >&2
  return 1
}

docker compose "${compose_args[@]}" up -d --build
wait_http_ok "http://127.0.0.1:$api_port/health"
wait_http_ok "http://127.0.0.1:$mcp_port/health"

smoke_args=(
  --api-base-url "http://127.0.0.1:$api_port"
  --mcp-base-url "http://127.0.0.1:$mcp_port"
)
if [[ "$require_eval_history" == "true" ]]; then
  smoke_args+=(--require-eval-history)
fi
bash scripts/check-production-smoke.sh "${smoke_args[@]}"
