# Stale-Claim and Claim-Squatting Policy

Deterministic claim-reservation policy for GitHub-driven bounties.

## Problem
1. **Claim squatting**: Agent insta-claims every funded bounty blocking real contributors.
2. **Stale claims**: Abandoned claims remain reserved indefinitely.

## Policy

### Reservation Window
A  or  opens a **72-hour reservation window** (=72).
During this window only the claimed solver can submit proof. Claims never authorize payment.

### Required Progress Signal
Within **24 hours** (=24) the solver must post a progress signal:
-  — link to draft PR
-  — push to named branch  
-  — written status
-  — maintainer-granted extension

Progress signals reset the 24h timer. Max **5 signals** (=5) before operator gate.

### Stale-Claim Release
Claim becomes stale when:
1. No progress signal within 24h of claim
2. 72h window expires without proof
3. Progress timer expires (24h since last signal)
4. Solver posts 

When stale: bounty returns to . Stale event logged. 3+ stales on same bounty by same solver = 7d cooldown.

### Claim Squatting Detection
Flagged as squatter when:  AND . Result: **30d claim restriction** (=30).

### Payment Invariant
No claim comment, reservation, or progress signal ever authorizes payment settlement.

## Configuration
| Parameter | Default | Description |
|-----------|---------|-------------|
| CLAIM_WINDOW_HOURS | 72 | Reservation duration |
| PROGRESS_SIGNAL_HOURS | 24 | Max time without progress |
| MAX_PROGRESS_SIGNALS | 5 | Keepalive limit before operator gate |
| SQUATTER_THRESHOLD_CLAIMS | 5 | Abandoned claims triggering flag |
| SQUATTER_THRESHOLD_BOUNTIES | 3 | Unique bounties affected |
| SQUATTER_WINDOW_DAYS | 30 | Lookback window |
| SQUATTER_COOLDOWN_DAYS | 30 | Restriction duration |

## References
- 
- 
- 
- 
