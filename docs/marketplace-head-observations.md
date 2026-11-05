# Marketplace head observations

Marketplace readiness and projection checks retain their absolute 12-second
observation deadline. Exact-reader and readiness deadlines start before database
reads; the shared projection path starts its existing head deadline after reader
snapshots. Head admission consumes the remaining deadline, rather than failing
after a separate two-second wait while the observation still has time available.

All four scalar head reads, wire spacing, provider holds, capacity limits and
freshness checks remain in force. No read receives a new 12-second budget.
Other transports, including recovery and transaction paths, keep their existing
admission policy. The observation transport rejects mutations and unknown methods.
Its caller must retain the outer timeout around HTTP responses and retries.

Cancellation drops the waiter without reserving a future timestamp or starting
a background task. This addresses short local queues; it cannot guarantee
availability during provider failures or exhausted capacity.

The admission and transport regressions exercise cancellation, exhausted deadlines,
negative holds and the original incident timing. Production acceptance still
requires observing the combined release against the configured providers.

A healthy reader can finish its next normal poll after the request observes a new
safe batch. Only a reader whose sole problem is block lag may recheck its durable
database progress. The first recheck is immediate; subsequent checks are at least
one second apart, at most twelve in total, and share the original observation
deadline. The original safe/latest targets and safe timestamp never change. The
20-block limit and 90-second heartbeat limit remain unchanged. Failed, quarantined,
stale or inconsistent readers cannot be retried into a successful response.

A recheck reads the completed heartbeat before its cursor in a read-only,
repeatable-read database snapshot. Current projection loaders then read their
reader evidence, scoped rows and canonical certification together in a new full
snapshot, bounded by the original observation. They never attach a new cursor to
old feed rows. Autonomous terms remain selected by canonical creation hashes;
Beta3 current-state rows beyond the boundary remain unavailable, not empty.
Historical financial reads retain their existing certification and privacy rules.

Database progress adds no chain RPC calls or worker polling. It can bridge a
short ordinary poll gap but cannot promise success when the next poll falls after
the remaining deadline. Those requests still return unavailable. Waiting also
cannot renew a head near its maximum age or extend a captured projection's expiry.
