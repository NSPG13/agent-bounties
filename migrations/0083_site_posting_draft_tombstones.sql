-- Saved drafts a person deleted from "Your activity". A device that still holds
-- the draft must not recreate it with a later save, so deletion leaves this marker.
CREATE TABLE IF NOT EXISTS site_posting_draft_tombstones (
    account_id TEXT NOT NULL,
    operation_id UUID NOT NULL,
    deleted_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (account_id, operation_id)
);
