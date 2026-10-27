use std::collections::BTreeMap;

use codex_rollout_trace::AgentOrigin;
use codex_rollout_trace::AgentThread;
use codex_rollout_trace::CodexTurn;
use codex_rollout_trace::ConversationBody;
use codex_rollout_trace::ConversationItem;
use codex_rollout_trace::ConversationItemKind;
use codex_rollout_trace::ConversationPart;
use codex_rollout_trace::ConversationRole;
use codex_rollout_trace::ExecutionStatus;
use codex_rollout_trace::ExecutionWindow;
use codex_rollout_trace::InferenceCall;
use codex_rollout_trace::RolloutStatus;
use codex_rollout_trace::RolloutTrace;
use codex_rollout_trace::TerminalOperation;
use codex_rollout_trace::TerminalOperationKind;
use codex_rollout_trace::TerminalRequest;
use codex_rollout_trace::TerminalResult;
use codex_rollout_trace::TokenUsage;
use codex_rollout_trace::ToolCall;
use codex_rollout_trace::ToolCallKind;
use codex_rollout_trace::ToolCallRequester;
use codex_rollout_trace::ToolCallSummary;
use pretty_assertions::assert_eq;

use super::{FailureClass, TaskFamily, select_relevant, summarize_rollout};

fn base_trace(status: RolloutStatus) -> RolloutTrace {
    let mut trace = RolloutTrace {
        schema_version: 1,
        trace_id: "trace-test".to_string(),
        rollout_id: "rollout-test".to_string(),
        started_at_unix_ms: 100,
        ended_at_unix_ms: Some(200),
        status,
        root_thread_id: "root".to_string(),
        threads: BTreeMap::new(),
        codex_turns: BTreeMap::new(),
        conversation_items: BTreeMap::new(),
        inference_calls: BTreeMap::new(),
        code_cells: BTreeMap::new(),
        tool_calls: BTreeMap::new(),
        terminal_sessions: BTreeMap::new(),
        terminal_operations: BTreeMap::new(),
        compactions: BTreeMap::new(),
        compaction_requests: BTreeMap::new(),
        interaction_edges: BTreeMap::new(),
        raw_payloads: BTreeMap::new(),
    };
    trace.threads.insert(
        "root".to_string(),
        AgentThread {
            thread_id: "root".to_string(),
            agent_path: "/root".to_string(),
            nickname: None,
            origin: AgentOrigin::Root,
            execution: window(ExecutionStatus::Completed),
            default_model: None,
            conversation_item_ids: Vec::new(),
        },
    );
    trace
}

fn window(status: ExecutionStatus) -> ExecutionWindow {
    ExecutionWindow {
        started_at_unix_ms: 100,
        started_seq: 1,
        ended_at_unix_ms: Some(200),
        ended_seq: Some(2),
        status,
    }
}

fn add_user_input(trace: &mut RolloutTrace, text: &str) {
    let item_id = format!("input-{}", trace.conversation_items.len());
    trace.conversation_items.insert(
        item_id.clone(),
        ConversationItem {
            item_id: item_id.clone(),
            responses_item_id: None,
            thread_id: "root".to_string(),
            codex_turn_id: None,
            first_seen_at_unix_ms: 100,
            role: ConversationRole::User,
            channel: None,
            kind: ConversationItemKind::Message,
            agent_message: None,
            body: ConversationBody {
                parts: vec![ConversationPart::Text {
                    text: text.to_string(),
                }],
            },
            call_id: None,
            produced_by: vec![codex_rollout_trace::ProducerRef::UserInput],
        },
    );
    trace
        .threads
        .get_mut("root")
        .expect("root thread")
        .conversation_item_ids
        .push(item_id);
}

fn add_turn(trace: &mut RolloutTrace) {
    trace.codex_turns.insert(
        "turn-1".to_string(),
        CodexTurn {
            codex_turn_id: "turn-1".to_string(),
            thread_id: "root".to_string(),
            execution: window(ExecutionStatus::Completed),
            input_item_ids: Vec::new(),
        },
    );
}

