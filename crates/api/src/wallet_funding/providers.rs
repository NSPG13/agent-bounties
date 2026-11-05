use super::*;
use base64::{
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    Engine,
};
use hmac::{Hmac, Mac};
use url::Url;

type ProviderResult<T> = Result<T, &'static str>;

fn client() -> ProviderResult<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "provider_unavailable")
}

pub fn configured(provider: &str) -> bool {
    match provider {
        "coinbase" => {
            env::var("COINBASE_ONRAMP_KEY_ID").is_ok()
                && env::var("COINBASE_ONRAMP_KEY_SECRET").is_ok()
        }
        "moonpay" => {
            env::var("MOONPAY_PUBLISHABLE_KEY").is_ok() && env::var("MOONPAY_SECRET_KEY").is_ok()
        }
        "metamask" => true,
        _ => false,
    }
}

async fn coinbase(
    method: &str,
    host: &str,
    path: &str,
    query: &[(&str, &str)],
    body: Option<&Value>,
) -> ProviderResult<Value> {
    let name = env::var("COINBASE_ONRAMP_KEY_ID").map_err(|_| "provider_not_configured")?;
    let secret = env::var("COINBASE_ONRAMP_KEY_SECRET").map_err(|_| "provider_not_configured")?;
    let key = jsonwebtoken::EncodingKey::from_ec_pem(secret.replace("\\n", "\n").as_bytes())
        .map_err(|_| "provider_not_configured")?;
    // CDP ES256 JWT, including a fresh nonce in the protected header.
    let header=URL_SAFE_NO_PAD.encode(serde_json::to_vec(&json!({"typ":"JWT","alg":"ES256","kid":name,"nonce":Uuid::new_v4().simple().to_string()})).map_err(|_|"provider_unavailable")?);
    let now = Utc::now().timestamp();
    let claims=URL_SAFE_NO_PAD.encode(serde_json::to_vec(&json!({"sub":name,"iss":"cdp","nbf":now,"exp":now+120,"uri":format!("{method} {host}{path}")})).map_err(|_|"provider_unavailable")?);
    let data = format!("{header}.{claims}");
    let signature =
        jsonwebtoken::crypto::sign(data.as_bytes(), &key, jsonwebtoken::Algorithm::ES256)
            .map_err(|_| "provider_not_configured")?;
    let mut request = client()?
        .request(
            method.parse().map_err(|_| "provider_unavailable")?,
            format!("https://{host}{path}"),
        )
        .query(query)
        .bearer_auth(format!("{data}.{signature}"));
    if let Some(body) = body {
        request = request.json(body);
    }
    let reply = request.send().await.map_err(|_| "provider_unavailable")?;
    if !reply.status().is_success() {
        return Err(match reply.status().as_u16() {
            400 | 422 => "quote_or_region_unavailable",
            401 | 403 => "provider_not_configured",
            429 => "provider_rate_limited",
            _ => "provider_unavailable",
        });
    }
    reply.json().await.map_err(|_| "provider_response_invalid")
}

pub async fn coinbase_options(country: &str, subdivision: &str) -> ProviderResult<Value> {
    let config = coinbase(
        "GET",
        "api.developer.coinbase.com",
        "/onramp/v1/buy/config",
        &[],
        None,
    )
    .await?;
    let countries = config["countries"]
        .as_array()
        .or_else(|| config["data"]["countries"].as_array())
        .ok_or("provider_response_invalid")?;
    let region = countries
        .iter()
        .find(|v| v["id"] == country)
        .ok_or("region_unavailable")?;
    if country == "US"
        && (subdivision.is_empty()
            || !region["subdivisions"]
                .as_array()
                .is_some_and(|a| a.iter().any(|v| v == subdivision)))
    {
        return Err("subdivision_required_or_unavailable");
    }
    let options = coinbase(
        "GET",
        "api.developer.coinbase.com",
        "/onramp/v1/buy/options",
        &[
            ("country", country),
            ("subdivision", subdivision),
            ("networks", "base"),
        ],
        None,
    )
    .await?;
    let options = options.get("data").unwrap_or(&options);
    if !options["purchase_currencies"]
        .as_array()
        .is_some_and(|currencies| {
            currencies.iter().any(|v| {
                v["symbol"] == "USDC"
                    && v["networks"]
                        .as_array()
                        .is_some_and(|a| a.iter().any(|n| n["name"] == "base"))
            })
        })
    {
        return Err("base_usdc_unavailable");
    }
    Ok(
        json!({"payment_methods":region["payment_methods"],"payment_currencies":options["payment_currencies"]}),
    )
}

