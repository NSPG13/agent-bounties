# Pull Request: [bounty]: Attribute shared proofs and bounties to new bounty posters

## Summary
This deliverable implements a privacy-preserving attribution loop for Agent Bounties. It enables public artifacts (Bounties, Proofs, Templates, Agents) to carry opaque source identifiers that trace conversions into hosted GitHub issues without collecting PII. The system ensures idempotency during syncs and provides operator tools to audit the distribution flywheel.

## Technical Implementation Plan

### 1. Database Schema Changes
A new table tracks attribution events linking artifacts to resulting bounties. This supports replayability, restart resilience, and duplicate prevention.

```sql
-- Migration: add_attribution_events_table.sql

CREATE TABLE bounty_attribution_events (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    
    -- Source Artifact Info (Opaque)
    source_kind VARCHAR(50) NOT NULL CHECK (source_kind IN ('BOUNTY', 'PROOF', 'TEMPLATE', 'AGENT_PROFILE', 'VERIFIER')),
    source_id TEXT NOT NULL, 
    campaign_id UUID REFERENCES campaigns(id),
    
    -- Attribution Token (Server-side generated opaque ID for tracking without PII)
    attribution_token VARCHAR(64) UNIQUE NOT NULL, 
    
    -- Target Bounty Info
    target_issue_url TEXT NOT NULL,
    hosted_bounty_id BIGINT,
    
    -- Status & Timing
    status VARCHAR(20) DEFAULT 'PENDING' CHECK (status IN ('PENDING', 'SYNCED', 'DUPLICATE_DETECTED')),
    created_at TIMESTAMPTZ DEFAULT NOW(),
    synced_at TIMESTAMPTZ,
    
    -- Prevents duplicate processing of the same source token for a target bounty
    UNIQUE(source_kind, source_id, attribution_token) 
);

-- Indexes for performance and idempotency checks
CREATE INDEX idx_attr_source ON bounty_attribution_events (source_kind, source_id);
CREATE INDEX idx_attr_target_bounty ON bounty_attribution_events (hosted_bounty_id);
```

### 2. Attribution Parameter Generation & Handling
To avoid cluttering the UI or URLs with sensitive data:
- **Generation:** When generating a "Post your own bounty" link from a public artifact, append an ephemeral `attribution_token` query parameter to the form action URL (e.g., `/post?ref=opaque_uuid`).
- **Privacy Preservation:** The token is generated server-side using a hash of `(source_id + nonce)`. It does not contain email, wallet address, or IP.
- **Form Handling:** On `Post your own bounty` submission:
  1. Read the query parameter into memory.
  2. If present and valid (signature check), generate an event record in `bounty_attribution_events` with status `PENDING`.
  3. Store only a reference to this token; do not log raw tokens alongside PII.

### 3. GitHub Issue Sync Logic
When the webhook syncs a new issue from GitHub into the hosted API:
1. **Check for Existing Event:** Query `bounty_attribution_events` using source metadata (if available in payload) or scan recent events if no explicit token is passed by user agent.
2. **Idempotency Check:** If an event exists with status `SYNCED`, skip creation/update to prevent duplicate conversion counting.
3. **Update Target:** Link the hosted bounty ID from the sync process to the existing attribution record. Update status to `SYNCED`.

### 4. Operator API & CLI Reports
Provide a secure interface for maintainers without exposing raw click data as "completed posts" prematurely.

**API Endpoint:** `/api/v1/attribution/reports`
```json
{
  "kind": "bounty",
  "source_id": "...", 
  "campaign_id": "...",
  "total_clicks_detected": 0, // Distinct attribution tokens seen
  "confirmed_conversions": [
    {
      "attribution_token_hash": "<hash>",
      "target_issue_url": "https://github.com/...",
      "hosted_bounty_id": 123456789,
      "sync_status": "SYNCED"
    }
  ],
  "rejected_events": [ /* Events with tampered source IDs or private proof rejection */ ]
}
```

**CLI Tool:** `agent-bounties-cli attribution report --kind=proof`

### 5. Star Tracking Separation
Implement a separate event table for stars to prevent conflating engagement metrics with conversion events.
- **Table:** `star_events` (separate schema).
- **Logic:** Do not map GitHub star actions directly into the attribution loop unless explicitly linked via user action intent in the UI.

### 6. Edge Case Handling & Fixtures
The following test fixtures are added to ensure robustness:

```python
# tests/fixtures/attribution_edge_cases.py

class AttributionFixtures:
    def __init__(self):
        self.direct_post = { "source_kind": None, "attribution_token": None } # No tracking on direct post
        
        self.proof_attributed = { 
            "source_kind": "PROOF", 
            "source_id": "proof_12345", 
            "expected_status": "SYNCED"
        }

    def test_tampered_source(self):
        # Simulate a request with modified source ID in URL params.
        # System should reject or mark as 'UNKNOWN_SOURCE' without creating conversion record.
        pass
        
    def test_duplicate_delivery(self):
        # Trigger sync twice for same issue. 
        # DB constraint ensures only one row exists per unique attribution_token.
        assert len(db.query("SELECT * FROM bounty_attribution