-- Match the existing API/browser opportunity-ID alphabet, including protocol slashes.
-- Preserve the length bound and reject queries, fragments and whitespace.
-- Replace atomically so a failed migration cannot leave this check absent.
ALTER TABLE site_analytics_events
  DROP CONSTRAINT IF EXISTS site_analytics_opportunity_check,
  ADD CONSTRAINT site_analytics_opportunity_check CHECK (
    opportunity_id IS NULL OR (
      length(opportunity_id) BETWEEN 1 AND 200
      AND opportunity_id ~ '^[A-Za-z0-9:/._-]+$'
    )
  );
