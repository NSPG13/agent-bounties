use cloud_agent::{
    CloudAgentService, CloudModelUsage, CloudObjectivePlan, CloudObjectivePlanRequest,
};
use serde::{Deserialize, Serialize};
use std::{env, fs, path::PathBuf, process::ExitCode, time::Instant};

#[derive(Deserialize)]
struct Corpus {
    cases: Vec<Case>,
}

#[derive(Clone, Deserialize)]
struct Case {
    name: String,
    objective: String,
    #[serde(default)]
    context: Option<String>,
    #[serde(default)]
    constraints: Vec<String>,
    max_tasks: u8,
    #[serde(default)]
    solver_budget_usdc: Option<String>,
}

#[derive(Serialize)]
struct CaseResult {
    name: String,
    run: u32,
    duration_ms: u128,
    plan: Option<CloudObjectivePlan>,
    usage: Vec<CloudModelUsage>,
    error: Option<String>,
}

#[derive(Serialize)]
struct BenchmarkResult {
    schema_version: &'static str,
    model: String,
    reasoning_effort: String,
    runs: u32,
    results: Vec<CaseResult>,
}

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(result) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&result).expect("result serializes")
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("cloud-agent benchmark failed: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<BenchmarkResult, String> {
    let corpus_path = env::args_os().nth(1).map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from("benchmarks/openai-build-week/objective-compiler-corpus.json")
    });
    let runs = env::var("CLOUD_AGENT_BENCHMARK_RUNS")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|value| *value > 0 && *value <= 5)
        .unwrap_or(1);
    let model = required_env("CLOUD_AGENT_MODEL")?;
    let reasoning_effort =
        env::var("CLOUD_AGENT_REASONING_EFFORT").unwrap_or_else(|_| "low".to_string());
    if env::var("CLOUD_AGENT_CAPTURE_USAGE").as_deref() != Ok("true") {
        return Err("CLOUD_AGENT_CAPTURE_USAGE=true is required".to_string());
    }
    let corpus: Corpus = serde_json::from_str(
        &fs::read_to_string(&corpus_path)
            .map_err(|error| format!("read {}: {error}", corpus_path.display()))?,
    )
    .map_err(|error| format!("parse {}: {error}", corpus_path.display()))?;
    let service = CloudAgentService::from_env().map_err(|error| error.to_string())?;
    if !service.readiness().available {
        return Err("cloud agent is not ready".to_string());
    }

    let mut results = Vec::with_capacity(corpus.cases.len() * runs as usize);
    for run in 1..=runs {
        for case in &corpus.cases {
            let started = Instant::now();
            let outcome = service
                .compile_objective(CloudObjectivePlanRequest {
                    objective: case.objective.clone(),
                    context: case.context.clone(),
                    constraints: case.constraints.clone(),
                    max_tasks: case.max_tasks,
                    solver_budget_usdc: case.solver_budget_usdc.clone(),
                    source_url: None,
                    idempotency_key: None,
                })
                .await;
            let usage = service.take_usage();
            let duration_ms = started.elapsed().as_millis();
            results.push(match outcome {
                Ok(plan) => CaseResult {
                    name: case.name.clone(),
                    run,
                    duration_ms,
                    plan: Some(plan),
                    usage,
                    error: None,
                },
                Err(error) => CaseResult {
                    name: case.name.clone(),
                    run,
                    duration_ms,
                    plan: None,
                    usage,
                    error: Some(error.to_string()),
                },
            });
        }
    }

    Ok(BenchmarkResult {
        schema_version: "agent-bounties/cloud-model-benchmark-raw-v1",
        model,
        reasoning_effort,
        runs,
        results,
    })
}

fn required_env(key: &str) -> Result<String, String> {
    env::var(key)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("{key} is required"))
}
