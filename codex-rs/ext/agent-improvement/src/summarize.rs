use codex_rollout_trace::ConversationPart;
use codex_rollout_trace::ConversationRole;
use codex_rollout_trace::ExecutionStatus;
use codex_rollout_trace::RolloutStatus;
use codex_rollout_trace::RolloutTrace;
use codex_rollout_trace::TerminalRequest;
use codex_rollout_trace::ToolCallKind;

use crate::AgentImprovementEvidence;
use crate::ExperienceCard;
use crate::FailureClass;
use crate::TaskFamily;
use crate::schema::MAX_ACTIVITY_LABELS;
use crate::schema::MAX_CARD_SIGNALS;
use crate::schema::topic_hashes_for_text;

/// Converts one reduced rollout trace into a deterministic, privacy-bounded card.
pub fn summarize_rollout(trace: &RolloutTrace) -> ExperienceCard {
    let conversation_text = root_user_text(trace);
    let command_text = terminal_command_text(trace);
    let task_family = classify_task_family(&conversation_text, &command_text, trace);
    let evidence = evidence(trace);
    let activity_labels = activity_labels(trace);
    let topic_hashes = topic_hashes_for_text(&format!("{conversation_text} {command_text}"));
    let policy_blocked = has_policy_boundary(trace);
    let failure_class = classify_failure(trace, &evidence, task_family, policy_blocked);

    let mut signals = Vec::new();
    if matches!(&trace.status, RolloutStatus::Completed) {
        push_signal(&mut signals, "rollout_completed");
    }
    if evidence.turn_count > 0 {
        push_signal(&mut signals, "turn_activity_observed");
    }
    if evidence.terminal_operation_count > 0 {
        push_signal(&mut signals, "terminal_activity_observed");
    }
    if evidence.failed_terminal_operation_count > 0 {
        push_signal(&mut signals, "command_failed");
    }
    if evidence.failed_tool_count > 0 {
        push_signal(&mut signals, "tool_failed");
    }
    if evidence.failed_inference_count > 0 {
        push_signal(&mut signals, "inference_failed");
    }
    if evidence.verification_step_count > 0 {
        push_signal(&mut signals, "verification_observed");
    }
    if policy_blocked {
        push_signal(&mut signals, "policy_boundary_observed");
    }
    if matches!(failure_class, FailureClass::IncompleteVerification) {
        push_signal(&mut signals, "verification_missing");
    }
    if matches!(failure_class, FailureClass::Cancelled) {
        push_signal(&mut signals, "cancelled");
    }
    if evidence.turn_count == 0
        && evidence.inference_count == 0
        && evidence.tool_count == 0
        && evidence.terminal_operation_count == 0
    {
        push_signal(&mut signals, "no_runtime_evidence");
    }

    let verification = verification_labels(failure_class, &evidence);
    let recommendation = recommendation(failure_class);
    ExperienceCard::new_with_details(
        trace.trace_id.clone(),
        trace.ended_at_unix_ms.unwrap_or(trace.started_at_unix_ms),
        task_family,
        failure_class,
        evidence,
        topic_hashes,
        activity_labels,
        signals,
        recommendation,
        verification,
    )
}

fn evidence(trace: &RolloutTrace) -> AgentImprovementEvidence {
    let mut evidence = AgentImprovementEvidence {
        turn_count: bounded_count(trace.codex_turns.len()),
        inference_count: bounded_count(trace.inference_calls.len()),
        tool_count: bounded_count(trace.tool_calls.len()),
        terminal_operation_count: bounded_count(trace.terminal_operations.len()),
        ..Default::default()
    };

    for inference in trace.inference_calls.values() {
        if is_failed_execution(&inference.execution.status) {
            evidence.failed_inference_count = evidence.failed_inference_count.saturating_add(1);
        }
        if let Some(usage) = &inference.usage {
            evidence.token_usage.input_tokens = evidence
                .token_usage
                .input_tokens
                .saturating_add(usage.input_tokens);
            evidence.token_usage.output_tokens = evidence
                .token_usage
                .output_tokens
                .saturating_add(usage.output_tokens);
            evidence.token_usage.reasoning_output_tokens = evidence
                .token_usage
                .reasoning_output_tokens
                .saturating_add(usage.reasoning_output_tokens);
        }
    }

    for tool in trace.tool_calls.values() {
        if is_failed_execution(&tool.execution.status) {
            evidence.failed_tool_count = evidence.failed_tool_count.saturating_add(1);
        }
    }

    for operation in trace.terminal_operations.values() {
        if terminal_operation_failed(operation) {
            evidence.failed_terminal_operation_count =
                evidence.failed_terminal_operation_count.saturating_add(1);
        }
        if is_verification_command(&operation.request)
            && operation
                .result
                .as_ref()
                .and_then(|result| result.exit_code)
                == Some(0)
        {
            evidence.verification_step_count = evidence.verification_step_count.saturating_add(1);
        }
    }

    evidence
}

