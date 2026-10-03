#!/usr/bin/env node
"use strict";
// Read-only: no account, wallet, signature or payment. Node20+.
const proof = require("../site/payout-proof.js");
proof.read(fetch, process.argv[2] || "lifetime").then((result) => {
  process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
  if (result.audit.status !== "ready") process.exitCode = 1;
}).catch((error) => { process.stderr.write(`${error.message}\n`); process.exitCode = 1; });
