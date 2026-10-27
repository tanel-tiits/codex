use std::fs;

use pretty_assertions::assert_eq;
use tempfile::tempdir;

use super::candidate::{CandidateStatus, CandidateTrigger, ExperienceCandidate};
use super::candidates::{
    append_candidate, candidates_path, find_candidate, load_candidates, update_candidate_status,
};
use super::schema::{AgentImprovementEvidence, ExperienceCard, FailureClass, TaskFamily};

fn candidate(trace_id: &str) -> ExperienceCandidate {
    ExperienceCandidate::from_card(
        ExperienceCard::new(
            trace_id,
            100,
            TaskFamily::Coding,
            FailureClass::CommandFailure,
            AgentImprovementEvidence::default(),
            vec!["command_failed".to_string()],
            "Reproduce the first failing command.",
            vec!["reproduce_first_failing_command".to_string()],
        ),
        100,
        CandidateTrigger::ThreadStop,
        None,
    )
}

fn turn_candidate(trace_id: &str, trigger: CandidateTrigger) -> ExperienceCandidate {
    ExperienceCandidate::from_card(
        ExperienceCard::new(
            trace_id,
            100,
            TaskFamily::Coding,
            FailureClass::CommandFailure,
            AgentImprovementEvidence::default(),
            vec!["command_failed".to_string()],
            "Reproduce the first failing command.",
            vec!["reproduce_first_failing_command".to_string()],
        ),
        100,
        trigger,
        Some("turn-1".to_string()),
    )
}

#[test]
fn candidate_store_is_idempotent_and_status_updates_are_append_only() -> std::io::Result<()> {
    let directory = tempdir().expect("temporary candidate directory");
    let pending = candidate("trace-1");

    assert!(append_candidate(directory.path(), &pending, 8).expect("append candidate"));
    assert!(!append_candidate(directory.path(), &pending, 8).expect("deduplicate candidate"));
    let accepted = update_candidate_status(
        directory.path(),
        &pending.candidate_id,
        CandidateStatus::Accepted,
    )
    .expect("accept candidate")
    .expect("candidate exists");
    assert_eq!(accepted.status, CandidateStatus::Accepted);
    assert_eq!(
        load_candidates(directory.path()).expect("load candidates"),
        vec![accepted]
    );
    assert_eq!(
        fs::read_to_string(candidates_path(directory.path()))?
            .lines()
            .count(),
        2
    );
    Ok(())
}

#[test]
fn pending_queue_limit_does_not_block_after_rejection() {
    let directory = tempdir().expect("temporary candidate directory");
    let first = candidate("trace-first");
    let second = candidate("trace-second");
    assert!(append_candidate(directory.path(), &first, 1).expect("append first"));
    assert!(append_candidate(directory.path(), &second, 1).is_err());
    update_candidate_status(
        directory.path(),
        &first.candidate_id,
        CandidateStatus::Rejected,
    )
    .expect("reject first");
    assert!(append_candidate(directory.path(), &second, 1).expect("append after rejection"));
}

#[test]
fn malformed_candidate_records_are_skipped() {
    let directory = tempdir().expect("temporary candidate directory");
    let valid = candidate("trace-valid");
    fs::write(
        candidates_path(directory.path()),
        format!(
            "not-json\n{}\n",
            serde_json::to_string(&valid).expect("serialize candidate")
        ),
    )
    .expect("write candidate store");

    assert_eq!(
        load_candidates(directory.path()).expect("load candidates"),
        vec![valid]
    );
    assert_eq!(
        find_candidate(directory.path(), "missing")
            .expect("find missing candidate")
            .map(|candidate| candidate.status),
        None
    );
}

#[test]
fn pending_candidates_are_not_active_cards() {
    let directory = tempdir().expect("temporary candidate directory");
    let pending = candidate("trace-pending");
    append_candidate(directory.path(), &pending, 8).expect("append candidate");

    assert!(!directory.path().join("cards.jsonl").exists());
    assert_eq!(
        load_candidates(directory.path()).unwrap()[0].status,
        CandidateStatus::Pending
    );
}

#[test]
fn lifecycle_notifications_for_one_turn_share_one_candidate() {
    let directory = tempdir().expect("temporary candidate directory");
    let turn_stop = turn_candidate("trace-turn", CandidateTrigger::TurnStop);
    let turn_error = turn_candidate("trace-turn", CandidateTrigger::TurnError);

    assert!(append_candidate(directory.path(), &turn_stop, 8).expect("append turn stop"));
    assert!(!append_candidate(directory.path(), &turn_error, 8).expect("deduplicate turn"));
    assert_eq!(
        load_candidates(directory.path()).expect("load candidates"),
        vec![turn_stop]
    );
}
