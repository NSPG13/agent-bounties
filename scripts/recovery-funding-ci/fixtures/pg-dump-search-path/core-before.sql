--
-- PostgreSQL database dump
--

\restrict R5zvuFM61Q68m4lTkrsgehEj1nFbMP2U7zLxFpZgZckAte9RSVz1xEyLufVD57f

-- Dumped from database version 16.15 (Debian 16.15-1.pgdg13+2)
-- Dumped by pg_dump version 16.15 (Debian 16.15-1.pgdg13+2)

SET statement_timeout = 0;
SET lock_timeout = 0;
SET idle_in_transaction_session_timeout = 0;
SET client_encoding = 'UTF8';
SET standard_conforming_strings = on;
SELECT pg_catalog.set_config('search_path', '', false);
SET check_function_bodies = false;
SET xmloption = content;
SET client_min_messages = warning;
SET row_security = off;

--
-- Data for Name: agents; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: audience_members; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: audience_interactions; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: autonomous_bounty_events; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: autonomous_bounty_terms; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--

INSERT INTO recovery_funding_core.autonomous_bounty_terms (terms_hash, policy_hash, acceptance_criteria_hash, benchmark_hash, evidence_schema_hash, creator_wallet, document, created_at) VALUES ('0xd5df1c619082f54d98c1935e456db983e80bc4099518b1110d8c710eb5608e61', '0x5e1daa0699b8d63b9ed5c8d3a75db5aac8133dd5b777aa7bd3a4cca2a5dd199d', '0xb0bbafec77028c8fd0d0b2dacef5e9f28738b5ffe3edbf9f44180b61560b0fad', '0xa64c9c07c3afd7648b7c9396fd5a42aec8a9505824fec8b05868c3cc61ef50c9', '0x404e0caede36af0bb066c58ec16e3cdb779213eff948638dcb484422a42f779c', '0xf0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0', '{"goal": "Deliver the test deliverable.", "title": "Test deliverable", "benchmark": {"engine": "customer_review_v1", "reviewer": "paying_customer", "acceptance": "all_published_criteria", "delivery_deadline": 1794400998}, "source_url": null, "contract_terms": {"network": "base-mainnet", "claim_bond": {"amount": 5000000, "currency": "usdc"}, "platform_fee": {"amount": 7500000, "currency": "usdc"}, "solver_reward": {"amount": 100000000, "currency": "usdc"}, "creation_nonce": "0x128488e5c1894768b1f1dda1a7578ff57edb83c2cf9b4a6fbbce150457b959e2", "creator_wallet": "0xf0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0", "initial_funding": {"amount": 112500000, "currency": "usdc"}, "verifier_reward": {"amount": 5000000, "currency": "usdc"}, "funding_deadline": 1794400998, "platform_fee_bps": 750, "protocol_version": "agent-bounties/autonomous-v2", "settlement_token": "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913", "claim_window_seconds": 604800, "platform_fee_recipient": "0x884834e884d6e93462655a2820140ad03e6747bc", "verification_window_seconds": 259200}, "schema_version": "agent-bounties/terms-v1", "evidence_schema": {"type": "object", "required": ["artifact_url", "artifact_sha256"], "properties": {"artifact_url": {"type": "string", "pattern": "^https://"}, "artifact_sha256": {"type": "string", "pattern": "^sha256:[0-9a-f]{64}$"}}}, "discovery_source": "managed_card_quote", "acceptance_criteria": ["Test acceptance criteria"], "verification_policy": {"engine": "customer_review_v1", "mechanism": "signed_quorum", "threshold": 1, "verifiers": ["0xa0ee7a142d267c1f36714e4a8f75612f20a79720"], "public_disclosure": "Customer review."}}', '2026-11-10 12:43:18.881137+00');


