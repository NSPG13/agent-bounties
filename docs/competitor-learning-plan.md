# Competitor And Adjacent-Protocol Learning Plan

Research date: 2026-07-15. Adoption figures below are public snapshots or
platform-reported figures, not audited market-share claims.

## Executive View

Agent Bounties should remain an open, wallet-neutral outcome market for
verifiable digital work. Its defensible center is the combination of canonical
escrow, explicit verification policy, machine-first discovery, evidence-bound
reputation, and completed payment loops. Competitors show that distribution and
low-friction posting matter as much as contract correctness, but they also show
that posted reward volume is not the same as task liquidity.

The highest-value changes are:

1. make the first earning loop one machine-readable request per state change;
2. expose weighted rubrics, deterministic evidence, and appeal rules before
   funding;
3. let any wallet boost a reward and make every settled result visibly public;
4. support signed negotiation and poster-as-evaluator without requiring a third
   party for objective work;
5. compose jobs into objective graphs and portable reputation without requiring
   a platform token.

## Direct Markets

### Algora

- **Positioning:** GitHub-native cash bounties for open-source work.
- **How it works:** a maintainer can create a bounty from an issue comment;
  solvers submit normal pull requests and payout follows the maintainer's merge
  and approval flow. Its API exposes bounties, tasks, claims, solvers, issues,
  and pull requests.
- **Adoption evidence:** public project pages show meaningful posted inventory,
  but examples also show a large gap between open reward value and completed
  payouts. This is evidence that inventory alone does not create liquidity.
- **Does well:** familiar GitHub workflow, one-comment posting, fiat compliance
  and payout abstraction, public project and earner statistics.
- **Does poorly for our goal:** maintainer approval and merge remain human
  settlement gates; the interface is contributor-first rather than autonomous
  agent-first.
- **Implement:** one-comment posting/claiming, PR-native evidence, public
  completion-rate and time-to-first-claim metrics, payout abstraction.
- **Avoid:** optimizing for posted dollars or open issue count instead of
  completed-and-paid loops.

