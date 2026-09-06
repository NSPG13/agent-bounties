use domain::Money;
use github_app::{
    origin::{
        github::{
            plan_github_origin_draft, plan_github_webhook_origin_draft, GitHubWebhookDraftInput,
            GitHubWebhookTrigger,
        },
        linear::{plan_linear_origin_draft, LinearWebhookDraftInput, LinearWebhookTrigger},
        plan_origin_progress_callback, plan_origin_result_callback,
        runtime::{
            authenticate_and_plan_github_webhook, authenticate_and_plan_linear_webhook,
            plan_provider_http_requests, GitHubRuntimeConfig, LinearRuntimeConfig,
            ProviderHttpMethod, ProviderRequestBinding,
        },
        CanonicalSettlementEvent, OriginCompletionStatus, OriginProgressInput,
        OriginProgressStatus, OriginProvider, OriginResultInput, OriginSettlementReceipt,
        OriginSourceReference, OriginVerificationEvidence, OriginWriteOperation,
    },
    GitHubCreateCommentInput,
};
use hmac::{Hmac, Mac};
use serde_json::json;
use sha2::Sha256;
use uuid::Uuid;

type TestHmacSha256 = Hmac<Sha256>;

fn webhook_signature(secret: &[u8], body: &[u8], github_prefix: bool) -> String {
    let mut mac = TestHmacSha256::new_from_slice(secret).unwrap();
    mac.update(body);
    let digest = hex::encode(mac.finalize().into_bytes());
    if github_prefix {
        format!("sha256={digest}")
    } else {
        digest
    }
}

fn github_input() -> GitHubCreateCommentInput {
    GitHubCreateCommentInput {
        repository: "agent-bounties/agent-bounties".to_string(),
        issue_url: "https://github.com/agent-bounties/agent-bounties/issues/1274".to_string(),
        title: "Return verified work to source issues".to_string(),
        body: "Build a provider-neutral origin adapter without provider-side authority."
            .to_string(),
        comment_body: "/agent-bounty create 25 USDC".to_string(),
        contributor_login: Some("maintainer".to_string()),
        comment_id: Some("5000".to_string()),
        existing_idempotency_keys: vec![],
    }
}

fn github_webhook_input(trigger: GitHubWebhookTrigger) -> GitHubWebhookDraftInput {
    GitHubWebhookDraftInput {
        repository: "agent-bounties/agent-bounties".to_string(),
        issue_number: "1274".to_string(),
        issue_url: "https://github.com/agent-bounties/agent-bounties/issues/1274".to_string(),
        title: "Return verified work to source issues".to_string(),
        body: "## Goal\nReturn provider-neutral proof.\n\n## Acceptance criteria\n- Keep the origin open before settlement\n\n## Verifier\nRun the focused origin adapter tests."
            .to_string(),
        solver_reward: Money::new(8_000_000, "usdc").unwrap(),
        trigger,
        actor_login: Some("maintainer".to_string()),
        event_id: "github-delivery-1".to_string(),
        existing_idempotency_keys: vec![],
    }
}

fn linear_input() -> LinearWebhookDraftInput {
    LinearWebhookDraftInput {
        workspace_id: "workspace-1".to_string(),
        issue_id: "issue-uuid-1".to_string(),
        identifier: "ENG-42".to_string(),
        issue_url: "https://linear.app/agent-bounties/issue/ENG-42/return-proof".to_string(),
        title: "Return proof to the originating issue".to_string(),
        description: "## Goal\nReturn useful proof without granting payment authority.\n\n## Acceptance criteria\n- Upsert one status comment\n- Close only after settlement\n\n## Verifier\nReplay the origin adapter integration tests."
            .to_string(),
        command_text: "@AgentBounties /agent-bounty create 8 USDC".to_string(),
        trigger: LinearWebhookTrigger::Mention,
        actor_id: Some("user-1".to_string()),
        event_id: Some("event-1".to_string()),
        existing_idempotency_keys: vec![],
    }
}

