--- a/scripts/test_standing_meta_v4_release_audit.py
+++ b/scripts/test_standing_meta_v4_release_audit.py
@@ -1,5 +1,6 @@
 #!/usr/bin/env python3

+from datetime import datetime, timezone
 from __future__ import annotations

 import importlib.util
 from pathlib import Path
--- a/scripts/test_bounty_inventory_guard.py
+++ b/scripts/test_bounty_inventory_guard.py
@@ -10,6 +10,7 @@
 from pathlib import Path

 ROOT = Path(__file__).resolve().parents[1]
 SCRIPT = Path(__file__).resolve().parent / "bounty_inventory_guard.py"
+FIXTURES = Path(__file__).resolve().parent / "claim_readiness_fixtures"
 
 def run_guard(
--- a/scripts/plan_bounded_agent_action.py
+++ b/scripts/plan_bounded_agent_action.py
@@ -10,6 +10,11 @@
 import json
 from pathlib import Path

+class ClaimReadiness:
+    def __init__(self, reward: int, refundable_bond: int, external_spend: int, gross_cash_margin: int, blocker: str):
+        self.reward = reward
+        self.refundable_bond = refundable_bond
+        self.external_spend = external_spend
+        self.gross_cash_margin = gross_cash_margin
+        self.blocker = blocker
+
 from bounded_agent_create import validate_creation_plan
 from inspect_bounded_agent_wallet import call, inspect, word_address, word_uint, words
 from plan_bounded_agent_budget import (