fn classify_failure(
    trace: &RolloutTrace,
    evidence: &AgentImprovementEvidence,
    task_family: TaskFamily,
    policy_blocked: bool,
) -> FailureClass {
    if policy_blocked {
        return FailureClass::PolicyBlocked;
    }

    let cancelled = matches!(&trace.status, RolloutStatus::Aborted)
        || trace
            .inference_calls
            .values()
            .any(|call| is_cancelled_execution(&call.execution.status))
        || trace
            .tool_calls
            .values()
            .any(|call| is_cancelled_execution(&call.execution.status))
        || trace
            .terminal_operations
            .values()
            .any(|operation| is_cancelled_execution(&operation.execution.status));
    if cancelled {
        return FailureClass::Cancelled;
    }
    if evidence.failed_terminal_operation_count > 0 {
        return FailureClass::CommandFailure;
    }
    if evidence.failed_tool_count > 0 {
        return FailureClass::ToolFailure;
    }
    if evidence.failed_inference_count > 0 {
        return FailureClass::ModelFailure;
    }

    if matches!(&trace.status, RolloutStatus::Completed)
        && matches!(
            task_family,
            TaskFamily::Coding | TaskFamily::Testing | TaskFamily::Review
        )
        && evidence.terminal_operation_count > 0
        && evidence.verification_step_count == 0
    {
        return FailureClass::IncompleteVerification;
    }
    if matches!(&trace.status, RolloutStatus::Completed) {
        FailureClass::Completed
    } else {
        FailureClass::Unknown
    }
}

fn classify_task_family(
    conversation_text: &str,
    command_text: &str,
    trace: &RolloutTrace,
) -> TaskFamily {
    let text = format!("{conversation_text} {command_text}").to_ascii_lowercase();
    if has_any(
        &text,
        &[
            "test",
            "pytest",
            "jest",
            "vitest",
            "cargo test",
            "unit test",
        ],
    ) {
        return TaskFamily::Testing;
    }
    if has_any(&text, &["review", "code review", "pull request", "diff"]) {
        return TaskFamily::Review;
    }
    if has_any(&text, &["plan", "planning", "roadmap", "design"]) {
        return TaskFamily::Planning;
    }
    if has_any(&text, &["readme", "documentation", "document", "docs"]) {
        return TaskFamily::Documentation;
    }
    if has_any(
        &text,
        &[
            "investigate",
            "investigation",
            "debug",
            "diagnose",
            "root cause",
        ],
    ) {
        return TaskFamily::Investigation;
    }
    if has_any(
        &text,
        &[
            "code",
            "coding",
            "implement",
            "implementation",
            "fix",
            "bug",
            "refactor",
            "feature",
        ],
    ) || trace
        .tool_calls
        .values()
        .any(|tool| matches!(&tool.kind, ToolCallKind::ApplyPatch))
        || !trace.terminal_operations.is_empty()
    {
        return TaskFamily::Coding;
    }
    TaskFamily::General
}