fn source(provider: OriginProvider) -> OriginSourceReference {
    match provider {
        OriginProvider::GitHub => OriginSourceReference {
            provider,
            workspace: "agent-bounties/agent-bounties".to_string(),
            external_id: "1274".to_string(),
            display_id: "#1274".to_string(),
            url: "https://github.com/agent-bounties/agent-bounties/issues/1274".to_string(),
        },
        OriginProvider::Linear => OriginSourceReference {
            provider,
            workspace: "workspace-1".to_string(),
            external_id: "issue-uuid-1".to_string(),
            display_id: "ENG-42".to_string(),
            url: "https://linear.app/agent-bounties/issue/ENG-42/return-proof".to_string(),
        },
    }
}

fn result_input(provider: OriginProvider) -> OriginResultInput {
    OriginResultInput {
        source: source(provider),
        bounty_id: Uuid::parse_str("10000000-0000-0000-0000-000000000001").unwrap(),
        status_url: "https://agentbounties.app/bounties/example".to_string(),
        artifact_url: Some("https://github.com/example/repo/pull/7".to_string()),
        verification: Some(OriginVerificationEvidence {
            passed: true,
            committed_policy_matched: true,
            summary: "Origin adapter tests passed".to_string(),
            evidence_url: "https://agentbounties.app/proofs/example".to_string(),
        }),
        settlement: Some(OriginSettlementReceipt {
            event: CanonicalSettlementEvent::BountySettled,
            canonical_contract_verified: true,
            confirmed: true,
            chain_id: 8453,
            transaction_hash: format!("0x{}", "ab".repeat(32)),
            log_index: 4,
            receipt_url: "https://agentbounties.app/settlements/example".to_string(),
        }),
        existing_idempotency_keys: vec![],
    }
}

#[test]
fn github_adapter_reuses_review_only_planner_and_enforces_public_floor() {
    let plan = plan_github_origin_draft(github_input());
    assert!(plan.ready_for_human_review);
    let draft = plan.draft.expect("normalized GitHub draft");
    assert_eq!(draft.source.provider, OriginProvider::GitHub);
    assert_eq!(draft.source.external_id, "1274");
    assert_eq!(draft.reward.solver, Money::new(25_000_000, "usdc").unwrap());
    assert!(draft.acceptance_criteria.is_empty());
    assert!(draft.verifier.requires_review);
    assert!(!draft.ready_for_publish);
    assert_eq!(plan.authority, Default::default());

    let mut below_floor = github_input();
    below_floor.comment_body = "/agent-bounty create 1 USDC".to_string();
    let plan = plan_github_origin_draft(below_floor);
    assert!(!plan.ready_for_human_review);
    assert!(plan.error.unwrap().contains("at least 2 USDC"));
}

#[test]
fn github_adapter_extracts_only_explicit_acceptance_and_verifier_sections() {
    let mut input = github_input();
    input.body = "A short issue summary.\n\n## Acceptance criteria\n- cargo test passes\n- receipt is returned\n\n## Verifier\nRun the focused origin adapter test."
        .to_string();
    let plan = plan_github_origin_draft(input);
    let draft = plan.draft.expect("normalized GitHub draft");
    assert_eq!(draft.acceptance_criteria.len(), 2);
    assert!(!draft.verifier.requires_review);
    assert!(!draft
        .fields_requiring_review
        .contains(&"acceptance criteria".to_string()));
}

#[test]
fn github_webhook_adapter_supports_assignment_and_mention() {
    for trigger in [
        GitHubWebhookTrigger::Assignment,
        GitHubWebhookTrigger::Mention,
    ] {
        let plan = plan_github_webhook_origin_draft(github_webhook_input(trigger));
        assert!(plan.ready_for_human_review);
        assert_eq!(plan.authority, Default::default());
        let draft = plan.draft.expect("normalized GitHub webhook draft");
        assert_eq!(draft.source.external_id, "1274");
        assert_eq!(draft.acceptance_criteria.len(), 1);
        assert!(!draft.verifier.requires_review);
        assert_eq!(
            draft.trigger,
            match trigger {
                GitHubWebhookTrigger::Assignment => github_app::origin::OriginTrigger::Assignment,
                GitHubWebhookTrigger::Mention => github_app::origin::OriginTrigger::Mention,
            }
        );
        assert!(!draft.ready_for_publish);
    }
}

