ALTER TABLE autonomous_bounty_events
  ADD COLUMN IF NOT EXISTS block_time_verified BOOLEAN NOT NULL DEFAULT FALSE;

CREATE INDEX IF NOT EXISTS idx_autonomous_bounty_events_unverified_blocks
  ON autonomous_bounty_events (network, block_number)
  WHERE block_time_verified = FALSE;

CREATE INDEX IF NOT EXISTS idx_autonomous_bounty_events_solver_leaderboard
  ON autonomous_bounty_events (network, occurred_at, block_number, log_index)
  WHERE block_time_verified = TRUE AND kind = 'bounty_settled';
