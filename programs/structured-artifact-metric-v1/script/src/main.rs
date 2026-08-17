use competition_metric_core::{
    execute_structured_artifact_program, StructuredArtifactProgramInput,
    StructuredArtifactProgramWireInput,
};
use sha2::{Digest, Sha256};
use sp1_sdk::{
    Elf, HashableKey, ProveRequest, Prover, ProverClient, ProvingKey, SP1Stdin,
};
use std::{env, fs, path::PathBuf};
use tiny_keccak::{Hasher, Keccak};

const ELF: Elf = Elf::Static(include_bytes!(env!("OPEN_COMPETITION_V2_METRIC_ELF")));

#[tokio::main]
async fn main() {
    let mut args = env::args().skip(1);
    let first = args.next().expect(
        "usage: script --capabilities | FIXTURE_JSON [execute|groth16|plonk] [PROOF_OUT]",
    );
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
                "profile_id": "structured-artifact-metric-v1",
                "sp1_version": "6.4.0-agent-bounties-sp1-safe-v5",
                "sp1_commit": "f6a2dffc42c322d0a6d8f5b5ae06fb76986ae12d",
                "sp1_circuit_commit": "f6a2dffc42c322d0a6d8f5b5ae06fb76986ae12d",
                "sp1_runtime_commit": "be3678e4374484782ac203d596c2103b8ae43352",
                "gpu_proving_enabled": false,
                "backends": backends,
                "proof_systems": ["groth16", "plonk"]
            })
        );
        return;
    }
    let fixture_path = first;
    let mode = args.next().unwrap_or_else(|| "execute".to_string());
    let proof_out = args.next().map(PathBuf::from);
    assert!(args.next().is_none(), "unexpected extra argument");

    let input: StructuredArtifactProgramInput =
        serde_json::from_slice(&fs::read(&fixture_path).expect("failed to read fixture JSON"))
            .expect("fixture JSON does not match StructuredArtifactProgramInput");
    let expected = execute_structured_artifact_program(&input).expect("fixture is invalid");
    let mut stdin = SP1Stdin::new();
    stdin.write(&StructuredArtifactProgramWireInput::from(&input));

    let client = ProverClient::from_env().await;
    let pk = client.setup(ELF).await.expect("SP1 setup failed");
    let vkey = pk.verifying_key().bytes32();
    let elf_keccak256 = keccak256(&ELF);
    let elf_sha256 = Sha256::digest(&*ELF);
    match mode.as_str() {
        "execute" => {
            let (public_values, report) = client
                .execute(ELF, stdin)
                .await
                .expect("SP1 execution failed");
            if public_values.as_slice() != expected.journal {
                eprintln!("structured artifact guest execution report: {report:#?}");
            }
            assert_eq!(public_values.as_slice(), expected.journal);
            println!(
                "{}",
                serde_json::json!({
                    "mode": "execute",
                    "profile_id": "structured-artifact-metric-v1",
                    "program_vkey": vkey,
                    "elf_keccak256": format!("0x{}", hex::encode(elf_keccak256)),
                    "elf_sha256": hex::encode(elf_sha256),
                    "journal_hex": format!("0x{}", hex::encode(public_values.as_slice())),
                    "cycles": report.total_instruction_count() + report.total_syscall_count()
                })
            );
        }
        "groth16" | "plonk" => {
            let proof = if mode == "groth16" {
                client.prove(&pk, stdin).groth16().await.expect("Groth16 proving failed")
            } else {
                client.prove(&pk, stdin).plonk().await.expect("PLONK proving failed")
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
                    "profile_id": "structured-artifact-metric-v1",
                    "program_vkey": vkey,
                    "elf_keccak256": format!("0x{}", hex::encode(elf_keccak256)),
                    "elf_sha256": hex::encode(elf_sha256),
                    "proof_hex": format!("0x{}", hex::encode(proof.bytes())),
                    "journal_hex": format!("0x{}", hex::encode(proof.public_values.as_slice()))
                })
            );
        }
        _ => panic!("mode must be execute, groth16, or plonk"),
    }
}

fn keccak256(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Keccak::v256();
    hasher.update(bytes);
    let mut output = [0_u8; 32];
    hasher.finalize(&mut output);
    output
}
