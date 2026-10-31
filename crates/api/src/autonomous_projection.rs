//! Each autonomous protocol uses its own factory, reader boundary and events.
use super::*;

pub(super) fn protocols(network: &str) -> Result<Vec<&'static str>, StatusCode> {
    let descriptor = base_network_descriptor(network).map_err(|_| StatusCode::BAD_REQUEST)?;
    let mut protocols = vec![AUTONOMOUS_PROTOCOL_VERSION];
    // An absent v2 deployment is optional. A partially configured or invalid
    // deployment must remain visible as unavailable, never as unchecked success.
    if !matches!(
        service_runtime::autonomous_v2_deployment_for_chain(descriptor.chain_id),
        Ok(None)
    ) {
        protocols.push(AUTONOMOUS_V2_PROTOCOL_VERSION);
    }
    Ok(protocols)
}

pub(super) fn label(protocol: &str) -> &'static str {
    if protocol == AUTONOMOUS_V2_PROTOCOL_VERSION {
        "autonomous-v2"
    } else {
        "autonomous-v1"
    }
}

pub(super) fn factory(network: &str, protocol: &str) -> Result<String, StatusCode> {
    match protocol {
        AUTONOMOUS_PROTOCOL_VERSION => {
            configured_autonomous_planner(network).map(|p| p.factory_contract)
        }
        AUTONOMOUS_V2_PROTOCOL_VERSION => {
            configured_autonomous_v2_deployment(network).map(|d| d.factory)
        }
        _ => Err(StatusCode::BAD_REQUEST),
    }
}

/// Resolve identity from canonical factory creation, not a user-supplied
/// protocol or the health of a sibling reader. This lookup grants no freshness.
pub(super) async fn protocol_for_contract(
    state: &SharedState,
    network: &str,
    contract: &str,
) -> Result<&'static str, StatusCode> {
    let network = service_runtime::bounty_read_inputs::network_key(network)
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let mut selected = None;
    for protocol in protocols(network)? {
        let Ok(factory) = factory(network, protocol) else {
            continue;
        };
        let history = store
            .list_autonomous_bounty_history_for_contract(network, &factory, contract)
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        if !history.is_empty() && selected.replace(protocol).is_some() {
            return Err(StatusCode::SERVICE_UNAVAILABLE);
        }
    }
    selected.ok_or(StatusCode::NOT_FOUND)
}

pub(super) async fn load(
    state: &SharedState,
    network: &str,
    protocol: &str,
    contract: Option<&str>,
    observation: Option<&mut marketplace_readiness::ProjectionObservation>,
) -> Result<marketplace_readiness::CheckedProjection<Vec<AutonomousBountyFeedItem>>, StatusCode> {
    let network = service_runtime::bounty_read_inputs::network_key(network)
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let factory = factory(network, protocol)?;
    let boundary = match observation {
        Some(observation) => observation.take(network, protocol, &factory)?,
        None => {
            marketplace_readiness::require_current_reader(state, network, protocol, &factory)
                .await?
        }
    };
    store
        .require_autonomous_canonical_checkpoint(network, &factory, boundary.block_number)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let mut events = if let Some(contract) = contract {
        store
            .list_autonomous_bounty_history_for_contract(network, &factory, contract)
            .await
    } else {
        store
            .list_autonomous_bounty_events_for_factory(network, &factory, false)
            .await
    }
    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    events.retain(|event| event.block_number <= boundary.block_number);
    let mut terms = Vec::new();
    for hash in scoped_autonomous_terms_hashes(&events) {
        if let Some(record) = store
            .get_autonomous_bounty_terms(&hash)
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        {
            terms.push(record);
        }
    }
    let mut feed = build_autonomous_bounty_feed(events, terms, false)
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    if feed
        .iter()
        .any(|item| is_autonomous_v2_item(item) != (protocol == AUTONOMOUS_V2_PROTOCOL_VERSION))
    {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    if let Some(contract) = contract {
        feed.retain(|item| item.bounty_contract.eq_ignore_ascii_case(contract));
    }
    service_runtime::verifier_checkpoints::apply_current(state.store.as_ref(), network, &mut feed)
        .await;
    state.recovery_reservations.apply(&mut feed, false);
    boundary.ensure_current()?;
    Ok(boundary.bind(feed))
}

/// Earning-only feeds include independently current protocols. The opportunity
/// endpoint additionally exposes each unavailable protocol in coverage metadata.
pub(super) async fn load_current(
    state: &SharedState,
    network: &str,
    contract: Option<&str>,
) -> Result<marketplace_readiness::CheckedProjection<Vec<AutonomousBountyFeedItem>>, StatusCode> {
    if let Some(contract) = contract {
        let protocol = protocol_for_contract(state, network, contract).await?;
        return load(state, network, protocol, Some(contract), None).await;
    }
    let mut observation = marketplace_readiness::observe_projection_readers(state, network).await;
    let mut current = Vec::new();
    for protocol in protocols(network)? {
        if let Ok(checked) = load(state, network, protocol, None, Some(&mut observation)).await {
            current.push(checked);
        }
    }
    marketplace_readiness::CheckedProjection::concat(current)
}
