use std::path::Path;

use anyhow::Result;
use codex_agent_improvement_extension::AgentImprovementEvidence;
use codex_agent_improvement_extension::AttributionEvidence;
use codex_agent_improvement_extension::CandidateTrigger;
use codex_agent_improvement_extension::EVALUATION_MANIFEST_SCHEMA_VERSION;
use codex_agent_improvement_extension::EVALUATION_RESULT_SCHEMA_VERSION;
use codex_agent_improvement_extension::EvaluationAggregate;
use codex_agent_improvement_extension::EvaluationBudget;
use codex_agent_improvement_extension::EvaluationCondition;
use codex_agent_improvement_extension::EvaluationConditionProvenance;
use codex_agent_improvement_extension::EvaluationConditionResult;
use codex_agent_improvement_extension::EvaluationManifest;
use codex_agent_improvement_extension::EvaluationMemory;
use codex_agent_improvement_extension::EvaluationMemoryProvenance;
use codex_agent_improvement_extension::EvaluationRepository;
use codex_agent_improvement_extension::EvaluationResult;
use codex_agent_improvement_extension::EvaluationScenario;
use codex_agent_improvement_extension::EvaluationScenarioResult;
use codex_agent_improvement_extension::EvaluationSplit;
use codex_agent_improvement_extension::EvaluationStatus;
use codex_agent_improvement_extension::EvaluationTask;
use codex_agent_improvement_extension::EvaluatorKind;
use codex_agent_improvement_extension::EvaluatorResult;
use codex_agent_improvement_extension::EvaluatorSpec;
use codex_agent_improvement_extension::EvaluatorStatus;
use codex_agent_improvement_extension::ExperienceCandidate;
use codex_agent_improvement_extension::ExperienceCard;
use codex_agent_improvement_extension::FailureClass;
use codex_agent_improvement_extension::LiveEvaluationPlan;
use codex_agent_improvement_extension::LiveEvaluationScenario;
use codex_agent_improvement_extension::MetricDelta;
use codex_agent_improvement_extension::TaskFamily;
use codex_agent_improvement_extension::append_candidate;
use codex_agent_improvement_extension::append_card;
use codex_rollout_trace::ExecutionStatus;
use codex_rollout_trace::RawTraceEventPayload;
use codex_rollout_trace::RolloutStatus;
use codex_rollout_trace::TraceWriter;
use serde_json::Value;
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};
use tempfile::TempDir;

fn codex_command(codex_home: &Path) -> Result<assert_cmd::Command> {
    let mut command = assert_cmd::Command::new(codex_utils_cargo_bin::cargo_bin("codex")?);
    command.env("CODEX_HOME", codex_home);
    Ok(command)
}

fn minimal_completed_bundle(root: &Path) -> Result<std::path::PathBuf> {
    let bundle = root.join("trace-bundle");
    let writer = TraceWriter::create(
        &bundle,
        "trace-cli-test".to_string(),
        "rollout-cli-test".to_string(),
        "thread-root".to_string(),
    )?;
    writer.append(RawTraceEventPayload::RolloutStarted {
        trace_id: "trace-cli-test".to_string(),
        root_thread_id: "thread-root".to_string(),
    })?;
    writer.append(RawTraceEventPayload::ThreadStarted {
        thread_id: "thread-root".to_string(),
        agent_path: "/root".to_string(),
        metadata_payload: None,
    })?;
    writer.append(RawTraceEventPayload::CodexTurnStarted {
        codex_turn_id: "turn-1".to_string(),
        thread_id: "thread-root".to_string(),
    })?;
    writer.append(RawTraceEventPayload::CodexTurnEnded {
        codex_turn_id: "turn-1".to_string(),
        status: ExecutionStatus::Completed,
    })?;
    writer.append(RawTraceEventPayload::ThreadEnded {
        thread_id: "thread-root".to_string(),
        status: RolloutStatus::Completed,
    })?;
    writer.append(RawTraceEventPayload::RolloutEnded {
        status: RolloutStatus::Completed,
    })?;
    Ok(bundle)
}