#[test]
fn github_webhook_adapter_deduplicates_events_and_enforces_public_floor() {
    let input = github_webhook_input(GitHubWebhookTrigger::Assignment);
    let first = plan_github_webhook_origin_draft(input.clone());
    let key = first.idempotency_key.expect("GitHub event idempotency key");

    let mut retry = input.clone();
    retry.existing_idempotency_keys = vec![key];
    let retry = plan_github_webhook_origin_draft(retry);
    assert!(retry.duplicate);
    assert!(!retry.ready_for_human_review);
    assert!(retry.draft.is_none());

    let mut below_floor = input;
    below_floor.solver_reward = Money::new(1_000_000, "usdc").unwrap();
    let below_floor = plan_github_webhook_origin_draft(below_floor);
    assert!(!below_floor.ready_for_human_review);
    assert!(below_floor.error.unwrap().contains("at least 2 USDC"));
}

#[test]
fn github_runtime_authenticates_and_allowlists_assignment_and_mention_events() {
    let secret = b"github-provider-secret";
    let config = GitHubRuntimeConfig {
        app_login: "agent-bounties".to_string(),
        assignment_solver_reward: Money::new(8_000_000, "usdc").unwrap(),
        mention_solver_reward: Money::new(5_000_000, "usdc").unwrap(),
    };
    let issue = json!({
        "number": 1274,
        "html_url": "https://github.com/agent-bounties/agent-bounties/issues/1274",
        "title": "Return verified work",
        "body": "## Goal\nReturn verified work.\n\n## Acceptance criteria\n- Focused tests pass"
    });
    let assignment = serde_json::to_vec(&json!({
        "action": "assigned",
        "repository": { "full_name": "agent-bounties/agent-bounties" },
        "sender": { "login": "maintainer" },
        "installation": { "id": 7 },
        "issue": issue,
        "assignee": { "login": "agent-bounties" }
    }))
    .unwrap();
    let plan = authenticate_and_plan_github_webhook(
        &assignment,
        &webhook_signature(secret, &assignment, true),
        "issues",
        "delivery-assigned-1",
        secret,
        &config,
        vec![],
    );
    assert!(plan.authenticated);
    let draft = plan.draft_plan.unwrap().draft.expect("assignment draft");
    assert_eq!(draft.trigger, github_app::origin::OriginTrigger::Assignment);
    assert_eq!(draft.reward.solver.amount, 8_000_000);

    let mention = serde_json::to_vec(&json!({
        "action": "created",
        "repository": { "full_name": "agent-bounties/agent-bounties" },
        "sender": { "login": "maintainer" },
        "installation": { "id": 7 },
        "issue": {
            "number": 1274,
            "html_url": "https://github.com/agent-bounties/agent-bounties/issues/1274",
            "title": "Return verified work",
            "body": "Return verified work with replayable evidence."
        },
        "comment": { "id": 44, "body": "@agent-bounties please delegate this issue" }
    }))
    .unwrap();
    let plan = authenticate_and_plan_github_webhook(
        &mention,
        &webhook_signature(secret, &mention, true),
        "issue_comment",
        "delivery-mention-1",
        secret,
        &config,
        vec![],
    );
    assert!(plan.authenticated);
    let draft = plan
        .draft_plan
        .as_ref()
        .unwrap()
        .draft
        .as_ref()
        .expect("mention draft");
    assert_eq!(draft.trigger, github_app::origin::OriginTrigger::Mention);
    assert_eq!(draft.reward.solver.amount, 5_000_000);

    let replay = authenticate_and_plan_github_webhook(
        &mention,
        &webhook_signature(secret, &mention, true),
        "issue_comment",
        "attacker-controlled-different-delivery-id",
        secret,
        &config,
        vec![],
    );
    assert_eq!(plan.event_id, replay.event_id);
    assert_eq!(
        plan.draft_plan.as_ref().unwrap().idempotency_key,
        replay.draft_plan.as_ref().unwrap().idempotency_key
    );
    let serialized = serde_json::to_string(&plan).unwrap();
    assert!(!serialized.contains("github-provider-secret"));

    let unsupported = authenticate_and_plan_github_webhook(
        &mention,
        &webhook_signature(secret, &mention, true),
        "pull_request",
        "delivery-unsupported-1",
        secret,
        &config,
        vec![],
    );
    assert!(!unsupported.authenticated);
    assert!(unsupported.draft_plan.is_none());

    let rejected = authenticate_and_plan_github_webhook(
        &mention,
        "sha256=ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
        "issue_comment",
        "delivery-mention-1",
        secret,
        &config,
        vec![],
    );
    assert!(!rejected.authenticated);
    assert!(rejected.draft_plan.is_none());
    assert!(rejected.error.unwrap().contains("invalid GitHub"));
}

