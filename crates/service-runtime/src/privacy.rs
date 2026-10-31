//! Shared read projections. A child record never makes its private parent public.
use crate::{auth, bounty_status, BountyStatusLookupError};
use app::{AgentPayoutStatusResponse, BountyNetwork, BountyStatusResponse};
use db::PostgresStore;
use domain::{Bounty, Id, PrivacyLevel, ProofRecord, VerifierResult};
use std::{collections::HashSet, sync::Mutex};

pub const REDACTED_PROOF_SUMMARY: &str =
    "Verified completion. Artifact contents and verifier details are private.";

pub fn bounty_is_public(network: &BountyNetwork, id: Id) -> bool {
    network
        .bounties
        .get(&id)
        .is_some_and(|b| b.privacy != PrivacyLevel::Private)
}

/// Apply privacy after checking the original network's confirmed funding state.
pub fn public_claimable_bounties(network: &BountyNetwork) -> Vec<Bounty> {
    network
        .list_claimable_bounties()
        .into_iter()
        .filter(|bounty| bounty.privacy != PrivacyLevel::Private)
        .collect()
}

pub fn proof_is_public(network: &BountyNetwork, proof: &ProofRecord) -> bool {
    proof.privacy != PrivacyLevel::Private && bounty_is_public(network, proof.bounty_id)
}

fn proof_is_redacted(network: &BountyNetwork, proof: &ProofRecord) -> bool {
    proof.privacy == PrivacyLevel::RedactedPublicProof
        || network
            .bounties
            .get(&proof.bounty_id)
            .is_some_and(|b| b.privacy == PrivacyLevel::RedactedPublicProof)
}

/// Project one coherent proof without cloning unrelated accounts or payment records.
pub fn public_proof_details(
    network: &BountyNetwork,
    id: Id,
) -> Option<(ProofRecord, VerifierResult)> {
    let proof = network.proofs.get(&id)?;
    if !proof_is_public(network, proof) {
        return None;
    }
    let verifier = network.verifier_results.get(&proof.verifier_result_id)?;
    if verifier.bounty_id != proof.bounty_id || verifier.submission_id != proof.submission_id {
        return None;
    }
    let mut verifier = verifier.clone();
    let mut projected = proof.clone();
    if proof_is_redacted(network, proof) {
        projected.privacy = PrivacyLevel::RedactedPublicProof;
        projected.public_summary = REDACTED_PROOF_SUMMARY.into();
        verifier.summary = REDACTED_PROOF_SUMMARY.into();
    }
    Some((projected, verifier))
}

/// A fresh projection prevents private records from contributing to aggregate totals.
/// Account contacts, private wallet bindings, ledgers and operator notes are not copied.
pub fn public_network(network: &BountyNetwork) -> BountyNetwork {
    projected_network(network, &HashSet::new())
}

fn projected_network(network: &BountyNetwork, allowed: &HashSet<Id>) -> BountyNetwork {
    let visible = |id: Id| allowed.contains(&id) || bounty_is_public(network, id);
    let mut projection = BountyNetwork {
        agents: network.agents.clone(),
        capabilities: network.capabilities.clone(),
        bounties: network
            .bounties
            .iter()
            .filter(|(id, _)| visible(**id))
            .map(|(id, b)| (*id, b.clone()))
            .collect(),
        settlements: network
            .settlements
            .iter()
            .filter(|(_, s)| visible(s.bounty_id))
            .map(|(id, s)| (*id, s.clone()))
            .collect(),
        reputation_events: network
            .reputation_events
            .iter()
            .filter(|(_, e)| visible(e.bounty_id))
            .map(|(id, e)| (*id, e.clone()))
            .collect(),
        verifier_results: network
            .verifier_results
            .iter()
            .filter(|(_, v)| visible(v.bounty_id))
            .map(|(id, v)| (*id, v.clone()))
            .collect(),
        proofs: network
            .proofs
            .iter()
            .filter(|(_, p)| {
                visible(p.bounty_id)
                    && (allowed.contains(&p.bounty_id) || proof_is_public(network, p))
            })
            .map(|(id, p)| (*id, p.clone()))
            .collect(),
        template_signals: network
            .template_signals
            .iter()
            .filter(|(_, s)| visible(s.bounty_id))
            .map(|(id, s)| (*id, s.clone()))
            .collect(),
        ..BountyNetwork::default()
    };
    for agent in projection.agents.values_mut() {
        agent.payout_wallet = None;
    }
    for proof in projection.proofs.values_mut() {
        if !allowed.contains(&proof.bounty_id) && proof_is_redacted(network, proof) {
            proof.privacy = PrivacyLevel::RedactedPublicProof;
            proof.public_summary = REDACTED_PROOF_SUMMARY.into();
        }
    }
    let redacted_verifiers: HashSet<_> = network
        .proofs
        .values()
        .filter(|p| p.privacy != PrivacyLevel::Public)
        .map(|p| p.verifier_result_id)
        .collect();
    for verifier in projection.verifier_results.values_mut() {
        if !allowed.contains(&verifier.bounty_id)
            && (network
                .bounties
                .get(&verifier.bounty_id)
                .is_some_and(|b| b.privacy == PrivacyLevel::RedactedPublicProof)
                || redacted_verifiers.contains(&verifier.id))
        {
            verifier.summary = REDACTED_PROOF_SUMMARY.into();
        }
    }
    projection
}