fn current_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock after epoch")
        .as_millis() as i64
}

fn promotion_result(evaluated_at_unix_ms: i64) -> EvaluationResult {
    let condition = |enabled: bool, task_success: bool, trace_id: &str| EvaluationConditionResult {
        provenance: EvaluationConditionProvenance {
            execution_config_fingerprint: "cli-execution-v1".to_string(),
            policy_fingerprint: "cli-policy-v1".to_string(),
            worktree_fingerprint: "cli-worktree-v1".to_string(),
            memory: EvaluationMemoryProvenance {
                enabled,
                card_set_fingerprint: if enabled {
                    "cli-cards-treatment".to_string()
                } else {
                    "cli-cards-none".to_string()
                },
                retrieval_policy_fingerprint: "cli-retrieval-v1".to_string(),
                card_schema_version: 2,
            },
        },
        trace_id: Some(trace_id.to_string()),
        status: EvaluationStatus::Evaluated,
        error_code: None,
        metrics: None,
        evaluator: EvaluatorResult {
            status: if task_success {
                EvaluatorStatus::Passed
            } else {
                EvaluatorStatus::Failed
            },
            task_success: Some(task_success),
            verification_success: Some(task_success),
            checks: Vec::new(),
            judge_score_bps: None,
        },
    };
    EvaluationResult {
        schema_version: EVALUATION_RESULT_SCHEMA_VERSION,
        manifest_id: "cli-promotion-manifest".to_string(),
        evaluated_at_unix_ms,
        scenario_count: 1,
        all: EvaluationAggregate::default(),
        held_out: EvaluationAggregate::default(),
        by_task_family: BTreeMap::new(),
        scenarios: vec![EvaluationScenarioResult {
            scenario_id: "cli-promotion-scenario".to_string(),
            task_family: TaskFamily::Coding,
            split: EvaluationSplit::HeldOut,
            task_fingerprint: "cli-task-v1".to_string(),
            repository_snapshot_fingerprint: "cli-repo-v1".to_string(),
            evaluator: EvaluatorKind::HumanJudge,
            baseline: condition(false, false, "cli-baseline-trace"),
            treatment: condition(true, true, "cli-treatment-trace"),
            delta: MetricDelta {
                task_success: Some(1),
                verification_success: Some(1),
                input_tokens: None,
                output_tokens: None,
                reasoning_output_tokens: None,
                wall_time_ms: None,
                tool_calls: None,
                terminal_operations: None,
                retry_count: None,
                repeated_failure_count: None,
                policy_event_count: None,
            },
            attribution:
                codex_agent_improvement_extension::AttributionAssessment::ExposedAndOutcomeImproved,
        }],
    }
}

#[test]
fn summarize_command_is_json_and_persistence_is_explicit() -> Result<()> {
    let home = TempDir::new()?;
    let traces = TempDir::new()?;
    let bundle = minimal_completed_bundle(traces.path())?;
    let card_output = traces.path().join("card.json");
    let store = traces.path().join("store");

    let mut read_only = codex_command(home.path())?;
    let output = read_only
        .args([
            "debug",
            "agent-improvement",
            "summarize",
            bundle.to_str().expect("UTF-8 bundle path"),
            "--output",
            card_output.to_str().expect("UTF-8 output path"),
        ])
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!store.exists());

    let stdout_card: Value = serde_json::from_slice(&output.stdout)?;
    let file_card: Value = serde_json::from_slice(&std::fs::read(&card_output)?)?;
    assert_eq!(stdout_card, file_card);
    assert_eq!(stdout_card["schema_version"], 2);
    assert_eq!(stdout_card["failure_class"], "completed");

    let mut persisted = codex_command(home.path())?;
    let output = persisted
        .args([
            "debug",
            "agent-improvement",
            "summarize",
            bundle.to_str().expect("UTF-8 bundle path"),
            "--store",
            store.to_str().expect("UTF-8 store path"),
        ])
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let cards = std::fs::read_to_string(store.join("cards.jsonl"))?;
    assert_eq!(cards.lines().count(), 1);

    let mut render = codex_command(home.path())?;
    let output = render
        .args([
            "debug",
            "agent-improvement",
            "render",
            "--store",
            store.to_str().expect("UTF-8 store path"),
            "--query",
            "completed",
        ])
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let rendered = String::from_utf8(output.stdout)?;
    assert!(rendered.contains("task=general"));
    assert!(rendered.contains("outcome=completed"));

    Ok(())
}