#[test]
fn linear_runtime_authenticates_delegation_and_mention_projections() {
    let secret = b"linear-provider-secret";
    let config = LinearRuntimeConfig {
        agent_id: "agent-id-1".to_string(),
        agent_mention: "AgentBounties".to_string(),
        delegation_solver_reward: Money::new(9_000_000, "usdc").unwrap(),
        mention_solver_reward: Money::new(6_000_000, "usdc").unwrap(),
    };
    let issue = json!({
        "id": "issue-uuid-1",
        "identifier": "ENG-42",
        "url": "https://linear.app/agent-bounties/issue/ENG-42/return-proof",
        "title": "Return proof",
        "description": "## Goal\nReturn proof.\n\n## Acceptance criteria\n- Tests pass",
        "assignee": { "id": "agent-id-1" }
    });
    let delegation = serde_json::to_vec(&json!({
        "action": "update",
        "type": "Issue",
        "organizationId": "workspace-1",
        "webhookId": "webhook-1",
        "actor": { "id": "user-1" },
        "updatedFrom": { "assigneeId": "previous-assignee" },
        "data": issue
    }))
    .unwrap();
    let plan = authenticate_and_plan_linear_webhook(
        &delegation,
        &webhook_signature(secret, &delegation, false),
        secret,
        &config,
        vec![],
    );
    assert!(plan.authenticated);
    let draft = plan.draft_plan.unwrap().draft.expect("delegation draft");
    assert_eq!(draft.trigger, github_app::origin::OriginTrigger::Delegation);
    assert_eq!(draft.reward.solver.amount, 9_000_000);

    let unrelated_update = serde_json::to_vec(&json!({
        "action": "update",
        "type": "Issue",
        "organizationId": "workspace-1",
        "webhookId": "webhook-unrelated",
        "actor": { "id": "user-1" },
        "updatedFrom": { "title": "Old title" },
        "data": issue
    }))
    .unwrap();
    let rejected = authenticate_and_plan_linear_webhook(
        &unrelated_update,
        &webhook_signature(secret, &unrelated_update, false),
        secret,
        &config,
        vec![],
    );
    assert!(!rejected.authenticated);
    assert!(rejected.draft_plan.is_none());
    assert!(rejected.error.unwrap().contains("assignee transition"));

    let mention = serde_json::to_vec(&json!({
        "action": "create",
        "type": "Comment",
        "organizationId": "workspace-1",
        "webhookId": "webhook-1",
        "actor": { "id": "user-1" },
        "data": {
            "id": "comment-1",
            "body": "@AgentBounties please delegate this",
            "issue": {
                "id": "issue-uuid-1",
                "identifier": "ENG-42",
                "url": "https://linear.app/agent-bounties/issue/ENG-42/return-proof",
                "title": "Return proof",
                "description": "Return useful proof."
            }
        }
    }))
    .unwrap();
    let plan = authenticate_and_plan_linear_webhook(
        &mention,
        &webhook_signature(secret, &mention, false),
        secret,
        &config,
        vec![],
    );
    assert!(plan.authenticated);
    let draft = plan.draft_plan.unwrap().draft.expect("mention draft");
    assert_eq!(draft.trigger, github_app::origin::OriginTrigger::Mention);
    assert_eq!(draft.reward.solver.amount, 6_000_000);

    let rejected = authenticate_and_plan_linear_webhook(
        &mention,
        "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
        secret,
        &config,
        vec![],
    );
    assert!(!rejected.authenticated);
    assert!(rejected.draft_plan.is_none());
    assert!(rejected.error.unwrap().contains("invalid Linear"));
}