--
-- Data for Name: autonomous_canonical_checkpoints; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: autonomous_submission_evidence; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: help_requests; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: bounties; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: base_escrow_events; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: base_indexer_heartbeats; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: base_log_cursors; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: claim_candidates; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: bond_sponsorships; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: bounty_image_assets; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: capabilities; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: chatgpt_action_intents; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: claims; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: contributor_contacts; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: creator_open_external_creations; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: sponsored_actions; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: creator_open_requests; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: creator_open_indexes; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: site_auth_accounts; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: creator_push_subscriptions; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: verifier_review_notifications; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: creator_push_deliveries; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: discoverability_snapshots; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: discovery_responses; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: discovery_route_usage_hourly; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: distribution_acquisitions; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: distribution_acquisition_assists; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: distribution_acquisition_handoffs; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: distribution_competition_bindings; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: distribution_handoff_failures; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: distribution_rail_usage_hourly; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: distribution_wallet_exclusions; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: escrows; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: eval_runs; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: external_interface_usage_hourly; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: funding_contributions; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: funding_intents; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: gas_sponsorship_daily_budget; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: github_origin_installations; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: github_origin_drafts; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: interface_usage_hourly; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: ledger_entries; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: legal_acceptances; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: managed_service_quotes; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--

INSERT INTO recovery_funding_core.managed_service_quotes (quote_id, buyer_principal, quote_digest, quote, issued_at, expires_at) VALUES ('7aa46aab-a1e5-45ce-b291-a875a84f5db1', 'fixture-buyer', '1809b1ddfc92c95875807a99183d40c8f1b3de9356224e1888b394fbcb452c46', '{"input": {"terms": {"remedies": "Test remedies", "deliverable": "Test deliverable", "terms_version": "test-1", "delivery_deadline": 1794400998, "acceptance_criteria": "Test acceptance criteria"}, "bounty": {"factory": "0xc33e2ae33bb9580837ea59df18e57fa1039ae58a", "network": "base-mainnet", "terms_hash": "0xd5df1c619082f54d98c1935e456db983e80bc4099518b1110d8c710eb5608e61", "creation_nonce": "0x128488e5c1894768b1f1dda1a7578ff57edb83c2cf9b4a6fbbce150457b959e2", "funding_wallet": "0xf0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0", "funding_deadline": 1794400998, "predicted_bounty": "0x52eb1b0a79118eb34f54a0fc3cc803c83c315706", "verification_setup": "customer_review_v1", "create_call_keccak256": "0xc39132b6e0ed354f819650a95432cc34c644d5d94b6655324f988eb0db8e72c6"}, "currency": "mxn", "livemode": true, "quote_id": "7aa46aab-a1e5-45ce-b291-a875a84f5db1", "issued_at": 1794314597, "expires_at": 1794321798, "price_basis": {"reference": "synthetic-test-price", "valid_until": 1794321798, "minor_units_per_usdc_numerator": 2000, "minor_units_per_usdc_denominator": 1}, "buyer_principal": "fixture-buyer", "additional_costs": [], "stripe_account_id": "acct_fixture_only", "solver_reward_micro_usdc": 100000000, "verifier_reserve_micro_usdc": 5000000}, "lines": [{"code": "solver_reward", "label": "Solver reward", "amount_minor": 200000, "amount_micro_usdc": 100000000}, {"code": "verification", "label": "Verification reserve", "amount_minor": 10000, "amount_micro_usdc": 5000000}, {"code": "platform_fee", "label": "Platform fee (7.5% of solver reward)", "amount_minor": 15000, "amount_micro_usdc": 7500000}], "schema": "agent-bounties/managed-service-quote-v2", "total_minor": 225000, "platform_fee_bps": 750, "platform_fee_micro_usdc": 7500000, "bounty_target_micro_usdc": 112500000}', 1794314597, 1794321798);


--
-- Data for Name: managed_service_orders; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--

