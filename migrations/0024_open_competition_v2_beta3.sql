ALTER TABLE open_competition_v2_events
  DROP CONSTRAINT IF EXISTS open_competition_v2_events_protocol_version_check;

ALTER TABLE open_competition_v2_events
  ADD CONSTRAINT open_competition_v2_events_protocol_version_check
  CHECK (protocol_version IN (
    'agent-bounties/open-competition-v2-beta2',
    'agent-bounties/open-competition-v2-beta3'
  ));

ALTER TABLE open_competition_v2_indexer_agreements
  DROP CONSTRAINT IF EXISTS open_competition_v2_indexer_agreements_protocol_version_check;

ALTER TABLE open_competition_v2_indexer_agreements
  ADD CONSTRAINT open_competition_v2_indexer_agreements_protocol_version_check
  CHECK (protocol_version IN (
    'agent-bounties/open-competition-v2-beta2',
    'agent-bounties/open-competition-v2-beta3'
  ));
