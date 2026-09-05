# Paid-Rail Mainnet Canary Runbook

Run this once for each paid rail (`glama`, `mcp-so`, and `mcpservers`) after the
attributed route and migration are deployed. The browser and wallet steps are
deliberately human-controlled. No script, MCP tool, vendor, or integration may
receive a private key, seed phrase, wallet signature, or payout authority.

## Preconditions

1. Deploy migration `0032_distribution_attribution.sql` and the same generated
   `DISTRIBUTION_ATTRIBUTION_SIGNING_SECRET` to the API and MCP services.
2. Classify every creator, funder, solver, and verifier wallet used by the
   exercise as `synthetic_canary` through the operator-only wallet-exclusion
   endpoint **before** funding. Do not put the operator token on the command
   line or in an evidence artifact.
3. Run the side-effect-free rail probe three times and retain its output:

   ```bash
   python scripts/check-distribution-rail-mcp.py \
     --endpoint https://mcp.agentbounties.app \
     --repetitions 3 --canary-kind dry-run-v1
   ```

4. Confirm the operator report returns all eight required exclusion classes and
   at least 95% attribution coverage for any already-eligible external funded
   bounties. Coverage of zero eligible outcomes is `0`, never an automatic pass.

## One canonical canary per rail

1. Connect to `https://mcp.agentbounties.app/r/<rail>/mcp` with
   `x-agent-bounties-canary: mainnet-v1` and retain the exact signed acquisition
   identifier returned by the server in a private evidence store. Confirm the
   response echoes the canary classification and marks the acquisition
   ineligible for marketing measurement.
2. Call `prepare_bounty_post` with the active catalog-pinned verifier task, a
   2.00 USDC solver reward, the required verifier reward, complete binary
   acceptance criteria, and `crowdfund: false`.
3. Open only the returned first-party review URL. Confirm that reaching the
   wallet-opening boundary is acknowledged durably for the same acquisition
   and handoff. Review the exact terms, verifier, total amount, network, token,
   and predicted contract before approving any wallet request.
4. Sign on the first-party surface. A signature, wallet response, or transaction
   hash is not success. Wait for confirmed canonical
   `CanonicalBountyCreated`, `FundingAdded`, and `BountyBecameClaimable` events
   for the exact terms and contract.
5. Complete the task through an excluded canary solver, publish hash-matched
   evidence, run the precommitted verifier, and wait for confirmed canonical
   `BountySettled`. Do not manually relabel failed or unrelated evidence.
6. Confirm the private attribution join contains the same first-touch rail,
   handoff, terms hash, creator, canonical bounty, verifier evidence, and
   settlement. Also confirm the excluded canary changes none of the external
   funded-poster, CAC, or settled-GMV totals.

## Evidence required in the reviewed control file

For each rail, record three dry-run references and one mainnet reference that
links to the private acquisition/handoff join plus public canonical creation,
funding, verifier-evidence, and settlement proof. Set
`excluded_from_external_metrics: true` only after comparing the operator report
before and after the exercise. Never store the raw signed acquisition token,
operator token, wallet credential, or non-public prompt in the repository.

The activation gate remains blocked if any reference is missing, if the route
changes first touch, if a canary wallet was not excluded before funding, or if
settlement lacks valid hash-matched verifier evidence.
