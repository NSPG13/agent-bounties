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
