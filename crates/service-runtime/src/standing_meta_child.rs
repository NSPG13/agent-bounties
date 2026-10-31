//! Publish child terms only after the complete trusted-parent plan validates.
use chain_base::{
    AutonomousBountyTxPlanner, ChainBaseError, StandingMetaV2ChildPreparationPlan,
    StandingMetaV2ChildPreparationRequest, StandingMetaV2ParentContext,
};
use chrono::{DateTime, Utc};
use domain::AutonomousBountyTermsRecord;
use std::future::Future;

#[derive(Debug)]
pub enum ChildPreparationError<E> {
    InvalidPlan(ChainBaseError),
    Publication(E),
}

/// Shared by API and direct/remote MCP. The publication callback is never invoked
/// for an invalid plan. No signing or on-chain calls occur here, and a usable
/// preparation is returned only after durable publication succeeds.
pub async fn prepare_and_publish<E, F, Fut>(
    planner: &AutonomousBountyTxPlanner,
    request: &StandingMetaV2ChildPreparationRequest,
    parent: &StandingMetaV2ParentContext,
    now: DateTime<Utc>,
    publish: F,
) -> Result<StandingMetaV2ChildPreparationPlan, ChildPreparationError<E>>
where
    F: FnOnce(AutonomousBountyTermsRecord) -> Fut,
    Fut: Future<Output = Result<(), E>>,
{
    let mut plan = planner
        .plan_standing_meta_v2_child(request, parent, now)
        .map_err(ChildPreparationError::InvalidPlan)?;
    publish(plan.terms.clone())
        .await
        .map_err(ChildPreparationError::Publication)?;
    plan.hosted_terms_published = true;
    plan.current_state = "hosted_routed_v3_child_terms_published_parent_unclaimed".to_string();
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::Money;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn fixture() -> (
        AutonomousBountyTxPlanner,
        StandingMetaV2ChildPreparationRequest,
        StandingMetaV2ParentContext,
        DateTime<Utc>,
    ) {
        let registry: serde_json::Value = serde_json::from_str(include_str!(
            "../../verifier-sdk/regression-profiles-v1.json"
        ))
        .unwrap();
        let profile = &registry["profiles"][0];
        let now = DateTime::parse_from_rfc3339("2026-10-28T21:22:26Z")
            .unwrap()
            .with_timezone(&Utc);
        let request: StandingMetaV2ChildPreparationRequest = serde_json::from_value(json!({
            "parent_bounty_contract": "0x3333333333333333333333333333333333333333",
            "parent_solver": "0x1111111111111111111111111111111111111111",
            "intended_child_solver": "0x2222222222222222222222222222222222222222",
            "title": "Validate the exact published Agent Card fixture",
            "goal": "Pass the pinned Agent Card field and smoke-script checks.",
            "acceptance_criteria": ["The exact command passes its stated limited checks."],
            "benchmark_source": profile["sources"][0],
            "runner_manifest": profile["runner_manifest"],
            "funding_deadline": now.timestamp() as u64 + 3600,
        }))
        .unwrap();
        // Simulates a reviewed future binding without changing the inactive
        // production table or accepting a client-supplied profile selection.
        let parent = StandingMetaV2ParentContext {
            reviewed_child_profile_id: Some(profile["id"].as_str().unwrap().to_string()),
            protocol_version: chain_base::STANDING_META_V3_ROUTED_PROTOCOL_VERSION.to_string(),
            bounty_contract: request.parent_bounty_contract.clone(),
            bounty_id: format!("0x{}", "a".repeat(64)),
            creator: "0x4444444444444444444444444444444444444444".to_string(),
            round: 3,
            solver_reward: Money::new(2_000_000, "usdc").unwrap(),
            child_target: Money::new(1_000_000, "usdc").unwrap(),
            funding_deadline: 1_791_676_800,
        };
        let planner = AutonomousBountyTxPlanner::new(
            crate::CANONICAL_BASE_MAINNET_BOUNTY_FACTORY,
            crate::CANONICAL_BASE_MAINNET_BOUNTY_IMPLEMENTATION,
        )
        .unwrap();
        (planner, request, parent, now)
    }

    #[tokio::test]
    async fn routed_child_shared_flow_rejects_before_publication() {
        let (planner, request, parent, now) = fixture();
        let calls = AtomicUsize::new(0);
        for case in 0..5 {
            let mut candidate = request.clone();
            let mut context = parent.clone();
            match case {
                0 => candidate.benchmark_source.commit = "b".repeat(40),
                1 => candidate.runner_manifest.command = vec!["true".to_string()],
                2 => context.reviewed_child_profile_id = None,
                3 => candidate.funding_deadline = None,
                _ => candidate.funding_deadline = Some(now.timestamp() as u64),
            }
            let result = prepare_and_publish(&planner, &candidate, &context, now, |_| {
                calls.fetch_add(1, Ordering::SeqCst);
                async { Ok::<_, &str>(()) }
            })
            .await;
            let Err(ChildPreparationError::InvalidPlan(error)) = result else {
                panic!("invalid child reached publication in case {case}");
            };
            assert_eq!(
                error.standing_meta_child_preparation_problem().0,
                match case {
                    0 | 1 => "standing_meta_child_profile_mismatch",
                    2 => "verification_child_profile_unapproved",
                    3 => "standing_meta_child_funding_deadline_required",
                    _ => "standing_meta_child_funding_deadline_expired",
                }
            );
            assert_eq!(calls.load(Ordering::SeqCst), 0);
        }
        let expected = planner
            .plan_standing_meta_v2_child(&request, &parent, now)
            .unwrap();
        let plan = prepare_and_publish(&planner, &request, &parent, now, |terms| {
            calls.fetch_add(1, Ordering::SeqCst);
            assert_eq!(terms.terms_hash, expected.terms.terms_hash);
            assert_eq!(
                serde_json::to_value(&terms).unwrap(),
                serde_json::to_value(&expected.terms).unwrap()
            );
            async { Ok::<_, &str>(()) }
        })
        .await
        .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(plan.hosted_terms_published);
        assert!(!plan.pre_claim_wallet_calls.is_empty());
        assert_eq!(plan.terms.terms_hash, expected.terms.terms_hash);
    }

    #[tokio::test]
    async fn routed_child_shared_flow_withholds_plan_when_publication_fails() {
        let (planner, request, parent, now) = fixture();
        let calls = AtomicUsize::new(0);
        let result = prepare_and_publish(&planner, &request, &parent, now, |_| {
            calls.fetch_add(1, Ordering::SeqCst);
            async { Err("database unavailable") }
        })
        .await;
        assert!(matches!(
            result,
            Err(ChildPreparationError::Publication("database unavailable"))
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