pub async fn prepare_coinbase(
    input: &TopupInput,
    attempt: Uuid,
    shortfall: u64,
    return_url: &str,
    ip: &str,
) -> ProviderResult<(String, Value)> {
    let options = coinbase_options(&input.country, &input.subdivision).await?;
    if !options["payment_methods"]
        .as_array()
        .is_some_and(|a| a.iter().any(|v| v["id"] == input.payment_method))
    {
        return Err("payment_method_unavailable");
    }
    let currency = options["payment_currencies"]
        .as_array()
        .and_then(|a| a.iter().find(|v| v["id"] == input.payment_currency))
        .ok_or("currency_unavailable")?;
    let limit = currency["limits"]
        .as_array()
        .and_then(|a| a.iter().find(|v| v["id"] == input.payment_method))
        .ok_or("payment_method_unavailable")?;
    let body = json!({"purchaseCurrency":"USDC","destinationNetwork":"base","destinationAddress":input.wallet,
        "purchaseAmount":units(shortfall),"paymentCurrency":input.payment_currency,"paymentMethod":input.payment_method,
        "country":input.country,"subdivision":input.subdivision,"redirectUrl":return_url,"clientIp":ip,"partnerUserRef":attempt.to_string()});
    let first = coinbase(
        "POST",
        "api.cdp.coinbase.com",
        "/platform/v2/onramp/sessions",
        &[],
        Some(&body),
    )
    .await;
    let result = match first {
        Ok(value) => value,
        // A provider minimum is a real quote input, never a guessed $20 top-up.
        Err("quote_or_region_unavailable") => {
            let minimum = limit["min"].as_str().ok_or("minimum_unavailable")?;
            let mut minimum_body = body;
            minimum_body
                .as_object_mut()
                .ok_or("provider_response_invalid")?
                .remove("purchaseAmount");
            minimum_body["paymentAmount"] = json!(minimum);
            coinbase(
                "POST",
                "api.cdp.coinbase.com",
                "/platform/v2/onramp/sessions",
                &[],
                Some(&minimum_body),
            )
            .await?
        }
        Err(code) => return Err(code),
    };
    let quote = &result["quote"];
    if quote["purchaseCurrency"] != "USDC"
        || quote["destinationNetwork"] != "base"
        || quote["paymentCurrency"] != input.payment_currency
        || number(&quote["paymentTotal"])? <= 0.0
        || !quote["fees"].is_array()
    {
        return Err("provider_response_invalid");
    }
    let received = parse_units(&quote["purchaseAmount"])?;
    if received < shortfall {
        return Err("quote_below_shortfall");
    }
    let url = result["session"]["onrampUrl"]
        .as_str()
        .ok_or("provider_response_invalid")?;
    checked_url(url, &["pay.coinbase.com"])?;
    Ok((
        url.into(),
        json!({"payment_total":quote["paymentTotal"],"payment_currency":quote["paymentCurrency"],"fees":quote["fees"],
        "received_usdc":units(received),"excess_usdc":units(received-shortfall),"minimum_payment":limit["min"],"network":"base-mainnet","wallet":input.wallet}),
    ))
}

async fn moonpay_get(path: &str, query: &[(&str, String)], secret: bool) -> ProviderResult<Value> {
    let key = env::var(if secret {
        "MOONPAY_SECRET_KEY"
    } else {
        "MOONPAY_PUBLISHABLE_KEY"
    })
    .map_err(|_| "provider_not_configured")?;
    let reply = client()?
        .get(format!("https://api.moonpay.com{path}"))
        .query(&[("apiKey", key)])
        .query(query)
        .send()
        .await
        .map_err(|_| "provider_unavailable")?;
    if !reply.status().is_success() {
        return Err(if reply.status().as_u16() == 404 {
            "order_not_found"
        } else {
            "quote_or_region_unavailable"
        });
    }
    reply.json().await.map_err(|_| "provider_response_invalid")
}