INSERT INTO recovery_funding_core.managed_service_orders (order_id, quote_id, buyer_principal, quote_digest, accepted_at, idempotency_key) VALUES ('ff8747cb-83d9-41d5-bc4f-2c31088e43a5', '7aa46aab-a1e5-45ce-b291-a875a84f5db1', 'fixture-buyer', '1809b1ddfc92c95875807a99183d40c8f1b3de9356224e1888b394fbcb452c46', 1794314598, 'ab3c3365-3c10-4a1b-954f-60fc7051fdaa');


--
-- Data for Name: managed_stripe_paid_receipts; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--

INSERT INTO recovery_funding_core.managed_stripe_paid_receipts (order_id, stripe_account_id, livemode, payment_intent_id, session_id, quote_digest, amount_minor, currency) VALUES ('ff8747cb-83d9-41d5-bc4f-2c31088e43a5', 'acct_fixture_only', true, 'pi_ff8747cb83d941d5bc4f2c31088e43a5', 'cs_live_ff8747cb83d941d5bc4f2c31088e43a5', '1809b1ddfc92c95875807a99183d40c8f1b3de9356224e1888b394fbcb452c46', 225000, 'mxn');


--
-- Data for Name: managed_bounty_funding_operations; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--

INSERT INTO recovery_funding_core.managed_bounty_funding_operations (order_id, operation_id, plan, plan_digest, reserved_at, lease_token, lease_until, tx_hash, outcome, funded_tx_hash, funded_block, last_review_attempt_at, last_recovery_attempt_at) VALUES ('ff8747cb-83d9-41d5-bc4f-2c31088e43a5', '407dfd59-d810-4e07-9201-2fcd2448e803', '{"factory": "0xc33e2ae33bb9580837ea59df18e57fa1039ae58a", "network": "base-mainnet", "calldata": "0x261fef320000000000000000000000000000000000000000000000000000000005f5e10000000000000000000000000000000000000000000000000000000000004c4b40d5df1c619082f54d98c1935e456db983e80bc4099518b1110d8c710eb5608e615e1daa0699b8d63b9ed5c8d3a75db5aac8133dd5b777aa7bd3a4cca2a5dd199db0bbafec77028c8fd0d0b2dacef5e9f28738b5ffe3edbf9f44180b61560b0fada64c9c07c3afd7648b7c9396fd5a42aec8a9505824fec8b05868c3cc61ef50c9404e0caede36af0bb066c58ec16e3cdb779213eff948638dcb484422a42f779c000000000000000000000000000000000000000000000000000000006af462e60000000000000000000000000000000000000000000000000000000000093a80000000000000000000000000000000000000000000000000000000000003f4800000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000240128488e5c1894768b1f1dda1a7578ff57edb83c2cf9b4a6fbbce150457b959e20000000000000000000000000000000000000000000000000000000000000001000000000000000000000000a0ee7a142d267c1f36714e4a8f75612f20a79720", "order_id": "ff8747cb-83d9-41d5-bc4f-2c31088e43a5", "creation_nonce": "0x128488e5c1894768b1f1dda1a7578ff57edb83c2cf9b4a6fbbce150457b959e2", "funding_wallet": "0xf0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0", "funding_deadline": 1794400998, "predicted_bounty": "0x52eb1b0a79118eb34f54a0fc3cc803c83c315706", "target_micro_usdc": 112500000, "create_call_keccak256": "0xc39132b6e0ed354f819650a95432cc34c644d5d94b6655324f988eb0db8e72c6"}', '152e89887345deabc3f17202f487802d58f5738a05b05e181eeb9afdbf24dfea', '2026-11-10 12:43:18.908568+00', NULL, NULL, '0xbf8e1e3a79ca0c34553f2fdc8f00371ea936d39893e5652a7df2aa3ded70fd62', 'broadcast', NULL, NULL, NULL, NULL);


