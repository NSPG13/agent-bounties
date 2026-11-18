#!/bin/sh
# PREPARATION ONLY. Root must admit/pin helpers, policy and artifact before use.
set -eu
PATH=/usr/bin:/bin
LC_ALL=C
export PATH LC_ALL
[ "$#" -eq 7 ] || exit 2
mode=$1 sid=$2 revision=$3 launcher=$4 worker=$5 policy=$6 policy_hash=$7
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
printf '%s\n' "$sid" | grep -Eq '^srv-[a-z0-9]{20}$'
printf '%s\n' "$revision" | grep -Eq '^[0-9a-f]{40}$'
printf '%s\n' "$launcher" "$worker" "$policy_hash" | awk 'length($0)!=64 || $0 !~ /^[0-9a-f]+$/ {exit 2}'
[ "$(sha256sum -- "$policy" | cut -d ' ' -f 1)" = "$policy_hash" ]
case "$mode" in
 funding-held) role=parent; expected_mode=held; signing=false; executable=/usr/local/bin/managed-funding-launcher; artifact=$launcher ;;
 funding-active) role=worker; expected_mode=active; signing=true; executable=/usr/local/bin/managed-funding-worker; artifact=$worker ;;
 *) exit 2 ;;
esac
awk -F '\t' -v role="$role" -v sid="$sid" -v revision="$revision" -v mode="$expected_mode" -v signing="$signing" '
 $1!=role {bad=1}
 $2=="RENDER_SERVICE_ID" {ids++;if($3!=sid) bad=1}
 $2=="RENDER_GIT_COMMIT" {revs++;if($3!=revision) bad=1}
 $2=="RECOVERY_FUNDING_MODE" {modes++;if($3!=mode) bad=1}
 $2=="ENABLE_HOSTED_FINANCIAL_SIGNING" {flags++;if($3!=signing) bad=1}
 END {if(bad || ids!=1 || revs!=1 || modes!=1 || flags!=1) exit 23}' "$policy"

files_exact() {
 [ "$(sha256sum /usr/local/bin/managed-funding-launcher | cut -d ' ' -f 1)" = "$launcher" ]
 [ "$(sha256sum /usr/local/bin/managed-funding-worker | cut -d ' ' -f 1)" = "$worker" ]
}
process_identity() {
 [ "$(readlink /proc/1/exe)" = "$executable" ]
 [ "$(sha256sum /proc/1/exe | cut -d ' ' -f 1)" = "$artifact" ]
 awk -v executable="$executable" 'BEGIN {RS="\0"} {n++;if(n!=1 || $0!=executable) bad=1} END {if(bad || n!=1) exit 21}' /proc/1/cmdline
 awk '{sub(/^[0-9]+ \(.*\) /,"");if($2!=0 || $20 !~ /^[0-9]+$/) exit 22;print "1:" $2 ":" $20}' /proc/1/stat
}
. "$here/task_children.sh"
files_exact
identity=$(timeout --signal=TERM --kill-after=1s 5s env -i /usr/local/bin/managed-funding-launcher --build-identity 2>/dev/null)
printf '%s\n' "$identity" | awk -v sid="$sid" -v revision="$revision" -v capability=funding -f "$here/identity.awk"
tasks_before=$(tasks_snapshot)
before=$(process_identity)
awk -v role="$role" -v policy="$policy" -f "$here/environment.awk" /proc/1/environ
after=$(process_identity)
[ "$before" = "$after" ]
tasks_after=$(tasks_snapshot)
[ "$tasks_before" = "$tasks_after" ]
files_exact
printf 'all_pid1_tasks_observed_without_children=true\nstable_task_snapshots=true\nactual_funding_process_identity_stable=true\ninspection_complete=true\n'
