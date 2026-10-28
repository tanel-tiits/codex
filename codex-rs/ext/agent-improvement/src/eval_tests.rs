use std::fs;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Result;
use codex_rollout_trace::ExecutionStatus;
use codex_rollout_trace::RawPayloadKind;
use codex_rollout_trace::RawToolCallRequester;
use codex_rollout_trace::RawTraceEventContext;
use codex_rollout_trace::RawTraceEventPayload;
use codex_rollout_trace::RolloutStatus;
use codex_rollout_trace::ToolCallKind;
use codex_rollout_trace::ToolCallSummary;
use codex_rollout_trace::TraceWriter;
use serde_json::json;
use tempfile::TempDir;

use super::AttributionAssessment;
use super::AttributionEvidence;
use super::CheckResult;
use super::EvaluationBudget;
use super::EvaluationCondition;
use super::EvaluationErrorCode;
use super::EvaluationManifest;
use super::EvaluationMemory;
use super::EvaluationRepository;
use super::EvaluationRetrievalEvidence;
use super::EvaluationScenario;
use super::EvaluationSplit;
use super::EvaluationStatus;
use super::EvaluationTask;
use super::EvaluatorKind;
use super::EvaluatorSpec;
use super::PolicyStatus;
use super::PostRunEvidence;
use super::TraceStatus;
use super::VerificationCheck;
use super::evaluate_manifest_at;
use super::write_evaluation_result;

fn manifest_for(
    root: &Path,
    baseline_trace: PathBuf,
    treatment_trace: PathBuf,
    evaluator: EvaluatorSpec,
) -> EvaluationManifest {
    let primary = root.join("primary");
    let baseline_worktree = root.join("baseline-worktree");
    let treatment_worktree = root.join("treatment-worktree");
    let card_store = root.join("active-card-store");
    for path in [
        &primary,
        &baseline_worktree,
        &treatment_worktree,
        &card_store,
    ] {
        fs::create_dir_all(path).expect("create fixture path");
    }

    EvaluationManifest {
        schema_version: super::EVALUATION_MANIFEST_SCHEMA_VERSION,
        manifest_id: "manifest-fixture".to_string(),
        scenarios: vec![EvaluationScenario {
            scenario_id: "scenario-parser-1".to_string(),
            task_family: crate::TaskFamily::Coding,
            split: EvaluationSplit::HeldOut,
            task: EvaluationTask {
                task_id: "parser-task".to_string(),
                prompt: Some("fix parser and run cargo test".to_string()),
                reference: None,
                fingerprint: "task-fingerprint".to_string(),
            },
            repository: EvaluationRepository {
                snapshot_fingerprint: "repo-fingerprint".to_string(),
                primary_checkout: Some(primary),
            },
            baseline: EvaluationCondition {
                trace_bundle: baseline_trace,
                worktree: baseline_worktree,
                worktree_fingerprint: "worktree-fingerprint".to_string(),
                execution_config_fingerprint: "config-fingerprint".to_string(),
                policy_fingerprint: "policy-fingerprint".to_string(),
                memory: EvaluationMemory {
                    enabled: false,
                    card_set_fingerprint: "cards-none".to_string(),
                    retrieval_policy_fingerprint: "retrieval-v1".to_string(),
                    card_schema_version: 2,
                    card_store_path: None,
                },
                retrieval: EvaluationRetrievalEvidence::default(),
                judge: None,
                post_run: None,
            },
            treatment: EvaluationCondition {
                trace_bundle: treatment_trace,
                worktree: treatment_worktree,
                worktree_fingerprint: "worktree-fingerprint".to_string(),
                execution_config_fingerprint: "config-fingerprint".to_string(),
                policy_fingerprint: "policy-fingerprint".to_string(),
                memory: EvaluationMemory {
                    enabled: true,
                    card_set_fingerprint: "cards-treatment-v1".to_string(),
                    retrieval_policy_fingerprint: "retrieval-v1".to_string(),
                    card_schema_version: 2,
                    card_store_path: Some(card_store),
                },
                retrieval: EvaluationRetrievalEvidence::default(),
                judge: None,
                post_run: None,
            },
            evaluator,
            budget: EvaluationBudget::default(),
            attribution: AttributionEvidence::default(),
        }],
    }
}