fn root_user_text(trace: &RolloutTrace) -> String {
    let Some(thread) = trace.threads.get(&trace.root_thread_id) else {
        return String::new();
    };
    thread
        .conversation_item_ids
        .iter()
        .filter_map(|item_id| trace.conversation_items.get(item_id))
        .filter(|item| matches!(&item.role, ConversationRole::User))
        .flat_map(|item| item.body.parts.iter())
        .filter_map(|part| match part {
            ConversationPart::Text { text } | ConversationPart::Summary { text } => {
                Some(text.as_str())
            }
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn terminal_command_text(trace: &RolloutTrace) -> String {
    trace
        .terminal_operations
        .values()
        .filter_map(|operation| match &operation.request {
            TerminalRequest::ExecCommand {
                display_command, ..
            } => Some(display_command.as_str()),
            TerminalRequest::WriteStdin { .. } => None,
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn has_policy_boundary(trace: &RolloutTrace) -> bool {
    trace.terminal_operations.values().any(|operation| {
        operation.result.as_ref().is_some_and(|result| {
            contains_policy_marker(&result.stdout)
                || contains_policy_marker(&result.stderr)
                || result
                    .formatted_output
                    .as_deref()
                    .is_some_and(contains_policy_marker)
        })
    }) || trace.tool_calls.values().any(|tool| match &tool.summary {
        codex_rollout_trace::ToolCallSummary::Generic {
            input_preview,
            output_preview,
            ..
        } => {
            input_preview.as_deref().is_some_and(contains_policy_marker)
                || output_preview
                    .as_deref()
                    .is_some_and(contains_policy_marker)
        }
        _ => false,
    })
}

fn contains_policy_marker(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    [
        "approval required",
        "permission denied",
        "operation not permitted",
        "not allowed",
        "sandbox",
        "policy blocked",
        "approval denied",
        "rejected",
    ]
    .iter()
    .any(|marker| text.contains(marker))
}

fn terminal_operation_failed(operation: &codex_rollout_trace::TerminalOperation) -> bool {
    is_failed_execution(&operation.execution.status)
        || operation
            .result
            .as_ref()
            .and_then(|result| result.exit_code)
            .is_some_and(|exit_code| exit_code != 0)
}

fn is_verification_command(request: &TerminalRequest) -> bool {
    let TerminalRequest::ExecCommand {
        display_command, ..
    } = request
    else {
        return false;
    };
    let command = display_command.to_ascii_lowercase();
    has_any(
        &command,
        &[
            "cargo test",
            "npm test",
            "pnpm test",
            "yarn test",
            "pytest",
            "jest",
            "vitest",
            "go test",
            "dotnet test",
            "mvn test",
            "gradle test",
            "rspec",
            " test",
        ],
    )
}

fn verification_labels(
    failure_class: FailureClass,
    evidence: &AgentImprovementEvidence,
) -> Vec<String> {
    if evidence.verification_step_count > 0 {
        return vec!["test_like_command_succeeded".to_string()];
    }
    let label = match failure_class {
        FailureClass::CommandFailure => "reproduce_first_failing_command",
        FailureClass::ToolFailure => "check_tool_preconditions",
        FailureClass::ModelFailure => "reduce_to_one_verifiable_step",
        FailureClass::PolicyBlocked => "request_required_authority",
        FailureClass::Cancelled => "resume_from_last_checkpoint",
        FailureClass::IncompleteVerification => "run_targeted_tests",
        FailureClass::Completed => "preserve_observed_workflow",
        FailureClass::Unknown => "inspect_reduced_trace",
    };
    vec![label.to_string()]
}

fn recommendation(failure_class: FailureClass) -> &'static str {
    match failure_class {
        FailureClass::Completed => {
            "Preserve the observed verification steps and reuse them only when the task context matches."
        }
        FailureClass::CommandFailure => {
            "Reproduce the first failing command and inspect its bounded result before editing."
        }
        FailureClass::ToolFailure => {
            "Check the tool preconditions and execution environment before retrying."
        }
        FailureClass::ModelFailure => {
            "Reduce the next attempt to one verifiable step and inspect the model boundary."
        }
        FailureClass::PolicyBlocked => {
            "Treat the approval or sandbox boundary as a hard constraint and request the required authority."
        }
        FailureClass::Cancelled => {
            "Resume from the last known checkpoint and re-verify existing changes before continuing."
        }
        FailureClass::IncompleteVerification => {
            "Run targeted tests and a relevant regression check before claiming completion."
        }
        FailureClass::Unknown => {
            "Inspect the reduced trace and identify the first observable failure before retrying."
        }
    }
}

fn push_signal(signals: &mut Vec<String>, signal: &str) {
    if signals.len() < MAX_CARD_SIGNALS && !signals.iter().any(|existing| existing == signal) {
        signals.push(signal.to_string());
    }
}

fn bounded_count(count: usize) -> u32 {
    count.min(u32::MAX as usize) as u32
}

fn is_failed_execution(status: &ExecutionStatus) -> bool {
    matches!(
        status,
        ExecutionStatus::Failed | ExecutionStatus::Cancelled | ExecutionStatus::Aborted
    )
}

fn is_cancelled_execution(status: &ExecutionStatus) -> bool {
    matches!(
        status,
        ExecutionStatus::Cancelled | ExecutionStatus::Aborted
    )
}

fn has_any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| {
        if needle.contains(' ') {
            text.contains(needle)
        } else {
            text.split(|character: char| !character.is_ascii_alphanumeric())
                .any(|token| token == *needle)
        }
    })
}

fn activity_labels(trace: &RolloutTrace) -> Vec<String> {
    let mut labels = Vec::new();
    for tool in trace.tool_calls.values() {
        push_bounded(&mut labels, tool_label(&tool.kind), MAX_ACTIVITY_LABELS);
    }
    for operation in trace.terminal_operations.values() {
        let label = match &operation.request {
            TerminalRequest::ExecCommand { .. } => "exec_command",
            TerminalRequest::WriteStdin { .. } => "write_stdin",
        };
        push_bounded(&mut labels, label, MAX_ACTIVITY_LABELS);
    }
    if !trace.code_cells.is_empty() {
        push_bounded(&mut labels, "code_mode", MAX_ACTIVITY_LABELS);
    }
    labels
}

fn tool_label(kind: &ToolCallKind) -> &'static str {
    match kind {
        ToolCallKind::ExecCommand => "exec_command",
        ToolCallKind::WriteStdin => "write_stdin",
        ToolCallKind::ApplyPatch => "apply_patch",
        ToolCallKind::Mcp { .. } => "mcp",
        ToolCallKind::Web => "web",
        ToolCallKind::ImageGeneration => "image_generation",
        ToolCallKind::SpawnAgent => "spawn_agent",
        ToolCallKind::AssignAgentTask => "assign_agent_task",
        ToolCallKind::SendMessage => "send_message",
        ToolCallKind::WaitAgent => "wait_agent",
        ToolCallKind::CloseAgent => "close_agent",
        ToolCallKind::Other { .. } => "other_tool",
    }
}

fn push_bounded(values: &mut Vec<String>, value: &str, max_items: usize) {
    if values.len() < max_items && !values.iter().any(|existing| existing == value) {
        values.push(value.to_string());
    }
}
