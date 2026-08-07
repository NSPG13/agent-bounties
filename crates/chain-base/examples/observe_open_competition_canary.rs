use chain_base::{
    observe_open_competition_safe_state, OpenCompetitionReleaseManifest, OpenCompetitionStateQuery,
    OpenCompetitionVerifierCatalog,
};
use serde_json::Value;

#[tokio::main]
async fn main() {
    let manifest_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "deployments/open-competition-v1-base-mainnet.json".to_string());
    let manifest: Value = serde_json::from_str(
        &std::fs::read_to_string(&manifest_path).expect("read deployment manifest"),
    )
    .expect("parse deployment manifest");
    let release: OpenCompetitionReleaseManifest =
        serde_json::from_value(manifest["release_manifest"].clone()).expect("release manifest");
    let catalog: OpenCompetitionVerifierCatalog =
        serde_json::from_value(manifest["verifier_catalog"].clone()).expect("verifier catalog");
    let bounty_contract = manifest["hidden_canary"]["bounty_contract"]
        .as_str()
        .expect("canary bounty")
        .to_string();
    let solver = manifest["hidden_canary"]["winner"]
        .as_str()
        .expect("canary winner")
        .to_string();
    let rpc_url = std::env::var("BASE_MAINNET_RPC_URL")
        .unwrap_or_else(|_| "https://base.drpc.org".to_string());

    match observe_open_competition_safe_state(
        &rpc_url,
        &OpenCompetitionStateQuery {
            release,
            bounty_contract,
            solver: Some(solver),
            verifier_profile: catalog
                .profiles
                .into_iter()
                .next()
                .expect("verifier profile"),
        },
    )
    .await
    {
        Ok(state) => println!(
            "{}",
            serde_json::to_string_pretty(&state).expect("serialize state")
        ),
        Err(error) => {
            eprintln!("{error:?}\n{error}");
            std::process::exit(1);
        }
    }
}