#[test]
fn linear_adapter_extracts_explicit_contract_and_deduplicates_events() {
    let input = linear_input();
    let plan = plan_linear_origin_draft(input.clone());
    assert!(plan.ready_for_human_review);
    let key = plan.idempotency_key.clone().expect("idempotency key");
    let draft = plan.draft.expect("normalized Linear draft");
    assert_eq!(draft.source.provider, OriginProvider::Linear);
    assert_eq!(draft.source.display_id, "ENG-42");
    assert_eq!(draft.acceptance_criteria.len(), 2);
    assert_eq!(draft.reward.solver.amount, 8_000_000);
    assert!(!draft.verifier.requires_review);
    assert!(draft
        .fields_requiring_review
        .iter()
        .any(|field| field.contains("wallet")));
    assert!(!draft.ready_for_publish);

    let mut retry = input;
    retry.existing_idempotency_keys = vec![key];
    let retry_plan = plan_linear_origin_draft(retry);
    assert!(retry_plan.duplicate);
    assert!(!retry_plan.ready_for_human_review);
    assert!(retry_plan.draft.is_none());
}

#[test]
fn result_callback_upserts_before_closing_and_is_retry_safe_for_both_providers() {
    for provider in [OriginProvider::GitHub, OriginProvider::Linear] {
        let input = result_input(provider);
        let first = plan_origin_result_callback(input.clone());
        let second = plan_origin_result_callback(input);
        assert_eq!(first, second);
        assert_eq!(first.status, OriginCompletionStatus::Settled);
        assert!(first.close_origin);
        assert_eq!(first.operations.len(), 2);
        let OriginWriteOperation::UpsertStatusComment {
            idempotency_key: status_key,
            markdown,
            ..
        } = &first.operations[0]
        else {
            panic!("proof/status must be written before close");
        };
        assert!(markdown.contains("verified and settled"));
        assert!(markdown.contains("BountySettled"));
        let OriginWriteOperation::CloseIssue {
            depends_on_idempotency_key,
            ..
        } = &first.operations[1]
        else {
            panic!("second operation must close the issue");
        };
        assert_eq!(depends_on_idempotency_key, status_key);
        assert_eq!(first.authority, Default::default());
    }
}

#[test]
fn progress_callback_updates_one_comment_and_never_closes_the_origin() {
    let input = OriginProgressInput {
        source: source(OriginProvider::Linear),
        bounty_id: Uuid::new_v4(),
        status: OriginProgressStatus::CanonicalFundingConfirmed,
        status_url: "https://agentbounties.app/bounties/example".to_string(),
        canonical_evidence_url: Some(
            "https://agentbounties.app/evidence/funding/example".to_string(),
        ),
        existing_idempotency_keys: vec![],
    };
    let plan = plan_origin_progress_callback(input.clone());
    assert!(plan.ready);
    assert_eq!(plan.operations.len(), 1);
    let OriginWriteOperation::UpsertStatusComment { markdown, .. } = &plan.operations[0] else {
        panic!("progress must only upsert the status comment");
    };
    assert!(markdown.contains("canonical funding confirmed"));
    assert!(markdown.contains("cannot hold keys"));

    let mut next_status = input;
    next_status.status = OriginProgressStatus::CanonicalClaimConfirmed;
    next_status.canonical_evidence_url =
        Some("https://agentbounties.app/evidence/claim/example".to_string());
    let next_plan = plan_origin_progress_callback(next_status);
    let OriginWriteOperation::UpsertStatusComment {
        idempotency_key: next_key,
        stable_marker: next_marker,
        ..
    } = &next_plan.operations[0]
    else {
        panic!("next progress state must upsert the status comment");
    };
    let OriginWriteOperation::UpsertStatusComment {
        idempotency_key: first_key,
        stable_marker: first_marker,
        ..
    } = &plan.operations[0]
    else {
        unreachable!();
    };
    assert_ne!(first_key, next_key);
    assert_eq!(first_marker, next_marker);

    let missing_evidence = plan_origin_progress_callback(OriginProgressInput {
        source: source(OriginProvider::GitHub),
        bounty_id: Uuid::new_v4(),
        status: OriginProgressStatus::CanonicalClaimConfirmed,
        status_url: "https://agentbounties.app/bounties/example".to_string(),
        canonical_evidence_url: None,
        existing_idempotency_keys: vec![],
    });
    assert!(!missing_evidence.ready);
    assert!(missing_evidence.operations.is_empty());
}