--
-- Data for Name: managed_customer_reviews; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: managed_deposit_payouts; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: managed_deposit_refund_operations; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: managed_deposit_refund_events; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: managed_deposit_refund_execution; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: managed_recovered_refund_operations; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: managed_recovered_refund_events; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: managed_recovered_refund_evidence; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: managed_recovered_refund_execution; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: managed_refund_operations; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: managed_refund_events; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: managed_refund_execution; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: managed_refund_observations; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: managed_service_actions; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--

INSERT INTO recovery_funding_core.managed_service_actions (order_id, quote_digest, action, state) VALUES ('ff8747cb-83d9-41d5-bc4f-2c31088e43a5', '1809b1ddfc92c95875807a99183d40c8f1b3de9356224e1888b394fbcb452c46', 'prepare_checkout', 'completed');


--
-- Data for Name: managed_stripe_bank_actions; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--

INSERT INTO recovery_funding_core.managed_stripe_bank_actions (order_id, action, state, funding_operation_id) VALUES ('ff8747cb-83d9-41d5-bc4f-2c31088e43a5', 'reconcile_bank_cash', 'held', '407dfd59-d810-4e07-9201-2fcd2448e803');


--
-- Data for Name: managed_stripe_checkout_operations; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--

INSERT INTO recovery_funding_core.managed_stripe_checkout_operations (order_id, operation_id, stripe_account_id, livemode, plan, plan_digest, session_id, payment_intent_id, checkout_url) VALUES ('ff8747cb-83d9-41d5-bc4f-2c31088e43a5', 'e2484fbc-f45a-43c3-add3-112dba065ca8', 'acct_fixture_only', true, '{"body": {"mode": "payment", "ui_mode": "hosted_page", "metadata": {"purpose": "managed_service_order", "managed_order_id": "ff8747cb-83d9-41d5-bc4f-2c31088e43a5", "merchant_account_id": "acct_fixture_only", "managed_operation_id": "e2484fbc-f45a-43c3-add3-112dba065ca8", "managed_quote_digest": "1809b1ddfc92c95875807a99183d40c8f1b3de9356224e1888b394fbcb452c46"}, "cancel_url": "https://agentizable.ai/account.html?managed_order=ff8747cb-83d9-41d5-bc4f-2c31088e43a5", "expires_at": 1794318198, "line_items": [{"quantity": 1, "price_data": {"currency": "mxn", "unit_amount": 200000, "product_data": {"name": "Solver reward"}}}, {"quantity": 1, "price_data": {"currency": "mxn", "unit_amount": 10000, "product_data": {"name": "Verification reserve"}}}, {"quantity": 1, "price_data": {"currency": "mxn", "unit_amount": 15000, "product_data": {"name": "Platform fee (7.5% of solver reward)"}}}], "success_url": "https://agentizable.ai/account.html?managed_order=ff8747cb-83d9-41d5-bc4f-2c31088e43a5&checkout_session_id={CHECKOUT_SESSION_ID}", "adaptive_pricing": {"enabled": false}, "client_reference_id": "ff8747cb-83d9-41d5-bc4f-2c31088e43a5", "payment_intent_data": {"metadata": {"purpose": "managed_service_order", "managed_order_id": "ff8747cb-83d9-41d5-bc4f-2c31088e43a5", "merchant_account_id": "acct_fixture_only", "managed_operation_id": "e2484fbc-f45a-43c3-add3-112dba065ca8", "managed_quote_digest": "1809b1ddfc92c95875807a99183d40c8f1b3de9356224e1888b394fbcb452c46"}}, "integration_identifier": "agentbounties-managed-subgkmpn"}, "policy": {"currency": "mxn", "livemode": true, "maximum_minor": 1000000, "minimum_minor": 1000, "adaptive_pricing": false, "stripe_account_id": "acct_fixture_only", "payment_method_configuration": null}, "order_id": "ff8747cb-83d9-41d5-bc4f-2c31088e43a5", "created_at": 1794314598, "expires_at": 1794318198, "api_version": "2026-09-30.endive", "operation_id": "e2484fbc-f45a-43c3-add3-112dba065ca8", "quote_digest": "1809b1ddfc92c95875807a99183d40c8f1b3de9356224e1888b394fbcb452c46", "retry_before": 1794397398, "idempotency_key": "managed-order:ff8747cb-83d9-41d5-bc4f-2c31088e43a5:checkout:e2484fbc-f45a-43c3-add3-112dba065ca8"}', '29c6c8b3886beec89938d3be55ad75591c4413eb2864f86a3ebb376e454cfd43', 'cs_live_ff8747cb83d941d5bc4f2c31088e43a5', 'pi_ff8747cb83d941d5bc4f2c31088e43a5', 'https://checkout.stripe.com/c/pay/cs_live_ff8747cb83d941d5bc4f2c31088e43a5');