fn trace_bundle(root: &Path, name: &str, status: RolloutStatus, command: Option<&str>) -> PathBuf {
    let bundle = root.join(name);
    let writer = TraceWriter::create(
        &bundle,
        format!("trace-{name}"),
        format!("rollout-{name}"),
        "thread-root".to_string(),
    )
    .expect("create trace writer");
    writer
        .append(RawTraceEventPayload::RolloutStarted {
            trace_id: format!("trace-{name}"),
            root_thread_id: "thread-root".to_string(),
        })
        .expect("rollout start");
    writer
        .append(RawTraceEventPayload::ThreadStarted {
            thread_id: "thread-root".to_string(),
            agent_path: "/root".to_string(),
            metadata_payload: None,
        })
        .expect("thread start");
    writer
        .append(RawTraceEventPayload::CodexTurnStarted {
            codex_turn_id: "turn-1".to_string(),
            thread_id: "thread-root".to_string(),
        })
        .expect("turn start");

    if let Some(command) = command {
        let tool_id = format!("tool-{name}");
        let invocation = writer
            .write_json_payload(
                RawPayloadKind::ToolInvocation,
                &json!({
                    "tool_name": "exec_command",
                    "payload": { "cmd": command }
                }),
            )
            .expect("invocation payload");
        writer
            .append_with_context(
                RawTraceEventContext {
                    thread_id: Some("thread-root".to_string()),
                    codex_turn_id: Some("turn-1".to_string()),
                },
                RawTraceEventPayload::ToolCallStarted {
                    tool_call_id: tool_id.clone(),
                    model_visible_call_id: Some(format!("call-{name}")),
                    code_mode_runtime_tool_id: None,
                    requester: RawToolCallRequester::Model,
                    kind: ToolCallKind::ExecCommand,
                    summary: ToolCallSummary::Generic {
                        label: "exec_command".to_string(),
                        input_preview: None,
                        output_preview: None,
                    },
                    invocation_payload: Some(invocation),
                },
            )
            .expect("tool start");
        let runtime_start = writer
            .write_json_payload(
                RawPayloadKind::ToolRuntimeEvent,
                &json!({
                    "call_id": tool_id,
                    "turn_id": "turn-1",
                    "command": command.split_whitespace().collect::<Vec<_>>(),
                    "cwd": "/fixture"
                }),
            )
            .expect("runtime start payload");
        writer
            .append_with_context(
                RawTraceEventContext {
                    thread_id: Some("thread-root".to_string()),
                    codex_turn_id: Some("turn-1".to_string()),
                },
                RawTraceEventPayload::ToolCallRuntimeStarted {
                    tool_call_id: format!("tool-{name}"),
                    runtime_payload: runtime_start,
                },
            )
            .expect("runtime start");
        let succeeded = status == RolloutStatus::Completed;
        let policy_boundary = command.to_ascii_lowercase().contains("sandbox");
        let runtime_end = writer
            .write_json_payload(
                RawPayloadKind::ToolRuntimeEvent,
                &json!({
                    "call_id": format!("tool-{name}"),
                    "process_id": format!("process-{name}"),
                    "turn_id": "turn-1",
                    "command": command.split_whitespace().collect::<Vec<_>>(),
                    "cwd": "/fixture",
                    "aggregated_output": if policy_boundary {
                        "sandbox: operation not permitted\n"
                    } else if succeeded {
                        "ok\n"
                    } else {
                        "failed\n"
                    },
                    "exit_code": if succeeded { 0 } else { 1 },
                    "status": if succeeded { "completed" } else { "failed" }
                }),
            )
            .expect("runtime end payload");
        writer
            .append_with_context(
                RawTraceEventContext {
                    thread_id: Some("thread-root".to_string()),
                    codex_turn_id: Some("turn-1".to_string()),
                },
                RawTraceEventPayload::ToolCallRuntimeEnded {
                    tool_call_id: format!("tool-{name}"),
                    status: if succeeded {
                        ExecutionStatus::Completed
                    } else {
                        ExecutionStatus::Failed
                    },
                    runtime_payload: runtime_end,
                },
            )
            .expect("runtime end");
        let result_payload = writer
            .write_json_payload(
                RawPayloadKind::ToolResult,
                &json!({
                    "type": "direct_response",
                    "response_item": {
                        "type": "function_call_output",
                        "output": if policy_boundary {
                            "sandbox: operation not permitted\n"
                        } else if succeeded {
                            "ok\n"
                        } else {
                            "failed\n"
                        }
                    }
                }),
            )
            .expect("result payload");
        writer
            .append_with_context(
                RawTraceEventContext {
                    thread_id: Some("thread-root".to_string()),
                    codex_turn_id: Some("turn-1".to_string()),
                },
                RawTraceEventPayload::ToolCallEnded {
                    tool_call_id: format!("tool-{name}"),
                    status: if succeeded {
                        ExecutionStatus::Completed
                    } else {
                        ExecutionStatus::Failed
                    },
                    result_payload: Some(result_payload),
                },
            )
            .expect("tool end");
    }

    writer
        .append(RawTraceEventPayload::CodexTurnEnded {
            codex_turn_id: "turn-1".to_string(),
            status: match status {
                RolloutStatus::Completed => ExecutionStatus::Completed,
                RolloutStatus::Failed => ExecutionStatus::Failed,
                RolloutStatus::Aborted => ExecutionStatus::Cancelled,
                RolloutStatus::Running => ExecutionStatus::Running,
            },
        })
        .expect("turn end");
    writer
        .append(RawTraceEventPayload::ThreadEnded {
            thread_id: "thread-root".to_string(),
            status: status.clone(),
        })
        .expect("thread end");
    writer
        .append(RawTraceEventPayload::RolloutEnded { status })
        .expect("rollout end");
    bundle
}

