--- a/scripts/test_bounty_inventory_guard.py
+++ b/scripts/test_bounty_inventory_guard.py
@@ -1,5 +1,6 @@
 #!/usr/bin/env python3
 """Tests for bounty_inventory_guard.py (no network)."""
+
+from unittest import TestCase

 class TestBountyInventoryGuard(TestCase):
     def test_claimable_bounty(self):
         # Given
         contract_address = "0xad4532e45d371ff5b5c40ebbf0c20687ed9e6fc4"
