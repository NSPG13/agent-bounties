import { readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const replacement = fileURLToPath(new URL("./src/reconnecting-store.js", import.meta.url));
const original = `function createStore(dbName, storeName) {
    const request = indexedDB.open(dbName);
    request.onupgradeneeded = () => request.result.createObjectStore(storeName);
    const dbp = promisifyRequest(request);
    return (txMode, callback) => dbp.then((db) => callback(db.transaction(storeName, txMode).objectStore(storeName)));
}`;

// Patch just the connection factory in the pinned dependency. Retain upstream
// serialization, migration, all key/value operations and license notices. Fail
// closed on an upstream change so SDK upgrades require an explicit review.
export const storageRecoveryPlugin = {
  name: "phone-wallet-storage-recovery",
  setup(builder) {
    builder.onLoad({ filter: /[\\/]idb-keyval[\\/]dist[\\/]index\.js$/ }, async ({ path: filename }) => {
      const source = (await readFile(filename, "utf8")).replace(/\r\n/g, "\n");
      if (source.split(original).length !== 2) throw new Error("Review the updated idb-keyval connection factory before rebuilding the phone wallet.");
      return {
        contents: source.replace(original, `import { createReconnectingStore as createStore } from ${JSON.stringify(replacement)};`),
        loader: "js", resolveDir: path.dirname(filename),
      };
    });
  },
};