fn trace_outcome_evaluator() -> EvaluatorSpec {
    EvaluatorSpec {
        kind: EvaluatorKind::TraceOutcome,
        checks: Vec::new(),
        require_all_checks: true,
    }
}

fn post_run_evaluator() -> EvaluatorSpec {
    EvaluatorSpec {
        kind: EvaluatorKind::PostRunVerification,
        checks: vec![VerificationCheck {
            check_id: "hidden-tests".to_string(),
            command_tokens: vec!["python3".to_string(), "hidden_tests.py".to_string()],
            expected_exit_code: 0,
            preflight_command_tokens: None,
            preflight_expected_exit_code: 0,
        }],
        require_all_checks: true,
    }
}

#[test]
fn paired_eval_reports_held_out_outcomes_and_keeps_private_inputs_out() -> Result<()> {
    let temp = TempDir::new()?;
    let traces = temp.path().join("traces");
    fs::create_dir_all(&traces)?;
    let baseline = trace_bundle(
        &traces,
        "baseline",
        RolloutStatus::Failed,
        Some("cargo test"),
    );
    let treatment = trace_bundle(
        &traces,
        "treatment",
        RolloutStatus::Completed,
        Some("cargo test"),
    );
    let manifest = manifest_for(
        &temp.path().join("scenario"),
        baseline,
        treatment,
        trace_outcome_evaluator(),
    );

    let result = evaluate_manifest_at(&manifest, 123)?;
    assert_eq!(result.held_out.scenario_count, 1);
    assert_eq!(result.held_out.baseline_success_count, 0);
    assert_eq!(result.held_out.treatment_success_count, 1);
    assert_eq!(result.scenarios[0].delta.task_success, Some(1));
    assert_eq!(
        result.scenarios[0]
            .baseline
            .metrics
            .as_ref()
            .unwrap()
            .failure_class,
        crate::FailureClass::CommandFailure
    );
    assert_eq!(
        result.scenarios[0]
            .treatment
            .metrics
            .as_ref()
            .unwrap()
            .verification_attempts,
        1
    );
    assert_eq!(
        result.scenarios[0].evaluator_version,
        super::EVALUATOR_VERSION
    );
    assert_eq!(
        result.scenarios[0].evaluator_fingerprint,
        manifest.scenarios[0].evaluator.fingerprint()
    );

    let json = serde_json::to_string(&result)?;
    assert!(!json.contains("fix parser and run cargo test"));
    assert!(!json.contains("baseline-worktree"));
    assert!(!json.contains("active-card-store"));
    assert!(!json.contains("command_tokens"));
    Ok(())
}

