-- Private consented measurement only. No payment authority or public identifiers.
ALTER TABLE distribution_acquisitions DROP CONSTRAINT IF EXISTS distribution_acquisition_rail_check;
ALTER TABLE distribution_acquisitions ADD CONSTRAINT distribution_acquisition_rail_check CHECK (first_touch_rail IN ('bankr','github','linear','vscode','cursor','cline','openclaw','claude-custom','chatgpt-dev','glama','mcp-so','mcpservers','glama-paid','mcp-so-paid','mcpmarket','mcpmarket-paid','google-ads','website'));
ALTER TABLE distribution_acquisition_assists DROP CONSTRAINT IF EXISTS distribution_assist_rail_check;
ALTER TABLE distribution_acquisition_assists ADD CONSTRAINT distribution_assist_rail_check CHECK (rail IN ('bankr','github','linear','vscode','cursor','cline','openclaw','claude-custom','chatgpt-dev','glama','mcp-so','mcpservers','glama-paid','mcp-so-paid','mcpmarket','mcpmarket-paid','google-ads','website'));
ALTER TABLE distribution_rail_usage_hourly DROP CONSTRAINT IF EXISTS distribution_rail_usage_rail_check;
ALTER TABLE distribution_rail_usage_hourly ADD CONSTRAINT distribution_rail_usage_rail_check CHECK (rail IN ('bankr','github','linear','vscode','cursor','cline','openclaw','claude-custom','chatgpt-dev','glama','mcp-so','mcpservers','glama-paid','mcp-so-paid','mcpmarket','mcpmarket-paid','google-ads','website'));

CREATE TABLE IF NOT EXISTS google_ads_clicks (
  id UUID PRIMARY KEY,
  acquisition_id UUID NOT NULL REFERENCES distribution_acquisitions(id),
  click_hash TEXT UNIQUE,
  identifier_kind TEXT NOT NULL CHECK (identifier_kind IN ('gclid','gbraid','wbraid')),
  ciphertext TEXT,
  campaign TEXT NOT NULL CHECK (length(campaign) BETWEEN 1 AND 64),
  consent_version TEXT NOT NULL CHECK (consent_version = 'google-ads-outcomes-v1'),
  observed_at TIMESTAMPTZ NOT NULL,
  expires_at TIMESTAMPTZ NOT NULL,
  revoked_at TIMESTAMPTZ,
  CHECK (expires_at <= observed_at + interval '90 days')
);
CREATE INDEX IF NOT EXISTS google_ads_clicks_acquisition ON google_ads_clicks(acquisition_id, observed_at);
CREATE TABLE IF NOT EXISTS google_ads_funnel (
  acquisition_id UUID NOT NULL REFERENCES distribution_acquisitions(id),
  operation_id UUID NOT NULL,
  stage TEXT NOT NULL CHECK (stage IN ('draft','login','wallet','wallet_review','funding_started')),
  observed_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (acquisition_id, operation_id, stage)
);
CREATE TABLE IF NOT EXISTS google_ads_outbox (
  id UUID PRIMARY KEY,
  click_id UUID NOT NULL REFERENCES google_ads_clicks(id),
  network TEXT NOT NULL,
  bounty_contract TEXT NOT NULL,
  outcome TEXT NOT NULL CHECK (outcome IN ('external_bounty_funded','verified_bounty_paid')),
  event_at TIMESTAMPTZ NOT NULL,
  status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending','leased','accepted','rejected','suppressed')),
  attempts INTEGER NOT NULL DEFAULT 0,
  next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  lease_id UUID,
  request_id TEXT,
  destination_customer TEXT,
  destination_action TEXT,
  processing_status TEXT,
  diagnostics_checked_at TIMESTAMPTZ,
  processing_errors INTEGER,
  processing_warnings INTEGER,
  processing_reasons JSONB,
  error_code TEXT,
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE(network,bounty_contract,outcome)
);
CREATE TABLE IF NOT EXISTS google_ads_campaign_costs (
  campaign TEXT NOT NULL,
  day DATE NOT NULL,
  currency TEXT NOT NULL CHECK (currency = 'MXN'),
  cost_micros BIGINT NOT NULL CHECK (cost_micros >= 0),
  clicks BIGINT NOT NULL CHECK (clicks >= 0),
  source TEXT NOT NULL CHECK (source IN ('google_ads_export','google_ads_api')),
  imported_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (campaign,day)
);

