//! Exact source and time fences shared by email and push deadline reminders.

// `j` is the durable notification. An email status is not lifecycle evidence.
pub(crate) const SOURCE: &str = r#"j.submitted_at <= statement_timestamp()
 AND j.review_deadline IS NOT NULL
 AND ((j.notification_kind='submission' AND EXISTS (
   SELECT 1 FROM autonomous_bounty_events s WHERE s.network=j.network
   AND s.contract_address=j.bounty_contract AND s.bounty_id=j.bounty_id
   AND s.log_key=j.submission_log_key AND s.kind='submission_added'
   AND s.block_time_verified AND s.data->>'round'=j.round::TEXT
   AND s.occurred_at=j.submitted_at
   AND s.data->>'verification_expires_at'=floor(extract(epoch FROM j.review_deadline))::BIGINT::TEXT
   AND NOT EXISTS (SELECT 1 FROM autonomous_bounty_events later
     WHERE later.network=s.network AND later.contract_address=s.contract_address
     AND later.bounty_id=s.bounty_id AND later.block_time_verified
     AND (later.block_number,later.log_index)>(s.block_number,s.log_index)
     AND later.kind IN ('bounty_became_claimable','bounty_claimed','submission_added',
       'submission_rejected','bounty_settled','claim_expired','submission_expired','bounty_cancelled'))))
 OR (j.notification_kind='creator_open_submission' AND EXISTS (
   SELECT 1 FROM creator_open_indexes i
   CROSS JOIN LATERAL jsonb_each(i.projection->'entries') e
   WHERE i.network=j.network AND i.bounty_contract=j.bounty_contract
   AND i.available AND i.checked_at>statement_timestamp()-interval '120 seconds'
   AND i.checkpoint IS NOT NULL AND i.projection->>'protocol'='agent-bounties/creator-open-v1'
   AND i.projection->>'status'='open' AND e.value->>'status'='pending'
   AND i.projection->>'bounty_id'=j.bounty_id AND i.projection->>'creator'=j.verifier_wallet
   AND e.key::BIGINT=j.round
   AND 'creator-open:'||(e.value->>'submission_hash')||':'||(e.value->>'evidence_hash')||':'||(e.value->>'review_deadline')=j.submission_log_key
   AND to_timestamp((e.value->>'review_deadline')::BIGINT)=j.review_deadline
   AND to_timestamp((e.value->>'review_deadline')::BIGINT-(i.projection->>'review_window')::BIGINT)=j.submitted_at)))"#;

pub(crate) const STAGES: &str =
    "(VALUES ('remaining_24h',86400),('remaining_6h',21600),('remaining_1h',3600),('overdue',0))";

// A late retry cannot turn an old reminder into an extra urgent notification.
// Recheck this at send admission, after any lease or source read has waited.
pub(crate) const WINDOW: &str = r#"d.stage IN ('remaining_24h','remaining_6h','remaining_1h','overdue')
 AND statement_timestamp() >= j.review_deadline - make_interval(secs => CASE d.stage
   WHEN 'remaining_24h' THEN 86400 WHEN 'remaining_6h' THEN 21600
   WHEN 'remaining_1h' THEN 3600 ELSE 0 END)
 AND statement_timestamp() < CASE d.stage
   WHEN 'remaining_24h' THEN j.review_deadline-interval '6 hours'
   WHEN 'remaining_6h' THEN j.review_deadline-interval '1 hour'
   WHEN 'remaining_1h' THEN j.review_deadline
   ELSE j.review_deadline+interval '24 hours' END"#;
