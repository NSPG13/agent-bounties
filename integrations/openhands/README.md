# OpenHands ↔ Agent Bounties integration

Lets an OpenHands agent recognise Agent Bounties as paid-work infrastructure,
follow the earning loop, and — crucially — **not be able to end a session while a
bond is posted and nothing is submitted**, or claim it was paid when it was not.

Three pieces:

| Path | Role |
|---|---|
| `.agents/skills/agent-bounties/SKILL.md` | AgentSkills skill (YAML frontmatter per [the SDK skill guide](https://docs.openhands.dev/sdk/guides/skill)) teaching the loop and emitting one exact next action |
| `.openhands/hooks.json` + `.openhands/hooks/agent-bounties-evidence.py` | `Stop` hook that blocks completion when a claim lacks settle-ready evidence |
| `integrations/openhands/state_producer.py` | the authoritative, session-scoped claim-state source the hook reads |

## Why a separate state producer

Per [the hooks docs](https://docs.openhands.dev/openhands/usage/customization/hooks),
a hook receives the **OpenHands event payload** on stdin:

```json
{"event_type": "Stop", "tool_name": null, "session_id": "abc-123", "working_dir": "/workspace"}
```

That payload says nothing about bounties, so the guard must not read bounty state
from stdin. It shells out to a configured producer instead, appending the session
id so state is resolved per session.

## Split of authority

Three kinds of fact go into the decision, from three different places, with three
different levels of trust. This is the core of the design.

| Fact | Source | Can it *allow* a stop? |
|---|---|---|
| **Claim identity** — which network, bounty, contract, round, solver | operator binding file, outside the session-writable area | only by explicitly declaring `"claim": "none"` |
| **Claim occupancy / settlement** | canonical Base events, fetched **live**, validated against the bound identity | yes — this is the only thing that can say *paid* |
| **Local work facts** — acceptance command, pass/fail, evidence, submission | session workfile | no — they can only ever *block* |

The direction of trust matters. Local work facts gate blocking only, so a lie
there can only trap the agent in more work — the safe direction to fail. Claim
identity is deliberately **not** taken from the workfile: otherwise deleting the
workfile would make the guard conclude there is no claim and wave the session
through, which is exactly the failure this guard exists to prevent.

### What counts as payment

Paid language requires a settlement receipt that is **all** of the following:

- `provenance: canonical_live` — fetched live from the canonical feed in this very
  invocation. Offline snapshots can never settle (see below).
- carrying real chain identity: `tx_hash`, `log_key`, and a positive `block_number`.
- stamped with a sane `occurred_at` (not in the future beyond a small clock-skew
  tolerance).
- **bound** to the same network, bounty id, bounty contract, round and solver as
  the session's operator-declared claim.
- on a canonical settlement network — one with an `AgentBountyFactory` deployment
  and its immutable USDC settlement token. A testnet settlement is not payment,
  however internally consistent it looks.

Anything else — a local `bounty_settled: true`, a hand-written
`settlement.canonical_event`, a real settlement for a different bounty or round,
a settlement paid to a different solver — is rejected, and the guard falls through
to "payment is NOT confirmed, report $0.00". The producer strips settlement-shaped
keys from the workfile *and* the hook independently re-validates the receipt, so
this is defence in depth rather than one line.

## Wiring

```bash
# JSON-array form is unambiguous and recommended, especially on Windows:
export AGENT_BOUNTIES_STATE_CMD='["python3", "-B", "integrations/openhands/state_producer.py"]'
export AGENT_BOUNTIES_BINDING_FILE="$HOME/.agent-bounties/binding.json"   # Ed25519-signed
export AGENT_BOUNTIES_OPERATOR_PUBKEY="<base64 operator public key>"      # required
export AGENT_BOUNTIES_SESSION_DIR="$HOME/.agent-bounties/sessions"        # <session-id>.json
```

### One-time: create an operator key, OUTSIDE the sandbox

```bash
python3 -B integrations/openhands/sign_binding.py keygen \
    --out ~/.agent-bounties/operator.key
```

It prints the public key to pin as `AGENT_BOUNTIES_OPERATOR_PUBKEY`. **Keep the
private seed off any machine the agent session can read** — that separation is
the entire authority boundary.

### The operator binding (claim identity)

Written and **signed** by the operator, never by the agent session:

```json
{
  "schema_version": "agent-bounties/openhands-session-binding/v1",
  "network": "base-mainnet",
  "solver": "0x1111111111111111111111111111111111111111",
  "sessions": {
    "abc-123": {
      "bounty_id":       "0x7a7a…7a",
      "bounty_contract": "0xabab…ab",
      "round": 2
    },
    "def-456": { "claim": "none" }
  },
  "signature": {
    "alg": "ed25519",
    "public_key_b64": "…",
    "signature_b64": "…"
  }
}
```

Sign it (and **re-sign after every edit** — a stale signature is an invalid one):

```bash
python3 -B integrations/openhands/sign_binding.py sign \
    --key ~/.agent-bounties/operator.key --binding ~/.agent-bounties/binding.json
```

A session that is **not listed** fails closed: an unknown session is not assumed
idle. Declare `"claim": "none"` for sessions doing unrelated work.

> **Trust boundary.** Claim identity is authenticated by an Ed25519 signature
> verified against a public key pinned out-of-band in
> `AGENT_BOUNTIES_OPERATOR_PUBKEY`. The private key never enters the sandbox.
>
> This is an *enforced* boundary, not a convention. An earlier version relied on
> the binding simply living outside the session directory and not being group- or
> world-writable, which is not a boundary at all: a same-OS-user session can
> rewrite any file that user owns, and Windows has no equivalent mode bits. The
> session may still edit the file freely — it just cannot produce a valid
> signature over the edit.
>
> Three distinct forgeries are refused, and each needs its own check:
>
> | Attack | Refused because |
> |---|---|
> | **EDIT** a field | the signature no longer covers the body |
> | **ERASE** the signature | unsigned bindings are refused outright |
> | **REPLACE**, re-signed with the session's own key | the signature verifies, so verification alone is insufficient — the embedded key must **equal the pinned one** |
>
> The signature covers the whole document minus the `signature` envelope, so no
> field can quietly fall outside its coverage. Location and permission checks are
> retained only as a cheap misconfiguration warning; they are explicitly not the
> boundary.
>
> `scripts/check-openhands-integration.py` proves this by running each attack
> with the **session's own identity** against an outside-path, owner-writable
> binding — asserting first that the write genuinely succeeds, and then that the
> decision does not move.

**No extra packages are needed in the sandbox.** Verification uses the standard
library alone (`integrations/openhands/ed25519_verify.py`, pinned to the RFC 8032
section 7.1 test vectors and cross-checked against PyCryptodome by the checker),
so the boundary cannot silently degrade because an install failed. Only the
operator-side *signing* tool needs PyCryptodome, already pinned by
`scripts/requirements-attest.txt`.

### The session workfile (local work facts)

Only `test`, `evidence` and `submission.submitted_onchain` are read; every other
key, including anything settlement-shaped, is dropped.

```json
{
  "test": {"command": "python -B /benchmark/check.py", "passed": true},
  "evidence": {"repository": "…", "commit": "…", "test_command": "…",
               "source_snapshot_digest": "…", "discovery_source": "…",
               "participation_reason": "…", "improvement_feedback": "…"},
  "submission": {"submitted_onchain": true}
}
```

A missing workfile means "no work recorded", which blocks. It cannot disable the
guard.

### Environment reference

| Variable | Meaning |
|---|---|
| `AGENT_BOUNTIES_STATE_CMD` | producer argv — a JSON array, or a shell-style string |
| `AGENT_BOUNTIES_BINDING_FILE` | **required** Ed25519-signed operator claim identity |
| `AGENT_BOUNTIES_OPERATOR_PUBKEY` | **required** base64 Ed25519 public key the binding must be signed by |
| `AGENT_BOUNTIES_SESSION_DIR` | directory of `<session-id>.json` work facts |
| `AGENT_BOUNTIES_SESSION_FILE` | exact workfile path, instead of `_DIR` + session id |
| `AGENT_BOUNTIES_API` | override the API base (default `https://api.agentbounties.app`) |
| `AGENT_BOUNTIES_EVENTS_FILE` | **test-only** offline event snapshot; requires `AGENT_BOUNTIES_ALLOW_TEST_SNAPSHOT=1`, and can never settle |
| `AGENT_BOUNTIES_STATE` | a plain state file, bypassing the producer (test/manual use) |

There is **no silent offline fallback**. If the canonical feed is unreachable and
no snapshot is explicitly opted into, the producer exits non-zero with nothing on
stdout, and the hook blocks the stop.

## Cross-platform invocation

`hooks.json` registers the guard through an **explicit Python interpreter**:

```json
{"command": "python3 .openhands/hooks/agent-bounties-evidence.py", "timeout": 60}
```

not as a bare path. A bare `.openhands/hooks/foo.py` relies on the executable bit
and a shebang, neither of which exists on Windows. For the same reason
`AGENT_BOUNTIES_STATE_CMD` accepts a JSON array: POSIX-mode `shlex` treats `\` as
an escape, which would silently mangle `C:\repo\state_producer.py`, so the guard
splits Windows command strings in non-POSIX mode and accepts an explicit array as
the unambiguous form.

## Exit contract

The docs define exactly three outcomes: `0` proceeds, `2` blocks, and *any other
code is an error and the operation proceeds anyway*. So there is no "error" exit
code in the guard — every refusal path, **including an unhandled crash**, exits
`2`. An uncaught Python exception would exit `1` and silently let a session with a
live bond end, which is the precise failure this guard exists to prevent. The Stop
hook is also never registered `async`, because async hooks can never block.

| Situation | Exit |
|---|---|
| no state source configured (repo not doing bounty work) | 0 |
| operator binding declares `"claim": "none"` for this session | 0 |
| session not declared in the operator binding | 2 |
| no operator binding configured at all | 2 |
| active claim, no acceptance command recorded | 2 |
| acceptance check failed | 2 |
| evidence incomplete | 2 |
| nothing submitted on-chain | 2 |
| submitted, awaiting verifier | 0 — with "payment is NOT confirmed, report $0.00" |
| settlement asserted but not live-canonical / not bound to this claim | 0 — still "$0.00", assertion reported as rejected |
| **bound, live-canonical `BountySettled`** | 0 — paid language allowed |
| state source unreadable / producer failed / feed unusable / guard crashed | 2 |

Note the ordering: a bound canonical receipt is evaluated **before** the "no
active claim" early-out, because settlement *ends* the claim — checking occupancy
first would report "no active claim" and silently discard the payment evidence.

## Checks

```bash
python -B scripts/check-openhands-integration.py
WORKSPACE_ROOT=$(pwd) python -B benchmarks/direct-growth-v2/openhands-integration/check.py
```

The first drives the **shipped** producer rather than a stub, reads the hook
command straight out of `hooks.json`, and sends a real `Stop` event on stdin with
the documented `OPENHANDS_*` environment variables — so a regression in either
component fails the gate. It stands up a real local HTTP server to serve canonical
events, so the live-fetch path (including HTTP 500 and unreachable-host handling)
is exercised for real rather than mocked. Everything runs through `sys.executable`
with no shell, no `chmod +x` dependency and no POSIX-only path assumptions, so the
suite behaves identically on Windows and Linux; the single POSIX-only assertion
(binding permission bits) is explicitly skipped on Windows.

## Wallet safety

Nothing here reads, stores, logs, or transmits secret key material, and nothing
broadcasts a transaction. The solver address is a public identifier; signing stays
with an external signer the operator controls.