#[test]
fn eval_command_writes_redacted_paired_results() -> Result<()> {
    let home = TempDir::new()?;
    let baseline_root = TempDir::new()?;
    let treatment_root = TempDir::new()?;
    let baseline_bundle = minimal_completed_bundle(baseline_root.path())?;
    let treatment_bundle = minimal_completed_bundle(treatment_root.path())?;
    let scenario_root = TempDir::new()?;
    let primary = scenario_root.path().join("primary");
    let baseline_worktree = scenario_root.path().join("baseline-worktree");
    let treatment_worktree = scenario_root.path().join("treatment-worktree");
    let card_store = scenario_root.path().join("active-card-store");
    for path in [
        &primary,
        &baseline_worktree,
        &treatment_worktree,
        &card_store,
    ] {
        std::fs::create_dir_all(path)?;
    }

    let condition = |trace_bundle: std::path::PathBuf,
                     worktree: std::path::PathBuf,
                     enabled: bool,
                     card_set_fingerprint: &str,
                     card_store_path: Option<std::path::PathBuf>|
     -> EvaluationCondition {
        EvaluationCondition {
            trace_bundle,
            worktree,
            worktree_fingerprint: "worktree-fingerprint".to_string(),
            execution_config_fingerprint: "config-fingerprint".to_string(),
            policy_fingerprint: "policy-fingerprint".to_string(),
            memory: EvaluationMemory {
                enabled,
                card_set_fingerprint: card_set_fingerprint.to_string(),
                retrieval_policy_fingerprint: "retrieval-v1".to_string(),
                card_schema_version: 2,
                card_store_path,
            },
            judge: None,
        }
    };
    let manifest = EvaluationManifest {
        schema_version: EVALUATION_MANIFEST_SCHEMA_VERSION,
        manifest_id: "cli-eval-manifest".to_string(),
        scenarios: vec![EvaluationScenario {
            scenario_id: "cli-scenario-1".to_string(),
            task_family: TaskFamily::Testing,
            split: EvaluationSplit::HeldOut,
            task: EvaluationTask {
                task_id: "cli-task".to_string(),
                prompt: None,
                reference: Some("fixture/cli-task".to_string()),
                fingerprint: "task-fingerprint".to_string(),
            },
            repository: EvaluationRepository {
                snapshot_fingerprint: "repo-fingerprint".to_string(),
                primary_checkout: Some(primary),
            },
            baseline: condition(
                baseline_bundle,
                baseline_worktree,
                false,
                "cards-none",
                None,
            ),
            treatment: condition(
                treatment_bundle,
                treatment_worktree,
                true,
                "cards-treatment-v1",
                Some(card_store),
            ),
            evaluator: EvaluatorSpec {
                kind: EvaluatorKind::TraceOutcome,
                checks: Vec::new(),
                require_all_checks: true,
            },
            budget: Default::default(),
            attribution: AttributionEvidence::default(),
        }],
    };
    let manifest_path = scenario_root.path().join("scenario-set.json");
    let result_path = scenario_root.path().join("results.json");
    std::fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest)?)?;

    let mut command = codex_command(home.path())?;
    let output = command
        .args([
            "debug",
            "agent-improvement",
            "eval",
            manifest_path.to_str().expect("UTF-8 manifest path"),
            "--output",
            result_path.to_str().expect("UTF-8 result path"),
        ])
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout: Value = serde_json::from_slice(&output.stdout)?;
    let file: Value = serde_json::from_slice(&std::fs::read(&result_path)?)?;
    assert_eq!(stdout, file);
    assert_eq!(file["schema_version"], 1);
    assert_eq!(file["held_out"]["scenario_count"], 1);
    assert!(!serde_json::to_string(&file)?.contains("fixture/cli-task"));
    Ok(())
}

