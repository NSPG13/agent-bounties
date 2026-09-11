--- a/scripts/next-agent-claim-action.mjs
+++ b/scripts/next-agent-claim-action.mjs
@@ -1,6 +1,6 @@
-import { assert } from 'console';
+import { assert } from 'console';
 import { assert } from '../utils/contract';
@@ -24,8 +24,6 @@
   const contract = new Contract();
   return contract.methods
     .claimNextAction()
-    .setOptions({
-      skipEncoding: false,
+    .send();
 };

 export function run(): void {
@@ -35,8 +33,6 @@
   const result = await contract.methods
     .claimNextAction()
-    .setOptions({
-      skipEncoding: false,
+    .send();
   await result.wait();
   assert.isTrue(
@@ -47,8 +43,6 @@
   const nextAgentAction = await contract.methods
     .claimNextAction()
-    .setOptions({
-      skipEncoding: false,
+    .send();
   await nextAgentAction.wait();
-  } catch (error) {
-    assert.falsy(error);
+  }
 }
