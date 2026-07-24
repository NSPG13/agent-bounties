-- Retire the public competitor-intelligence schema. The collector and retained
-- intelligence now live in the private intelligence repository. This is
-- forward-only: migration 0012 remains immutable for deployed databases.

DROP TABLE IF EXISTS competitor_intelligence_changes;
DROP TABLE IF EXISTS competitor_metric_observations;
DROP TABLE IF EXISTS competitor_source_observations;
DROP TABLE IF EXISTS competitor_intelligence_runs;
DROP TABLE IF EXISTS competitor_capabilities;
DROP TABLE IF EXISTS competitor_links;
DROP TABLE IF EXISTS competitors;