Sources: [Algora API](https://api.docs.algora.io/),
[pricing and payouts](https://algora.io/pricing).

### ClawTasks

- **Positioning:** a task market designed for OpenClaw agents, originally with
  Base USDC payment and solver stake.
- **How it works:** agents can onboard from a compact `skill.md`, discover work,
  and claim tasks through machine-oriented calls.
- **Adoption evidence:** its current public site says paid bounties are being
  wound down while reliability, review flow, and worker quality are hardened;
  the public activity snapshot showed no activity.
- **Does well:** direct agent onboarding, curl/skill-first orientation, explicit
  stake, Base-native payment framing.
- **Does poorly:** payment capability outran recovery and quality operations.
- **Implement:** portable skill distribution and compact agent instructions.
- **Avoid:** claiming payment reliability before external end-to-end loops and
  incident repayment paths are proven.

Sources: [ClawTasks](https://clawtasks.com/),
[terms](https://clawtasks.com/terms).

### Pump.fun GO

- **Positioning:** a social bounty and grant surface embedded in Pump.fun's
  existing Solana distribution.
- **How it works:** users or automated agents post funded bounties, others boost
  reward pools, submissions and rulings are public, and winners claim on-chain
  payouts. The UI emphasizes open rewards, recent payouts, earners, spenders,
  creators, and sharing.
- **Adoption evidence:** a public bot-walkthrough bounty received four
  submissions and paid one winner 0.5 SOL (about $39 at settlement), with the
  ruling, evidence, and receipt displayed publicly.
- **Does well:** reward boosting, one-screen creation, adjacent distribution,
  paid-proof pages, social rankings, multiple-winner configuration, bot-based
  posting.
- **Does poorly for our goal:** human/social UI dominates; terms preserve broad
  operator discretion; the surrounding speculative-token system adds legal,
  moderation, and incentive risk.
- **Implement:** `Boost reward` on every bounty, recent-settlement feed,
  evidence-linked payout cards, creator/solver/funder profiles, configurable
  winner splits, and bot posting.
- **Avoid:** bounty tokens, bonding curves, speculative rewards, physical tasks,
  and operator discretion that can override a valid immutable policy.

Sources: [GO bounty feed](https://pump.fun/go/bounties),
[settled bot walkthrough](https://pump.fun/go/d2b9b519-1424-44fa-9872-2d3bb846ceeb),
[GO terms](https://pump.fun/docs/go-fun-terms),
[Pump bonding curve](https://pump.fun/docs/bonding-curve).

## Funding And Role Protocols

### Gitcoin Allo

- **Positioning:** composable capital-allocation infrastructure rather than a
  solver marketplace.
- **How it works:** registry-owned pools accept contributions; an allocation
  strategy controls eligibility, allocation, and distribution. Funding must use
  the pool method so accounting and strategy state remain canonical.
- **Does well:** separates pooled capital from allocation policy, supports
  permissionless funding, and makes strategy replaceable.
- **Gap:** it does not solve agent discovery, task execution, or proof quality.
- **Implement:** keep one canonical token per bounty, prevent raw transfers from
  becoming accounting evidence, and expose audited verification/allocation
  modules as policy plugins.

Sources: [Allo pools](https://docs.allo.gitcoin.co/overview/pool),
[flow of funds](https://docs.allo.gitcoin.co/allo/flow-of-funds).

### StandardBounties

- **Positioning:** an interoperable EVM bounty registry with explicit issuer,
  approver, contributor, fulfiller, and submitter roles.
- **How it works:** anyone can issue or contribute; fulfillments reference
  off-chain content; an approver chooses accepted work and payouts; relayers
  can submit meta-transactions.
- **Adoption evidence:** the repository retains hundreds of stars and forks but
  its deployments and documentation are from an older Ethereum era, so it is a
  design precedent rather than current usage evidence.
- **Does well:** role separation, permissionless co-funding, content-addressed
  artifacts, shared indexing, relayers.
- **Does poorly:** mutable issuer powers and approver discretion can weaken
  funded expectations.
- **Implement:** role-separated events, content hashes, relayers, and open
  indexing.
- **Avoid:** post-funding term mutation or unilateral issuer withdrawal.

Source: [StandardBounties repository](https://github.com/Bounties-Network/StandardBounties).

### ERC-8183 / Virtuals Agent Commerce Protocol

- **Positioning:** a minimal standard for agent jobs with escrow and evaluator
  attestation. Virtuals ACP adds discovery, negotiation, evaluation services,
  workflows, and agent identity around that kernel.
- **How it works:** ACP uses request, signed negotiation, transaction/escrow,
  and evaluation phases. Draft ERC-8183 standardizes `Open -> Funded ->
  Submitted -> Completed/Rejected/Expired`, with client, provider, and evaluator
  roles plus optional hooks. The evaluator may be the client/poster.
- **Adoption evidence:** Virtuals displays agents with thousands to tens of
  thousands of jobs and reported success/aGDP figures. Treat these as
  platform-reported transaction metrics, not independently verified quality.
- **Does well:** signed proof of agreement, capability offers, evaluator market,
  composable workflows, portable ERC-8004 identity/reputation, and a small
  interoperable escrow kernel.
- **Does poorly for our goal:** agent tokenization and `$VIRTUAL` add an
  unnecessary participation dependency; transaction counts do not prove useful
  outcomes; documentation has version drift.
- **Implement:** an ERC-8183 compatibility adapter, signed quote/negotiation,
  poster-as-evaluator, optional third-party evaluator, immutable reason/evidence
  hashes, objective graphs, and ERC-8004 export.
- **Avoid:** requiring a platform token or tokenizing every agent.

Sources: [ACP overview](https://whitepaper.virtuals.io/about-virtuals/agent-commerce-protocol-acp),
[technical flow](https://whitepaper.virtuals.io/about-virtuals/agent-commerce-protocol-acp/technical-deep-dive),
[ERC-8183 draft](https://eips.ethereum.org/EIPS/eip-8183).

## Verification Markets

### Code4rena

- **Positioning:** competitive smart-contract security review with open
  participation, judges, public findings, and prize pools.
- **How it works:** many wardens submit findings; judges determine severity and
  duplicates; rewards are distributed by a documented formula; post-judging QA
  and appeal paths exist. Bounty submissions use an anti-spam deposit.
- **Adoption evidence:** Code4rena reports more than 10,000 auditors and often
  100+ participants per audit; these are platform-reported figures.
- **Does well:** many-solver discovery, duplicate-aware payouts, paid judge
  roles, runnable proof-of-concept requirements, formal QA/appeal windows, and
  public reports.
- **Does poorly for tiny agent tasks:** fixed deposits and manual coordination
  are too expensive; winner-take-all assumptions do not fit duplicate discovery.
- **Implement:** multi-solver mode, duplicate clusters, pro-rata or severity
  rewards, verifier compensation, bounded appeal, and sandboxed regression
  tests as evidence.
- **Avoid:** a fixed high bond across all bounty values.

Sources: [competitions](https://docs.code4rena.com/competitions),
[awarding](https://docs.code4rena.com/awarding),
[bounties](https://docs.code4rena.com/bounties).

### Verdikta

- **Positioning:** decentralized AI judgment and automatic on-chain settlement,
  with a bounty application on Base.
- **How it works:** creators publish weighted rubrics and a multi-model jury.
  Agents register a wallet for an API key, upload work to IPFS, prepare an
  evaluation wallet, start the funded evaluation, poll/finalize, and receive
  ETH if the threshold passes. A creator-approval window can precede the jury.
- **Adoption evidence:** the public agent page showed zero bounties while the
  main listing returned an authentication error in the observed snapshot. The
  protocol is live, but public market adoption is currently low or not reliably
  observable.
- **Does well:** explicit weighted criteria, immutable rubric/jury details,
  multi-model feedback, status diagnosis, timeout recovery, and first-class
  agent API documentation.
- **Does poorly:** API-key registration, public-IPFS privacy, solver-paid
  evaluation, two required chain transactions, and first-passing-winner rules
  create friction and gaming risk. AI consensus is not objective truth.
- **Implement:** normalized weighted rubric schema, committed judge/model
  versions, independent verdicts, machine-readable feedback, replay fixtures,
  and verifier-failure appeal/slashing.
- **Avoid:** mandatory API keys, mandatory solver-paid evaluation, unbounded AI
  authority, and calling multi-model consensus objective without an appeal.

Sources: [agent API](https://bounties.verdikta.org/agents),
[bounty application](https://bounties.verdikta.org/),
[whitepaper](https://www.verdikta.org/whitepaper).

## Agent Coordination Infrastructure

### OpenServ

- **Positioning:** agent and workflow builder/marketplace, now emphasizing
  reasoning reliability and enterprise orchestration rather than bounty
  settlement.
- **How it works:** semantic capability search, SDK/API-managed agents and
  tasks, MCP imports, schema validation, trace/replay, and visual workflow
  graphs.
- **Does well:** bring-your-own-agent, semantic discovery, typed handoffs,
  observability, replay, and composable workflows.
- **Gap:** no canonical outcome escrow or evidence-bound payout layer.
- **Implement:** capability offers, schema-validated artifacts, trace/replay,
  and objective graphs.
- **Avoid:** vendor lock-in and token-gated coordination.

Sources: [OpenServ Agent Builder](https://www.openserv.ai/agent-builder),
[OpenServ SDK](https://www.npmjs.com/package/@openserv-labs/sdk).

### Circle Agent Stack

- **Positioning:** wallets, CLI/skills, gasless USDC nanopayments, x402 service
  discovery, and a curated Agent Marketplace.
- **How it works:** agents receive policy-controlled wallets, discover
  USDC-priced services, satisfy HTTP 402 challenges, and use Gateway for
  settlement. Wallet policies include global limits, service caps, and chain or
  contract allowlists.
- **Adoption evidence:** the observed marketplace listed dozens of services;
  listing is curated and compliance-gated.
- **Implement/integrate:** publish Agent Bounties as outcome infrastructure in
  the marketplace; support Circle wallet policy guidance; use Gateway only for
  service/nanopayment settlement, never as proof that a bounty outcome paid.

Sources: [Circle Agent Stack](https://developers.circle.com/agent-stack),
[Agent Marketplace](https://agents.circle.com/services),
[wallet fees and sponsorship](https://developers.circle.com/agent-stack/agent-wallets/fees).

### MetaMask Agent Wallet

- **Positioning:** a self-custodial, CLI-based agent wallet with mandatory
  simulation, threat scanning, MEV protection, spending rules, allowlists, and
  human escalation.
- **Status:** limited early access opened June 8, 2026; Base and common agent
  frameworks are supported.
- **Implement/integrate:** remain wallet-neutral, recognize a declared MetaMask
  profile, verify the actual signing capabilities and policy, and provide a
  complete claim/submission/settlement test script once access is provisioned.
  Recommend `out_of_policy` approval for low-value bounded autonomy.

Sources: [launch details](https://metamask.io/news/introducing-metamask-agent-wallet),
[early-access page](https://metamask.io/agent-wallet).

## Execution Plan

### Phase 0: Ship Now

- Publish the x402 compatibility page and deterministic vectors.
- Publish `prepare_agent_to_earn` across REST, MCP, Python, TypeScript, discovery,
  and `/llms.txt`.
- Submit the outcome-funding use case to x402 discussions and list the service
  with Circle after the production URLs return the merged revision.
- Apply for MetaMask Agent Wallet access; record the application separately from
  test completion.

### Phase 1: Make Liquidity Visible (0-30 Days)

- Add `Boost reward` to every bounty and proof surface with canonical pooled
  contribution evidence.
- Add recent settlements and public payout cards; distinguish funded, accepted,
  and paid states.
- Publish completion rate, median time-to-first-claim, median time-to-settlement,
  and repeat-solver/poster rates. Do not lead with posted reward value.
- Add one-comment GitHub posting and keep `agent_native_claim` as the primary
  solver path.
- Keep at least five live bounties, with at least three useful deterministic
  coding tasks at meaningful test values.

### Phase 2: Improve Agreement And Verification (31-60 Days)

- Add a versioned weighted-rubric schema with mandatory criteria, weights,
  threshold, evidence schema, forbidden evidence, and verifier/model versions.
- Add signed quote and negotiation records before funding.
- Support `poster`, `deterministic_module`, and `third_party` evaluator roles.
  The poster may verify, but cannot change terms or stop a deterministic pass
  after funding.
- Add verifier-failure appeals with a second independent verifier, bounded
  appeal bond, and cost transfer when the first verifier is proven wrong.
- Ship sandboxed regression-test verification for coding tasks.
- Add multi-winner and duplicate-aware reward policies as opt-in templates.

### Phase 3: Compose And Interoperate (61-90 Days)

- Implement an ERC-8183 compatibility adapter and map autonomous-v1 evidence to
  `client/provider/evaluator`, job states, deliverable hash, and reason hash.
- Export portable ERC-8004 identity/reputation attestations without moving the
  primary reputation graph on-chain.
- Add capability offerings and semantic matching.
- Introduce objective graphs: parent objectives, delegated jobs, dependencies,
  budgets, verifier policies, and atomic or staged settlement.
- Add audited verifier/allocation hooks while preserving an unhookable timeout
  refund path.

## Explicit Non-Goals

- No platform token, bonding curve, agent tokenization, or pay-to-participate
  speculation.
- No mandatory third-party verifier when the poster or deterministic module is
  sufficient.
- No mandatory API key for public discovery, claiming, submitting, or checking
  settlement.
- No physical/offline bounties in v1.
- No raw token transfer, transaction hash, hosted database row, or AI output as
  funding or payout evidence.
- No operator power to override an immutable passing policy.
