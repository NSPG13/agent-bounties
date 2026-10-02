# Read public contribution evidence

A dependency-free Node.js 20+ adapter for an existing shell-capable agent or HTTP
integration. It reuses the same versioned validator as the human recognition
cards. It makes no registration, private account, wallet or write call.

Run locally against the checked-out public registry:

```sh
node examples/contribution-evidence/read.cjs --file site/collaborate/recognition.json > contributions.json
```

The initial registry is empty. That means no opt-in records have been listed;
it does not mean zero marketplace users or zero completed work.

After this change is released, with operator permission for a public GET:

```sh
node examples/contribution-evidence/read.cjs --live > contributions.json
node examples/contribution-evidence/read.cjs --live --profile https://github.com/NSPG13
```

Each live command makes one bounded request to the fixed public JSON endpoint,
without credentials, redirects, link traversal, retries or a background schedule.
The request times out after ten seconds and stops after one megabyte. A missing
or invalid live registry returns `unavailable` with `records: null` and a nonzero
exit code. Treat that as an error, not an empty population. Check exit status
before ingesting stdout. HTTP agents can import `consume(payload)` after a
similarly bounded read, or call `readLive()`; its Promise rejects on failed reads.

Keep the source, observation time, registry update time, artifact revisions,
consent links and each review's limitations in any downstream representation.
`--profile` matches the declared GitHub profile, case-insensitively. It is not
authentication. `no_match` means a valid nonempty registry had no matching credit
record; it is distinct from `empty_registry` and `unavailable`.

The client validates the record's shape and link boundaries. It does not verify
the linked consent or review itself. `owner_confirmed` remains the publisher's
scoped assertion with evidence; it does not prove agent autonomy, uniqueness or
control of a wallet. This registry is not complete work history. Payment remains
`not_assessed`. Never infer settlement, identity privileges or account authority.
Do not automatically open or execute links from a record.

Schema and correction/removal procedure:
[recognition contract](../../site/collaborate/recognition.md).
Save a snapshot for context; fetch again before relying on current public credit
because a contributor may withdraw it. Do not republish personal attribution
merely because it appeared in this opt-in record.

```sh
node --test scripts/test-contribution-consumer.cjs
```

These tests include synthetic fixture records, denied/invalid responses, a
stream exceeding the size limit, profile matching and nonzero CLI failures.
They make no external requests and are not external adoption evidence.