#[test]
fn trace_completion_and_policy_status_are_reported_separately() -> Result<()> {
    let temp = TempDir::new()?;
    let traces = temp.path().join("traces");
    fs::create_dir_all(&traces)?;
    let baseline = trace_bundle(
        &traces,
        "baseline",
        RolloutStatus::Completed,
        Some("sandbox-check"),
    );
    let treatment = trace_bundle(&traces, "treatment", RolloutStatus::Completed, None);
    let manifest = manifest_for(
        &temp.path().join("scenario"),
        baseline,
        treatment,
        trace_outcome_evaluator(),
    );

    let result = evaluate_manifest_at(&manifest, 123)?;
    let baseline = &result.scenarios[0].baseline;
    let metrics = baseline.metrics.as_ref().expect("baseline metrics");
    assert_eq!(metrics.trace_status, TraceStatus::Completed);
    assert_eq!(metrics.policy_status, PolicyStatus::BoundaryObserved);
    assert_eq!(metrics.failure_class, crate::FailureClass::PolicyBlocked);
    assert_eq!(baseline.evaluator.task_success, Some(true));
    Ok(())
}

#[test]
fn verification_evaluator_reports_failed_checks_and_cost_limits() -> Result<()> {
    let temp = TempDir::new()?;
    let traces = temp.path().join("traces");
    fs::create_dir_all(&traces)?;
    let baseline = trace_bundle(&traces, "baseline", RolloutStatus::Completed, None);
    let treatment = trace_bundle(&traces, "treatment", RolloutStatus::Completed, None);
    let mut manifest = manifest_for(
        &temp.path().join("scenario"),
        baseline,
        treatment,
        EvaluatorSpec {
            kind: EvaluatorKind::Verification,
            checks: vec![VerificationCheck {
                check_id: "tests".to_string(),
                command_tokens: vec!["cargo".to_string(), "test".to_string()],
                expected_exit_code: 0,
                preflight_command_tokens: None,
                preflight_expected_exit_code: 0,
            }],
            require_all_checks: true,
        },
    );

    let result = evaluate_manifest_at(&manifest, 123)?;
    let scenario = &result.scenarios[0];
    assert_eq!(scenario.baseline.status, EvaluationStatus::Evaluated);
    assert_eq!(scenario.baseline.evaluator.task_success, Some(false));
    assert_eq!(scenario.baseline.evaluator.checks[0].check_id, "tests");
    assert!(!scenario.baseline.evaluator.checks[0].success);

    manifest.scenarios[0].budget.max_trace_bytes = 1;
    let limited = evaluate_manifest_at(&manifest, 123)?;
    assert_eq!(
        limited.scenarios[0].baseline.error_code,
        Some(EvaluationErrorCode::TraceTooLarge)
    );
    assert_eq!(
        limited.scenarios[0].treatment.status,
        EvaluationStatus::CostLimited
    );
    Ok(())
}

#[test]
fn post_run_evaluator_uses_private_evidence_and_separates_trace_status() -> Result<()> {
    let temp = TempDir::new()?;
    let traces = temp.path().join("traces");
    fs::create_dir_all(&traces)?;
    let baseline = trace_bundle(&traces, "baseline", RolloutStatus::Completed, None);
    let treatment = trace_bundle(&traces, "treatment", RolloutStatus::Completed, None);
    let mut manifest = manifest_for(
        &temp.path().join("scenario"),
        baseline,
        treatment,
        post_run_evaluator(),
    );
    let evidence = Some(PostRunEvidence {
        checks: vec![CheckResult {
            check_id: "hidden-tests".to_string(),
            success: true,
        }],
    });
    manifest.scenarios[0].baseline.post_run = evidence.clone();
    manifest.scenarios[0].treatment.post_run = evidence;
    manifest.scenarios[0].treatment.retrieval = EvaluationRetrievalEvidence {
        selected_card_ids: vec![
            "card-private-selection-b".to_string(),
            "card-private-selection-a".to_string(),
        ],
        exposure_count: 3,
    };

    let result = evaluate_manifest_at(&manifest, 123)?;
    let scenario = &result.scenarios[0];
    assert_eq!(scenario.baseline.evaluator.task_success, Some(true));
    assert_eq!(
        scenario.treatment.evaluator.verification_success,
        Some(true)
    );
    assert_eq!(
        scenario.treatment.evaluator.checks[0],
        CheckResult {
            check_id: "hidden-tests".to_string(),
            success: true,
        }
    );
    let metrics = scenario.treatment.metrics.as_ref().expect("metrics");
    assert_eq!(metrics.trace_status, TraceStatus::Completed);
    assert_eq!(metrics.policy_status, PolicyStatus::Clean);
    assert_eq!(scenario.treatment.retrieval.selected_card_count, 2);
    assert_eq!(scenario.treatment.retrieval.exposure_count, 3);
    assert!(scenario.treatment.retrieval.selection_fingerprint.is_some());

    let first_selection_fingerprint = scenario.treatment.retrieval.selection_fingerprint.clone();
    let mut reordered_manifest = manifest.clone();
    reordered_manifest.scenarios[0]
        .treatment
        .retrieval
        .selected_card_ids
        .reverse();
    let reordered = evaluate_manifest_at(&reordered_manifest, 123)?;
    assert_eq!(
        reordered.scenarios[0]
            .treatment
            .retrieval
            .selection_fingerprint,
        first_selection_fingerprint
    );

    let json = serde_json::to_string(&result)?;
    assert!(!json.contains("python3"));
    assert!(!json.contains("hidden_tests.py"));
    assert!(!json.contains("card-private-selection"));
    Ok(())
}