--
-- Data for Name: managed_stripe_checkout_execution; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--

INSERT INTO recovery_funding_core.managed_stripe_checkout_execution (order_id, lease_token, lease_until, next_attempt_at, create_attempts, last_create_at, last_result) VALUES ('ff8747cb-83d9-41d5-bc4f-2c31088e43a5', NULL, NULL, '2026-11-10 12:43:48.902191+00', 0, NULL, 'bound');


--
-- Data for Name: managed_stripe_disputes; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: managed_stripe_event_inbox; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--

INSERT INTO recovery_funding_core.managed_stripe_event_inbox (stripe_account_id, livemode, event_id, order_id, operation_id, body_sha256, event_fingerprint, event, event_created_at, outcome, disposition) VALUES ('acct_fixture_only', true, 'evt_ff8747cb83d941d5bc4f2c31088e43a5', 'ff8747cb-83d9-41d5-bc4f-2c31088e43a5', 'e2484fbc-f45a-43c3-add3-112dba065ca8', 'ea38971aa0e686c528ce773c8b6a37cedb747286392e704bd98d61bf8520b00c', 'e4586e81c98065fae818653c7a3fb7fff2d3768ae5fae912b5a438100dcb7f18', '{"outcome": "paid", "currency": "mxn", "event_id": "evt_ff8747cb83d941d5bc4f2c31088e43a5", "livemode": true, "order_id": "ff8747cb-83d9-41d5-bc4f-2c31088e43a5", "session_id": "cs_live_ff8747cb83d941d5bc4f2c31088e43a5", "body_sha256": "ea38971aa0e686c528ce773c8b6a37cedb747286392e704bd98d61bf8520b00c", "amount_minor": 225000, "operation_id": "e2484fbc-f45a-43c3-add3-112dba065ca8", "quote_digest": "1809b1ddfc92c95875807a99183d40c8f1b3de9356224e1888b394fbcb452c46", "event_created_at": 1794314598, "payment_intent_id": "pi_ff8747cb83d941d5bc4f2c31088e43a5", "stripe_account_id": "acct_fixture_only"}', 1794314598, 'paid', 'processed');


--
-- Data for Name: relay_transaction_outbox; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--