#[test]
fn result_callback_never_closes_for_unverified_or_unconfirmed_work() {
    let mut awaiting_verification = result_input(OriginProvider::GitHub);
    awaiting_verification.verification = None;
    let plan = plan_origin_result_callback(awaiting_verification);
    assert_eq!(
        plan.status,
        OriginCompletionStatus::SubmittedAwaitingVerification
    );
    assert!(!plan.close_origin);
    assert_eq!(plan.operations.len(), 1);

    let mut failed_verification = result_input(OriginProvider::GitHub);
    failed_verification.verification.as_mut().unwrap().passed = false;
    let plan = plan_origin_result_callback(failed_verification);
    assert_eq!(plan.status, OriginCompletionStatus::VerificationFailed);
    assert!(!plan.close_origin);

    let mut unconfirmed = result_input(OriginProvider::Linear);
    unconfirmed.settlement.as_mut().unwrap().confirmed = false;
    let plan = plan_origin_result_callback(unconfirmed);
    assert_eq!(
        plan.status,
        OriginCompletionStatus::VerifiedAwaitingSettlement
    );
    assert!(!plan.close_origin);
    assert!(plan
        .blocked_reasons
        .iter()
        .any(|reason| reason.contains("confirmed canonical")));
}

#[test]
fn provider_runtime_builds_exact_ordered_allowlisted_request_batches() {
    let github_result = plan_origin_result_callback(result_input(OriginProvider::GitHub));
    let github = plan_provider_http_requests(
        &source(OriginProvider::GitHub),
        &github_result.operations,
        ProviderRequestBinding::GitHub {
            existing_status_comment_id: None,
        },
    );
    assert!(github.ready);
    assert_eq!(github.requests.len(), 2);
    assert_eq!(github.requests[0].method, ProviderHttpMethod::Post);
    assert_eq!(
        github.requests[0].url,
        "https://api.github.com/repos/agent-bounties/agent-bounties/issues/1274/comments"
    );
    assert_eq!(github.requests[1].method, ProviderHttpMethod::Patch);
    assert_eq!(
        github.requests[1].depends_on_idempotency_key.as_deref(),
        Some(github.requests[0].operation_idempotency_key.as_str())
    );
    assert!(github
        .requests
        .iter()
        .all(|request| !request.network_write_performed));

    let github_update = plan_provider_http_requests(
        &source(OriginProvider::GitHub),
        &github_result.operations[..1],
        ProviderRequestBinding::GitHub {
            existing_status_comment_id: Some(77),
        },
    );
    assert!(github_update.ready);
    assert_eq!(github_update.requests[0].method, ProviderHttpMethod::Patch);
    assert!(github_update.requests[0]
        .url
        .ends_with("/issues/comments/77"));

    let linear_result = plan_origin_result_callback(result_input(OriginProvider::Linear));
    let linear = plan_provider_http_requests(
        &source(OriginProvider::Linear),
        &linear_result.operations,
        ProviderRequestBinding::Linear {
            existing_status_comment_id: Some("comment-1".to_string()),
            completed_state_id: "completed-state-1".to_string(),
        },
    );
    assert!(linear.ready);
    assert_eq!(linear.requests.len(), 2);
    assert!(linear
        .requests
        .iter()
        .all(|request| request.url == "https://api.linear.app/graphql"));
    assert!(linear.requests[0].body["query"]
        .as_str()
        .unwrap()
        .contains("commentUpdate"));
    assert!(linear.requests[1].body["query"]
        .as_str()
        .unwrap()
        .contains("issueUpdate"));

    let close_only = plan_provider_http_requests(
        &source(OriginProvider::GitHub),
        &github_result.operations[1..],
        ProviderRequestBinding::GitHub {
            existing_status_comment_id: None,
        },
    );
    assert!(!close_only.ready);
    assert!(close_only.requests.is_empty());

    let empty = plan_provider_http_requests(
        &source(OriginProvider::GitHub),
        &[],
        ProviderRequestBinding::GitHub {
            existing_status_comment_id: None,
        },
    );
    assert!(!empty.ready);
}
