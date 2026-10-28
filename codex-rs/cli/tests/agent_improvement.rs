use std::path::Path;

use anyhow::Result;
use codex_agent_improvement_extension::AgentImprovementEvidence;
use codex_agent_improvement_extension::AttributionEvidence;
use codex_agent_improvement_extension::CandidateTrigger;
use codex_agent_improvement_extension::EVALUATION_MANIFEST_SCHEMA_VERSION;
use codex_agent_improvement_extension::EvaluationCondition;
use codex_agent_improvement_extension::EvaluationManifest;
use codex_agent_improvement_extension::EvaluationMemory;
use codex_agent_improvement_extension::EvaluationRepository;
use codex_agent_improvement_extension::EvaluationScenario;
use codex_agent_improvement_extension::EvaluationSplit;
use codex_agent_improvement_extension::EvaluationTask;
use codex_agent_improvement_extension::EvaluatorKind;
use codex_agent_improvement_extension::EvaluatorSpec;
use codex_agent_improvement_extension::ExperienceCandidate;
use codex_agent_improvement_extension::ExperienceCard;
use codex_agent_improvement_extension::FailureClass;
use codex_agent_improvement_extension::TaskFamily;
use codex_agent_improvement_extension::append_candidate;
use codex_agent_improvement_extension::append_card;
use codex_rollout_trace::ExecutionStatus;
use codex_rollout_trace::RawTraceEventPayload;
use codex_rollout_trace::RolloutStatus;
use codex_rollout_trace::TraceWriter;
use serde_json::Value;
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
