-- Shared historical payout eligibility and component policy. Do not restrict to current factories.
              SELECT 'autonomous-v1'::text AS protocol, NULL::text AS factory_contract, network, contract_address, bounty_id, tx_hash, block_number, log_index, kind, occurred_at, kind = 'bounty_settled' AS settled,
                     CASE WHEN kind = 'bounty_settled'
                       THEN COALESCE((data->>'solver_reward')::numeric, 0)
                       ELSE 0 END AS solver_amount,
                     COALESCE((data->>'verifier_reward')::numeric, 0) AS verifier_amount,
                     0::numeric AS keeper_amount,
                     CASE WHEN kind = 'bounty_settled'
                       THEN COALESCE((data->>'timeout_bond_bonus')::numeric, 0)
                       ELSE 0 END AS bonus_amount
              FROM autonomous_bounty_events
              WHERE network = $NETWORK
                AND block_time_verified = TRUE
                AND occurred_at >= $START
                AND occurred_at < $END
                AND NOT lower(contract_address) = ANY($EXCLUDED)
                AND kind IN ('bounty_settled', 'submission_rejected')
              UNION ALL
              SELECT 'open-competition-v1'::text AS protocol, factory_contract, network, contract_address, bounty_id, tx_hash, block_number, log_index, kind, occurred_at, kind = 'bounty_settled' AS settled,
                     CASE WHEN kind = 'bounty_settled'
                       THEN COALESCE((data->>'solver_reward')::numeric, 0)
                       ELSE 0 END AS solver_amount,
                     CASE WHEN kind = 'bounty_settled'
                       THEN COALESCE((data->>'verifier_reward')::numeric, 0)
                       ELSE COALESCE((data->>'bond_paid_to_verifier')::numeric, 0)
                     END AS verifier_amount,
                     0::numeric AS keeper_amount,
                     CASE WHEN kind = 'bounty_settled'
                       THEN COALESCE((data->>'timeout_bond_bonus')::numeric, 0)
                       ELSE 0 END AS bonus_amount
              FROM open_competition_events
              WHERE network = $NETWORK
                AND block_time_verified = TRUE
                AND occurred_at >= $START
                AND occurred_at < $END
                AND NOT lower(contract_address) = ANY($EXCLUDED)
                AND kind IN ('bounty_settled', 'competition_submission_rejected')
              UNION ALL
              SELECT 'open-competition-v2'::text AS protocol, factory_contract, network, contract_address, bounty_id, tx_hash, block_number, log_index, kind, occurred_at, TRUE AS settled,
                     COALESCE((data->>'solver_reward')::numeric, 0) AS solver_amount,
                     0::numeric AS verifier_amount,
                     COALESCE((data->>'keeper_reward')::numeric, 0) AS keeper_amount,
                     0::numeric AS bonus_amount
              FROM open_competition_v2_events
              WHERE network = $NETWORK
                AND occurred_at >= $START
                AND occurred_at < $END
                AND NOT lower(contract_address) = ANY($EXCLUDED)
                AND kind = 'competition_settled'
