use super::*;
use std::time::Duration;

pub(super) async fn options() -> Json<Value> {
    mcp_json(json!({
        "options": serde_json::from_str::<Value>(include_str!("../../../site/posting-options.json")).expect("checked posting options"),
        "review_url": "https://agentizable.ai/post.html",
        "stage_tool": "prepare_bounty_post",
        "sponsored_setup":{"capabilities_endpoint":"/v1/base/sponsored-setup/capabilities","prepare_endpoint":"/v1/base/sponsored-setup/prepare","status_endpoint":"/v1/base/sponsored-setup/status","review_url":"https://agentizable.ai/setup.html","actions":["wallet_deploy","participant_register"],"principal_moved":"0","instructions":"Check live availability and give the person the unsigned request as a private proposal file for first-party review. Do not place identity attestation signatures or owner signatures in URLs, comments or logs. Setup does not fund the wallet."},
        "creator_open_candidate":{"protocol":"agent-bounties/creator-open-v1","capabilities_endpoint":"/v1/base/creator-open/capabilities","prepare_endpoint":"/v1/base/creator-open/prepare","import_endpoint":"/v1/base/creator-open/import","import_requires_wallet":false,"status_tool":"get_posting_status","status_arguments":{"protocol":"creator-open-v1"},"review_url":"https://agentizable.ai/creator-open.html","activation":"Separately gated release. Check live capabilities before the first-party review."},
        "status_tool": "get_posting_status",
        "wallet_readiness": "Pass the public wallet address to get_posting_status for a safe-block Base USDC balance and public gas sponsorship availability. A balance does not prove ownership or connect a wallet.",
        "approval": "Terms approval and all wallet signatures remain on the first-party page. Reuse posting_operation_id and the existing draft when continuing."
    }))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PostingStatusArgs {
    pub bounty_contract: Option<String>,
    pub protocol: Option<String>,
    pub operation_id: Option<Uuid>,
    pub wallet: Option<String>,
}

pub(super) fn status_schema() -> Value {
    json!({"type":"object", "properties": {
        "protocol":{"type":"string","enum":["autonomous-v1","creator-open-v1"],"description":"Select the deployed protocol explicitly. Creator-open is a separately gated release; unavailable never means unfunded."},
        "bounty_contract":{"type":"string", "pattern":"^0x[0-9a-fA-F]{40}$", "description":"Original predicted or created Base bounty address. Never generate a replacement to escape a pending wallet request."},
        "operation_id":{"type":"string", "format":"uuid", "description":"Existing posting operation. Private approval status requires the caller's authenticated account session in transport headers; the identifier alone never grants access."},
        "wallet":{"type":"string", "pattern":"^0x[0-9a-fA-F]{40}$", "description":"Public Base wallet address for a read-only USDC balance. Never a signature, key or session credential."}
    }, "required":[], "additionalProperties":false})
}

pub(super) async fn status(headers: HeaderMap, Json(args): Json<PostingStatusArgs>) -> Json<Value> {
    let creator_open = match args.protocol.as_deref().unwrap_or("autonomous-v1") {
        "autonomous-v1" => false,
        "creator-open-v1" => true,
        _ => return mcp_error("Unknown posting protocol"),
    };
    if creator_open && args.operation_id.is_some() {
        return mcp_error("Creator-open uses its versioned approval hash; an autonomous-v1 posting operation cannot authorize it.");
    }
    let wallet = match args.wallet {
        Some(value) => match chain_base::normalize_evm_address(&value) {
            Ok(value) if value != "0x0000000000000000000000000000000000000000" => Some(value),
            _ => return mcp_error("Invalid public wallet address"),
        },
        None => None,
    };
    let contract = match args.bounty_contract {
        Some(value) => match chain_base::normalize_evm_address(&value) {
            Ok(value) => Some(value),
            Err(_) => return mcp_error("Invalid original bounty address"),
        },
        None => None,
    };
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()
    {
        Ok(client) => client,
        Err(_) => return mcp_error("Posting status client unavailable"),
    };
    let api = public_base_url_from_env();
    let sponsorship = match client
        .get(format!(
            "{}/v1/base/gas-sponsorship",
            api.trim_end_matches('/')
        ))
        .send()
        .await
    {
        Ok(response) if response.status().is_success() => response
            .json::<Value>()
            .await
            .unwrap_or(json!({"status":"unavailable"})),
        _ => {
            json!({"status":"unavailable", "next_action":"Keep the same posting operation and retry. No customer-paid fallback."})
        }
    };
    let mut value = json!({"gas_sponsorship":sponsorship, "paid":false,
        "review_url":args.operation_id.map(|id| format!("https://agentizable.ai/post.html?operation_id={id}#bounty-preview")).unwrap_or_else(|| "https://agentizable.ai/post.html".into()),
        "approval_state":"Review the private saved draft on the first-party page; this public tool cannot read or grant account approval.",
        "evidence_boundary":"Missing indexed data is not proof that a wallet transaction was never sent. Keep the same signed authorization until it is consumed, revoked or expired."});
    if let Some(wallet) = wallet {
        value["wallet_readiness"] = match client
            .get(format!(
                "{}/v1/base/posting-wallets/{wallet}",
                api.trim_end_matches('/')
            ))
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => response
                .json::<Value>()
                .await
                .unwrap_or(json!({"status":"unavailable"})),
            _ => {
                json!({"status":"unavailable", "wallet":wallet, "next_action":"Retry the balance observation; do not infer a zero balance."})
            }
        };
    }
    if let Some(id) = args.operation_id {
        // No credential is accepted in tool arguments, and credentials go only
        // to the configured first-party upstream. Redirects are forbidden.
        // The API still verifies the account cookie and operation ownership.
        value["private_draft"] = if let Some(cookie) = headers.get(header::COOKIE) {
            match client
                .get(format!(
                    "{}/v1/site-auth/posting-drafts/{id}",
                    api_upstream_base_url_from_env().trim_end_matches('/')
                ))
                .header(header::COOKIE, cookie)
                .send()
                .await
            {
                Ok(response) if response.status().is_success() => {
                    match response.json::<Value>().await {
                        Ok(draft) => draft_summary(&draft),
                        Err(_) => json!({"status":"unavailable"}),
                    }
                }
                Ok(response) => json!({"status": match response.status().as_u16() {
                    401 => "authentication_required", 403 => "account_access_denied", 404 => "not_found_for_this_account", _ => "unavailable"
                }}),
                Err(_) => json!({"status":"unavailable"}),
            }
        } else {
            json!({"status":"authentication_required"})
        };
        value["approval_state"] = json!("Authenticated private_draft approval is tied to its exact hash and may change. Review and wallet consent still happen on the first-party page; this tool cannot grant them.");
    }
    if creator_open {
        value["protocol"] = "agent-bounties/creator-open-v1".into();
        value["canonical_status"] = "unavailable".into();
        value["approval_state"]="Creator-open preparation is separate from the original posting form. Check release availability before preparing; this status tool cannot approve terms or sign.".into();
        value["review_url"] = json!(match &contract {
            Some(address) => format!(
                "https://agentizable.ai/creator-open.html?network=base-mainnet&bounty={address}"
            ),
            None => "https://agentizable.ai/creator-open.html".into(),
        });
        let mut request = client
            .get(format!(
                "{}/v1/base/creator-open/bounties",
                api.trim_end_matches('/')
            ))
            .query(&[("network", "base-mainnet")]);
        if let Some(address) = &contract {
            request = request.query(&[("bounty_contract", address)]);
        }
        if let Ok(response) = request.send().await {
            if response.status().is_success() {
                if let Ok(feed) = response.json::<Value>().await {
                    if feed["protocol"] == "agent-bounties/creator-open-v1"
                        && feed["items"].is_array()
                    {
                        value["canonical_status"] = "indexed".into();
                        value["creator_open_bounties"] = feed;
                    }
                }
            }
        }
        return mcp_json(value);
    }
    if let Some(contract) = contract {
        match client
            .get(format!(
                "{}/v1/base/autonomous-bounties/feed?network=base-mainnet&claimable_only=false",
                api.trim_end_matches('/')
            ))
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => {
                match response.json::<Vec<Value>>().await {
                    Ok(feed) => {
                        value["canonical_bounty"] = feed
                            .into_iter()
                            .find(|item| {
                                item["bounty_contract"]
                                    .as_str()
                                    .is_some_and(|address| address.eq_ignore_ascii_case(&contract))
                            })
                            .unwrap_or(Value::Null);
                    }
                    _ => value["canonical_status"] = json!("unavailable"),
                }
            }
            _ => value["canonical_status"] = json!("unavailable"),
        }
    }
    mcp_json(value)
}

