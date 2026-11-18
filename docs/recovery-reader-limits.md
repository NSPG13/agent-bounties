# Recovery reader runtime limits

The dedicated recovery launcher starts isolated reader processes. Saved Render
settings only affect a child when explicitly forwarded by this launcher.

Both autonomous readers receive configured values for:

- `BASE_INDEXER_POLL_SECONDS` and `BASE_INDEXER_CONFIRMATIONS`
- `BASE_INDEXER_MAX_BLOCKS_PER_QUERY`
- `BASE_INDEXER_RETRY_INITIAL_SECONDS`, `BASE_INDEXER_RETRY_MAX_SECONDS`, and
  `BASE_INDEXER_EXIT_AFTER_FAILURES`
- `BASE_RPC_READ_WIRE_INTERVAL_MS`

Unset values retain the reader binary defaults. Values are passed literally;
the binary performs validation. No shell evaluation or silent replacement of
invalid settings occurs. Standby, the binding check and the optional creator
reader do not receive these newly forwarded settings. The V2 factory and start
block remain fixed; V1-only bootstrap/archive settings remain V1-only. No signer,
notification or broadcast capability is added.

Read pacing applies per process and endpoint. Two readers can make requests
concurrently, and API/funding processes have separate capacity needs. A one-second
interval in each child does not establish a shared one-request-per-second quota.
The sleep between polls also does not rate-limit the requests within a poll.
Provider qualification must cover historical log ranges, contract address batches,
headers and event timestamps at the expected combined load. A successful chain-ID
or state read alone does not establish production capacity.

A successful poll after checkpoint revalidation can clear a failed heartbeat
before catch-up completes. This can unblock downstream automatic review signing
under existing financial configuration. Any production rollout must explicitly
control that consequence and verify current obligations before enabling it.
This correction grants no restart, deployment, history rewrite, quarantine
release, signature, payment or new provider authority.

Focused local checks:

```sh
PYTHONPATH=scripts python3 -m unittest scripts.test_recovery_reader_boundary scripts.test_recovery_revision_guard scripts.test_recovery_inventory
bash -n scripts/run-recovery-indexers.sh
```

The process fixtures execute the launcher with a temporary local reader stub;
they verify actual child environments, secret exclusion, fixed identity and
sibling shutdown. They do not exercise a database, live provider or deployed image.
