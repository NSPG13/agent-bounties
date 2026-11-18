# Fixed PID1 task observation. Sourced by the reviewed inspector only.
# Two matching snapshots are point-in-time evidence, not continuous monitoring.
task_ids() {
 count=0
 for task in /proc/1/task/[0-9]*; do
  [ -d "$task" ] || return 22
  tid=${task##*/}
  case "$tid" in ''|*[!0-9]*) return 22 ;; esac
  [ "$tid" -gt 0 ] || return 22
  count=$((count + 1)); [ "$count" -le 256 ] || return 22
  printf '%s\n' "$tid"
 done
 [ "$count" -gt 0 ] || return 22
}
task_identity() {
 awk -v tid="$1" '$1!=tid {exit 22} {sub(/^[0-9]+ \(.*\) /, ""); if(NF<20 || $20 !~ /^[0-9]+$/) exit 22; print tid ":" $20; n++} END {if(n!=1) exit 22}' "/proc/1/task/$1/stat" || return 22
}
tasks_snapshot() {
 tasks_before=$(task_ids) || return 22
 identities=
 for tid in $tasks_before; do
  first=$(task_identity "$tid") || return 22
  awk 'NF {exit 22}' "/proc/1/task/$tid/children" || return 22
  second=$(task_identity "$tid") || return 22
  [ "$first" = "$second" ] || return 22
  identities="${identities}${first}
"
 done
 tasks_after=$(task_ids) || return 22
 [ "$tasks_before" = "$tasks_after" ] || return 22
 printf '%s' "$identities"
}
