--- a/scripts/bounty_inventory_guard.py
+++ b/scripts/bounty_inventory_guard.py
@@ -1,5 +1,6 @@
 #!/usr/bin/env python3
 """Tests for bounty_inventory_guard.py (no network)."""
+
+import json

 def run_guard(
     *args: str, use_meta_defaults: bool = False
 ) -> subprocess.CompletedProcess:
@@ -10,3 +11,25 @@
     # ... (rest of the function remains the same)

+def test_tool_registry_drift() -> None:
+    with open('fixtures/tool_registry.json') as f:
+        expected_tools = json.load(f)
+    actual_tools = get_api_tools()  # assuming get_api_tools() is implemented
+    missing_tools = [tool for tool in expected_tools if tool not in actual_tools]
+    duplicate_tools = [tool for tool in actual_tools if actual_tools.count(tool) > 1]
+    renamed_tools = [tool for tool in expected_tools if tool not in actual_tools and any(other_tool.startswith(tool) for other_tool in actual_tools)]
+    if missing_tools or duplicate_tools or renamed_tools:
+        print("Tool registry drift detected:")
+        if missing_tools:
+            print("Missing tools:", missing_tools)
+        if duplicate_tools:
+            print("Duplicate tools:", duplicate_tools)
+        if renamed_tools:
+            print("Renamed tools:", renamed_tools)
+        sys.exit(1)

--- a/scripts/test_bounty_inventory_guard.py
+++ b/scripts/test_bounty_inventory_guard.py
@@ -1,5 +1,6 @@
 #!/usr/bin/env python3
 """Tests for bounty_inventory_guard.py (no network)."""
+
+import unittest

 class TestBountyInventoryGuard(unittest.TestCase):
     def test_tool_registry_drift(self) -> None:
         # ... (test implementation)