#[test]
fn prompt_debug_includes_relevant_experience_when_enabled() -> Result<()> {
    let home = TempDir::new()?;
    let store = home.path().join("agent-improvement");
    std::fs::write(
        home.path().join("config.toml"),
        "[agent_improvement]\nenabled = true\n",
    )?;
    append_card(
        &store,
        &ExperienceCard::new(
            "trace-prompt-debug",
            100,
            TaskFamily::Coding,
            FailureClass::CommandFailure,
            AgentImprovementEvidence::default(),
            vec!["parser_failure".to_string()],
            "Reproduce the first failing parser command.",
            vec!["reproduce_first_failing_command".to_string()],
        ),
    )?;

    let mut command = codex_command(home.path())?;
    let output = command
        .args(["debug", "prompt-input", "fix the parser"])
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let prompt: Value = serde_json::from_slice(&output.stdout)?;
    let prompt_json = serde_json::to_string(&prompt)?;
    assert!(prompt_json.contains("Historical experience from prior Codex runs"));
    assert!(prompt_json.contains("task=coding"));
    assert!(prompt_json.contains("parser_failure"));

    Ok(())
}

#[test]
fn candidate_review_requires_explicit_acceptance_before_prompt_retrieval() -> Result<()> {
    let home = TempDir::new()?;
    let store = home.path().join("agent-improvement");
    let card = ExperienceCard::new(
        "trace-candidate-cli",
        100,
        TaskFamily::Coding,
        FailureClass::CommandFailure,
        AgentImprovementEvidence::default(),
        vec!["parser_failure".to_string()],
        "Reproduce the first failing parser command.",
        vec!["reproduce_first_failing_command".to_string()],
    );
    let candidate = ExperienceCandidate::from_card(card, 100, CandidateTrigger::ThreadStop, None);
    append_candidate(&store, &candidate, 8)?;

    let mut list = codex_command(home.path())?;
    let output = list
        .args([
            "debug",
            "agent-improvement",
            "candidates",
            "list",
            "--store",
            store.to_str().expect("UTF-8 store path"),
        ])
        .output()?;
    assert!(output.status.success());
    let listed: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(listed.as_array().expect("candidate list").len(), 1);
    assert_eq!(listed[0]["status"], "pending");
    assert!(!store.join("cards.jsonl").exists());

    let mut prompt_before_accept = codex_command(home.path())?;
    let output = prompt_before_accept
        .args(["debug", "prompt-input", "fix the parser"])
        .output()?;
    assert!(output.status.success());
    let prompt: Value = serde_json::from_slice(&output.stdout)?;
    assert!(
        !serde_json::to_string(&prompt)?.contains("Historical experience from prior Codex runs")
    );

    let candidate_id = listed[0]["candidate_id"]
        .as_str()
        .expect("candidate ID")
        .to_string();
    let mut accept = codex_command(home.path())?;
    let output = accept
        .args([
            "debug",
            "agent-improvement",
            "candidates",
            "accept",
            "--store",
            store.to_str().expect("UTF-8 store path"),
            &candidate_id,
        ])
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(store.join("cards.jsonl").exists());

    std::fs::write(
        home.path().join("config.toml"),
        "[agent_improvement]\nenabled = true\n",
    )?;
    let mut prompt_after_accept = codex_command(home.path())?;
    let output = prompt_after_accept
        .args(["debug", "prompt-input", "fix the parser"])
        .output()?;
    assert!(output.status.success());
    let prompt: Value = serde_json::from_slice(&output.stdout)?;
    assert!(
        serde_json::to_string(&prompt)?.contains("Historical experience from prior Codex runs")
    );

    Ok(())
}

