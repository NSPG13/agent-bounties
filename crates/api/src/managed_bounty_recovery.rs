//! Recovery refunds have their own disabled-by-default runner; no review signer is needed.
use crate::{managed_payments, SharedState};
use managed_orders::stripe_refund::RefundMerchant;
use service_runtime::managed_refund::{
    execute_recovered_bounty_refund, reconcile_recovered_bounty_refund, ManagedRefundClient,
};

pub(crate) fn start(state: SharedState) {
    if state.store.is_none() {
        return;
    }
    tokio::spawn(async move {
        loop {
            if let Err(error) = pass(&state).await {
                eprintln!("managed recovery refund pass: {error}");
            }
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        }
    });
}
async fn pass(state: &SharedState) -> Result<(), &'static str> {
    let policy = managed_payments::policy(state).map_err(|_| "policy unavailable")?;
    let allow_create = state.managed_payments.recovered_refunds_enabled
        && state.managed_payments.refunds_enabled
        && (!policy.livemode || state.stripe_live_execution_enabled);
    let secret = state
        .stripe_secret_key
        .as_deref()
        .ok_or("merchant unavailable")?;
    let client = ManagedRefundClient::new(
        secret,
        RefundMerchant {
            stripe_account_id: policy.stripe_account_id.clone(),
            livemode: policy.livemode,
        },
    )
    .map_err(|_| "merchant configuration invalid")?;
    let store = managed_payments::store(state).map_err(|_| "store unavailable")?;
    // Observation is independent of new-creation permission and candidate failures.
    let observation = crate::managed_bounty_appeals::observe_refunds(
        state,
        chrono::Utc::now().timestamp().max(0) as u64,
        &mut vec![],
    )
    .await;
    for id in store
        .claim_managed_recovery_candidates(10)
        .await
        .map_err(|_| "queue unavailable")?
    {
        let target = match store.managed_recovery_target(id).await {
            Ok(t) => t,
            Err(_) => continue,
        };
        // The signed paid quote must belong to the configured merchant and mode.
        if target.order.quote.input().stripe_account_id != policy.stripe_account_id
            || target.order.quote.input().livemode != policy.livemode
        {
            continue;
        }
        let rpc = state
            .base_rpc_urls
            .resolve(&target.funding_plan.network)
            .ok();
        let existing = match store.managed_recovered_refund_plan(id).await {
            Ok(plan) => plan.is_some(),
            Err(_) => {
                eprintln!("managed recovery refund plan invalid: order={id}");
                continue;
            }
        };
        if (!allow_create || rpc.is_none()) && !existing {
            continue;
        }
        let result = tokio::time::timeout(std::time::Duration::from_secs(90), async {
            if allow_create {
                if let Some((_, rpc)) = rpc {
                    return execute_recovered_bounty_refund(store, &client, &rpc, id).await;
                }
            }
            reconcile_recovered_bounty_refund(store, &client, id).await
        })
        .await;
        if !matches!(result, Ok(Ok(_))) {
            eprintln!("managed recovery refund retained for reconciliation: order={id}");
        }
    }
    observation.map_err(|_| "refund observation incomplete")?;
    Ok(())
}
