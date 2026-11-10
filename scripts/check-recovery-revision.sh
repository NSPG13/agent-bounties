#!/bin/sh
# Pre-deploy admission only. No shell evaluation, network, database or startup.
set -eu
LC_ALL=C
export LC_ALL

invalid_expected() {
    printf '%s\n' 'recovery revision guard: expected one full lowercase revision' >&2
    exit 2
}

[ "$#" -eq 1 ] || invalid_expected
expected_revision=$1
[ "${#expected_revision}" -eq 40 ] || invalid_expected
case "$expected_revision" in *[!0-9a-f]*) invalid_expected ;; esac

deployed_revision=${RENDER_GIT_COMMIT-}
case "$deployed_revision" in
    ''|*[!0-9a-f]*)
        printf '%s\n' 'recovery revision guard: missing or invalid deployed revision' >&2
        exit 1
        ;;
esac
if [ "${#deployed_revision}" -ne 40 ] || [ "$deployed_revision" != "$expected_revision" ]; then
    printf '%s\n' 'recovery revision guard: deployed revision does not match' >&2
    exit 1
fi

# These fixed paths are the shipped runtime files, never caller-selected input.
# Compute every digest before emitting a complete receipt. No application starts.
artifact_error() {
    printf '%s\n' 'recovery revision guard: artifact receipt unavailable' >&2
    exit 1
}
file_digest() {
    [ -f "$1" ] && [ -r "$1" ] && [ -x "$1" ] || artifact_error
    digest_line=$(/usr/bin/sha256sum -- "$1" 2>/dev/null) || artifact_error
    digest=${digest_line%% *}
    [ "${#digest}" -eq 64 ] || artifact_error
    case "$digest" in *[!0-9a-f]*) artifact_error ;; esac
    printf '%s' "$digest"
}
api_path=/usr/local/bin/agent-bounties-service
reader_path=/usr/local/bin/recovery-reader
if [ -e "$api_path" ] && [ ! -e "$reader_path" ] && [ ! -L "$reader_path" ]; then
    artifact_kind=api
    binary_digest=$(file_digest "$api_path") || artifact_error
elif [ -e "$reader_path" ] && [ ! -e "$api_path" ] && [ ! -L "$api_path" ]; then
    artifact_kind=reader
    binary_digest=$(file_digest "$reader_path") || artifact_error
    supervisor_digest=$(file_digest /usr/local/bin/run-recovery-indexers) || artifact_error
    bash_digest=$(file_digest /bin/bash) || artifact_error
else
    artifact_error
fi
guard_digest=$(file_digest /usr/local/bin/check-recovery-revision) || artifact_error
printf 'schema=agentizable/recovery-artifact-receipt-v1\nsource_revision=%s\nartifact_kind=%s\n' "$expected_revision" "$artifact_kind"
printf 'binary_sha256=%s\n' "$binary_digest"
if [ "$artifact_kind" = reader ]; then
    printf 'supervisor_sha256=%s\nbash_sha256=%s\n' "$supervisor_digest" "$bash_digest"
fi
printf 'guard_sha256=%s\nreceipt_complete=true\n' "$guard_digest"