#[test]
fn candidate_rejection_keeps_the_card_inactive() -> Result<()> {
    let home = TempDir::new()?;
    let store = home.path().join("agent-improvement");
    let card = ExperienceCard::new(
        "trace-rejected-candidate-cli",
        100,
        TaskFamily::Testing,
        FailureClass::ToolFailure,
        AgentImprovementEvidence::default(),
        vec!["tool_failed".to_string()],
        "Inspect the tool failure before retrying.",
        vec!["reproduce_first_failing_command".to_string()],
    );
    let candidate = ExperienceCandidate::from_card(card, 100, CandidateTrigger::TurnError, None);
    append_candidate(&store, &candidate, 8)?;

    let candidate_id = candidate.candidate_id.clone();
    let mut reject = codex_command(home.path())?;
    let output = reject
        .args([
            "debug",
            "agent-improvement",
            "candidates",
            "reject",
            "--store",
            store.to_str().expect("UTF-8 store path"),
            &candidate_id,
        ])
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!store.join("cards.jsonl").exists());

    let mut list_pending = codex_command(home.path())?;
    let output = list_pending
        .args([
            "debug",
            "agent-improvement",
            "candidates",
            "list",
            "--store",
            store.to_str().expect("UTF-8 store path"),
        ])
        .output()?;
    assert!(output.status.success());
    let pending: Value = serde_json::from_slice(&output.stdout)?;
    assert!(pending.as_array().expect("pending list").is_empty());

    let mut list_all = codex_command(home.path())?;
    let output = list_all
        .args([
            "debug",
            "agent-improvement",
            "candidates",
            "list",
            "--all",
            "--store",
            store.to_str().expect("UTF-8 store path"),
        ])
        .output()?;
    assert!(output.status.success());
    let all: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(all[0]["candidate_id"], candidate_id);
    assert_eq!(all[0]["status"], "rejected");

    Ok(())
}

#[test]
fn proposal_drafting_is_read_only_and_evidence_validation_is_explicit() -> Result<()> {
    let home = TempDir::new()?;
    let store = home.path().join("agent-improvement");
    let card = ExperienceCard::new(
        "trace-proposal-cli",
        100,
        TaskFamily::Coding,
        FailureClass::CommandFailure,
        AgentImprovementEvidence::default(),
        vec!["command_failed".to_string()],
        "Reproduce the first failing command.",
        vec!["reproduce_first_failing_command".to_string()],
    );
    let candidate = ExperienceCandidate::from_card(card, 100, CandidateTrigger::ThreadStop, None);
    append_candidate(&store, &candidate, 8)?;
    let candidate_store_before = std::fs::read(store.join("candidates.jsonl"))?;
    let proposal_path = home.path().join("proposal.json");

    let mut draft = codex_command(home.path())?;
    let output = draft
        .args([
            "debug",
            "agent-improvement",
            "proposals",
            "draft-card",
            "--store",
            store.to_str().expect("UTF-8 store path"),
            &candidate.candidate_id,
            "--evaluation-scenario",
            "scenario-held-out",
            "--output",
            proposal_path.to_str().expect("UTF-8 proposal path"),
        ])
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let proposal: Value = serde_json::from_slice(&std::fs::read(&proposal_path)?)?;
    assert_eq!(proposal["status"], "proposed");
    assert_eq!(proposal["candidate_type"], "experience_card_update");
    assert_eq!(
        std::fs::read(store.join("candidates.jsonl"))?,
        candidate_store_before
    );
    assert!(!store.join("cards.jsonl").exists());

    let mut structural_validate = codex_command(home.path())?;
    let output = structural_validate
        .args([
            "debug",
            "agent-improvement",
            "proposals",
            "validate",
            proposal_path.to_str().expect("UTF-8 proposal path"),
        ])
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let mut evidence_validate = codex_command(home.path())?;
    let output = evidence_validate
        .args([
            "debug",
            "agent-improvement",
            "proposals",
            "validate",
            proposal_path.to_str().expect("UTF-8 proposal path"),
            "--candidate-store",
            store.to_str().expect("UTF-8 store path"),
        ])
        .output()?;
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("missing improvement evidence"));
    assert!(!store.join("cards.jsonl").exists());

    Ok(())
}

