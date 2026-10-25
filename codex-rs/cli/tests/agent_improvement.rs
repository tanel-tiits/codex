use std::path::Path;

use anyhow::Result;
use codex_agent_improvement_extension::AgentImprovementEvidence;
use codex_agent_improvement_extension::ExperienceCard;
use codex_agent_improvement_extension::FailureClass;
use codex_agent_improvement_extension::TaskFamily;
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
    assert_eq!(stdout_card["schema_version"], 1);
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