pub async fn bounty_status_for_principal(
    store: Option<&PostgresStore>,
    network: &Mutex<BountyNetwork>,
    id: Id,
    principal: &auth::Principal,
) -> Result<BountyStatusResponse, BountyStatusLookupError> {
    let status = bounty_status(store, network, id).await?;
    let allowed = auth::authorize_resource(store, principal, "bounty", &id.to_string(), "read")
        .await
        .map_err(|e| BountyStatusLookupError::Store(e.to_string()))?;
    project_bounty_status(status, allowed)
}

fn project_bounty_status(
    mut status: BountyStatusResponse,
    allowed: bool,
) -> Result<BountyStatusResponse, BountyStatusLookupError> {
    if status.bounty.privacy == PrivacyLevel::Private && !allowed {
        return Err(BountyStatusLookupError::NotFound("bounty not found".into()));
    }
    if !allowed {
        let hidden_submissions: HashSet<_> = status
            .proofs
            .iter()
            .filter(|p| p.privacy != PrivacyLevel::Public)
            .map(|p| p.submission_id)
            .collect();
        let hidden_verifiers: HashSet<_> = status
            .proofs
            .iter()
            .filter(|p| p.privacy != PrivacyLevel::Public)
            .map(|p| p.verifier_result_id)
            .collect();
        status
            .submissions
            .retain(|s| !hidden_submissions.contains(&s.id));
        status
            .verifier_results
            .retain(|v| !hidden_verifiers.contains(&v.id));
        status.proofs.retain(|p| p.privacy != PrivacyLevel::Private);
        status.risk_events.clear();
        for intent in &mut status.funding_intents {
            intent.external_reference = None;
            intent.stripe_success_url = None;
            intent.stripe_cancel_url = None;
            intent.source_organization_id = None;
        }
        for contribution in &mut status.funding_contributions {
            contribution.external_reference = None;
            contribution.funding_ledger_entry_id = None;
            contribution.refund_ledger_entry_id = None;
            contribution.source_organization_id = None;
        }
        for escrow in &mut status.escrows {
            escrow.external_reference = None;
        }
        if status.bounty.privacy == PrivacyLevel::RedactedPublicProof {
            status.submissions.clear();
            status.verifier_results.clear();
        }
        for proof in &mut status.proofs {
            if proof.privacy == PrivacyLevel::RedactedPublicProof
                || status.bounty.privacy == PrivacyLevel::RedactedPublicProof
            {
                proof.privacy = PrivacyLevel::RedactedPublicProof;
                proof.public_summary = REDACTED_PROOF_SUMMARY.into();
            }
        }
    }
    // Repair the representation of historical children without rewriting evidence.
    if status.bounty.privacy == PrivacyLevel::Private {
        for proof in &mut status.proofs {
            proof.privacy = PrivacyLevel::Private;
        }
    }
    Ok(status)
}