fn add_inference(trace: &mut RolloutTrace, status: ExecutionStatus) {
    trace.inference_calls.insert(
        "inference-1".to_string(),
        InferenceCall {
            inference_call_id: "inference-1".to_string(),
            thread_id: "root".to_string(),
            codex_turn_id: "turn-1".to_string(),
            execution: window(status),
            model: "test-model".to_string(),
            provider_name: "test-provider".to_string(),
            response_id: None,
            upstream_request_id: None,
            request_item_ids: Vec::new(),
            response_item_ids: Vec::new(),
            tool_call_ids_started_by_response: Vec::new(),
            usage: Some(TokenUsage {
                input_tokens: 10,
                cached_input_tokens: 2,
                cache_write_input_tokens: 0,
                output_tokens: 5,
                reasoning_output_tokens: 3,
            }),
            raw_request_payload_id: "request".to_string(),
            raw_response_payload_id: None,
        },
    );
}

fn add_failed_tool(trace: &mut RolloutTrace) {
    trace.tool_calls.insert(
        "tool-only".to_string(),
        ToolCall {
            tool_call_id: "tool-only".to_string(),
            mcp_call_id: None,
            model_visible_call_id: None,
            code_mode_runtime_tool_id: None,
            thread_id: "root".to_string(),
            started_by_codex_turn_id: Some("turn-1".to_string()),
            execution: window(ExecutionStatus::Failed),
            requester: ToolCallRequester::Model,
            kind: ToolCallKind::Mcp {
                server: "example".to_string(),
                tool: "lookup".to_string(),
            },
            model_visible_call_item_ids: Vec::new(),
            model_visible_output_item_ids: Vec::new(),
            terminal_operation_id: None,
            summary: ToolCallSummary::Generic {
                label: "lookup".to_string(),
                input_preview: None,
                output_preview: None,
            },
            raw_invocation_payload_id: None,
            raw_result_payload_id: None,
            raw_runtime_payload_ids: Vec::new(),
        },
    );
}

fn add_terminal(
    trace: &mut RolloutTrace,
    display_command: &str,
    exit_code: Option<i32>,
    stderr: &str,
    status: ExecutionStatus,
) {
    let operation_id = "operation-1".to_string();
    let tool_call_id = "tool-1".to_string();
    trace.tool_calls.insert(
        tool_call_id.clone(),
        ToolCall {
            tool_call_id: tool_call_id.clone(),
            mcp_call_id: None,
            model_visible_call_id: None,
            code_mode_runtime_tool_id: None,
            thread_id: "root".to_string(),
            started_by_codex_turn_id: Some("turn-1".to_string()),
            execution: window(status.clone()),
            requester: ToolCallRequester::Model,
            kind: ToolCallKind::ExecCommand,
            model_visible_call_item_ids: Vec::new(),
            model_visible_output_item_ids: Vec::new(),
            terminal_operation_id: Some(operation_id.clone()),
            summary: ToolCallSummary::Terminal {
                operation_id: operation_id.clone(),
            },
            raw_invocation_payload_id: None,
            raw_result_payload_id: None,
            raw_runtime_payload_ids: Vec::new(),
        },
    );
    trace.terminal_operations.insert(
        operation_id.clone(),
        TerminalOperation {
            operation_id,
            terminal_id: None,
            tool_call_id,
            kind: TerminalOperationKind::ExecCommand,
            execution: window(status),
            request: TerminalRequest::ExecCommand {
                command: display_command
                    .split_whitespace()
                    .map(str::to_string)
                    .collect(),
                display_command: display_command.to_string(),
                cwd: "/repo".to_string(),
                yield_time_ms: None,
                max_output_tokens: None,
            },
            result: Some(TerminalResult {
                exit_code,
                stdout: String::new(),
                stderr: stderr.to_string(),
                formatted_output: None,
                original_token_count: None,
                chunk_id: None,
            }),
            model_observations: Vec::new(),
            raw_payload_ids: Vec::new(),
        },
    );
}

#[test]
fn completed_test_trace_is_summarized_without_raw_output() {
    let mut trace = base_trace(RolloutStatus::Completed);
    add_user_input(&mut trace, "Implement the parser and run tests");
    add_turn(&mut trace);
    add_inference(&mut trace, ExecutionStatus::Completed);
    add_terminal(
        &mut trace,
        "cargo test -p example",
        Some(0),
        "secret terminal output should not be persisted",
        ExecutionStatus::Completed,
    );

    let card = summarize_rollout(&trace);
    assert_eq!(card.task_family, TaskFamily::Testing);
    assert_eq!(card.failure_class, FailureClass::Completed);
    assert_eq!(card.evidence.turn_count, 1);
    assert_eq!(card.evidence.inference_count, 1);
    assert_eq!(card.evidence.verification_step_count, 1);
    assert_eq!(card.evidence.token_usage.input_tokens, 10);
    assert!(
        card.activity_labels
            .iter()
            .any(|label| label == "exec_command")
    );
    assert!(!card.topic_hashes.is_empty());
    assert!(
        !serde_json::to_string(&card)
            .expect("serialize card")
            .contains("secret terminal output")
    );
}

