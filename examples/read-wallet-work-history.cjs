#!/usr/bin/env node
// Read-only public lookup. No wallet credentials, dependency installation or transactions.
const history = require('../site/work-history.js');
if (process.argv.length !== 3) {
  console.error('Usage: node examples/read-wallet-work-history.cjs 0xPUBLIC_SOLVER_WALLET');
  process.exitCode = 2;
} else {
  history.inspect(process.argv[2]).then(snapshot => {
    console.log(JSON.stringify(snapshot, null, 2));
    if (snapshot.status === 'unavailable') process.exitCode = 1;
  }).catch(error => { console.error(error.message); process.exitCode = 2; });
}
