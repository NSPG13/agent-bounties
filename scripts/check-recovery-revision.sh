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
exit 0