#[test]
fn completed_turn_is_summarized_before_rollout_end_is_recorded() {
    let mut trace = base_trace(RolloutStatus::Running);
    add_user_input(&mut trace, "Implement the parser");
    add_turn(&mut trace);

    let card = summarize_rollout(&trace);
    assert_eq!(card.failure_class, FailureClass::Completed);
}

#[test]
fn summarized_topic_keys_retrieve_a_matching_future_task() {
    let mut trace = base_trace(RolloutStatus::Completed);
    add_user_input(&mut trace, "Implement the parser");
    add_turn(&mut trace);
    let card = summarize_rollout(&trace);

    assert_eq!(
        select_relevant(std::slice::from_ref(&card), "fix the parser", 1),
        vec![card]
    );
}

#[test]
fn command_failure_is_distinguished_from_tool_and_model_failure() {
    let mut trace = base_trace(RolloutStatus::Failed);
    add_user_input(&mut trace, "Fix the coding issue");
    add_turn(&mut trace);
    add_terminal(
        &mut trace,
        "cargo check",
        Some(1),
        "compilation failed",
        ExecutionStatus::Failed,
    );

    let card = summarize_rollout(&trace);
    assert_eq!(card.task_family, TaskFamily::Coding);
    assert_eq!(card.failure_class, FailureClass::CommandFailure);
    assert!(card.signals.iter().any(|signal| signal == "command_failed"));
}

#[test]
fn failed_tool_is_classified_without_terminal_activity() {
    let mut trace = base_trace(RolloutStatus::Failed);
    add_user_input(&mut trace, "Investigate the lookup failure");
    add_turn(&mut trace);
    add_failed_tool(&mut trace);

    let card = summarize_rollout(&trace);
    assert_eq!(card.failure_class, FailureClass::ToolFailure);
    assert_eq!(card.evidence.failed_tool_count, 1);
    assert!(card.signals.iter().any(|signal| signal == "tool_failed"));
}

#[test]
fn policy_boundary_and_cancellation_are_explicit_classes() {
    let mut blocked = base_trace(RolloutStatus::Failed);
    add_user_input(&mut blocked, "Investigate the sandbox issue");
    add_turn(&mut blocked);
    add_terminal(
        &mut blocked,
        "cargo test",
        Some(1),
        "permission denied by sandbox",
        ExecutionStatus::Failed,
    );
    assert_eq!(
        summarize_rollout(&blocked).failure_class,
        FailureClass::PolicyBlocked
    );

    let mut cancelled = base_trace(RolloutStatus::Aborted);
    add_user_input(&mut cancelled, "Continue the implementation");
    add_turn(&mut cancelled);
    assert_eq!(
        summarize_rollout(&cancelled).failure_class,
        FailureClass::Cancelled
    );
}

#[test]
fn model_failure_and_missing_verification_are_deterministic() {
    let mut model_failed = base_trace(RolloutStatus::Failed);
    add_user_input(&mut model_failed, "Investigate the failure");
    add_turn(&mut model_failed);
    add_inference(&mut model_failed, ExecutionStatus::Failed);
    assert_eq!(
        summarize_rollout(&model_failed).failure_class,
        FailureClass::ModelFailure
    );

    let mut incomplete = base_trace(RolloutStatus::Completed);
    add_user_input(&mut incomplete, "Implement the change");
    add_turn(&mut incomplete);
    add_terminal(
        &mut incomplete,
        "cargo check",
        Some(0),
        "",
        ExecutionStatus::Completed,
    );
    let card = summarize_rollout(&incomplete);
    assert_eq!(card.failure_class, FailureClass::IncompleteVerification);
    assert!(
        card.verification
            .iter()
            .any(|label| label == "run_targeted_tests")
    );
}

#[test]
fn task_family_detection_is_conservative_and_bounded() {
    for (request, expected) in [
        ("Review the pull request", TaskFamily::Review),
        ("Write the README documentation", TaskFamily::Documentation),
        ("Make a plan for the migration", TaskFamily::Planning),
        ("Diagnose the regression", TaskFamily::Investigation),
        ("Check the latest release", TaskFamily::General),
    ] {
        let mut trace = base_trace(RolloutStatus::Completed);
        add_user_input(&mut trace, request);
        assert_eq!(summarize_rollout(&trace).task_family, expected);
    }
}
