# Open Competition V2 shadow verification

The shadow worker independently reads the canonical V2 event set through a
second RPC provider. Agreement requires matching block identities and the
complete event-set digest from the primary indexer and the shadow provider.
An agreement is a verification read model; canonical settlement events remain
the only proof of payment.

The long-running worker retains a process-local prefix only after agreement
has been persisted successfully. Every reuse requires identical configuration,
a non-regressing common-safe block, and the cached anchor hash from both RPCs.
It then scans the suffix for factory events and all previously or newly
discovered competition contracts. The full combined event set must still match
the primary database. A repeated safe block requires fresh anchor checks and a
new comparison with the primary event set, without repeating historical log
queries.

Both providers must return the same target block before and after scanning.
Out-of-range logs, unexpected emitters, conflicting duplicate logs, malformed
responses, or database errors fail verification. Disagreement and failed or
cancelled polls discard the cache. Reorgs, changed configuration, and cursor
regression require a full replay. A process restart also starts from the
deployment block; no cached history is reconstructed from the primary database.

The observation timestamp is captured before the poll starts. A long replay
cannot refresh old observations merely by completing late. Existing API
freshness and lag limits remain enforced during cold replay and recovery.

Validation:

```bash
cargo test -p worker open_competition_v2_shadow --lib
# Requires an isolated test database; also enforced by the Postgres CI job.
cargo test -p worker open_competition_v2_shadow::tests::postgres_shadow_cache_certifies_full_sets_and_recovers_from_disagreement -- --ignored --exact
```

Maintainer scope and contributor impact: [notice #1293](https://github.com/NSPG13/agent-bounties/issues/1293).
