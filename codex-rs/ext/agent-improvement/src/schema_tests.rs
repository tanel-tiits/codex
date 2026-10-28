use pretty_assertions::assert_eq;

use super::schema::{
    AgentImprovementEvidence, ConfidenceLevel, EvidenceFact, EvidenceProfile, ExperienceCard,
    ExperienceScope, FailureClass, FailureSignature, HypothesisKind, InterventionKind,
    ObservedStep, OperationClass, OutcomeKind, TaskFamily, ToolFamily, ToolchainFamily,
    VerificationState, topic_hashes_for_text, workspace_scope_key,
};

#[test]
fn card_json_round_trip_preserves_the_bounded_schema() {
    let card = ExperienceCard::new(
        "trace-1",
        123,
        TaskFamily::Coding,
        FailureClass::CommandFailure,
        AgentImprovementEvidence {
            terminal_operation_count: 2,
            failed_terminal_operation_count: 1,
            ..Default::default()
        },
        vec!["command_failed".to_string()],
        "Inspect the first failing command before retrying.",
        vec!["reproduce_failure".to_string()],
    );

    let encoded = serde_json::to_string(&card).expect("serialize card");
    let decoded: ExperienceCard = serde_json::from_str(&encoded).expect("deserialize card");
    assert_eq!(decoded, card);
    assert!(!encoded.contains("stdout"));
    assert!(!encoded.contains("prompt"));
}

#[test]
fn topic_retrieval_keys_are_opaque_and_bounded() {
    let hashes = topic_hashes_for_text("fix the parser and parser tests");
    assert_eq!(hashes.len(), 1);
    assert!(hashes[0].starts_with("topic_"));
    assert!(!hashes[0].contains("parser"));

    let card = ExperienceCard::new_with_details(
        "trace-topic",
        123,
        TaskFamily::Coding,
        FailureClass::Completed,
        AgentImprovementEvidence::default(),
        hashes,
        vec!["apply_patch".to_string()],
        Vec::new(),
        "Preserve the observed workflow.",
        vec!["verify".to_string()],
    );
    let encoded = serde_json::to_string(&card).expect("serialize topic card");
    assert!(!encoded.contains("parser"));
    assert!(card.is_bounded());
}

#[test]
fn card_id_is_deterministic_for_the_same_non_secret_inputs() {
    let make_card = || {
        ExperienceCard::new(
            "trace-2",
            456,
            TaskFamily::Testing,
            FailureClass::Completed,
            AgentImprovementEvidence::default(),
            vec!["tests_ran".to_string()],
            "Preserve the observed verification steps.",
            vec!["targeted_tests".to_string()],
        )
    };

    assert_eq!(make_card().card_id, make_card().card_id);
}

#[test]
fn rich_schema_distinguishes_observed_facts_from_hypotheses_and_recommendations() {
    let card = ExperienceCard::new_with_rich_details(
        "trace-rich",
        456,
        TaskFamily::Coding,
        FailureClass::CommandFailure,
        AgentImprovementEvidence::default(),
        topic_hashes_for_text("cargo parser"),
        vec!["exec_command".to_string()],
        vec!["command_failed".to_string()],
        "Reproduce the first failing command.",
        vec!["reproduce_first_failing_command".to_string()],
        ExperienceScope {
            workspace_key: Some(workspace_scope_key("/private/repo")),
            toolchain_family: Some(ToolchainFamily::Rust),
            ..Default::default()
        },
        FailureSignature {
            operation_class: OperationClass::Command,
            tool_family: ToolFamily::ExecCommand,
            verification_state: VerificationState::Failed,
            ..Default::default()
        },
        EvidenceProfile {
            observed_sequence: vec![
                ObservedStep::Turn,
                ObservedStep::Inference,
                ObservedStep::TerminalOperation,
            ],
            facts: vec![EvidenceFact::CommandFailed],
            hypotheses: vec![HypothesisKind::LikelyCommandEnvironmentIssue],
            interventions: vec![InterventionKind::ReproduceFirstFailingCommand],
            outcomes: vec![OutcomeKind::FailureObserved],
        },
        ConfidenceLevel::High,
    );
    let encoded = serde_json::to_string(&card).expect("serialize rich card");
    assert_eq!(card.schema_version, 2);
    assert_eq!(card.confidence, ConfidenceLevel::High);
    assert!(encoded.contains("command_failed"));
    assert!(encoded.contains("terminal_operation"));
    assert!(encoded.contains("likely_command_environment_issue"));
    assert!(!encoded.contains("/private/repo"));
    assert!(encoded.contains(&workspace_scope_key("/private/repo")));
}

#[test]
fn legacy_and_unknown_schema_versions_are_handled_explicitly() {
    let card = ExperienceCard::new(
        "trace-legacy",
        456,
        TaskFamily::Testing,
        FailureClass::IncompleteVerification,
        AgentImprovementEvidence::default(),
        vec!["verification_missing".to_string()],
        "Run targeted tests.",
        vec!["run_targeted_tests".to_string()],
    );
    let mut legacy = serde_json::to_value(&card).expect("serialize card");
    legacy["schema_version"] = serde_json::json!(1);
    let migrated: ExperienceCard = serde_json::from_value(legacy).expect("deserialize legacy");
    let migrated = migrated.normalize_for_load().expect("migrate legacy card");
    assert_eq!(migrated.schema_version, 2);
    assert_eq!(migrated.source_trace_id, "trace-legacy");
    assert!(migrated.is_bounded());

    let mut unknown = serde_json::to_value(&card).expect("serialize card");
    unknown["schema_version"] = serde_json::json!(42);
    let unknown: ExperienceCard = serde_json::from_value(unknown).expect("deserialize unknown");
    assert!(unknown.normalize_for_load().is_none());
}

#[test]
fn newer_unknown_fields_are_ignored_without_relaxing_bounds() {
    let card = ExperienceCard::new(
        "trace-forward-compatible",
        456,
        TaskFamily::Coding,
        FailureClass::Completed,
        AgentImprovementEvidence::default(),
        vec!["rollout_completed".to_string()],
        "Preserve the observed workflow.",
        vec!["preserve_observed_workflow".to_string()],
    );
    let mut value = serde_json::to_value(&card).expect("serialize card");
    value["future_evidence"] = serde_json::json!({"untrusted": true});
    let decoded: ExperienceCard = serde_json::from_value(value).expect("deserialize card");
    assert_eq!(decoded, card);
    assert!(decoded.is_bounded());
}
