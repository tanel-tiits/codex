use codex_context_fragments::ContextualUserFragment;
use codex_extension_api::ExtensionData;
use codex_extension_api::ExtensionRegistryBuilder;
use codex_extension_api::TurnInputContext;
use codex_extension_api::TurnInputContributor;
use codex_protocol::models::ContentItem;
use codex_protocol::user_input::UserInput;
use pretty_assertions::assert_eq;

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
    }
}

async fn contribute(
    config: AgentImprovementExtensionConfig,
    query: &str,
) -> Vec<Box<dyn ContextualUserFragment + Send>> {
    let extension = AgentImprovementExtension;
    let step_store = ExtensionData::new("step");
    let session_store = ExtensionData::new("session");
    let thread_store = ExtensionData::new("thread");
    let turn_store = ExtensionData::new("turn");
    thread_store.insert(config);
    let input = TurnInputContext {
        step_store: &step_store,
        turn_id: "turn-1".to_string(),
        user_input: vec![UserInput::Text {
            text: query.to_string(),
            text_elements: Vec::new(),
        }],
        environments: Vec::new(),
    };
    extension
        .contribute(input, None, &session_store, &thread_store, &turn_store)
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
async fn store_read_errors_do_not_fail_turn_preparation() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let store_path = tempdir.path().join("not-a-directory");
    std::fs::write(&store_path, "not a directory").expect("create conflicting store path");

    let fragments = contribute(config(&store_path, true), "fix parser tests").await;
    assert!(fragments.is_empty());
}

#[tokio::test]
async fn registry_install_registers_lifecycle_and_turn_input_hooks() {
    let mut builder = ExtensionRegistryBuilder::<codex_core::config::Config>::new();
    super::install(&mut builder);
    let registry = builder.build();
    assert_eq!(registry.thread_lifecycle_contributors().len(), 1);
    assert_eq!(registry.config_contributors().len(), 1);
    assert_eq!(registry.turn_input_contributors().len(), 1);
}
