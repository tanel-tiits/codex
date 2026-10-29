use codex_config::types::AgentImprovementCaptureMode;
use codex_config::types::AgentImprovementRetrievalMode;
use codex_context_fragments::ContextualUserFragment;
use codex_extension_api::ExtensionData;
use codex_extension_api::ExtensionRegistryBuilder;
use codex_extension_api::ThreadLifecycleContributor;
use codex_extension_api::ThreadStopInput;
use codex_extension_api::TurnInputContext;
use codex_extension_api::TurnInputContributor;
use codex_extension_api::TurnLifecycleContributor;
use codex_extension_api::TurnStopInput;
use codex_protocol::models::ContentItem;
use codex_protocol::protocol::SessionSource;
use codex_protocol::user_input::UserInput;
use codex_rollout_trace::ThreadStartedTraceMetadata;
use codex_rollout_trace::ThreadTraceContext;
use pretty_assertions::assert_eq;
use std::path::PathBuf;

use super::extension::{
    AgentImprovementExtension, AgentImprovementExtensionConfig, TEST_ADVISORY_CONTEXT_PREFIX,
};
use super::schema::{AgentImprovementEvidence, ExperienceCard, FailureClass, TaskFamily};
use super::store::append_card;

fn config(root: &std::path::Path, enabled: bool) -> AgentImprovementExtensionConfig {
    AgentImprovementExtensionConfig {
        enabled,
        root: root.to_path_buf(),
        max_cards: 8,
        max_context_tokens: 1_200,
        min_relevance_score: 0,
        capture: AgentImprovementCaptureMode::Off,
        max_candidates: 8,
        retrieval_mode: AgentImprovementRetrievalMode::Legacy,
        intervention_context_tokens: 480,
    }
}

fn capture_config(root: &std::path::Path) -> AgentImprovementExtensionConfig {
    AgentImprovementExtensionConfig {
        enabled: false,
        root: root.to_path_buf(),
        max_cards: 8,
        max_context_tokens: 1_200,
        min_relevance_score: 0,
        capture: AgentImprovementCaptureMode::Candidate,
        max_candidates: 8,
        retrieval_mode: AgentImprovementRetrievalMode::Legacy,
        intervention_context_tokens: 480,
    }
}

async fn contribute(
    config: AgentImprovementExtensionConfig,
    query: &str,
) -> Vec<Box<dyn ContextualUserFragment + Send>> {
    let step_store = ExtensionData::new("step");
    let session_store = ExtensionData::new("session");
    let thread_store = ExtensionData::new("thread");
    let turn_store = ExtensionData::new("turn");
    thread_store.insert(config);
    contribute_in_thread(
        &thread_store,
        query,
        &step_store,
        &session_store,
        &turn_store,
    )
    .await
}

async fn contribute_in_thread(
    thread_store: &ExtensionData,
    query: &str,
    step_store: &ExtensionData,
    session_store: &ExtensionData,
    turn_store: &ExtensionData,
) -> Vec<Box<dyn ContextualUserFragment + Send>> {
    let extension = AgentImprovementExtension;
    let input = TurnInputContext {
        step_store,
        turn_id: "turn-1".to_string(),
        user_input: vec![UserInput::Text {
            text: query.to_string(),
            text_elements: Vec::new(),
        }],
        environments: Vec::new(),
    };
    extension
        .contribute(input, None, session_store, thread_store, turn_store)
        .await
}

fn card(id: &str, family: TaskFamily, signal: &str, recommendation: &str) -> ExperienceCard {
    ExperienceCard::new(
        id,
        100,
        family,
        FailureClass::Completed,
        AgentImprovementEvidence::default(),
        vec![signal.to_string()],
        recommendation,
        vec!["test_like_command_succeeded".to_string()],
    )
}

fn fragment_text(fragment: &dyn ContextualUserFragment) -> String {
    match fragment.body() {
        ContentItem::InputText { text } => text,
        other => panic!("expected input text, got {other:?}"),
    }
}

#[tokio::test]
async fn disabled_configuration_contributes_no_context() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let fragments = contribute(config(tempdir.path(), false), "fix parser tests").await;
    assert!(fragments.is_empty());
}