#[test]
fn post_run_failure_and_missing_evidence_are_distinct() -> Result<()> {
    let temp = TempDir::new()?;
    let traces = temp.path().join("traces");
    fs::create_dir_all(&traces)?;
    let baseline = trace_bundle(&traces, "baseline", RolloutStatus::Completed, None);
    let treatment = trace_bundle(&traces, "treatment", RolloutStatus::Completed, None);
    let mut manifest = manifest_for(
        &temp.path().join("scenario"),
        baseline,
        treatment,
        post_run_evaluator(),
    );
    manifest.scenarios[0].baseline.post_run = Some(PostRunEvidence {
        checks: vec![CheckResult {
            check_id: "hidden-tests".to_string(),
            success: false,
        }],
    });
    let result = evaluate_manifest_at(&manifest, 123)?;
    assert_eq!(
        result.scenarios[0].baseline.evaluator.status,
        super::EvaluatorStatus::Failed
    );
    assert_eq!(
        result.scenarios[0].baseline.evaluator.task_success,
        Some(false)
    );
    assert_eq!(
        result.scenarios[0].treatment.status,
        EvaluationStatus::EvaluatorUnavailable
    );
    assert_eq!(
        result.scenarios[0].treatment.error_code,
        Some(EvaluationErrorCode::MissingEvaluatorEvidence)
    );
    Ok(())
}

#[test]
fn post_run_result_remains_available_when_trace_is_cost_limited() -> Result<()> {
    let temp = TempDir::new()?;
    let traces = temp.path().join("traces");
    fs::create_dir_all(&traces)?;
    let baseline = trace_bundle(&traces, "baseline", RolloutStatus::Completed, None);
    let treatment = trace_bundle(&traces, "treatment", RolloutStatus::Completed, None);
    let mut manifest = manifest_for(
        &temp.path().join("scenario"),
        baseline,
        treatment,
        post_run_evaluator(),
    );
    manifest.scenarios[0].baseline.post_run = Some(PostRunEvidence {
        checks: vec![CheckResult {
            check_id: "hidden-tests".to_string(),
            success: true,
        }],
    });
    manifest.scenarios[0].treatment.post_run = manifest.scenarios[0].baseline.post_run.clone();
    manifest.scenarios[0].budget.max_trace_bytes = 1;

    let result = evaluate_manifest_at(&manifest, 123)?;
    assert_eq!(
        result.scenarios[0].baseline.status,
        EvaluationStatus::CostLimited
    );
    assert_eq!(
        result.scenarios[0].baseline.evaluator.task_success,
        Some(true)
    );
    assert_eq!(
        result.scenarios[0].treatment.evaluator.task_success,
        Some(true)
    );
    Ok(())
}

#[test]
fn false_attribution_is_not_reported_as_memory_success() -> Result<()> {
    let temp = TempDir::new()?;
    let traces = temp.path().join("traces");
    fs::create_dir_all(&traces)?;
    let baseline = trace_bundle(
        &traces,
        "baseline",
        RolloutStatus::Failed,
        Some("cargo test"),
    );
    let treatment = trace_bundle(
        &traces,
        "treatment",
        RolloutStatus::Completed,
        Some("cargo test"),
    );
    let mut manifest = manifest_for(
        &temp.path().join("scenario"),
        baseline,
        treatment,
        trace_outcome_evaluator(),
    );

    let result = evaluate_manifest_at(&manifest, 123)?;
    assert_eq!(
        result.scenarios[0].attribution,
        AttributionAssessment::ImprovedWithoutMemoryEvidence
    );

    manifest.scenarios[0].attribution = AttributionEvidence {
        exposure: crate::AttributionState::Yes,
        followed: crate::AttributionState::No,
        evidence_id: Some("exposure-record-1".to_string()),
    };
    let result = evaluate_manifest_at(&manifest, 123)?;
    assert_eq!(
        result.scenarios[0].attribution,
        AttributionAssessment::ExposedNotFollowed
    );
    Ok(())
}

