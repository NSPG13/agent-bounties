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
                        let item = feed
                            .into_iter()
                            .find(|item| {
                                item["bounty_contract"]
                                    .as_str()
                                    .is_some_and(|address| address.eq_ignore_ascii_case(&contract))
                            })
                            .unwrap_or(Value::Null);
                        let confirmed = funding_confirmed(&item, &contract);
                        value["funding_confirmed"] = json!(confirmed);
                        value["status_label"] = json!(if confirmed {
                            "Funded"
                        } else {
                            "Not funded yet"
                        });
                        if confirmed {
                            value["share_url"] = json!(share_url(&contract));
                        }
                        value["canonical_bounty"] = item;
                    }
                    _ => value["canonical_status"] = json!("unavailable"),
                }
            }
            _ => value["canonical_status"] = json!("unavailable"),
        }
    }
    mcp_json(value)
}

fn amount(value: &Value) -> Option<u128> {
    match value {
        Value::String(text) if !text.is_empty() && text.bytes().all(|c| c.is_ascii_digit()) => {
            text.parse().ok()
        }
        Value::Number(number) => number.as_u64().map(u128::from),
        _ => None,
    }
}

fn is_hash(value: &Value) -> bool {
    value.as_str().is_some_and(|hash| {
        hash.len() == 66
            && hash.starts_with("0x")
            && hash[2..].bytes().all(|c| c.is_ascii_hexdigit())
    })
}

/// "Funded" is said only with indexed canonical evidence for this exact bounty:
/// valid terms, full funding, and its creation, FundingAdded and
/// BountyBecameClaimable events. A wallet response or transaction hash is not enough.
fn funding_confirmed(item: &Value, contract: &str) -> bool {
    let same = |value: &Value| {
        value
            .as_str()
            .is_some_and(|address| address.eq_ignore_ascii_case(contract))
    };
    let (Some(target), Some(funded)) = (
        amount(&item["target_amount"]),
        amount(&item["funded_amount"]),
    ) else {
        return false;
    };
    if !same(&item["bounty_contract"])
        || !is_hash(&item["bounty_id"])
        || item["terms_valid"] != json!(true)
        || item["validation_errors"]
            .as_array()
            .is_some_and(|errors| !errors.is_empty())
        || !matches!(
            item["status"].as_str(),
            Some("claimable" | "claimed" | "submitted" | "paid")
        )
        || target == 0
        || funded < target
    {
        return false;
    }
    let events: Vec<&Value> = item["events"]
        .as_array()
        .map(|events| {
            events
                .iter()
                .filter(|event| {
                    event["bounty_id"] == item["bounty_id"]
                        && is_hash(&event["tx_hash"])
                        && event["block_number"]
                            .as_u64()
                            .is_some_and(|block| block > 0)
                        && event["log_index"].as_u64().is_some()
                })
                .collect()
        })
        .unwrap_or_default();
    let created = events.iter().any(|event| {
        event["kind"] == "canonical_bounty_created" && same(&event["data"]["bounty_contract"])
    });
    let funding = events.iter().any(|event| {
        event["kind"] == "funding_added"
            && same(&event["contract_address"])
            && amount(&event["data"]["funded_amount"]).is_some_and(|value| value >= target)
    });
    let claimable = events.iter().any(|event| {
        event["kind"] == "bounty_became_claimable" && same(&event["contract_address"])
    });
    created && funding && claimable
}

fn share_url(contract: &str) -> String {
    format!(
        "https://agentizable.ai/participate.html?bountyContract={}&network=base-mainnet",
        contract.to_ascii_lowercase()
    )
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

    fn confirmed_item(contract: &str) -> Value {
        let id = format!("0x{}", "1".repeat(64));
        let event = |kind: &str, data: Value| json!({"kind":kind,"bounty_id":id,"contract_address":contract,"tx_hash":format!("0x{}", "2".repeat(64)),"block_number":10,"log_index":0,"data":data});
        json!({"bounty_contract":contract,"bounty_id":id,"terms_valid":true,"validation_errors":[],"status":"claimable",
            "target_amount":"2220000","funded_amount":"2220000","events":[
                event("canonical_bounty_created", json!({"bounty_contract":contract})),
                event("funding_added", json!({"funded_amount":"2220000"})),
                event("bounty_became_claimable", json!({}))]})
    }

    #[test]
    fn funded_and_share_link_only_with_full_canonical_evidence() {
        let contract = format!("0x{}", "ab".repeat(20));
        let item = confirmed_item(&contract);
        assert!(funding_confirmed(
            &item,
            &contract.to_uppercase().replace("0X", "0x")
        ));
        assert_eq!(share_url(&contract), format!("https://agentizable.ai/participate.html?bountyContract={contract}&network=base-mainnet"));
        let other = format!("0x{}", "cd".repeat(20));
        assert!(
            !funding_confirmed(&item, &other),
            "another bounty's record never confirms this one"
        );
        for (path, broken) in [
            ("funded_amount", json!("2219999")),
            ("status", json!("funding")),
            ("terms_valid", json!(false)),
            ("validation_errors", json!(["bad terms"])),
            ("target_amount", json!("0")),
        ] {
            let mut copy = item.clone();
            copy[path] = broken;
            assert!(
                !funding_confirmed(&copy, &contract),
                "{path} must block Funded"
            );
        }
        for index in 0..3 {
            let mut copy = item.clone();
            copy["events"].as_array_mut().unwrap().remove(index);
            assert!(
                !funding_confirmed(&copy, &contract),
                "every canonical event is required"
            );
        }
        let mut unconfirmed = item.clone();
        unconfirmed["events"][1]["block_number"] = json!(0);
        assert!(
            !funding_confirmed(&unconfirmed, &contract),
            "an unmined transaction is not funding"
        );
        assert!(!funding_confirmed(&Value::Null, &contract));
    }
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
