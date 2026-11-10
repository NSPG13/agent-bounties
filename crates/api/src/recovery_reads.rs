//! Explicit public reads before ordinary initialization, migrations or workers.
//! The full router and all mutating/auth/provider constructors stay unreachable.
use super::*;
use axum::http::Method;
use tower_http::cors::CorsLayer;

const PATHS: [&str; 4] = [
    "/health",
    "/readiness/marketplace",
    "/v1/opportunities",
    "/v1/base/autonomous-bounties/feed",
];
const ORIGINS: [&str; 4] = [
    "https://agentizable.ai",
    "https://www.agentizable.ai",
    "https://agentbounties.app",
    "https://www.agentbounties.app",
];

fn public_preflight(request: &Request) -> bool {
    let headers = request.headers();
    headers.get_all(header::ORIGIN).iter().count() == 1
        && headers
            .get(header::ORIGIN)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| ORIGINS.contains(&v))
        && headers
            .get_all(header::ACCESS_CONTROL_REQUEST_METHOD)
            .iter()
            .count()
            == 1
        && headers
            .get(header::ACCESS_CONTROL_REQUEST_METHOD)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| matches!(v, "GET" | "HEAD"))
        && headers
            .get_all(header::ACCESS_CONTROL_REQUEST_HEADERS)
            .iter()
            .count()
            <= 1
        && headers
            .get(header::ACCESS_CONTROL_REQUEST_HEADERS)
            .is_none_or(|v| {
                v.to_str().is_ok_and(|v| {
                    v.split(',').all(|name| {
                        matches!(
                            name.trim().to_ascii_lowercase().as_str(),
                            "accept" | "cache-control"
                        )
                    })
                })
            })
}

fn read_state(
    store: PostgresStore,
    rpc: BaseRpcUrlConfig,
    public_base_url: String,
    reservations: AutonomousBountyRecoveryReservations,
) -> SharedState {
    Arc::new(AppState {
        security_auth: security_auth::SecurityAuthState::disabled(),
        network: Arc::new(Mutex::new(BountyNetwork::default())),
        eval_runs: Arc::new(Mutex::new(Vec::new())),
        stripe_webhook_secret: None,
        stripe_webhook_livemode: None,
        stripe_reconciliation_gate: Arc::new(tokio::sync::Mutex::new(())),
        allow_unsigned_stripe_webhooks: false,
        stripe_secret_key: None,
        stripe_live_execution_enabled: false,
        stripe_public_checkout_enabled: false,
        managed_payments: managed_payments::ManagedPaymentsConfig::default(),
        stripe_api_base_url: STRIPE_API_BASE_URL.to_string(),
        stripe_payment_method_configuration: None,
        store: Some(store),
        base_rpc_urls: rpc,
        marketplace_head_rpc: marketplace_head_rpc::HeadRpcOverrides::from_env(),
        base_broadcast_enabled: false,
        operator_api_token: None,
        discoverability_ingest_token: None,
        analytics_exclusion_token: None,
        distribution_attribution_signing_secret: None,
        distribution_excluded_wallet_classes: db::DISTRIBUTION_EXCLUSION_CLASSES
            .iter()
            .map(|v| (*v).to_string())
            .collect(),
        public_base_url,
        mcp_base_url: "https://mcp.agentizable.ai".to_string(),
        x402_relayer: X402HostedRelayerConfig::default(),
        bond_sponsor: BondSponsorConfig::default(),
        recovery_reservations: reservations,
        cloud_agent: Arc::new(CloudAgentService),
        discovery_webhooks: None,
        neynar_social: None,
    })
}

async fn opportunities(
    State(state): State<SharedState>,
    Query(query): Query<OpportunityQuery>,
) -> Result<Json<OpportunityProjectionResponse>, StatusCode> {
    validate_opportunity_query(&query)?;
    let network = query.network.as_deref().unwrap_or("base-mainnet");
    // The canonical/offchain projections read the durable store themselves.
    // Legacy requests get real fresh read hydration, never a synthetic empty
    // inventory or ordinary startup's potentially stale in-memory snapshot.
    let needs_legacy = query
        .source_type
        .as_deref()
        .is_none_or(|s| s == "legacy_bounty")
        && exact_canonical_identity(query.opportunity_id.as_deref(), network)?.is_none();
    if needs_legacy {
        let store = state
            .store
            .as_ref()
            .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
        let network = hydrate_network(store)
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        let mut request_state = (*state).clone();
        request_state.network = Arc::new(Mutex::new(network));
        return list_opportunities(State(Arc::new(request_state)), Query(query)).await;
    }
    list_opportunities(State(state), Query(query)).await
}

async fn unavailable() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(serde_json::json!({
            "schema": "agentizable/recovery-boot-v1", "ready": false,
            "status": "unavailable", "reason": "recovery_read_only"
        })),
    )
        .into_response()
}

async fn boundary(request: Request, next: Next) -> Response {
    let allowed = PATHS.contains(&request.uri().path())
        && (matches!(*request.method(), Method::GET | Method::HEAD)
            || (request.method() == Method::OPTIONS && public_preflight(&request)));
    let mut response = if allowed {
        match tokio::time::timeout(Duration::from_secs(15), next.run(request)).await {
            Ok(response) => response,
            Err(_) => unavailable().await,
        }
    } else {
        unavailable().await
    };
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        "x-agent-bounties-boot-mode",
        HeaderValue::from_static("recovery-read-only"),
    );
    response.headers_mut().insert(
        "x-agent-bounties-x402-relay",
        HeaderValue::from_static("disabled"),
    );
    response
}

fn router(state: SharedState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route(
            "/readiness/marketplace",
            get(marketplace_readiness::readiness),
        )
        .route("/v1/opportunities", get(opportunities))
        .route(
            "/v1/base/autonomous-bounties/feed",
            get(autonomous_bounty_feed),
        )
        .fallback(unavailable)
        .with_state(state)
        .layer(
            CorsLayer::new()
                .allow_origin(ORIGINS.map(HeaderValue::from_static))
                .allow_methods([Method::GET, Method::HEAD])
                .allow_headers([header::ACCEPT, header::CACHE_CONTROL]),
        )
        // Only exact public preflights bypass the method boundary. No private
        // credentials, arbitrary headers or GET routes with side effects.
        .layer(middleware::from_fn(boundary))
}

pub(super) async fn serve() -> anyhow::Result<()> {
    anyhow::ensure!(
        env::args_os().len() == 1,
        "Recovery read-only mode does not accept command arguments"
    );
    let database_url = env::var("DATABASE_URL")
        .map_err(|_| anyhow::anyhow!("Recovery reads require database configuration"))?;
    let store = PostgresStore::connect_read_only(&database_url)
        .await
        .map_err(|_| anyhow::anyhow!("Recovery read-only database unavailable"))?;
    let reservations = AutonomousBountyRecoveryReservations::parse_csv(
        env::var("BASE_RECOVERY_RESERVED_BOUNTY_CONTRACTS")
            .ok()
            .as_deref(),
    )
    .map_err(|_| anyhow::anyhow!("Recovery reservation configuration is invalid"))?;
    let state = read_state(
        store,
        BaseRpcUrlConfig::from_env(),
        env::var("PUBLIC_BASE_URL").unwrap_or_else(|_| "https://api.agentizable.ai".to_string()),
        reservations,
    );
    let bind_addr = service_bind_addr(
        env::var("API_BIND_ADDR").ok().as_deref(),
        env::var("PORT").ok().as_deref(),
        "127.0.0.1:8080",
    );
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    axum::serve(listener, router(state)).await?;
    Ok(())
}
