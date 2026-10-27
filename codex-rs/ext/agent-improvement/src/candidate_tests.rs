use pretty_assertions::assert_eq;

use super::candidate::{CandidateStatus, CandidateTrigger, ExperienceCandidate};
use super::schema::{AgentImprovementEvidence, ExperienceCard, FailureClass, TaskFamily};

fn card() -> ExperienceCard {
    ExperienceCard::new(
        "trace-candidate",
        100,
        TaskFamily::Coding,
        FailureClass::CommandFailure,
        AgentImprovementEvidence::default(),
        vec!["command_failed".to_string()],
        "Reproduce the first failing command.",
        vec!["reproduce_first_failing_command".to_string()],
    )
}

#[test]
fn candidate_identity_and_status_are_deterministic() {
    let first = ExperienceCandidate::from_card(
        card(),
        100,
        CandidateTrigger::ThreadStop,
        Some("turn-1".to_string()),
    );
    let second = ExperienceCandidate::from_card(
        card(),
        999,
        CandidateTrigger::ThreadStop,
        Some("turn-1".to_string()),
    );

    assert_eq!(first.candidate_id, second.candidate_id);
    assert_eq!(first.status, CandidateStatus::Pending);
    assert!(first.is_bounded());
    assert_eq!(
        first.with_status(CandidateStatus::Accepted).status,
        CandidateStatus::Accepted
    );
}

#[test]
fn candidate_serialization_contains_only_derived_card_fields() {
    let candidate = ExperienceCandidate::from_card(
        card(),
        100,
        CandidateTrigger::TurnError,
        Some("turn-2".to_string()),
    );
    let encoded = serde_json::to_string(&candidate).expect("serialize candidate");
    assert!(encoded.contains("pending"));
    assert!(encoded.contains("turn_error"));
    assert!(!encoded.contains("stdout"));
    assert!(!encoded.contains("secret"));
}
