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
