use pretty_assertions::assert_eq;

use super::schema::{
    AgentImprovementEvidence, ExperienceCard, FailureClass, TaskFamily, topic_hashes_for_text,
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