fn draft_summary(draft: &Value) -> Value {
    let Some(hash) = draft["draft_hash"]
        .as_str()
        .filter(|hash| hash.len() == 64 && hash.bytes().all(|c| c.is_ascii_hexdigit()))
    else {
        return json!({"status":"unavailable"});
    };
    let Some(operation) = draft["operation_id"]
        .as_str()
        .and_then(|id| Uuid::parse_str(id).ok())
    else {
        return json!({"status":"unavailable"});
    };
    // Return only status, never signed continuations, recovery internals, or
    // arbitrary upstream URLs. An inert retired draft can point to its child.
    json!({"status":"available", "operation_id":operation, "revision":draft["revision"],
        "draft_hash":hash,"approved":draft["approved_draft_hash"].as_str()==Some(hash),
        "retired_operation_id":draft["retired_operation_id"],
        "review_url":format!("https://agentizable.ai/post.html?operation_id={operation}#bounty-preview"),
        "funding_confirmed":false,"evidence_boundary":"Draft approval is not a wallet signature or canonical funding."})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn private_status_never_infers_approval_or_returns_recovery_authority() {
        assert_eq!(draft_summary(&json!({}))["status"], "unavailable");
        let id = Uuid::new_v4();
        let mut draft = json!({"operation_id":id,"draft_hash":"a".repeat(64),"approved_draft_hash":"b".repeat(64),"revision":4,"recovery_state":{"signature":"secret"},"continuation_url":"https://untrusted.invalid"});
        assert_eq!(draft_summary(&draft)["approved"], false);
        draft["approved_draft_hash"] = draft["draft_hash"].clone();
        let result = draft_summary(&draft);
        assert_eq!(result["approved"], true);
        assert_eq!(result["funding_confirmed"], false);
        assert!(!result.to_string().contains("secret"));
        assert!(!result.to_string().contains("untrusted"));
    }
}
