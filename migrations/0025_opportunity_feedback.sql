ALTER TABLE opportunity_comments
  ADD COLUMN IF NOT EXISTS feedback JSONB
  CHECK (
    feedback IS NULL
    OR (
      jsonb_typeof(feedback) = 'object'
      AND pg_column_size(feedback) <= 4096
      AND feedback - ARRAY[
        'stage',
        'discovery_source',
        'participation_reason',
        'friction',
        'recommendation',
        'evidence_reference',
        'wallet',
        'wallet_signature'
      ]::TEXT[] = '{}'::JSONB
    )
  );