INSERT INTO recovery_funding_core.relay_transaction_outbox (operation_id, network, relayer_address, nonce, tx_hash, signed_transaction, transaction_metadata, send_attempts, next_send_at, created_at) VALUES ('407dfd59-d810-4e07-9201-2fcd2448e803', 'base-mainnet', '0x7e5f4552091a69125d5dfcb7b8c2659029395bdf', 0, '0xbf8e1e3a79ca0c34553f2fdc8f00371ea936d39893e5652a7df2aa3ded70fd62', '0x02f902ea82210580010382627094f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f080b90284261fef320000000000000000000000000000000000000000000000000000000005f5e10000000000000000000000000000000000000000000000000000000000004c4b40d5df1c619082f54d98c1935e456db983e80bc4099518b1110d8c710eb5608e615e1daa0699b8d63b9ed5c8d3a75db5aac8133dd5b777aa7bd3a4cca2a5dd199db0bbafec77028c8fd0d0b2dacef5e9f28738b5ffe3edbf9f44180b61560b0fada64c9c07c3afd7648b7c9396fd5a42aec8a9505824fec8b05868c3cc61ef50c9404e0caede36af0bb066c58ec16e3cdb779213eff948638dcb484422a42f779c000000000000000000000000000000000000000000000000000000006af462e60000000000000000000000000000000000000000000000000000000000093a80000000000000000000000000000000000000000000000000000000000003f4800000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000240128488e5c1894768b1f1dda1a7578ff57edb83c2cf9b4a6fbbce150457b959e20000000000000000000000000000000000000000000000000000000000000001000000000000000000000000a0ee7a142d267c1f36714e4a8f75612f20a79720c001a06ee016c6080347fffe7f91a9cd77bae9399f79fb9336b9fdeab974d95543e727a05ab7b3f9bd7fe5113d7025bb2cfe2e3bb79609f0ad464bfd7ba307c0817fb36a', '{"relayer": "0x7e5f4552091a69125d5dfcb7b8c2659029395bdf", "tx_hash": "0xbf8e1e3a79ca0c34553f2fdc8f00371ea936d39893e5652a7df2aa3ded70fd62", "gas_limit": 25200, "estimated_gas": 21000, "max_fee_per_gas_wei": 3, "estimated_max_cost_wei": 75640, "max_priority_fee_per_gas_wei": 1}', 2, '2026-11-10 12:43:30.605066+00', '2026-11-10 12:43:20.401561+00');


--
-- Data for Name: nonce_retirement_plans; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: nonce_retirement_approvals; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: nonce_retirement_envelopes; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: nonce_retirement_confirmations; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: nonce_retirement_send_events; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: objective_aggregates; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: open_competition_entrant_relays; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: open_competition_events; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: open_competition_v2_events; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: open_competition_v2_indexer_agreements; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: open_competition_v2_programs; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: open_competition_v2_projections; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: open_competition_v2_proof_jobs; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: opportunity_comments; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: trial_bounties; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: opportunity_creation_progress; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: outreach_attempts; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: payment_events; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: posting_authorization_retirements; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: posting_creation_relays; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: x402_relay_attempts; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: posting_relay_transactions; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: submissions; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: verifier_results; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: proof_records; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: push_delivery_tests; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: quotes; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: recovery_obligations; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: regression_verification_runs; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: reputation_events; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: retained_nonce_dispatch_proposals; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: retained_nonce_dispatch_authorizations; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: review_deadline_push_deliveries; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: risk_events; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: risk_reviews; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: security_mutation_results; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: security_quota_reservations; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: security_resource_owners; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: security_resource_grants; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: security_wallet_challenges; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: security_wallet_sessions; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: settlements; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: site_analytics_events; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: site_auth_wallets; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: site_posting_drafts; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: social_mention_ingestions; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: stripe_reconciled_objects; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: template_signals; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: timeout_revalidation_gas_receipts; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: timeout_revalidation_authorizations; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: timeout_revalidation_events; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: unfunded_bounty_solutions; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: verification_artifacts; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: verification_artifact_disputes; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: verification_plans; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: verification_review_reports; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: verification_report_events; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: verification_review_assignments; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: verification_runs; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: verifier_review_contacts; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: verifier_review_deliveries; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: verifier_runtime_checkpoints; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: wallet_topup_attempts; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: webhook_subscriptions; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: webhook_deliveries; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- Data for Name: x402_relayer_leases; Type: TABLE DATA; Schema: recovery_funding_core; Owner: -
--



--
-- PostgreSQL database dump complete
--

\unrestrict R5zvuFM61Q68m4lTkrsgehEj1nFbMP2U7zLxFpZgZckAte9RSVz1xEyLufVD57f
