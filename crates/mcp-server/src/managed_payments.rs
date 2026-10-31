//! Forward only caller authority to the same managed-service API used by the website.
use super::*;
pub const NAMES: &[&str] = &[
    "issue_managed_service_quote",
    "issue_managed_bounty_quote",
    "read_managed_service_quote",
    "accept_managed_service_quote",
    "get_managed_service_order",
    "open_managed_service_checkout",
    "request_managed_service_refund",
    "get_managed_bounty_review",
    "record_managed_bounty_review",
    "get_card_bounty_review",
    "appeal_card_bounty_review",
];
/// Public card bounty tools: no account; an appeal carries the solver wallet's own signature.
const PUBLIC: &[&str] = &["get_card_bounty_review", "appeal_card_bounty_review"];
pub fn descriptors() -> Vec<ToolDescriptor> {
    NAMES.iter().map(|name|{
    let (description,properties,required)=match *name {
        "issue_managed_service_quote"=>("Merchant operator only: persist an immutable itemized managed-service quote. The API validates the configured merchant, currency, limits and 7.5% fee. No charge or funding occurs.",json!({"quote":{"type":"object"}}),vec!["quote"]),
        "issue_managed_bounty_quote"=>("Price your posting draft for card payment. The server publishes autonomous-v2 terms created by the platform funding wallet, adds verification, the 7.5% fee and disclosed conversion and processing lines, and binds one exact bounty. Your entered reward stays the solver reward. Only customer review is supported. No charge or funding occurs; accept and open Checkout separately. Website session and original allowed Origin required.",json!({"document":{"type":"object","description":"Draft with title, goal, acceptance_criteria, benchmark {engine: customer_review_v1, delivery_deadline}, evidence_schema, optional source_url and image."},"solver_reward_micro_usdc":{"type":"integer","minimum":1}}),vec!["document","solver_reward_micro_usdc"]),
        "read_managed_service_quote"=>("Read your immutable itemized service offer and acceptance digest. Requires your verified website account session; a wallet session is not account authority.",json!({"id":{"type":"string","format":"uuid"}}),vec!["id"]),
        "accept_managed_service_quote"=>("Accept the exact offered digest and retry key as the signed-in buyer. Review solver reward, separate verification costs and 7.5% platform fee first. Acceptance is not payment or funding. Website session and original allowed Origin required.",json!({"id":{"type":"string","format":"uuid"},"quote_digest":{"type":"string","pattern":"^[a-f0-9]{64}$"},"idempotency_key":{"type":"string","maxLength":200}}),vec!["id","quote_digest","idempotency_key"]),
        "get_managed_service_order"=>("Read your managed-service order. Awaiting bank settlement does not mean funded or paid to a subcontractor. Website account session required.",json!({"id":{"type":"string","format":"uuid"}}),vec!["id"]),
        "get_managed_bounty_review"=>("Read your funded card bounty's review state: the canonical submission awaiting your verdict, its published artifact and evidence, the verification deadline, and verdicts already recorded. Website account session required.",json!({"id":{"type":"string","format":"uuid"}}),vec!["id"]),
        "record_managed_bounty_review"=>("Record your verdict on the exact canonical submission shown by get_managed_bounty_review, after inspecting its artifact and before its decide_by time. Silence until decide_by accepts the delivery only when pending_submission.auto_accept is true; for a delivery published too late it is false, nothing is accepted automatically and the submission expires on chain, returning the solver's bond. An acceptance is signed by the platform review key at once and relayed on chain when sponsorship is enabled; it pays the solver. A rejection needs a public reason of 20 to 2,000 characters and waits 48 hours for the solver's appeal before it is signed; it then forfeits the solver's bond and keeps the bounty funded for another attempt. If an appeal reverses your rejection, the solver is paid when the bounty settles and your review deposit is owed to the solver. One verdict per round; it cannot be changed. A signed verdict is not settlement. Website session and original allowed Origin required.",json!({"id":{"type":"string","format":"uuid"},"round":{"type":"integer","minimum":1},"submission_hash":{"type":"string","pattern":"^0x[0-9a-f]{64}$"},"evidence_hash":{"type":"string","pattern":"^0x[0-9a-f]{64}$"},"passed":{"type":"boolean"},"reason":{"type":"string","maxLength":2000}}),vec!["id","round","submission_hash","evidence_hash","passed"]),
        "get_card_bounty_review"=>("Read a card-paid bounty's public review state: the submission awaiting its customer with its decide_by time and whether silence accepts it (auto_accept), each verdict with its reason, appeal and ruling, signed attestations, and appeal_typed_data while a rejection can still be appealed. Names no customer. No authentication.",json!({"network":{"type":"string","enum":["base-mainnet","base-sepolia"]},"bounty_contract":{"type":"string","pattern":"^0x[0-9a-fA-F]{40}$"}}),vec!["network","bounty_contract"]),
        "appeal_card_bounty_review"=>("Appeal a customer's rejection of your own submission on a card-paid bounty before appeal.window_ends_at. Sign appeal_typed_data from get_card_bounty_review with message.reason set to your public reason (20 to 2,000 characters) using the solver wallet, and pass that signature. Never provide a private key or seed phrase. The rejection stays unsigned until Agentizable rules against the published criteria; a reversal pays you and the customer's review deposit, an upheld rejection forfeits your bond. A recorded appeal is not payment.",json!({"network":{"type":"string","enum":["base-mainnet","base-sepolia"]},"bounty_contract":{"type":"string","pattern":"^0x[0-9a-fA-F]{40}$"},"round":{"type":"integer","minimum":1},"solver":{"type":"string","pattern":"^0x[0-9a-fA-F]{40}$"},"reason":{"type":"string","minLength":20,"maxLength":2000},"signature":{"type":"string","pattern":"^0x([0-9a-fA-F]{2})+$"}}),vec!["network","bounty_contract","round","solver","reason","signature"]),
        "request_managed_service_refund"=>("Explicitly cancel your paid service before funding and request a full original-payment refund, including the platform fee. Reserves the order against funding. A recorded request is not a sent refund or bank credit. Website account session and original allowed Origin required.",json!({"id":{"type":"string","format":"uuid"}}),vec!["id"]),
        _=>("Create or recover Checkout for your accepted order under launch controls. Open the returned Stripe URL to pay; never send card data here. A session or return URL is not payment evidence. Website session and original allowed Origin required.",json!({"id":{"type":"string","format":"uuid"}}),vec!["id"]),
    };
    let mut descriptor=tool(name,description,object_tool_schema(properties,&required));
    if PUBLIC.contains(name) {return descriptor;}
    descriptor.authorization=Some(if *name=="issue_managed_service_quote"{ToolAuthorization{kind:"operator",header:"X-Operator-Token",bearer:true,required_when:"Always; merchant quote issuance only."}}else{ToolAuthorization{kind:"website_account",header:"Cookie",bearer:false,required_when:"Always; caller's verified website account session. Writes also require its original allowed Origin. Wallet or analytics tokens are insufficient."}});
    descriptor
}).collect()
}
fn resolve(name: &str, arguments: Value) -> Result<(String, Option<Value>), &'static str> {
    let mut args = arguments.as_object().cloned().ok_or("invalid_arguments")?;
    if name == "issue_managed_service_quote" {
        let quote = args
            .remove("quote")
            .filter(Value::is_object)
            .ok_or("invalid_quote")?;
        if !args.is_empty() {
            return Err("unexpected_arguments");
        }
        return Ok(("/v1/managed-service/quotes".into(), Some(quote)));
    }
    if name == "issue_managed_bounty_quote" {
        let document = args
            .remove("document")
            .filter(Value::is_object)
            .ok_or("invalid_document")?;
        let reward = args
            .remove("solver_reward_micro_usdc")
            .filter(|v| v.as_u64().is_some_and(|reward| reward > 0))
            .ok_or("invalid_solver_reward")?;
        if !args.is_empty() {
            return Err("unexpected_arguments");
        }
        return Ok((
            "/v1/managed-service/bounty-quotes".into(),
            Some(json!({"document": document, "solver_reward_micro_usdc": reward})),
        ));
    }
    if PUBLIC.contains(&name) {
        return card_bounty(name, args);
    }
    let id = args
        .remove("id")
        .and_then(|v| v.as_str().and_then(|s| Uuid::parse_str(s).ok()))
        .ok_or("invalid_id")?;
    let (path, body) = match name {
        "read_managed_service_quote" => (format!("/v1/managed-service/quotes/{id}"), None),
        "get_managed_service_order" => (format!("/v1/managed-service/orders/{id}"), None),
        "request_managed_service_refund" => (
            format!("/v1/managed-service/orders/{id}/refund"),
            Some(json!({})),
        ),
        "open_managed_service_checkout" => (
            format!("/v1/managed-service/orders/{id}/checkout"),
            Some(json!({})),
        ),
        "get_managed_bounty_review" => (format!("/v1/managed-service/orders/{id}/review"), None),
        "record_managed_bounty_review" => {
            let mut verdict = serde_json::Map::new();
            for key in [
                "round",
                "submission_hash",
                "evidence_hash",
                "passed",
                "reason",
            ] {
                if let Some(value) = args.remove(key) {
                    verdict.insert(key.into(), value);
                }
            }
            let valid = verdict
                .get("round")
                .and_then(Value::as_u64)
                .is_some_and(|r| r > 0)
                && ["submission_hash", "evidence_hash"]
                    .iter()
                    .all(|key| verdict.get(*key).is_some_and(Value::is_string))
                && verdict.get("passed").is_some_and(Value::is_boolean)
                && verdict.get("reason").is_none_or(Value::is_string);
            if !valid {
                return Err("invalid_verdict");
            }
            (
                format!("/v1/managed-service/orders/{id}/review"),
                Some(Value::Object(verdict)),
            )
        }
        "accept_managed_service_quote" => {
            let digest = args
                .remove("quote_digest")
                .filter(Value::is_string)
                .ok_or("invalid_digest")?;
            let key = args
                .remove("idempotency_key")
                .filter(Value::is_string)
                .ok_or("invalid_idempotency_key")?;
            (
                format!("/v1/managed-service/quotes/{id}/accept"),
                Some(json!({"quote_digest":digest,"idempotency_key":key})),
            )
        }
        _ => return Err("unknown_tool"),
    };
    if !args.is_empty() {
        return Err("unexpected_arguments");
    }
    Ok((path, body))
}
/// A public card bounty path from an exact network and address, and only the appeal fields.
fn card_bounty(
    name: &str,
    mut args: serde_json::Map<String, Value>,
) -> Result<(String, Option<Value>), &'static str> {
    let network = args
        .remove("network")
        .and_then(|v| v.as_str().map(str::to_owned))
        .filter(|n| n == "base-mainnet" || n == "base-sepolia")
        .ok_or("invalid_network")?;
    let contract = args
        .remove("bounty_contract")
        .and_then(|v| v.as_str().map(str::to_ascii_lowercase))
        .filter(|c| {
            c.len() == 42 && c.starts_with("0x") && c[2..].bytes().all(|b| b.is_ascii_hexdigit())
        })
        .ok_or("invalid_bounty_contract")?;
    let base = format!("/v1/managed-service/bounties/{network}/{contract}");
    if name == "get_card_bounty_review" {
        return if args.is_empty() {
            Ok((format!("{base}/review"), None))
        } else {
            Err("unexpected_arguments")
        };
    }
    let mut appeal = serde_json::Map::new();
    for key in ["round", "solver", "reason", "signature"] {
        appeal.insert(key.into(), args.remove(key).ok_or("invalid_appeal")?);
    }
    let valid = appeal["round"].as_u64().is_some_and(|r| r > 0)
        && ["solver", "reason", "signature"]
            .iter()
            .all(|key| appeal[*key].is_string());
    if !valid {
        return Err("invalid_appeal");
    }
    if !args.is_empty() {
        return Err("unexpected_arguments");
    }
    Ok((format!("{base}/appeal"), Some(Value::Object(appeal))))
}
pub async fn call(name: &str, headers: HeaderMap, args: Value) -> Json<Value> {
    for key in ["authorization", "cookie", "origin", "x-operator-token"] {
        if headers.get_all(key).iter().count() > 1 {
            return mcp_error("ambiguous_caller_credentials");
        }
    }
    match resolve(name, args) {
        Ok((path, body)) => secured_api_tool(&headers, &path, body).await,
        Err(error) => mcp_error(error),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn managed_tool_mapping_preserves_acceptance_and_rejects_user_payment_authority() {
        let id = Uuid::new_v4();
        let (path, body) = resolve(
            "accept_managed_service_quote",
            json!({"id":id,"quote_digest":"abc","idempotency_key":"retry"}),
        )
        .unwrap();
        assert_eq!(path, format!("/v1/managed-service/quotes/{id}/accept"));
        assert_eq!(
            body.unwrap(),
            json!({"quote_digest":"abc","idempotency_key":"retry"})
        );
        for name in [
            "get_managed_service_order",
            "open_managed_service_checkout",
            "read_managed_service_quote",
            "request_managed_service_refund",
        ] {
            assert!(resolve(name, json!({"id":id,"buyer_principal":"attacker"})).is_err());
            assert!(resolve(name, json!({"id":"../stripe/connect-transfers"})).is_err());
        }
        assert_eq!(
            resolve("request_managed_service_refund", json!({"id":id})).unwrap(),
            (
                format!("/v1/managed-service/orders/{id}/refund"),
                Some(json!({}))
            )
        );
        assert!(resolve(
            "open_managed_service_checkout",
            json!({"id":id,"paid":true})
        )
        .is_err());
        assert_eq!(
            resolve("open_managed_service_checkout", json!({"id":id}))
                .unwrap()
                .1,
            Some(json!({}))
        );
    }
    #[test]
    fn card_bounty_quote_forwards_only_the_draft_and_a_positive_reward() {
        let document = json!({"title":"Fixture"});
        assert_eq!(
            resolve(
                "issue_managed_bounty_quote",
                json!({"document":document,"solver_reward_micro_usdc":10_000_000})
            )
            .unwrap(),
            (
                "/v1/managed-service/bounty-quotes".to_string(),
                Some(json!({"document":document,"solver_reward_micro_usdc":10_000_000}))
            )
        );
        for args in [
            json!({"document":document,"solver_reward_micro_usdc":0}),
            json!({"document":document,"solver_reward_micro_usdc":-1}),
            json!({"document":"draft","solver_reward_micro_usdc":1}),
            json!({"document":document,"solver_reward_micro_usdc":1,"buyer_principal":"attacker"}),
        ] {
            assert!(resolve("issue_managed_bounty_quote", args).is_err());
        }
    }
    #[test]
    fn card_bounty_tools_forward_only_an_exact_address_and_the_signed_appeal() {
        let contract = format!("0x{}", "AB".repeat(20));
        let lower = contract.to_ascii_lowercase();
        assert_eq!(
            resolve(
                "get_card_bounty_review",
                json!({"network":"base-sepolia","bounty_contract":contract})
            )
            .unwrap(),
            (
                format!("/v1/managed-service/bounties/base-sepolia/{lower}/review"),
                None
            )
        );
        let appeal = json!({"round":2,"solver":"0x5e","reason":"The report is in section 2.","signature":"0x12"});
        let mut args = appeal.clone();
        args["network"] = json!("base-mainnet");
        args["bounty_contract"] = json!(contract);
        assert_eq!(
            resolve("appeal_card_bounty_review", args.clone()).unwrap(),
            (
                format!("/v1/managed-service/bounties/base-mainnet/{lower}/appeal"),
                Some(appeal)
            )
        );
        for (key, value) in [
            ("network", json!("ethereum")),
            ("bounty_contract", json!("../../appeals")),
            ("round", json!(0)),
            ("signature", json!(null)),
            ("private_key", json!("0x11")),
        ] {
            let mut bad = args.clone();
            bad[key] = value;
            assert!(resolve("appeal_card_bounty_review", bad).is_err(), "{key}");
        }
        assert!(resolve(
            "get_card_bounty_review",
            json!({"network":"base-sepolia","bounty_contract":contract,"id":"x"})
        )
        .is_err());
    }
    #[test]
    fn bounty_review_forwards_only_the_verdict_fields() {
        let id = Uuid::new_v4();
        assert_eq!(
            resolve("get_managed_bounty_review", json!({"id":id})).unwrap(),
            (format!("/v1/managed-service/orders/{id}/review"), None)
        );
        let verdict = json!({"round":1,"submission_hash":"0x11","evidence_hash":"0x22",
            "passed":false,"reason":"The artifact omits the requested report."});
        let mut args = verdict.clone();
        args["id"] = json!(id);
        assert_eq!(
            resolve("record_managed_bounty_review", args).unwrap(),
            (
                format!("/v1/managed-service/orders/{id}/review"),
                Some(verdict)
            )
        );
        for bad in [
            json!({"id":id,"round":0,"submission_hash":"0x11","evidence_hash":"0x22","passed":true}),
            json!({"id":id,"round":1,"submission_hash":"0x11","evidence_hash":"0x22","passed":"yes"}),
            json!({"id":id,"round":1,"evidence_hash":"0x22","passed":true}),
            json!({"id":id,"round":1,"submission_hash":"0x11","evidence_hash":"0x22","passed":true,"signature":"0x00"}),
            json!({"id":"../stripe","round":1,"submission_hash":"0x11","evidence_hash":"0x22","passed":true}),
        ] {
            assert!(resolve("record_managed_bounty_review", bad).is_err());
        }
    }
    #[tokio::test]
    async fn managed_tools_reject_duplicate_credentials_before_proxying() {
        let mut headers = HeaderMap::new();
        headers.append("authorization", "Bearer first".parse().unwrap());
        headers.append("authorization", "Bearer second".parse().unwrap());
        let result = call(
            "get_managed_service_order",
            headers,
            json!({"id":Uuid::new_v4()}),
        )
        .await;
        assert!(result.0["error"].as_str().unwrap().contains("ambiguous"));
    }
}