pub async fn moonpay_options(input: &TopupInput) -> ProviderResult<Value> {
    if !configured("moonpay") {
        return Err("provider_not_configured");
    }
    let countries = moonpay_get("/v3/countries", &[], false).await?;
    if !countries.as_array().is_some_and(|rows| {
        rows.iter().any(|v| {
            v["alpha2"] == input.country && v["isBuyAllowed"] == true && v["isAllowed"] == true
        })
    }) {
        return Err("region_unavailable");
    }
    let currencies = moonpay_get("/v3/currencies", &[], false).await?;
    let rows = currencies.as_array().ok_or("provider_response_invalid")?;
    let asset = rows
        .iter()
        .find(|v| v["code"] == "usdc_base")
        .ok_or("base_usdc_unavailable")?;
    if asset["isSuspended"] == true
        || asset["notAllowedCountries"]
            .as_array()
            .is_some_and(|rows| rows.iter().any(|v| v == &input.country))
    {
        return Err("region_unavailable");
    }
    if input.country == "US"
        && (input.subdivision.is_empty()
            || asset["isSupportedInUs"] != true
            || asset["notAllowedUSStates"]
                .as_array()
                .is_some_and(|rows| rows.iter().any(|v| v == &input.subdivision)))
    {
        return Err("subdivision_required_or_unavailable");
    }
    let fiats: Vec<Value> = rows
        .iter()
        .filter(|v| v["type"] == "fiat" && v["isSuspended"] != true)
        .filter_map(|v| {
            v["code"]
                .as_str()
                .map(|id| json!({"id":id.to_uppercase(),"minimum":v["minBuyAmount"]}))
        })
        .collect();
    Ok(
        json!({"payment_currencies":fiats,"payment_methods":[{"id":"CARD"}],"card_eligibility":"confirmed_by_provider_at_checkout"}),
    )
}

pub async fn prepare_moonpay(
    input: &TopupInput,
    attempt: Uuid,
    shortfall: u64,
    return_url: &str,
    ip: &str,
) -> ProviderResult<(String, Value)> {
    // The unchanged legacy MCP endpoint remains available. Guided checkout adds
    // a provider quote and external order reference to the same HMAC URL protocol.
    moonpay_options(input).await?;
    if input.payment_method != "CARD" {
        return Err("payment_method_unavailable");
    }
    let currencies = moonpay_get("/v3/currencies", &[], false).await?;
    let currencies = currencies.as_array().ok_or("provider_response_invalid")?;
    let asset = currencies
        .iter()
        .find(|v| v["code"] == "usdc_base")
        .ok_or("base_usdc_unavailable")?;
    if asset["isSuspended"] == true
        || asset["notAllowedCountries"]
            .as_array()
            .is_some_and(|a| a.iter().any(|v| v == &input.country))
    {
        return Err("region_unavailable");
    }
    let fiat = currencies
        .iter()
        .find(|v| v["code"] == input.payment_currency.to_lowercase())
        .ok_or("currency_unavailable")?;
    let minimum = number(&fiat["minBuyAmount"])?;
    let mut amount = minimum;
    let mut quote = Value::Null;
    for _ in 0..3 {
        quote = moonpay_get(
            "/v3/currencies/usdc_base/buy_quote",
            &[
                ("baseCurrencyCode", input.payment_currency.to_lowercase()),
                ("baseCurrencyAmount", format!("{amount:.2}")),
                ("paymentMethod", "credit_debit_card".into()),
                ("areFeesIncluded", "false".into()),
            ],
            false,
        )
        .await?;
        let received = parse_units(&quote["quoteCurrencyAmount"])?;
        if received >= shortfall {
            break;
        }
        let price = number(&quote["quoteCurrencyPrice"])?;
        amount =
            ((amount + (shortfall - received) as f64 / 1_000_000.0 * price) * 100.0).ceil() / 100.0;
    }
    let received = parse_units(&quote["quoteCurrencyAmount"])?;
    if received < shortfall
        || quote["quoteCurrency"]["code"] != "usdc_base"
        || quote["baseCurrency"]["code"] != input.payment_currency.to_lowercase()
        || number(&quote["totalAmount"])? <= 0.0
        || amount > number(&fiat["maxBuyAmount"])?
    {
        return Err("quote_below_shortfall");
    }
    let key = env::var("MOONPAY_PUBLISHABLE_KEY").map_err(|_| "provider_not_configured")?;
    let secret = env::var("MOONPAY_SECRET_KEY").map_err(|_| "provider_not_configured")?;
    // Sandbox must never be described as a real Base wallet top-up.
    if !key.starts_with("pk_live_") || !secret.starts_with("sk_live_") {
        return Err("sandbox_does_not_fund_base");
    }
    let mut url = Url::parse("https://buy.moonpay.com/").map_err(|_| "provider_unavailable")?;
    url.query_pairs_mut().extend_pairs([
        ("apiKey", key.as_str()),
        ("currencyCode", "usdc_base"),
        ("walletAddress", input.wallet.as_str()),
        (
            "baseCurrencyCode",
            input.payment_currency.to_lowercase().as_str(),
        ),
        ("baseCurrencyAmount", format!("{amount:.2}").as_str()),
        ("externalTransactionId", attempt.to_string().as_str()),
        ("redirectURL", return_url),
        ("ipAddress", ip),
    ]);
    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret.as_bytes()).map_err(|_| "provider_not_configured")?;
    mac.update(format!("?{}", url.query().ok_or("provider_unavailable")?).as_bytes());
    url.query_pairs_mut()
        .append_pair("signature", &STANDARD.encode(mac.finalize().into_bytes()));
    Ok((
        url.to_string(),
        json!({"expires_at":quote["expiresAt"],"payment_total":quote["totalAmount"],"payment_currency":input.payment_currency,"fees":{"provider":quote["feeAmount"],"network":quote["networkFeeAmount"]},
        "received_usdc":units(received),"excess_usdc":units(received-shortfall),"minimum_payment":minimum,"network":"base-mainnet","wallet":input.wallet}),
    ))
}

