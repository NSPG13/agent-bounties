//! One proxy path for direct and remote MCP. Caller credentials only.
use super::*;
pub const NAMES: &[&str] = &[
    "find_verification_checks",
    "validate_verification_plan",
    "run_verification_checks",
    "get_verification_run",
    "get_verification_artifact",
    "upload_verification_artifact",
    "import_verification_github",
    "list_review_tasks",
    "assign_review_task",
    "accept_review_task",
    "save_verification_review",
    "get_verification_review",
    "prepare_artifact_submission",
    "confirm_artifact_submission",
    "open_artifact_dispute",
    "resolve_artifact_dispute",
];
pub fn descriptors() -> Vec<ToolDescriptor> {
    NAMES.iter().map(|name|{
        let (description,properties,required)=match *name {
            "find_verification_checks"=>("Find versioned broad checks, exact parameter schemas, limits and what each check proves. Checks do not approve payment.",json!({}),vec![]),
            "validate_verification_plan"=>("Validate complete criterion coverage and checker compatibility. Get the plan schema from find_verification_checks. The creator must accept the meaning before funding.",json!({"plan":{"type":"object"}}),vec!["plan"]),
            "run_verification_checks"=>("Queue checks on exact durably stored files. Returns an existing run for identical inputs. Needs a verified website or wallet session; never accepts solver-supplied test outcomes.",json!({"artifact_id":{"type":"string","format":"uuid"},"plan":{"type":"object"}}),vec!["artifact_id","plan"]),
            "get_verification_run"=>("Read the caller's stored check report. Pass is limited to each check's claim; missing review is not approval.",json!({"id":{"type":"string","format":"uuid"}}),vec!["id"]),
            "get_verification_artifact"=>("Read an authorized immutable file manifest.",json!({"id":{"type":"string","format":"uuid"}}),vec!["id"]),
            "upload_verification_artifact"=>("Store authenticated exact file bytes before a submission transaction. Base64 input. Remote MCP requests are limited to 2 MiB including encoding; for larger files use the API/CLI (100 MiB total) or exact GitHub commit import. Idempotency key required.",json!({"idempotency_key":{"type":"string","maxLength":128},"files":{"type":"array","maxItems":1024,"items":{"type":"object","properties":{"path":{"type":"string"},"base64":{"type":"string"}},"required":["path","base64"],"additionalProperties":false}}}),vec!["idempotency_key","files"]),
            "list_review_tasks"=>("List your pending review tasks and deadlines. Assignment grants no wallet signing authority.",json!({}),vec![]),
            "assign_review_task"=>("Assign your review task to a verified agent principal. The agent must accept readiness. Your existing authorized wallet still signs the verdict on a first-party page.",json!({"id":{"type":"string","format":"uuid"},"assignee_principal":{"type":"string"}}),vec!["id","assignee_principal"]),
            "accept_review_task"=>("Accept a task assigned to your authenticated principal. Accepting does not grant signing authority.",json!({"id":{"type":"string","format":"uuid"}}),vec!["id"]),
            "save_verification_review"=>("Save each criterion's explanation and evidence before wallet signing. Authorized reviewer or accepted assigned agent only. Preserve the legacy assessment hash. Wallet signing remains on first-party pages.",json!({"network":{"type":"string"},"attestation":{"type":"object"},"assessment":{"type":"object"},"previous_report_id":{"type":["string","null"]}}),vec!["network","attestation","assessment"]),
            "get_verification_review"=>("Read a review you authored or may sign. Drafts, signatures, broadcasts and confirmed outcomes stay separate.",json!({"id":{"type":"string","format":"uuid"}}),vec!["id"]),
            "prepare_artifact_submission"=>("Recheck stored bytes, then prepare an existing-contract submission for the authenticated solver. This does not sign or broadcast. Sign only on first-party wallet pages.",json!({"id":{"type":"string","format":"uuid"},"network":{"type":"string"},"bounty_contract":{"type":"string"},"solver_wallet":{"type":"string"},"evidence":{"type":"object"}}),vec!["id","network","bounty_contract","solver_wallet","evidence"]),
            "confirm_artifact_submission"=>("Link stored files to an indexed canonical submission by its authenticated solver. Publish files only when explicitly requested; a broadcast is not confirmation.",json!({"id":{"type":"string","format":"uuid"},"network":{"type":"string"},"bounty_contract":{"type":"string"},"round":{"type":"integer","minimum":0},"publish_files":{"type":"boolean"}}),vec!["id","network","bounty_contract","round","publish_files"]),
            "open_artifact_dispute"=>("Pause file cleanup while your dispute is open. Only the owner or a granted participant may do this. Returns your existing open dispute on retry; does not change the bounty outcome.",json!({"id":{"type":"string","format":"uuid"},"reason":{"type":"string","minLength":1,"maxLength":2000}}),vec!["id","reason"]),
            "resolve_artifact_dispute"=>("Withdraw your own file-retention dispute with a reason. Other participants' holds remain; normal cleanup may resume. Does not change canonical outcomes.",json!({"id":{"type":"string","format":"uuid"},"reason":{"type":"string","minLength":1,"maxLength":2000}}),vec!["id","reason"]),
            _=>("Import selected files from an exact public GitHub commit. A branch name is not accepted; caller authentication and idempotency key required.",json!({"idempotency_key":{"type":"string","maxLength":128},"repository":{"type":"string"},"commit":{"type":"string","pattern":"^[a-fA-F0-9]{40}$"},"paths":{"type":"array","maxItems":64,"items":{"type":"string"}}}),vec!["idempotency_key","repository","commit","paths"]),
        };
        let mut descriptor=tool(name,description,object_tool_schema(properties,&required));
        if !matches!(*name,"find_verification_checks"|"validate_verification_plan") {descriptor.authorization=Some(ToolAuthorization{kind:"verified_principal",header:"Authorization",bearer:true,required_when:"Always; website or wallet session. Authenticate on the first-party website."});}
        descriptor
    }).collect()
}
pub async fn call(name: &str, mut headers: HeaderMap, mut arguments: Value) -> Json<Value> {
    let path = match name {
        "find_verification_checks" => "/v1/verification/checkers".to_string(),
        "validate_verification_plan" => "/v1/verification/plans/validate".to_string(),
        "run_verification_checks" => "/v1/verification/runs".to_string(),
        "upload_verification_artifact" => "/v1/verification/artifacts".to_string(),
        "import_verification_github" => "/v1/verification/artifacts/import-github".to_string(),
        "list_review_tasks" => "/v1/verification/review-tasks".to_string(),
        "save_verification_review" => "/v1/verification/reviews".to_string(),
        "assign_review_task"
        | "accept_review_task"
        | "get_verification_review"
        | "prepare_artifact_submission"
        | "open_artifact_dispute"
        | "resolve_artifact_dispute"
        | "confirm_artifact_submission" => {
            let Some(id) = arguments
                .as_object_mut()
                .and_then(|v| v.remove("id"))
                .and_then(|v| v.as_str().and_then(|s| Uuid::parse_str(s).ok()))
            else {
                return Json(json!({"error":"invalid_id"}));
            };
            let (resource, suffix) = match name {
                "assign_review_task" => ("review-tasks", "/assign"),
                "accept_review_task" => ("review-tasks", "/accept"),
                "get_verification_review" => ("reviews", ""),
                "prepare_artifact_submission" => ("artifacts", "/prepare-submission"),
                "open_artifact_dispute" => ("artifacts", "/disputes"),
                "resolve_artifact_dispute" => ("disputes", "/resolve"),
                _ => ("artifacts", "/confirm-submission"),
            };
            format!("/v1/verification/{resource}/{id}{suffix}")
        }
        "get_verification_run" | "get_verification_artifact" => {
            let Some(id) = arguments
                .get("id")
                .and_then(Value::as_str)
                .and_then(|s| Uuid::parse_str(s).ok())
            else {
                return Json(json!({"error":"invalid_id"}));
            };
            format!(
                "/v1/verification/{}/{id}",
                if name == "get_verification_run" {
                    "runs"
                } else {
                    "artifacts"
                }
            )
        }
        _ => return Json(json!({"error":"unknown_tool"})),
    };
    if let Some(value) = arguments
        .as_object_mut()
        .and_then(|o| o.remove("idempotency_key"))
    {
        let Some(key) = value
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 128)
            .and_then(|s| s.parse().ok())
        else {
            return Json(json!({"error":"invalid_idempotency_key"}));
        };
        headers.insert("idempotency-key", key);
    }
    let body = if matches!(
        name,
        "find_verification_checks"
            | "get_verification_run"
            | "get_verification_artifact"
            | "get_verification_review"
            | "list_review_tasks"
    ) {
        None
    } else if name == "validate_verification_plan" {
        Some(arguments.get("plan").cloned().unwrap_or(Value::Null))
    } else {
        Some(arguments)
    };
    secured_api_tool(&headers, &path, body).await
}
