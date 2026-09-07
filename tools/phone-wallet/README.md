# Phone-wallet bundle

Install the pinned dependencies with `npm ci --ignore-scripts --no-fund`, then
run `npm run build` or `npm run check`. The bundle is lazy-loaded by the shared
phone-wallet adapter; increment its cache version when changing shipped code.

## Closed browser storage recovery

The pinned idb-keyval 6.2.1 connection factory caches a database handle forever.
An embedded browser that closes that handle leaves every subsequent transaction
failing with `InvalidStateError`, including wallet connection and balance reads.

`storage-recovery-plugin.mjs` replaces only that exact factory at bundle time.
The replacement reopens the same database on close/version change, or retries
transaction acquisition once when the cached handle is closing. It does not
retry a transaction callback, an asynchronous transaction failure, a wallet
request, or a signature. Existing SDK serialization, legacy migration, database
names, records, session prefixes and payment journals remain unchanged.

The build rejects an unexpected upstream factory or a missing replacement.
Review this shim when upgrading idb-keyval; remove it once upstream provides
equivalent recovery and the regression suite passes without it.

From the repository root, run `node --test scripts/test-phone-wallet.js` and
`node scripts/test-phone-wallet-storage.cjs`. The latter also needs
`npm ci --prefix tools/browser-layout` and that package's Playwright Chromium.
It reproduces the upstream failure, then exercises real IndexedDB through the
pinned WalletConnect driver with synthetic records and blocked external traffic.
Pages runs this suite before deploying. Reverting the shim, regenerated bundle,
adapter and version references together rolls back the change without deleting
or migrating any user records.