pub async fn reconcile(attempt: &db::WalletTopupAttempt) -> ProviderResult<(&'static str, Value)> {
    let value = match attempt.provider.as_str() {
        "coinbase" => {
            coinbase(
                "GET",
                "api.developer.coinbase.com",
                &format!("/onramp/v1/buy/user/{}/transactions", attempt.id),
                &[],
                None,
            )
            .await?
        }
        "moonpay" => {
            moonpay_get(&format!("/v1/transactions/ext/{}", attempt.id), &[], true).await?
        }
        _ => return Ok(("unknown", json!({}))),
    };
    Ok(reconcile_response(
        &attempt.provider,
        &attempt.wallet,
        &value,
    ))
}

fn reconcile_response(provider: &str, wallet: &str, value: &Value) -> (&'static str, Value) {
    let unknown = || ("unknown", json!({}));
    let row = if provider == "coinbase" {
        let Some(rows) = value["transactions"].as_array() else {
            return unknown();
        };
        // One reference must identify exactly one delivery; extra orders are ambiguous.
        if rows.len() != 1
            || value["next_page_key"]
                .as_str()
                .is_some_and(|v| !v.is_empty())
        {
            return unknown();
        }
        let row = &rows[0];
        if !row["wallet_address"]
            .as_str()
            .is_some_and(|v| v.eq_ignore_ascii_case(wallet))
            || row["purchase_network"] != "base"
            || row["purchase_currency"] != "USDC"
        {
            return unknown();
        }
        row
    } else if provider == "moonpay" {
        let row = if let Some(rows) = value.as_array() {
            if rows.len() != 1 {
                return unknown();
            }
            &rows[0]
        } else {
            value
        };
        if !row["walletAddress"]
            .as_str()
            .is_some_and(|v| v.eq_ignore_ascii_case(wallet))
            || row["currency"]["code"] != "usdc_base"
        {
            return unknown();
        }
        row
    } else {
        return unknown();
    };
    let (status, code) = match row["status"].as_str() {
        Some("completed" | "ONRAMP_TRANSACTION_STATUS_SUCCESS") => ("completed", None),
        Some("failed" | "ONRAMP_TRANSACTION_STATUS_FAILED") => {
            ("failed", Some("provider_purchase_failed"))
        }
        Some("waitingAuthorization") => ("pending", Some("bank_authentication_required")),
        Some("waitingPayment") => ("pending", Some("payment_pending")),
        _ => ("pending", None),
    };
    let reference = row[if provider == "moonpay" {
        "id"
    } else {
        "transaction_id"
    }]
    .as_str()
    .and_then(|v| Uuid::parse_str(v).ok());
    let support = if provider == "moonpay" {
        reference
            .map(|id| format!("https://buy.moonpay.com/transaction_receipt?transactionId={id}"))
            .unwrap_or_else(|| "https://support.moonpay.com/".into())
    } else {
        "https://help.coinbase.com/en/coinbase/trading-and-funding/coinbase-pay/using-onramp".into()
    };
    // Do not retain raw provider responses, free-text failure reasons, bank data,
    // credentials or customer identifiers in diagnostics or the public API.
    (
        status,
        json!({"recovery_code":code,"order_reference":reference,"support_url":support}),
    )
}