#[tokio::test]
async fn missing_store_contributes_no_context() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let fragments = contribute(config(tempdir.path(), true), "fix parser tests").await;
    assert!(fragments.is_empty());
}

#[tokio::test]
async fn relevant_cards_are_marked_advisory_and_untrusted() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    append_card(
        tempdir.path(),
        &card(
            "trace-parser",
            TaskFamily::Coding,
            "parser",
            "Run the parser tests.",
        ),
    )
    .expect("append relevant card");
    append_card(
        tempdir.path(),
        &card(
            "trace-review",
            TaskFamily::Review,
            "review",
            "Review the diff.",
        ),
    )
    .expect("append unrelated card");

    let fragments = contribute(config(tempdir.path(), true), "fix the parser").await;
    assert_eq!(fragments.len(), 1);
    let text = fragment_text(fragments[0].as_ref());
    assert!(text.contains(TEST_ADVISORY_CONTEXT_PREFIX));
    assert!(text.contains("task=coding"));
    assert!(!text.contains("task=review"));
}

#[tokio::test]
async fn intervention_mode_injects_one_card_once_per_task_and_resets_for_new_task() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let intervention_card = card(
        "trace-intervention",
        TaskFamily::Coding,
        "parser",
        "Run the focused parser verification before editing more files.",
    );
    let card_id = intervention_card.card_id.clone();
    append_card(tempdir.path(), &intervention_card).expect("append intervention card");

    let mut intervention_config = config(tempdir.path(), true);
    intervention_config.retrieval_mode = AgentImprovementRetrievalMode::Intervention;
    intervention_config.intervention_context_tokens = 480;
    let step_store = ExtensionData::new("step");
    let session_store = ExtensionData::new("session");
    let thread_store = ExtensionData::new("thread");
    let turn_store = ExtensionData::new("turn");
    thread_store.insert(intervention_config);

    let first = contribute_in_thread(
        &thread_store,
        "fix the parser",
        &step_store,
        &session_store,
        &turn_store,
    )
    .await;
    assert_eq!(first.len(), 1);
    let first_text = fragment_text(first[0].as_ref());
    assert!(first_text.contains("Historical advisory intervention"));
    assert!(first_text.contains("Action:"));
    assert!(first_text.contains("Verification:"));
    assert!(!first_text.contains(&card_id));

    let repeated = contribute_in_thread(
        &thread_store,
        "fix the parser",
        &step_store,
        &session_store,
        &turn_store,
    )
    .await;
    assert!(repeated.is_empty());

    let new_task = contribute_in_thread(
        &thread_store,
        "implement the database migration",
        &step_store,
        &session_store,
        &turn_store,
    )
    .await;
    assert_eq!(new_task.len(), 1);

    let unrelated = contribute_in_thread(
        &thread_store,
        "review the documentation",
        &step_store,
        &session_store,
        &turn_store,
    )
    .await;
    assert!(unrelated.is_empty());
}

#[tokio::test]
async fn store_read_errors_do_not_fail_turn_preparation() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let store_path = tempdir.path().join("not-a-directory");
    std::fs::write(&store_path, "not a directory").expect("create conflicting store path");

    let fragments = contribute(config(&store_path, true), "fix parser tests").await;
    assert!(fragments.is_empty());
}

