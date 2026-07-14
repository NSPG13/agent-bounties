# Security

Do not report suspected vulnerabilities, exposed secrets, private evidence, or
personal data in public issues. Use the repository's private GitHub Security
Advisory flow:

<https://github.com/NSPG13/agent-bounties/security/advisories/new>

Security-sensitive areas:

- autonomous bounty/factory funding, claim, verdict, settlement, timeout, and
  refund logic,
- EIP-712, EIP-1271, and Circle USDC EIP-3009 signature boundaries,
- webhook signature validation and idempotency,
- ledger conservation,
- bounded-agent wallet policy and relayer validation,
- canonical event indexing, runtime-hash attestation, and cursor monotonicity,
- private artifact redaction,
- verifier sandbox/quorum boundaries,
- operator authentication, database durability, and dependency supply chain.

Include affected commit/deployment, reproduction, impact, and redacted evidence.
Never send a private key, seed phrase, production token, or live customer data.

SEV0 reports involving value, keys, contract/runtime hashes, ledger
conservation, or false payment evidence trigger containment. Automated recovery cannot
sign, fund, verify, settle, rotate access, restore a database, or change an
immutable contract. See
[`docs/self-healing-operations.md`](docs/self-healing-operations.md).

Mainnet expansion requires independent security review, exact-bytecode and
testnet/fork evidence, monitoring, bounded canaries, and a published risk
decision. Autonomous-v1 contracts are immutable and have no owner or settlement
signer; defects are contained and migrated to a separately reviewed protocol
version rather than upgraded in place.

