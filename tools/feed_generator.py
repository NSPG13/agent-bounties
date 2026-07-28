--- a/scripts/plan_bounded_agent_action.py
+++ b/scripts/plan_bounded_agent_action.py
@@ -10,6 +10,9 @@
 from bounded_agent_create import validate_creation_plan
 from inspect_bounded_agent_wallet import call, inspect, word_address, word_uint, words
 from plan_bounded_agent_budget import (
+    ActivationLifecycle,
+    ACTIVATION_LIFECYCLES,
+    get_activation_lifecycle,
     ROOT,
     calldata,
     encode,
     keccak_hex,
     require_address,
@@ -50,7 +53,7 @@
     # ... (rest of the function remains the same)
 
-    if action == "claim":
+    if action == "claim" and get_activation_lifecycle(contract_address) == ActivationLifecycle.CLAIMABLE:
         # ... (rest of the claim logic remains the same)
 
-    if action == "submit":
+    if action == "submit" and get_activation_lifecycle(contract_address) == ActivationLifecycle.CLAIMED:
         # ... (rest of the submit logic remains the same)
 
 --- a/scripts/bounded_agent_create.py
+++ b/scripts/bounded_agent_create.py
@@ -10,6 +10,9 @@
 from inspect_bounded_agent_wallet import call, inspect, word_address, word_uint, words
 from plan_bounded_agent_budget import (
+    ActivationLifecycle,
+    ACTIVATION_LIFECYCLES,
+    get_activation_lifecycle,
     ROOT,
     calldata,
     encode,
@@ -50,7 +53,7 @@
     # ... (rest of the function remains the same)
 
-    if status == "claimable":
+    if status == "claimable" and get_activation_lifecycle(contract_address) == ActivationLifecycle.CLAIMABLE:
         # ... (rest of the claimable logic remains the same)
 
 --- a/scripts/plan_bounded_agent_budget.py
+++ b/scripts/plan_bounded_agent_budget.py
@@ -10,6 +10,20 @@
 from _shared.rpc import rpc
 
+class ActivationLifecycle:
+    CLAIMABLE = "claimable"
+    CLAIMED = "claimed"
+    SUBMITTED = "submitted"
+    VERIFYING = "verifying"
+
+ACTIVATION_LIFECYCLES = {
+    ActivationLifecycle.CLAIMABLE,
+    ActivationLifecycle.CLAIMED,
+    ActivationLifecycle.SUBMITTED,
+    ActivationLifecycle.VERIFYING,
+}
+
+def get_activation_lifecycle(contract_address: str) -> ActivationLifecycle:
+    # Replace with actual logic to determine the activation lifecycle
+    # For now, just return the claimable lifecycle
+    return ActivationLifecycle.CLAIMABLE
+
 ROOT = Path(__file__).resolve().parents[1]
 calldata = {}