#[test]
fn proposal_promotion_cli_is_manual_scoped_and_reversible() -> Result<()> {
    let home = TempDir::new()?;
    let store = home.path().join("agent-improvement");
    let created_at = current_unix_ms();
    let card = ExperienceCard::new(
        "trace-promotion-cli",
        created_at,
        TaskFamily::Coding,
        FailureClass::CommandFailure,
        AgentImprovementEvidence::default(),
        vec!["command_failed".to_string()],
        "Reproduce the first failing parser command.",
        vec!["reproduce_first_failing_command".to_string()],
    );
    let candidate =
        ExperienceCandidate::from_card(card, created_at, CandidateTrigger::ThreadStop, None);
    append_candidate(&store, &candidate, 8)?;
    let proposal_path = home.path().join("promotion-proposal.json");
    let evaluation_path = home.path().join("promotion-results.json");
    std::fs::write(
        &evaluation_path,
        serde_json::to_vec_pretty(&promotion_result(created_at + 1))?,
    )?;

    let mut draft = codex_command(home.path())?;
    let output = draft
        .args([
            "debug",
            "agent-improvement",
            "proposals",
            "draft-card",
            "--store",
            store.to_str().expect("UTF-8 store path"),
            &candidate.candidate_id,
            "--evaluation-scenario",
            "cli-promotion-scenario",
            "--output",
            proposal_path.to_str().expect("UTF-8 proposal path"),
        ])
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let mut validate = codex_command(home.path())?;
    let output = validate
        .args([
            "debug",
            "agent-improvement",
            "proposals",
            "validate",
            proposal_path.to_str().expect("UTF-8 proposal path"),
            "--state-store",
            store.to_str().expect("UTF-8 store path"),
            "--candidate-store",
            store.to_str().expect("UTF-8 store path"),
            "--evaluation-results",
            evaluation_path.to_str().expect("UTF-8 evaluation path"),
            "--actor",
            "reviewer",
        ])
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout)?["state"],
        "validated"
    );

    let mut evaluate = codex_command(home.path())?;
    let output = evaluate
        .args([
            "debug",
            "agent-improvement",
            "proposals",
            "evaluate",
            proposal_path.to_str().expect("UTF-8 proposal path"),
            "--store",
            store.to_str().expect("UTF-8 store path"),
            "--candidate-store",
            store.to_str().expect("UTF-8 store path"),
            "--evaluation-results",
            evaluation_path.to_str().expect("UTF-8 evaluation path"),
            "--actor",
            "evaluator",
        ])
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout)?["state"],
        "evaluated"
    );

    for (operation, actor) in [("accept", "reviewer"), ("promote", "reviewer")] {
        let mut command = codex_command(home.path())?;
        let output = command
            .args([
                "debug",
                "agent-improvement",
                "proposals",
                operation,
                proposal_path.to_str().expect("UTF-8 proposal path"),
                "--store",
                store.to_str().expect("UTF-8 store path"),
                "--actor",
                actor,
            ])
            .output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert!(store.join("cards.jsonl").exists());

    let mut status = codex_command(home.path())?;
    let output = status
        .args([
            "debug",
            "agent-improvement",
            "proposals",
            "status",
            proposal_path.to_str().expect("UTF-8 proposal path"),
            "--store",
            store.to_str().expect("UTF-8 store path"),
        ])
        .output()?;
    assert!(output.status.success());
    let active_status: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(active_status["state"], "active");
    assert!(active_status["evaluation"].is_object());

    let mut render = codex_command(home.path())?;
    let output = render
        .args([
            "debug",
            "agent-improvement",
            "render",
            "--store",
            store.to_str().expect("UTF-8 store path"),
            "--query",
            "fix the parser",
        ])
        .output()?;
    assert!(output.status.success());
    assert!(String::from_utf8(output.stdout)?.contains("first failing parser command"));

    let mut rollback = codex_command(home.path())?;
    let output = rollback
        .args([
            "debug",
            "agent-improvement",
            "proposals",
            "rollback",
            proposal_path.to_str().expect("UTF-8 proposal path"),
            "--store",
            store.to_str().expect("UTF-8 store path"),
            "--actor",
            "reviewer",
        ])
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout)?["state"],
        "retired"
    );

    Ok(())
}