pub async fn agent_payout_status(
    store: Option<&PostgresStore>,
    network: &Mutex<BountyNetwork>,
    id: Id,
    principal: &auth::Principal,
) -> Result<AgentPayoutStatusResponse, BountyStatusLookupError> {
    let private_ids: Vec<Id> = {
        let network = network.lock().expect("state poisoned");
        network
            .bounties
            .values()
            .filter(|b| b.privacy == PrivacyLevel::Private)
            .map(|b| b.id)
            .collect()
    };
    let mut allowed = HashSet::new();
    for bounty_id in private_ids {
        if auth::authorize_resource(store, principal, "bounty", &bounty_id.to_string(), "read")
            .await
            .map_err(|e| BountyStatusLookupError::Store(e.to_string()))?
        {
            allowed.insert(bounty_id);
        }
    }
    let owns_agent = auth::authorize_resource(store, principal, "agent", &id.to_string(), "read")
        .await
        .map_err(|e| BountyStatusLookupError::Store(e.to_string()))?;
    let network = network.lock().expect("state poisoned");
    let mut status = projected_network(&network, &allowed)
        .agent_payout_status(id)
        .map_err(|e| BountyStatusLookupError::NotFound(e.to_string()))?;
    if owns_agent {
        status.agent = network.agents.get(&id).expect("agent exists").clone();
    }
    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use domain::{
        Agent, Bounty, CapabilityClass, Escrow, EscrowStatus, FundingContribution,
        FundingContributionStatus, FundingIntent, FundingIntentStatus, FundingMode, Money,
        PaymentRail, PayoutIntent, PayoutStatus, ReputationEvent, Settlement, Submission,
        TemplateSignal, VerificationDecision, VerifierKind, VerifierResult,
    };
    use uuid::Uuid;

    fn add_bounty(
        network: &mut BountyNetwork,
        agent_id: Id,
        privacy: PrivacyLevel,
        amount: i64,
    ) -> Id {
        let title = if privacy == PrivacyLevel::Private {
            "PRIVATE_PARENT_ONLY"
        } else {
            "Public request"
        };
        let bounty = Bounty::new(
            title,
            "patch-with-tests",
            Money::new(amount, "usdc").unwrap(),
            FundingMode::Simulated,
            privacy.clone(),
        );
        let id = bounty.id;
        let summary = if privacy == PrivacyLevel::Public {
            "PUBLIC_EVIDENCE"
        } else {
            "PRIVATE_ARTIFACT_SECRET"
        };
        let submission = Submission {
            id: Uuid::new_v4(),
            bounty_id: id,
            solver_agent_id: agent_id,
            artifact_digest: "digest".into(),
            artifact_uri: summary.into(),
            submitted_at: Utc::now(),
        };
        let verifier = VerifierResult {
            id: Uuid::new_v4(),
            bounty_id: id,
            submission_id: submission.id,
            verifier_agent_id: Some(agent_id),
            kind: VerifierKind::Manual,
            decision: VerificationDecision::Accepted,
            summary: summary.into(),
            confidence: 1.0,
            signed_payload_hash: "digest".into(),
            created_at: Utc::now(),
        };
        // Historical children may say Public even when their parent does not.
        let proof = ProofRecord {
            id: Uuid::new_v4(),
            bounty_id: id,
            submission_id: submission.id,
            verifier_result_id: verifier.id,
            proof_hash: "proof-hash".into(),
            public_summary: summary.into(),
            privacy: PrivacyLevel::Public,
            created_at: Utc::now(),
        };
        let settlement = Settlement {
            id: Uuid::new_v4(),
            bounty_id: id,
            proof_record_id: proof.id,
            rail: PaymentRail::Simulated,
            payout_intents: vec![PayoutIntent {
                id: Uuid::new_v4(),
                bounty_id: id,
                recipient_agent_id: agent_id,
                rail: PaymentRail::Simulated,
                amount: bounty.amount.clone(),
                status: PayoutStatus::Paid,
            }],
            platform_fee: Money::zero("usdc"),
            created_at: Utc::now(),
        };
        let reputation = ReputationEvent {
            id: Uuid::new_v4(),
            agent_id,
            bounty_id: id,
            capability_class: CapabilityClass::Coding,
            template_slug: bounty.template_slug.clone(),
            delta: 10,
            reason: "Accepted completion".into(),
            created_at: Utc::now(),
        };
        let signal = TemplateSignal {
            id: Uuid::new_v4(),
            bounty_id: id,
            proof_record_id: proof.id,
            template_slug: bounty.template_slug.clone(),
            capability_class: CapabilityClass::Coding,
            verifier_kind: VerifierKind::Manual,
            amount: bounty.amount.clone(),
            success: true,
            created_at: Utc::now(),
        };
        network.bounties.insert(id, bounty);
        network.submissions.insert(submission.id, submission);
        network.verifier_results.insert(verifier.id, verifier);
        network.proofs.insert(proof.id, proof);
        network.settlements.insert(settlement.id, settlement);
        network.reputation_events.insert(reputation.id, reputation);
        network.template_signals.insert(signal.id, signal);
        id
    }

    #[test]
    fn claimable_projection_excludes_private_work_and_keeps_confirmed_funding_checks() {
        let mut network = BountyNetwork::default();
        let mut expected = HashSet::new();
        for (privacy, funded) in [
            (PrivacyLevel::Public, true),
            (PrivacyLevel::Private, true),
            (PrivacyLevel::RedactedPublicProof, true),
            (PrivacyLevel::Public, false),
        ] {
            let mut bounty = Bounty::new(
                "Prepared work",
                "patch-with-tests",
                Money::new(100, "usdc").unwrap(),
                FundingMode::Simulated,
                privacy.clone(),
            );
            bounty.mark_funded("terms").unwrap();
            bounty.make_claimable().unwrap();
            if funded {
                let escrow = Escrow {
                    id: Uuid::new_v4(),
                    bounty_id: bounty.id,
                    rail: PaymentRail::Simulated,
                    token: "usdc".into(),
                    amount: bounty.amount.clone(),
                    status: EscrowStatus::Funded,
                    external_reference: None,
                };
                network.escrows.insert(escrow.id, escrow);
                if privacy != PrivacyLevel::Private {
                    expected.insert(bounty.id);
                }
            }
            network.bounties.insert(bounty.id, bounty);
        }
        assert_eq!(network.list_claimable_bounties().len(), 3);
        let visible = public_claimable_bounties(&network);
        assert_eq!(
            visible.iter().map(|b| b.id).collect::<HashSet<_>>(),
            expected
        );
        assert_eq!(network.bounties.len(), 4);
        assert_eq!(network.escrows.len(), 3);
    }

    #[tokio::test]
    async fn private_parents_and_their_totals_never_enter_public_projections() {
        let mut network = BountyNetwork::default();
        let mut agent = Agent::new("public-solver");
        agent.payout_wallet = Some("PRIVATE_WALLET_BINDING".into());
        network.agents.insert(agent.id, agent.clone());
        let public = add_bounty(&mut network, agent.id, PrivacyLevel::Public, 100);
        let private = add_bounty(&mut network, agent.id, PrivacyLevel::Private, 900);
        let redacted = add_bounty(
            &mut network,
            agent.id,
            PrivacyLevel::RedactedPublicProof,
            200,
        );

        let projection = public_network(&network);
        assert_eq!(projection.bounties.len(), 2);
        assert!(
            projection.bounties.contains_key(&public)
                && projection.bounties.contains_key(&redacted)
        );
        assert!(!projection.bounties.contains_key(&private));
        assert_eq!(projection.proofs.len(), 2);
        assert_eq!(projection.verifier_results.len(), 2);
        assert_eq!(projection.template_signals.len(), 2);
        assert_eq!(projection.reputation_events.len(), 2);
        let payout = projection.agent_payout_status(agent.id).unwrap();
        assert_eq!(payout.totals[0].total_minor, 300);
        assert_eq!(payout.payouts.len(), 2);
        assert_eq!(payout.agent.payout_wallet, None);
        assert_eq!(
            projection
                .template_signals
                .values()
                .map(|s| s.amount.amount)
                .sum::<i64>(),
            300
        );
        let exposed = serde_json::to_string(&projection.proofs).unwrap()
            + &serde_json::to_string(&projection.verifier_results).unwrap();
        assert!(!exposed.contains("PRIVATE_ARTIFACT_SECRET"));
        assert!(exposed.contains("PUBLIC_EVIDENCE"));
        for proof in network.proofs.values() {
            let details = public_proof_details(&network, proof.id);
            if proof.bounty_id == private {
                assert!(details.is_none());
            } else {
                let (projected, verifier) = details.unwrap();
                let expected = if proof.bounty_id == redacted {
                    REDACTED_PROOF_SUMMARY
                } else {
                    "PUBLIC_EVIDENCE"
                };
                assert_eq!(projected.public_summary, expected);
                assert_eq!(verifier.summary, expected);
            }
        }

        let network = Mutex::new(network);
        assert!(matches!(
            bounty_status_for_principal(None, &network, private, &auth::Principal::Anonymous).await,
            Err(BountyStatusLookupError::NotFound(_))
        ));
        let owner =
            bounty_status_for_principal(None, &network, private, &auth::Principal::Operator)
                .await
                .unwrap();
        assert_eq!(owner.proofs[0].privacy, PrivacyLevel::Private);
        assert_eq!(owner.proofs[0].public_summary, "PRIVATE_ARTIFACT_SECRET");
        let owner_payout =
            agent_payout_status(None, &network, agent.id, &auth::Principal::Operator)
                .await
                .unwrap();
        assert_eq!(owner_payout.totals[0].total_minor, 1200);
        assert_eq!(owner_payout.agent.payout_wallet, agent.payout_wallet);
        assert_eq!(
            network
                .lock()
                .unwrap()
                .proofs
                .values()
                .find(|p| p.bounty_id == private)
                .unwrap()
                .privacy,
            PrivacyLevel::Public,
            "projection never rewrites historical evidence"
        );
    }

    #[test]
    fn public_status_keeps_evidence_and_redacts_hosted_payment_bookkeeping() {
        let mut network = BountyNetwork::default();
        let agent = Agent::new("solver");
        let id = add_bounty(&mut network, agent.id, PrivacyLevel::Public, 100);
        let mut status = network.status(id).unwrap();
        status.funding_intents.push(FundingIntent {
            id: Uuid::new_v4(),
            bounty_id: id,
            contributor_agent_id: Some(agent.id),
            source_organization_id: Some(Uuid::new_v4()),
            rail: PaymentRail::StripeFiat,
            amount: Money::new(100, "usdc").unwrap(),
            status: FundingIntentStatus::Applied,
            external_reference: Some("PRIVATE_PAYMENT_REFERENCE".into()),
            stripe_success_url: Some("https://example.invalid/private-return".into()),
            stripe_cancel_url: Some("https://example.invalid/private-cancel".into()),
            created_at: Utc::now(),
        });
        status.funding_contributions.push(FundingContribution {
            id: Uuid::new_v4(),
            bounty_id: id,
            contributor_agent_id: Some(agent.id),
            source_organization_id: Some(Uuid::new_v4()),
            rail: PaymentRail::StripeFiat,
            amount: Money::new(100, "usdc").unwrap(),
            status: FundingContributionStatus::Applied,
            funding_ledger_entry_id: Some(Uuid::new_v4()),
            refund_ledger_entry_id: Some(Uuid::new_v4()),
            settlement_id: None,
            external_reference: Some("PRIVATE_PAYMENT_REFERENCE".into()),
            created_at: Utc::now(),
        });
        status.escrows.push(Escrow {
            id: Uuid::new_v4(),
            bounty_id: id,
            rail: PaymentRail::StripeFiat,
            token: "usdc".into(),
            amount: Money::new(100, "usdc").unwrap(),
            status: EscrowStatus::Funded,
            external_reference: Some("PRIVATE_PAYMENT_REFERENCE".into()),
        });
        let public = project_bounty_status(status.clone(), false).unwrap();
        let serialized = serde_json::to_string(&public).unwrap();
        assert!(
            !serialized.contains("PRIVATE_PAYMENT_REFERENCE")
                && !serialized.contains("https://example.invalid/private")
        );
        assert!(public.funding_intents[0].source_organization_id.is_none());
        assert!(public.funding_contributions[0]
            .funding_ledger_entry_id
            .is_none());
        assert!(public.funding_contributions[0]
            .refund_ledger_entry_id
            .is_none());
        assert_eq!(public.bounty.amount.amount, 100);
        assert_eq!(public.funding_contributions[0].amount.amount, 100);
        assert_eq!(public.proofs[0].public_summary, "PUBLIC_EVIDENCE");
        assert_eq!(public.submissions.len(), 1);
        let owned = project_bounty_status(status, true).unwrap();
        assert_eq!(
            owned.funding_intents[0].external_reference.as_deref(),
            Some("PRIVATE_PAYMENT_REFERENCE")
        );
    }

    #[test]
    fn redacted_parent_or_child_never_exposes_artifacts_and_orphans_stay_hidden() {
        let mut network = BountyNetwork::default();
        let id = add_bounty(
            &mut network,
            Uuid::new_v4(),
            PrivacyLevel::RedactedPublicProof,
            100,
        );
        let status = project_bounty_status(network.status(id).unwrap(), false).unwrap();
        assert!(status.submissions.is_empty() && status.verifier_results.is_empty());
        assert_eq!(status.proofs[0].privacy, PrivacyLevel::RedactedPublicProof);
        assert_eq!(status.proofs[0].public_summary, REDACTED_PROOF_SUMMARY);
        network.bounties.get_mut(&id).unwrap().privacy = PrivacyLevel::Public;
        let proof = network.proofs.values_mut().next().unwrap();
        proof.privacy = PrivacyLevel::RedactedPublicProof;
        let status = project_bounty_status(network.status(id).unwrap(), false).unwrap();
        assert!(status.submissions.is_empty() && status.verifier_results.is_empty());
        assert_eq!(status.proofs[0].public_summary, REDACTED_PROOF_SUMMARY);
        network.bounties.remove(&id);
        assert!(public_network(&network).proofs.is_empty());
        let proof_id = *network.proofs.keys().next().unwrap();
        assert!(public_proof_details(&network, proof_id).is_none());
    }
}
