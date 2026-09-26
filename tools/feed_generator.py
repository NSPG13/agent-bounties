--- scripts/test_standing_meta_v4_release_audit.py
+++ scripts/test_standing_meta_v4_release_audit.py
@@ -0,0 +1,19 @@
+#!/usr/bin/env python3
+
+from __future__ import annotations
+
+import importlib.util
+from pathlib import Path
+import unittest
+
+
+SCRIPT = Path(__file__).with_name("standing_meta_v4_release_audit.py")
+SPEC = importlib.util.spec_from_file_location("standing_meta_v4_release_audit", SCRIPT)
+assert SPEC and SPEC.loader
+MODULE = importlib.util.module_from_spec(SPEC)
+SPEC.loader.exec_module(MODULE)
+
+
+from standing_meta_v4_release_audit import environment_payload
+
+
+class StandingMetaV4ReleaseAuditTests(unittest.TestCase):
+    def test_release_audit(self) -> None:
+        env = environment_payload()
+
