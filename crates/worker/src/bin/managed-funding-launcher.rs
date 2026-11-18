//! Bound admission for one managed-funding worker. No DB, RPC or signing code.
//! Secrets enter only a cleared exec environment after explicit admission.
use std::{env, process::Command, time::Duration};

const WORKER: &str = "/usr/local/bin/managed-funding-worker";
const DENIED: &str = include_str!("../../../../ops/recovery-reader-denied-services.txt");
const SETTINGS: &[&str] = &[
    "DATABASE_URL",
    "BASE_MAINNET_RPC_URL",
    "MANAGED_BOUNTY_FUNDING",
    "MANAGED_BOUNTY_FUNDING_DELEGATE_PRIVATE_KEY",
];
const V2: &[(&str, &str)] = &[
    (
        "BASE_MAINNET_BOUNTY_V2_FACTORY",
        "0xc33e2ae33bb9580837ea59df18e57fa1039ae58a",
    ),
    (
        "BASE_MAINNET_BOUNTY_V2_IMPLEMENTATION",
        "0x8420c9bd1ff8a6b1abc4230b349538612c58f3a6",
    ),
    ("BASE_MAINNET_BOUNTY_V2_PLATFORM_FEE_BPS", "750"),
    (
        "BASE_MAINNET_BOUNTY_V2_PLATFORM_FEE_RECIPIENT",
        "0x884834e884d6e93462655a2820140ad03e6747bc",
    ),
];
type Result<T> = std::result::Result<T, &'static str>;

