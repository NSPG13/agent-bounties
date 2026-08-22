use competition_metric_core::{execute_canonical_gmv_program, CanonicalGmvProgramInput};
use sha2::{Digest, Sha256};
use sp1_sdk::{Elf, HashableKey, ProveRequest, Prover, ProverClient, ProvingKey, SP1Stdin};
use std::{env, fs, path::PathBuf};
use tiny_keccak::{Hasher, Keccak};

const ELF: Elf = Elf::Static(include_bytes!(env!("OPEN_COMPETITION_V2_METRIC_ELF")));

#[tokio::main]
async fn main() {
    let mut args = env::args().skip(1);
    let first = args
        .next()
        .expect("usage: script --capabilities | SNAPSHOT_JSON [execute|groth16|plonk|release-candidate] [PROOF_OUT_OR_SOURCE_HASH]");
    if first == "--capabilities" {
        assert!(args.next().is_none(), "unexpected capability argument");
        let mut backends = vec!["cpu"];
        if cfg!(feature = "network") {
            backends.push("network");
        }
        println!(
            "{}",
            serde_json::json!({
                "schema_version": "agent-bounties/open-competition-v2-prover-capabilities-v1",
                "profile_id": "canonical-gmv-attribution-metric-v1",
                "sp1_version": "6.4.0-agent-bounties-sp1-safe-v5",
                "sp1_commit": "f6a2dffc42c322d0a6d8f5b5ae06fb76986ae12d",
                "sp1_circuit_commit": "f6a2dffc42c322d0a6d8f5b5ae06fb76986ae12d",
                "sp1_runtime_commit": "c2d292c260333a9e4f166cd1435e8ef4897c8b43",
                "gpu_proving_enabled": false,
                "backends": backends,
                "proof_systems": ["groth16", "plonk"]
            })
        );
        return;
    }
    let fixture_path = first;
    let mode = args.next().unwrap_or_else(|| "execute".to_string());
    let extra = args.next();
    assert!(args.next().is_none(), "unexpected extra argument");

    let mut input: CanonicalGmvProgramInput =
        serde_json::from_slice(&fs::read(&fixture_path).expect("failed to read snapshot JSON"))
            .expect("snapshot JSON does not match CanonicalGmvProgramInput");

    let client = ProverClient::from_env().await;
    let pk = client.setup(ELF).await.expect("SP1 setup failed");
    let vkey_raw = pk.verifying_key().bytes32_raw();
    let vkey = pk.verifying_key().bytes32();
    let elf_keccak256 = keccak256(&ELF);
    let elf_sha256 = Sha256::digest(&*ELF);
    if mode == "release-candidate" {
        let source_hash = extra
            .as_deref()
            .expect("release-candidate mode requires the canonical source hash");
        input.scope.program_vkey = vkey_raw;
        input.scope.source_hash = parse_bytes32_hex(source_hash, "source_hash");
        input.scope.elf_hash = elf_keccak256;
    }
    let expected = execute_canonical_gmv_program(&input).expect("snapshot input is invalid");
    let mut stdin = SP1Stdin::new();
    stdin.write(&input);
    match mode.as_str() {
        "execute" | "release-candidate" => {
            if mode == "execute" {
                assert!(
                    extra.is_none(),
                    "execute mode does not accept an extra argument"
                );
            }
            let (public_values, report) = client
                .execute(ELF, stdin)
                .await
                .expect("SP1 execution failed");
            assert_eq!(public_values.as_slice(), expected.journal);
            println!(
                "{}",
                serde_json::json!({
                    "mode": "execute",
                    "profile_id": "canonical-gmv-attribution-metric-v1",
                    "program_vkey": vkey,
                    "elf_keccak256": format!("0x{}", hex::encode(elf_keccak256)),
                    "elf_sha256": hex::encode(elf_sha256),
                    "journal_hex": format!("0x{}", hex::encode(public_values.as_slice())),
                    "cycles": report.total_instruction_count() + report.total_syscall_count(),
                    "release_candidate": mode == "release-candidate"
                })
            );
        }
        "groth16" | "plonk" => {
            let proof_out = extra.map(PathBuf::from);
            let proof = if mode == "groth16" {
                client
                    .prove(&pk, stdin)
                    .groth16()
                    .await
                    .expect("Groth16 proving failed")
            } else {
                client
                    .prove(&pk, stdin)
                    .plonk()
                    .await
                    .expect("PLONK proving failed")
            };
            client
                .verify(&proof, pk.verifying_key(), None)
                .expect("proof verification failed");
            assert_eq!(proof.public_values.as_slice(), expected.journal);
            if let Some(path) = proof_out {
                proof.save(path).expect("failed to save proof");
            }
            println!(
                "{}",
                serde_json::json!({
                    "mode": mode,
                    "profile_id": "canonical-gmv-attribution-metric-v1",
                    "program_vkey": vkey,
                    "elf_keccak256": format!("0x{}", hex::encode(elf_keccak256)),
                    "elf_sha256": hex::encode(elf_sha256),
                    "proof_hex": format!("0x{}", hex::encode(proof.bytes())),
                    "journal_hex": format!("0x{}", hex::encode(proof.public_values.as_slice()))
                })
            );
        }
        _ => panic!("mode must be execute, groth16, plonk, or release-candidate"),
    }
}

fn parse_bytes32_hex(value: &str, field: &str) -> [u8; 32] {
    let encoded = value
        .strip_prefix("0x")
        .unwrap_or_else(|| panic!("{field} must be 0x-prefixed bytes32 hex"));
    let decoded = hex::decode(encoded).unwrap_or_else(|_| panic!("{field} must be bytes32 hex"));
    decoded
        .try_into()
        .unwrap_or_else(|_| panic!("{field} must be exactly 32 bytes"))
}

fn keccak256(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Keccak::v256();
    hasher.update(bytes);
    let mut output = [0_u8; 32];
    hasher.finalize(&mut output);
    output
}
