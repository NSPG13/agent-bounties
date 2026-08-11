# inventory-state-breakdown-v1: canonical breakdown checker
import json, os, sys
from pathlib import Path
from datetime import datetime, timezone
ROOT = Path(os.environ.get("WORKSPACE_ROOT", "/workspace"))
FD = ROOT / "scripts" / "fixtures" / "inventory-state-breakdown"

def load_fixture(name):
    return json.loads((FD / f"{name}.json").read_text())

def compute_breakdown(inv):
    items = inv.get("items", []) or []
    counts = {"ready_to_earn":0,"in_progress":0,"submitted":0,"paid":0,"verification_unavailable":0}
    for i in items:
        s = i.get("status","")
        if s in counts: counts[s] += 1
    t = len(items)
    return {"inventory-state-breakdown-v1": {
        "ready_to_earn": counts["ready_to_earn"],
        "in_progress": counts["in_progress"],
        "submitted": counts["submitted"],
        "paid": counts["paid"],
        "verification_unavailable": counts["verification_unavailable"],
        "total": t,
        "generated_at": inv.get("generated_at", datetime.now(timezone.utc).isoformat()),
        "source": inv.get("source", "canonical"),
    }}

def check_fixture(name):
    inv = load_fixture(name)
    r = compute_breakdown(inv)
    b = r.get("inventory-state-breakdown-v1")
    assert isinstance(b, dict), "missing key"
    for k in ("ready_to_earn","in_progress","submitted","paid","verification_unavailable","generated_at","source"):
        assert k in b, f"missing {k}"
    for k in ("ready_to_earn","in_progress","submitted","paid","verification_unavailable"):
        assert isinstance(b[k], int), f"{k} not int"
    s = sum(b[k] for k in ("ready_to_earn","in_progress","submitted","paid","verification_unavailable"))
    assert s == b["total"], f"sum {s} != total {b["total"]}"
    print(f"  {name}: OK - {s} items")
    return True

errors = 0
for f in ("empty","mixed","degraded","stale"):
    try:
        if not check_fixture(f): errors += 1
    except Exception as e:
        print(f"  {f}: FAIL - {e}")
        errors += 1
if errors:
    print(f"{errors} fixtures failed")
    sys.exit(1)
print("inventory-state breakdown acceptance checks passed")