#[test]
fn card_lifecycle_commands_record_adoption_and_retirement() -> Result<()> {
    let home = TempDir::new()?;
    let store = home.path().join("agent-improvement");
    let card = ExperienceCard::new(
        "trace-card-lifecycle-cli",
        100,
        TaskFamily::Coding,
        FailureClass::CommandFailure,
        AgentImprovementEvidence::default(),
        vec!["parser_failure".to_string()],
        "Reproduce the first failing parser command.",
        vec!["reproduce_first_failing_command".to_string()],
    );
    let card_id = card.card_id.clone();
    append_card(&store, &card)?;

    let mut feedback = codex_command(home.path())?;
    let output = feedback
        .args([
            "debug",
            "agent-improvement",
            "cards",
            "feedback",
            "--store",
            store.to_str().expect("UTF-8 store path"),
            &card_id,
            "--value",
            "positive",
        ])
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let feedback_card: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(feedback_card["adoption"]["feedback"], "positive");

    let mut outcome = codex_command(home.path())?;
    let output = outcome
        .args([
            "debug",
            "agent-improvement",
            "cards",
            "outcome",
            "--store",
            store.to_str().expect("UTF-8 store path"),
            &card_id,
            "--followed",
            "yes",
            "--verified",
            "yes",
        ])
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let outcome_card: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(outcome_card["adoption"]["followed"], "yes");
    assert_eq!(outcome_card["adoption"]["verified"], "yes");

    let mut retire = codex_command(home.path())?;
    let output = retire
        .args([
            "debug",
            "agent-improvement",
            "cards",
            "retire",
            "--store",
            store.to_str().expect("UTF-8 store path"),
            &card_id,
        ])
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let retired_card: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(retired_card["status"], "retired");
    Ok(())
}

#[test]
fn evolve_command_runs_records_maintains_exports_and_forgets_explicitly() -> Result<()> {
    let home = TempDir::new()?;
    let store = TempDir::new()?;
    let mut card = ExperienceCard::new(
        "trace-evolve-cli",
        current_unix_ms(),
        TaskFamily::Coding,
        FailureClass::CommandFailure,
        AgentImprovementEvidence::default(),
        Vec::new(),
        "reproduce the first failing command",
        Vec::new(),
    );
    card.recurrence.count = 3;
    append_card(store.path(), &card)?;

    let report_path = store.path().join("briefs.json");
    let mut run = codex_command(home.path())?;
    let output = run
        .args([
            "debug",
            "agent-improvement",
            "evolve",
            "run",
            "--store",
            store.path().to_str().expect("UTF-8 store path"),
            "--output",
            report_path.to_str().expect("UTF-8 report path"),
        ])
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["cards_examined"], 1);
    let brief = report["briefs"]
        .as_array()
        .expect("brief array")
        .first()
        .expect("brief");
    let brief_id = brief["brief_id"].as_str().expect("brief ID");
    let fingerprint = brief["fingerprint"].as_str().expect("brief fingerprint");
    assert_eq!(
        serde_json::from_slice::<Value>(&std::fs::read(&report_path)?)?,
        report
    );

    let mut record = codex_command(home.path())?;
    let output = record
        .args([
            "debug",
            "agent-improvement",
            "evolve",
            "record",
            brief_id,
            "--store",
            store.path().to_str().expect("UTF-8 store path"),
            "--fingerprint",
            fingerprint,
            "--outcome",
            "improved",
            "--reason",
            "held-out task success improved",
        ])
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let event: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(event["outcome"], "improved");

    let mut maintain = codex_command(home.path())?;
    let output = maintain
        .args([
            "debug",
            "agent-improvement",
            "evolve",
            "maintain",
            "--store",
            store.path().to_str().expect("UTF-8 store path"),
            "--apply",
        ])
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let maintenance: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(maintenance["retired_count"], 0);

    let export_path = store.path().join("export.jsonl");
    let mut export = codex_command(home.path())?;
    let output = export
        .args([
            "debug",
            "agent-improvement",
            "evolve",
            "export",
            "--store",
            store.path().to_str().expect("UTF-8 store path"),
            "--output",
            export_path.to_str().expect("UTF-8 export path"),
        ])
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(std::fs::read_to_string(&export_path)?.lines().count(), 1);

    let archive_path = store.path().join("forget-archive.jsonl");
    let mut forget = codex_command(home.path())?;
    let output = forget
        .args([
            "debug",
            "agent-improvement",
            "evolve",
            "forget",
            "--store",
            store.path().to_str().expect("UTF-8 store path"),
            "--archive",
            archive_path.to_str().expect("UTF-8 archive path"),
            &card.card_id,
        ])
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let forgotten: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(forgotten["removed_card_ids"][0], card.card_id);
    assert!(std::fs::read_to_string(&archive_path)?.contains("trace-evolve-cli"));
    Ok(())
}