#[test]
fn validation_rejects_non_memory_pair_differences_and_overlap() -> Result<()> {
    let temp = TempDir::new()?;
    let traces = temp.path().join("traces");
    fs::create_dir_all(&traces)?;
    let baseline = trace_bundle(&traces, "baseline", RolloutStatus::Completed, None);
    let treatment = trace_bundle(&traces, "treatment", RolloutStatus::Completed, None);
    let mut manifest = manifest_for(
        &temp.path().join("scenario"),
        baseline,
        treatment,
        trace_outcome_evaluator(),
    );

    manifest.scenarios[0].treatment.retrieval.selected_card_ids =
        vec!["card-duplicate".to_string(), "card-duplicate".to_string()];
    assert!(manifest.validate().is_err());
    manifest.scenarios[0]
        .treatment
        .retrieval
        .selected_card_ids
        .clear();

    manifest.scenarios[0].treatment.policy_fingerprint = "different-policy".to_string();
    assert!(manifest.validate().is_err());

    manifest.scenarios[0].treatment.policy_fingerprint = "policy-fingerprint".to_string();
    manifest.scenarios[0].treatment.worktree = manifest.scenarios[0].baseline.worktree.clone();
    assert!(manifest.validate().is_err());
    Ok(())
}

#[test]
fn missing_trace_and_missing_judge_are_explicit_partial_results() -> Result<()> {
    let temp = TempDir::new()?;
    let traces = temp.path().join("traces");
    fs::create_dir_all(&traces)?;
    let treatment = trace_bundle(&traces, "treatment", RolloutStatus::Completed, None);
    let mut manifest = manifest_for(
        &temp.path().join("scenario"),
        temp.path().join("missing"),
        treatment,
        EvaluatorSpec {
            kind: EvaluatorKind::HumanJudge,
            checks: Vec::new(),
            require_all_checks: true,
        },
    );

    let result = evaluate_manifest_at(&manifest, 123)?;
    assert_eq!(
        result.scenarios[0].baseline.error_code,
        Some(EvaluationErrorCode::TraceUnavailable)
    );
    assert_eq!(
        result.scenarios[0].treatment.status,
        EvaluationStatus::EvaluatorUnavailable
    );
    assert_eq!(
        result.scenarios[0].treatment.error_code,
        Some(EvaluationErrorCode::MissingEvaluatorEvidence)
    );
    manifest.scenarios[0].treatment.judge = Some(super::JudgeLabel {
        label_id: "human-label-1".to_string(),
        task_success: Some(true),
        verification_success: Some(true),
        score_bps: Some(9_000),
    });
    let result = evaluate_manifest_at(&manifest, 123)?;
    assert_eq!(
        result.scenarios[0].treatment.evaluator.status,
        super::EvaluatorStatus::Passed
    );
    Ok(())
}

#[test]
fn result_writer_rejects_primary_checkout_and_card_store_paths() -> Result<()> {
    let temp = TempDir::new()?;
    let traces = temp.path().join("traces");
    fs::create_dir_all(&traces)?;
    let baseline = trace_bundle(&traces, "baseline", RolloutStatus::Completed, None);
    let treatment = trace_bundle(&traces, "treatment", RolloutStatus::Completed, None);
    let manifest = manifest_for(
        &temp.path().join("scenario"),
        baseline,
        treatment,
        trace_outcome_evaluator(),
    );
    let result = evaluate_manifest_at(&manifest, 123)?;

    let primary = manifest.scenarios[0]
        .repository
        .primary_checkout
        .as_ref()
        .expect("primary");
    assert!(write_evaluation_result(&manifest, &result, primary.join("results.json")).is_err());
    let card_store = manifest.scenarios[0]
        .treatment
        .memory
        .card_store_path
        .as_ref()
        .expect("card store");
    assert!(write_evaluation_result(&manifest, &result, card_store.join("results.json")).is_err());

    let output = temp.path().join("results.json");
    write_evaluation_result(&manifest, &result, &output)?;
    assert!(output.exists());
    Ok(())
}
