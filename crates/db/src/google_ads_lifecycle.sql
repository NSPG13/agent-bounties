WITH funded AS (
 SELECT * FROM google_ads_attributed_outcomes
 WHERE first_touch_campaign=$1 AND first_touch_rail='google-ads'
   AND outcome='external_bounty_funded' AND event_at >= $2 AND event_at < $3
), progress AS (
 SELECT f.bounty_contract,
   bool_or(e.kind='bounty_claimed') AS claimed,
   bool_or(e.kind='submission_added') AS submitted,
   bool_or(e.kind='bounty_settled') AS settled,
   EXISTS(SELECT 1 FROM google_ads_attributed_outcomes p WHERE p.network=f.network
     AND p.bounty_contract=f.bounty_contract AND p.outcome='verified_bounty_paid') AS paid
 FROM funded f LEFT JOIN autonomous_bounty_events e ON e.network=f.network
   AND lower(e.contract_address)=f.bounty_contract AND e.block_time_verified=TRUE
 GROUP BY f.network,f.bounty_contract
)
SELECT jsonb_build_object(
 'no_claim_observed',count(*) FILTER(WHERE NOT COALESCE(claimed,FALSE)),
 'claimed_no_submission_observed',count(*) FILTER(WHERE claimed AND NOT COALESCE(submitted,FALSE)),
 'submitted_no_verified_payment',count(*) FILTER(WHERE submitted AND NOT paid),
 'settled_without_matching_evidence',count(*) FILTER(WHERE settled AND NOT paid),
 'verified_paid',count(*) FILTER(WHERE paid)
) FROM progress;