#[test]
fn evolve_evaluate_dry_run_validates_a_private_held_out_plan() -> Result<()> {
    let home = TempDir::new()?;
    let store = TempDir::new()?;
    let mut card = ExperienceCard::new(
        "trace-live-plan-cli",
        current_unix_ms(),
        TaskFamily::Coding,
        FailureClass::CommandFailure,
        AgentImprovementEvidence::default(),
        Vec::new(),
        "reproduce the first failing command",
        Vec::new(),
    );
    card.recurrence.count = 3;
    append_card(store.path(), &card)?;

    let report_path = store.path().join("briefs.json");
    let mut run = codex_command(home.path())?;
    let output = run
        .args([
            "debug",
            "agent-improvement",
            "evolve",
            "run",
            "--store",
            store.path().to_str().expect("UTF-8 store path"),
            "--output",
            report_path.to_str().expect("UTF-8 report path"),
        ])
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout)?;
    let brief = report["briefs"].as_array().expect("brief array")[0].clone();
    let brief_id = brief["brief_id"].as_str().expect("brief ID").to_string();
    let brief_fingerprint = brief["fingerprint"]
        .as_str()
        .expect("brief fingerprint")
        .to_string();
    let plan = LiveEvaluationPlan {
        schema_version: codex_agent_improvement_extension::LIVE_EVALUATION_PLAN_SCHEMA_VERSION,
        plan_id: "plan-live-cli".to_string(),
        brief_id,
        brief_fingerprint,
        primary_checkout: home.path().to_path_buf(),
        scenarios: (0..2)
            .map(|index| LiveEvaluationScenario {
                scenario_id: format!("live-scenario-{index}"),
                task_family: TaskFamily::Coding,
                split: EvaluationSplit::HeldOut,
                task: EvaluationTask {
                    task_id: format!("live-task-{index}"),
                    prompt: Some("fix the bounded fixture".to_string()),
                    reference: None,
                    fingerprint: format!("live-task-fingerprint-{index}"),
                },
                evaluator: EvaluatorSpec {
                    kind: EvaluatorKind::TraceOutcome,
                    checks: Vec::new(),
                    require_all_checks: true,
                },
                budget: EvaluationBudget::default(),
            })
            .collect(),
    };
    let plan_path = store.path().join("live-plan.json");
    std::fs::write(&plan_path, serde_json::to_vec_pretty(&plan)?)?;
    let output_path = store.path().join("live-results.json");
    let run_root = store.path().join("live-runs");

    let mut evaluate = codex_command(home.path())?;
    let output = evaluate
        .args([
            "debug",
            "agent-improvement",
            "evolve",
            "evaluate",
            plan.brief_id.as_str(),
            "--briefs",
            report_path.to_str().expect("UTF-8 report path"),
            "--plan",
            plan_path.to_str().expect("UTF-8 plan path"),
            "--store",
            store.path().to_str().expect("UTF-8 store path"),
            "--run-root",
            run_root.to_str().expect("UTF-8 run root"),
            "--output",
            output_path.to_str().expect("UTF-8 output path"),
            "--dry-run",
        ])
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let dry_run: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(dry_run["dry_run"], true);
    assert_eq!(dry_run["scenario_count"], 2);
    assert!(!run_root.exists());
    assert!(!output_path.exists());
    Ok(())
}