pub fn checked_url(value: &str, hosts: &[&str]) -> ProviderResult<()> {
    let url = Url::parse(value).map_err(|_| "provider_response_invalid")?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || !hosts.contains(&url.host_str().unwrap_or(""))
        || url.port().is_some()
    {
        return Err("provider_response_invalid");
    }
    Ok(())
}
fn number(value: &Value) -> ProviderResult<f64> {
    let n = value
        .as_f64()
        .or_else(|| value.as_str()?.parse().ok())
        .ok_or("provider_response_invalid")?;
    if n.is_finite() && n >= 0.0 {
        Ok(n)
    } else {
        Err("provider_response_invalid")
    }
}
pub fn units(value: u64) -> String {
    format!("{}.{:06}", value / 1_000_000, value % 1_000_000)
}
pub fn parse_units(value: &Value) -> ProviderResult<u64> {
    let text = match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => return Err("provider_response_invalid"),
    };
    let mut parts = text.split('.');
    let whole = parts.next().ok_or("provider_response_invalid")?;
    let fraction = parts.next().unwrap_or("");
    if parts.next().is_some()
        || whole.is_empty()
        || !whole.chars().all(|c| c.is_ascii_digit())
        || fraction.len() > 6
        || !fraction.chars().all(|c| c.is_ascii_digit())
    {
        return Err("provider_response_invalid");
    }
    whole
        .parse::<u64>()
        .ok()
        .and_then(|v| v.checked_mul(1_000_000))
        .and_then(|v| v.checked_add(format!("{fraction:0<6}").parse().ok()?))
        .ok_or("provider_response_invalid")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reconciliation_rejects_wrong_wallet_network_and_ambiguous_orders() {
        let wallet = "0x1111111111111111111111111111111111111111";
        let row = json!({"wallet_address":wallet,"purchase_network":"base","purchase_currency":"USDC","status":"ONRAMP_TRANSACTION_STATUS_SUCCESS"});
        assert_eq!(
            reconcile_response("coinbase", wallet, &json!({"transactions":[row.clone()]})).0,
            "completed"
        );
        assert_eq!(
            reconcile_response(
                "coinbase",
                "0x2222222222222222222222222222222222222222",
                &json!({"transactions":[row.clone()]})
            )
            .0,
            "unknown"
        );
        assert_eq!(
            reconcile_response(
                "coinbase",
                wallet,
                &json!({"transactions":[row.clone(),row.clone()]})
            )
            .0,
            "unknown"
        );
        let mut wrong = row;
        wrong["purchase_network"] = json!("ethereum");
        assert_eq!(
            reconcile_response("coinbase", wallet, &json!({"transactions":[wrong]})).0,
            "unknown"
        );
    }
    #[test]
    fn bank_authentication_and_generic_declines_have_sanitized_recovery() {
        let wallet = "0x1111111111111111111111111111111111111111";
        let mut row = json!({"walletAddress":wallet,"currency":{"code":"usdc_base"},"status":"waitingAuthorization","id":"11111111-1111-4111-8111-111111111111","failureReason":"sensitive untrusted provider text","cardId":"secret"});
        let (status, details) = reconcile_response("moonpay", wallet, &row);
        assert_eq!(status, "pending");
        assert_eq!(details["recovery_code"], "bank_authentication_required");
        assert!(details["support_url"]
            .as_str()
            .unwrap()
            .contains("transaction_receipt?transactionId="));
        row["status"] = json!("failed");
        let (status, details) = reconcile_response("moonpay", wallet, &row);
        assert_eq!(status, "failed");
        assert_eq!(details["recovery_code"], "provider_purchase_failed");
        assert!(!details.to_string().contains("secret"));
        assert!(!details.to_string().contains("sensitive"));
    }
    #[test]
    fn quote_units_are_exact_and_fail_closed() {
        assert_eq!(parse_units(&json!("2.010001")), Ok(2_010_001));
        for bad in ["-1", "NaN", "1e9", "1.0000001", "18446744073709551615"] {
            assert!(parse_units(&json!(bad)).is_err());
        }
    }
    #[test]
    fn checkout_host_validation_rejects_credentials_and_lookalikes() {
        for bad in [
            "https://pay.coinbase.com.evil.test/buy",
            "https://user@pay.coinbase.com/buy",
            "http://pay.coinbase.com/buy",
            "https://pay.coinbase.com:444/buy",
        ] {
            assert!(checked_url(bad, &["pay.coinbase.com"]).is_err());
        }
        assert!(checked_url(
            "https://pay.coinbase.com/buy?sessionToken=test",
            &["pay.coinbase.com"]
        )
        .is_ok());
    }
}