#[tokio::test]
async fn turn_stop_captures_a_pending_candidate_while_thread_remains_open() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let trace_root = tempdir.path().join("traces");
    let store_root = tempdir.path().join("state");
    let trace = ThreadTraceContext::start_root_in_root_for_test(
        &trace_root,
        ThreadStartedTraceMetadata {
            thread_id: "thread-root".to_string(),
            agent_path: "/root".to_string(),
            task_name: None,
            nickname: None,
            agent_role: None,
            session_source: SessionSource::Exec,
            cwd: PathBuf::from("/workspace"),
            rollout_path: None,
            model: "test-model".to_string(),
            provider_name: "test-provider".to_string(),
            approval_policy: "never".to_string(),
            sandbox_policy: "read-only".to_string(),
        },
    )
    .expect("start trace");
    trace.record_codex_turn_started("turn-1");

    let extension = AgentImprovementExtension;
    let session_store = ExtensionData::new("session");
    let thread_store = ExtensionData::new("thread-root");
    let turn_store = ExtensionData::new("turn-1");
    thread_store.insert(capture_config(&store_root));
    thread_store.insert(trace);

    extension
        .on_turn_stop(TurnStopInput {
            session_store: &session_store,
            thread_store: &thread_store,
            turn_store: &turn_store,
        })
        .await;

    let candidates = super::candidates::load_candidates(&store_root).expect("load candidates");
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].trigger, super::CandidateTrigger::TurnStop);
    assert_eq!(candidates[0].source_turn_id.as_deref(), Some("turn-1"));
    assert!(!store_root.join("cards.jsonl").exists());
}

#[tokio::test]
async fn thread_stop_captures_a_pending_candidate_without_activating_a_card() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let trace_root = tempdir.path().join("traces");
    let store_root = tempdir.path().join("state");
    let trace = ThreadTraceContext::start_root_in_root_for_test(
        &trace_root,
        ThreadStartedTraceMetadata {
            thread_id: "thread-root".to_string(),
            agent_path: "/root".to_string(),
            task_name: None,
            nickname: None,
            agent_role: None,
            session_source: SessionSource::Exec,
            cwd: PathBuf::from("/workspace"),
            rollout_path: None,
            model: "test-model".to_string(),
            provider_name: "test-provider".to_string(),
            approval_policy: "never".to_string(),
            sandbox_policy: "read-only".to_string(),
        },
    )
    .expect("start trace");
    trace.record_codex_turn_started("turn-1");

    let extension = AgentImprovementExtension;
    let session_store = ExtensionData::new("session");
    let thread_store = ExtensionData::new("thread-root");
    thread_store.insert(capture_config(&store_root));
    thread_store.insert(trace);

    extension
        .on_thread_stop(ThreadStopInput {
            session_store: &session_store,
            thread_store: &thread_store,
        })
        .await;

    let candidates = super::candidates::load_candidates(&store_root).expect("load candidates");
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].status, super::CandidateStatus::Pending);
    assert!(!store_root.join("cards.jsonl").exists());
}

#[tokio::test]
async fn candidate_capture_store_errors_do_not_escape_thread_stop() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let trace_root = tempdir.path().join("traces");
    let store_root = tempdir.path().join("state");
    let trace = ThreadTraceContext::start_root_in_root_for_test(
        &trace_root,
        ThreadStartedTraceMetadata {
            thread_id: "thread-root".to_string(),
            agent_path: "/root".to_string(),
            task_name: None,
            nickname: None,
            agent_role: None,
            session_source: SessionSource::Exec,
            cwd: PathBuf::from("/workspace"),
            rollout_path: None,
            model: "test-model".to_string(),
            provider_name: "test-provider".to_string(),
            approval_policy: "never".to_string(),
            sandbox_policy: "read-only".to_string(),
        },
    )
    .expect("start trace");
    trace.record_codex_turn_started("turn-1");
    std::fs::write(&store_root, "not a directory").expect("create conflicting store path");

    let extension = AgentImprovementExtension;
    let session_store = ExtensionData::new("session");
    let thread_store = ExtensionData::new("thread-root");
    thread_store.insert(capture_config(&store_root));
    thread_store.insert(trace);

    extension
        .on_thread_stop(ThreadStopInput {
            session_store: &session_store,
            thread_store: &thread_store,
        })
        .await;

    assert!(store_root.is_file());
}

#[tokio::test]
async fn registry_install_registers_lifecycle_and_turn_input_hooks() {
    let mut builder = ExtensionRegistryBuilder::<codex_core::config::Config>::new();
    super::install(&mut builder);
    let registry = builder.build();
    assert_eq!(registry.thread_lifecycle_contributors().len(), 1);
    assert_eq!(registry.config_contributors().len(), 1);
    assert_eq!(registry.turn_lifecycle_contributors().len(), 1);
    assert_eq!(registry.turn_input_contributors().len(), 1);
}
