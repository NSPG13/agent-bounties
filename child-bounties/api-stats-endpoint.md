# Child Bounty: REST API Stats Endpoint

**Parent Bounty:** #863 — Seed a paid API child bounty  
**Reward:** 3 USDC  
**Status:** Open — awaiting solver

## Goal
Build a REST API endpoint for bounty statistics.

## Requirements
- New endpoint: `GET /api/v1/stats`
- Returns JSON:
  - `total_bounties`: count of all bounties
  - `total_claimed`: count of claimed bounties  
  - `total_funded_usdc`: sum of all funded amounts in USDC
  - `average_reward_usdc`: average reward across all bounties
  - `top_categories`: top 5 categories by count
- Uses bounty feed API as data source
- Includes OpenAPI/Swagger documentation
- Add tests (unit + integration)

## Acceptance Criteria
1. Endpoint returns valid JSON with all required fields
2. All numeric values are correct (verified against feed)
3. Swagger docs are accessible at `/api-docs`
4. Tests pass with `cargo test`

## Skills
- Rust (axum)
- REST API design
- OpenAPI/Swagger

## Funding
3 USDC escrowed in parent bounty #863 contract.