#[derive(Clone, Copy)]
struct Binding<'a> {
    service: &'a str,
    revision: &'a str,
}
fn full_revision(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn binding<'a>(service: Option<&'a str>, revision: Option<&'a str>) -> Result<Option<Binding<'a>>> {
    match (service, revision) {
        (None, None) => Ok(None),
        (Some(service), Some(revision))
            if service.len() == 24
                && service.starts_with("srv-")
                && service[4..]
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
                && !DENIED.lines().any(|id| id == service)
                && full_revision(revision) =>
        {
            Ok(Some(Binding { service, revision }))
        }
        _ => Err("Invalid compiled funding identity"),
    }
}
fn identity(value: Option<Binding<'_>>) -> String {
    let (binding, unbound) = match value {
        Some(value) => (
            format!(
                "{{\"service_id\":\"{}\",\"source_revision\":\"{}\",\"capability\":\"funding\"}}",
                value.service, value.revision
            ),
            false,
        ),
        None => ("null".to_owned(), true),
    };
    format!("{{\"schema\":\"agentizable/recovery-funding-build-v1\",\"binding\":{binding},\"unbound_inert_only\":{unbound}}}")
}
fn admitted_mode<F>(bound: Option<Binding<'_>>, lookup: &mut F) -> Result<bool>
where
    F: FnMut(&str) -> Option<String>,
{
    let mode = lookup("RECOVERY_FUNDING_MODE").unwrap_or_else(|| "held".into());
    if mode != "held" && mode != "active" {
        return Err("Invalid funding mode");
    }
    if let Some(bound) = bound {
        if lookup("RENDER_SERVICE_ID").as_deref() != Some(bound.service)
            || lookup("RENDER_GIT_COMMIT").as_deref() != Some(bound.revision)
        {
            return Err("Funding runtime identity does not match its artifact");
        }
    } else if mode == "active" {
        return Err("Unbound funding artifact is inert only");
    }
    if mode == "held" {
        return Ok(false);
    }
    if lookup("ENABLE_HOSTED_FINANCIAL_SIGNING").as_deref() != Some("true")
        || lookup("BASE_INDEXER_PROTOCOL").as_deref() != Some("managed-bounty-funding")
    {
        return Err("Managed funding is not explicitly admitted");
    }
    Ok(true)
}
fn valid_private_scalar(value: &str) -> bool {
    let value = value.strip_prefix("0x").unwrap_or(value);
    value.len() == 64
        && value.bytes().all(|b| b.is_ascii_hexdigit())
        && value.bytes().any(|b| b != b'0')
        && value.to_ascii_lowercase().as_str()
            < "fffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364141"
}
fn worker_command<F>(bound: Binding<'_>, lookup: &mut F) -> Result<Command>
where
    F: FnMut(&str) -> Option<String>,
{
    let mut command = Command::new(WORKER);
    command.env_clear();
    command
        .env("RENDER_SERVICE_ID", bound.service)
        .env("RENDER_GIT_COMMIT", bound.revision)
        .env("RECOVERY_FUNDING_MODE", "active")
        .env("ENABLE_HOSTED_FINANCIAL_SIGNING", "true")
        .env("BASE_INDEXER_PROTOCOL", "managed-bounty-funding")
        .env("RUST_LOG", "info");
    // Validate public identity before accessing any financial/dependency slot.
    for (key, expected) in V2 {
        if lookup(key).as_deref() != Some(*expected) {
            return Err("Noncanonical funding deployment");
        }
        command.env(key, expected);
    }
    for key in SETTINGS {
        let value = lookup(key).ok_or("Required funding configuration unavailable")?;
        if value.is_empty() || value.len() > 4096 || value.contains('\0') {
            return Err("Invalid funding configuration");
        }
        if *key == "MANAGED_BOUNTY_FUNDING_DELEGATE_PRIVATE_KEY" && !valid_private_scalar(&value) {
            return Err("Invalid funding delegate credential");
        }
        command.env(key, value);
    }
    Ok(command)
}
fn run() -> Result<()> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    let bound = binding(
        option_env!("RECOVERY_FUNDING_BOUND_SERVICE_ID"),
        option_env!("RECOVERY_FUNDING_BOUND_REVISION"),
    )?;
    if args == ["--build-identity"] {
        println!("{}", identity(bound));
        return Ok(());
    }
    if !args.is_empty() && args != ["--check-binding"] {
        return Err("Funding launcher arguments rejected");
    }
    let mut lookup = |key: &str| env::var(key).ok();
    let active = admitted_mode(bound, &mut lookup)?;
    if args == ["--check-binding"] {
        return Ok(());
    }
    if !active {
        // No dependency lookup, child, migration or signer. SIGTERM terminates
        // the process normally at the OS boundary; no catch-up work runs here.
        loop {
            std::thread::park_timeout(Duration::from_secs(3600));
        }
    }
    let mut command = worker_command(bound.ok_or("Missing funding identity")?, &mut lookup)?;
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let _ = command.exec();
        Err("Admitted funding worker could not start")
    }
    #[cfg(not(unix))]
    {
        let _ = &mut command;
        Err("Funding launcher requires Unix exec")
    }
}
fn main() {
    if let Err(error) = run() {
        // Errors are fixed labels; never format a Command or environment value.
        eprintln!("{error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    const SERVICE: &str = "srv-aaaaaaaaaaaaaaaaaaaa";
    const REVISION: &str = "1111111111111111111111111111111111111111";
    fn bound() -> Binding<'static> {
        binding(Some(SERVICE), Some(REVISION)).unwrap().unwrap()
    }
    fn settings() -> BTreeMap<String, String> {
        let mut values = BTreeMap::from([
            ("RENDER_SERVICE_ID".into(), SERVICE.into()),
            ("RENDER_GIT_COMMIT".into(), REVISION.into()),
            ("RECOVERY_FUNDING_MODE".into(), "active".into()),
            ("ENABLE_HOSTED_FINANCIAL_SIGNING".into(), "true".into()),
            (
                "BASE_INDEXER_PROTOCOL".into(),
                "managed-bounty-funding".into(),
            ),
        ]);
        for (key, value) in V2 {
            values.insert((*key).into(), (*value).into());
        }
        for key in SETTINGS {
            values.insert((*key).into(), "synthetic-slot".into());
        }
        values.insert(
            "MANAGED_BOUNTY_FUNDING_DELEGATE_PRIVATE_KEY".into(),
            format!("{:064x}", 1),
        );
        values
    }
    #[test]
    fn binding_rejects_partial_malformed_and_existing_services() {
        assert!(binding(None, None).unwrap().is_none());
        assert!(binding(Some(SERVICE), None).is_err());
        assert!(binding(None, Some(REVISION)).is_err());
        for service in DENIED.lines().chain(["bad", "srv-AAAAAAAAAAAAAAAAAAAA"]) {
            assert!(binding(Some(service), Some(REVISION)).is_err());
        }
        for revision in [
            "",
            "A",
            "gggggggggggggggggggggggggggggggggggggggg",
            "111111111111111111111111111111111111111\n",
        ] {
            assert!(binding(Some(SERVICE), Some(revision)).is_err());
        }
    }
    #[test]
    fn held_and_unbound_modes_never_read_sensitive_or_protocol_settings() {
        let mut values = settings();
        values.insert("RECOVERY_FUNDING_MODE".into(), "held".into());
        let mut read = vec![];
        assert!(!admitted_mode(Some(bound()), &mut |key| {
            read.push(key.to_owned());
            values.get(key).cloned()
        })
        .unwrap());
        assert_eq!(
            read,
            vec![
                "RECOVERY_FUNDING_MODE",
                "RENDER_SERVICE_ID",
                "RENDER_GIT_COMMIT"
            ]
        );
        assert!(!admitted_mode(None, &mut |_| None).unwrap());
        assert!(
            admitted_mode(None, &mut |key| values.get(key).cloned().map(|v| {
                if key == "RECOVERY_FUNDING_MODE" {
                    "active".into()
                } else {
                    v
                }
            }))
            .is_err()
        );
    }
    #[test]
    fn wrong_identity_mode_global_or_protocol_rejects_before_dependencies() {
        for (key, value) in [
            ("RENDER_SERVICE_ID", "srv-bbbbbbbbbbbbbbbbbbbb"),
            ("RENDER_GIT_COMMIT", "wrong"),
            ("RECOVERY_FUNDING_MODE", "true"),
            ("ENABLE_HOSTED_FINANCIAL_SIGNING", "false"),
            ("ENABLE_HOSTED_FINANCIAL_SIGNING", "TRUE"),
            ("BASE_INDEXER_PROTOCOL", "autonomous-v1"),
        ] {
            let mut values = settings();
            values.insert(key.into(), value.into());
            assert!(admitted_mode(Some(bound()), &mut |key| {
                assert!(!SETTINGS.contains(&key));
                values.get(key).cloned()
            })
            .is_err());
        }
    }
    #[test]
    fn command_has_no_args_and_only_exact_selected_environment() {
        let mut values = settings();
        values.insert("STRIPE_SECRET_KEY".into(), "do-not-inherit".into());
        values.insert("LD_PRELOAD".into(), "do-not-inherit".into());
        let mut read = vec![];
        let command = worker_command(bound(), &mut |key| {
            read.push(key.to_owned());
            values.get(key).cloned()
        })
        .unwrap();
        assert_eq!(command.get_program(), WORKER);
        assert_eq!(command.get_args().count(), 0);
        let env: BTreeMap<_, _> = command
            .get_envs()
            .map(|(k, v)| (k.to_str().unwrap(), v.unwrap().to_str().unwrap()))
            .collect();
        assert_eq!(env.len(), 14);
        assert!(!env.contains_key("STRIPE_SECRET_KEY") && !env.contains_key("LD_PRELOAD"));
        assert_eq!(env["BASE_INDEXER_PROTOCOL"], "managed-bounty-funding");
        assert_eq!(env["RUST_LOG"], "info");
        assert!(read
            .iter()
            .all(|key| SETTINGS.contains(&key.as_str()) || V2.iter().any(|(name, _)| name == key)));
    }
    #[test]
    fn public_pin_failure_precedes_sensitive_reads() {
        let mut values = settings();
        values.insert(V2[0].0.into(), "wrong".into());
        assert!(worker_command(bound(), &mut |key| {
            assert!(!SETTINGS.contains(&key));
            values.get(key).cloned()
        })
        .is_err());
    }
    #[test]
    fn missing_empty_oversize_and_invalid_delegate_fail_with_fixed_errors() {
        for key in SETTINGS {
            for value in [None, Some(String::new()), Some("x".repeat(4097))] {
                let mut values = settings();
                values.remove(*key);
                if let Some(value) = value {
                    values.insert((*key).into(), value);
                }
                let err = worker_command(bound(), &mut |key| values.get(key).cloned())
                    .err()
                    .unwrap();
                assert!(matches!(
                    err,
                    "Required funding configuration unavailable" | "Invalid funding configuration"
                ));
            }
        }
        for key in ["invalid", "0x0", &"0".repeat(64), &"f".repeat(64)] {
            assert!(!valid_private_scalar(key));
        }
        assert!(valid_private_scalar(&format!("0x{:064x}", 1)));
    }
    #[test]
    fn build_identity_contains_only_validated_public_binding() {
        assert!(identity(Some(bound())).contains("\"capability\":\"funding\""));
        assert!(identity(None).contains("\"unbound_inert_only\":true"));
    }
}
