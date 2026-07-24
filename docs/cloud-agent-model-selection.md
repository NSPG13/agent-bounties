# Cloud Agent Model Selection

Agent Bounties chooses cloud models with the exact Objective Compiler prompt,
strict response schema, repair pass, and deterministic validators used by the
hosted API. Headline benchmarks do not authorize a production model change.

Run the manually dispatched `Cloud Agent Model Benchmark` workflow. It compares:

- GPT-5.5 at low effort;
- GPT-5.6 Sol at low effort;
- GPT-5.6 Terra at low and medium effort;
- GPT-5.6 Luna at low effort.

Each candidate processes the committed six-case corpus. The report records hard
validation failures, expected-term coverage, provider calls, input, cached,
output and reasoning tokens, measured latency, and estimated standard API cost.
Reasoning tokens are a subset of billed output tokens and must not be charged
twice.

A candidate is eligible only when every run passes and average expected-term
coverage is at least 75%. Among candidates within five percentage points of the
best eligible coverage, the lowest measured cost wins, with latency as the
tiebreaker. Keep Sol when cheaper tiers fail this rule. Expand the corpus before
using the result for verification, wallet, payment, or settlement authority;
the cloud model remains advisory-only.

The benchmark uses the repository `CLOUD_AGENT_API_KEY` secret. It never prints
the key or stores prompts outside the normal API request. Raw responses and the
scored report remain workflow artifacts for 30 days.

## Current Decision

Runs [29765084174](https://github.com/NSPG13/agent-bounties/actions/runs/29765084174)
and [29766060671](https://github.com/NSPG13/agent-bounties/actions/runs/29766060671)
produced three observations for each of six cases. GPT-5.6 Luna at low effort
passed all 18 hard validations, retained 96.3% expected-term coverage, cost
$0.176 total including three repair calls, and had 9.9-second median latency.
GPT-5.6 Sol at low effort passed all 18 with 100% coverage, cost $1.049, and had
42.2-second median latency. GPT-5.5 timed out twice.

Production therefore uses `gpt-5.6-luna` with `low` effort. Roll back to the
`gpt-5.6` Sol alias if future corpus runs show any hard-validation regression or
more than a five-percentage-point quality deficit.