-- A read model of canonical mainnet events, never browser-reported success.
CREATE OR REPLACE VIEW google_ads_canonical_outcomes AS
WITH excluded AS (
 SELECT DISTINCT wallet_address FROM distribution_wallet_exclusions WHERE active = TRUE
), creations AS (
 SELECT DISTINCT network,bounty_id,lower(data->>'bounty_contract') AS bounty_contract,
 lower(data->>'creator') AS creator_wallet,lower(data->>'terms_hash') AS terms_hash,occurred_at AS created_at
 FROM autonomous_bounty_events
 WHERE kind='canonical_bounty_created' AND block_time_verified=TRUE AND network='base-mainnet'
 AND data->>'bounty_contract' IS NOT NULL AND data->>'terms_hash' IS NOT NULL
 AND data->>'creator' IS NOT NULL
 AND NOT EXISTS (SELECT 1 FROM excluded WHERE wallet_address=lower(data->>'creator'))
 AND NOT EXISTS (SELECT 1 FROM distribution_acquisition_handoffs h
   JOIN distribution_acquisitions a ON a.id=h.acquisition_id
   WHERE a.measurement_eligible=FALSE AND h.terms_hash=lower(data->>'terms_hash')
     AND h.creator_wallet=lower(data->>'creator'))
), economics AS (
 SELECT network,bounty_id,MAX((data->>'target_amount')::numeric) AS target
 FROM autonomous_bounty_events WHERE kind='canonical_bounty_economics_configured'
 AND block_time_verified=TRUE AND data->>'target_amount' ~ '^[0-9]+$'
 GROUP BY network,bounty_id
), contributions AS (
 SELECT c.*,f.occurred_at AS contributed_at,
 SUM((f.data->>'amount')::numeric) OVER (PARTITION BY c.network,c.bounty_contract ORDER BY f.block_number,f.log_index) AS total,
 e.target
 FROM creations c JOIN economics e USING(network,bounty_id)
 JOIN autonomous_bounty_events f ON f.network=c.network AND f.bounty_id=c.bounty_id
 AND lower(f.contract_address)=c.bounty_contract AND f.kind='funding_added' AND f.block_time_verified=TRUE
 WHERE e.target>0 AND f.data->>'amount' ~ '^[0-9]+$' AND f.data->>'contributor' IS NOT NULL
 AND NOT EXISTS (SELECT 1 FROM excluded WHERE wallet_address=lower(f.data->>'contributor'))
), funded AS (
 SELECT c.network,c.bounty_id,c.bounty_contract,c.creator_wallet,c.terms_hash,c.created_at,
 GREATEST(MIN(c.contributed_at),MIN(e.occurred_at)) AS funded_at
 FROM contributions c JOIN autonomous_bounty_events e ON e.network=c.network AND e.bounty_id=c.bounty_id
 AND lower(e.contract_address)=c.bounty_contract AND e.kind='bounty_became_claimable' AND e.block_time_verified=TRUE
 WHERE c.total>=c.target GROUP BY c.network,c.bounty_id,c.bounty_contract,c.creator_wallet,c.terms_hash,c.created_at
), verified AS (
 SELECT f.network,f.bounty_contract,MIN(s.occurred_at) AS paid_at
 FROM funded f JOIN autonomous_bounty_events s ON s.network=f.network AND s.bounty_id=f.bounty_id
 AND lower(s.contract_address)=f.bounty_contract AND s.kind='bounty_settled' AND s.block_time_verified=TRUE
 JOIN autonomous_submission_evidence e ON e.network=f.network AND e.bounty_id=f.bounty_id
 AND lower(e.bounty_contract)=f.bounty_contract AND e.round::text=s.data->>'round'
 AND lower(e.evidence_hash)=lower(s.data->>'evidence_hash')
 AND lower(e.solver_wallet)=lower(s.data->>'solver')
 WHERE s.data->>'solver' IS NOT NULL AND lower(s.data->>'solver')<>f.creator_wallet
 AND NOT EXISTS (SELECT 1 FROM excluded WHERE wallet_address=lower(s.data->>'solver'))
 GROUP BY f.network,f.bounty_contract
)
SELECT f.*,v.paid_at FROM funded f LEFT JOIN verified v USING(network,bounty_contract);

CREATE OR REPLACE VIEW google_ads_attributed_outcomes AS
SELECT DISTINCT ON (o.network,o.bounty_contract,e.outcome)
 o.network,o.bounty_contract,o.creator_wallet,a.id AS acquisition_id,a.first_touch_rail,
 c.id AS click_id,c.campaign,c.observed_at,c.expires_at,e.outcome,e.event_at,
 (SELECT c0.campaign FROM google_ads_clicks c0 WHERE c0.acquisition_id=a.id ORDER BY c0.observed_at,c0.id LIMIT 1) AS first_touch_campaign
FROM google_ads_canonical_outcomes o
JOIN distribution_acquisition_handoffs h ON h.terms_hash=o.terms_hash AND h.creator_wallet=o.creator_wallet
JOIN distribution_acquisitions a ON a.id=h.acquisition_id AND a.measurement_eligible=TRUE
JOIN google_ads_clicks c ON c.acquisition_id=a.id
CROSS JOIN LATERAL (VALUES ('external_bounty_funded',o.funded_at),('verified_bounty_paid',o.paid_at)) e(outcome,event_at)
WHERE h.terms_bound_at<=o.created_at AND h.prepared_at<=o.created_at
AND c.observed_at<=o.created_at AND c.revoked_at IS NULL
AND e.event_at IS NOT NULL AND e.event_at>=c.observed_at
ORDER BY o.network,o.bounty_contract,e.outcome,c.observed_at DESC,c.id;

CREATE OR REPLACE VIEW google_ads_eligible_outcomes AS
SELECT * FROM google_ads_attributed_outcomes WHERE event_at<=expires_at;
