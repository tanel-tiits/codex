use std::fs;

use anyhow::Result;
use pretty_assertions::assert_eq;
use tempfile::TempDir;

use crate::AgentImprovementEvidence;
use crate::AttributionState;
use crate::CandidateTrigger;
use crate::CardStatus;
use crate::EvolutionConfig;
use crate::EvolutionOutcome;
use crate::EvolutionSignalKind;
use crate::ExperienceCandidate;
use crate::ExperienceCard;
use crate::ExperienceScope;
use crate::FailureClass;
use crate::FeedbackState;
use crate::TaskFamily;
use crate::append_card;
use crate::apply_card_maintenance;
use crate::build_evolution_report;
use crate::compact_card_store;
use crate::evolve_root;
use crate::export_card_store;
use crate::forget_card_store;
use crate::load_evolution_history;
use crate::plan_card_maintenance;
use crate::record_evolution_outcome;

fn card(
    trace_id: &str,
    task_family: TaskFamily,
    failure_class: FailureClass,
    recommendation: &str,
    created_at_unix_ms: i64,
) -> ExperienceCard {
    ExperienceCard::new(
        trace_id,
        created_at_unix_ms,
        task_family,
        failure_class,
        AgentImprovementEvidence::default(),
        Vec::new(),
        recommendation,
        Vec::new(),
    )
}

#[test]
fn slow_loop_emits_deterministic_briefs_for_recurring_patterns() -> Result<()> {
    let now = 1_700_000_000_000;
    let mut recurring = card(
        "trace-recurring",
        TaskFamily::Coding,
        FailureClass::CommandFailure,
        "reproduce the failing command",
        now - 10_000,
    );
    recurring.recurrence.count = 3;
    recurring.recurrence.first_seen_at_unix_ms = now - 30_000;
    recurring.recurrence.last_seen_at_unix_ms = now - 10_000;

    let mut verification = card(
        "trace-verification",
        TaskFamily::Testing,
        FailureClass::IncompleteVerification,
        "run the targeted tests",
        now - 20_000,
    );
    verification
        .signals
        .push("verification_missing".to_string());
    verification.recurrence.count = 2;

    let mut not_followed = card(
        "trace-not-followed",
        TaskFamily::Review,
        FailureClass::Completed,
        "preserve the observed review workflow",
        now - 5_000,
    );
    not_followed.adoption.exposure_count = 3;
    not_followed.adoption.followed = AttributionState::No;

    let mut conflict_left = card(
        "trace-conflict-left",
        TaskFamily::Planning,
        FailureClass::ToolFailure,
        "check tool preconditions first",
        now - 4_000,
    );
    conflict_left.recurrence.count = 1;
    let conflict_right = card(
        "trace-conflict-right",
        TaskFamily::Planning,
        FailureClass::ToolFailure,
        "request a different tool before continuing",
        now - 3_000,
    );

    let mut gap_card = card(
        "trace-gap",
        TaskFamily::Documentation,
        FailureClass::Unknown,
        "inspect the reduced trace",
        now - 2_000,
    );
    gap_card.status = CardStatus::Retired;
    let gap_candidate =
        ExperienceCandidate::from_card(gap_card, now - 1_000, CandidateTrigger::TurnError, None);

    let cards = vec![
        recurring,
        verification,
        not_followed,
        conflict_left,
        conflict_right,
    ];
    let candidates = vec![gap_candidate];
    let config = EvolutionConfig::default();
    let first = build_evolution_report(&cards, &candidates, &[], &config, now)?;
    let second = build_evolution_report(&cards, &candidates, &[], &config, now)?;
    assert_eq!(first, second);

    let signals = first
        .briefs
        .iter()
        .map(|brief| brief.signal)
        .collect::<std::collections::BTreeSet<_>>();
    assert!(signals.contains(&EvolutionSignalKind::RecurringFailureSignature));
    assert!(signals.contains(&EvolutionSignalKind::RepeatedVerificationOmission));
    assert!(signals.contains(&EvolutionSignalKind::RetrievalNotFollowed));
    assert!(signals.contains(&EvolutionSignalKind::ConflictingRecommendations));
    assert!(signals.contains(&EvolutionSignalKind::CoverageGap));
    assert!(
        first
            .briefs
            .iter()
            .all(|brief| !brief.problem.contains("trace-"))
    );
    Ok(())
}

#[test]
fn repeated_failed_outcomes_suppress_the_same_experiment() -> Result<()> {
    let now = 1_700_000_000_000;
    let mut recurring = card(
        "trace-recurring",
        TaskFamily::Coding,
        FailureClass::CommandFailure,
        "reproduce the failing command",
        now,
    );
    recurring.recurrence.count = 3;
    let config = EvolutionConfig::default();
    let root = TempDir::new()?;

    let initial = build_evolution_report(&[recurring.clone()], &[], &[], &config, now)?;
    let brief = initial.briefs.first().expect("recurring brief").clone();
    for attempt in 0..config.max_failed_attempts {
        record_evolution_outcome(
            root.path(),
            brief.brief_id.clone(),
            brief.fingerprint.clone(),
            "reviewer",
            EvolutionOutcome::NoImprovement,
            Some(format!("result-{attempt}")),
            Some(0),
            Some(0),
            "held-out result did not improve",
            now + i64::from(attempt) + 1,
        )?;
    }
    let history = load_evolution_history(root.path())?;
    let report = build_evolution_report(&[recurring], &[], &history, &config, now)?;
    let suppressed = report
        .briefs
        .iter()
        .find(|candidate| candidate.brief_id == brief.brief_id)
        .expect("same brief remains inspectable");
    assert_eq!(suppressed.status, crate::ExperimentBriefStatus::Suppressed);
    assert_eq!(suppressed.priority, 0);
    assert_eq!(suppressed.stagnation.failed_attempt_count, 3);
    Ok(())
}

#[test]
fn clustering_keeps_repository_scopes_isolated() -> Result<()> {
    let now = 1_700_000_000_000;
    let mut first = card(
        "trace-repository-a",
        TaskFamily::Coding,
        FailureClass::CommandFailure,
        "reproduce the failing command",
        now,
    );
    first.recurrence.count = 3;
    first.scope = ExperienceScope::for_workspace_path("/private/repository-a");
    let mut second = card(
        "trace-repository-b",
        TaskFamily::Coding,
        FailureClass::CommandFailure,
        "reproduce the failing command",
        now,
    );
    second.recurrence.count = 3;
    second.scope = ExperienceScope::for_workspace_path("/private/repository-b");

    let report =
        build_evolution_report(&[first, second], &[], &[], &EvolutionConfig::default(), now)?;
    let scoped = report
        .briefs
        .iter()
        .filter(|brief| brief.signal == EvolutionSignalKind::RecurringFailureSignature)
        .collect::<Vec<_>>();
    assert_eq!(scoped.len(), 2);
    assert_ne!(
        scoped[0].target.scope.workspace_key,
        scoped[1].target.scope.workspace_key
    );
    Ok(())
}

#[test]
fn maintenance_plans_decay_negative_stale_and_excess_cards() -> Result<()> {
    let now = 1_700_000_000_000;
    let mut stale = card(
        "trace-stale",
        TaskFamily::Coding,
        FailureClass::CommandFailure,
        "reproduce the failing command",
        now - 300 * 24 * 60 * 60 * 1000,
    );
    stale.recurrence.last_seen_at_unix_ms = now - 300 * 24 * 60 * 60 * 1000;
    let mut negative = card(
        "trace-negative",
        TaskFamily::Testing,
        FailureClass::ToolFailure,
        "check tool preconditions",
        now - 1_000,
    );
    negative.adoption.feedback = FeedbackState::Negative;
    let recent = card(
        "trace-recent",
        TaskFamily::Review,
        FailureClass::Completed,
        "preserve the review workflow",
        now - 500,
    );
    let mut config = EvolutionConfig::default();
    config.max_active_cards = 1;
    let plan = plan_card_maintenance(&[stale, negative, recent], &config, now);
    assert_eq!(plan.active_card_count, 3);
    assert_eq!(plan.stale_card_ids.len(), 1);
    assert_eq!(plan.negative_feedback_card_ids.len(), 1);
    assert_eq!(plan.retention_candidate_ids.len(), 2);
    Ok(())
}

#[test]
fn explicit_store_maintenance_archives_compacts_exports_and_forgets() -> Result<()> {
    let root = TempDir::new()?;
    let archive = root.path().join("archive.jsonl");
    let export = root.path().join("export.jsonl");
    let forget_archive = root.path().join("forget-archive.jsonl");
    let first = card(
        "trace-one",
        TaskFamily::Coding,
        FailureClass::CommandFailure,
        "reproduce the failing command",
        1,
    );
    let mut updated = first.clone();
    updated.adoption.exposure_count = 1;
    let second = card(
        "trace-two",
        TaskFamily::Testing,
        FailureClass::ToolFailure,
        "check tool preconditions",
        2,
    );
    append_card(root.path(), &first)?;
    append_card(root.path(), &updated)?;
    append_card(root.path(), &second)?;

    let compacted = compact_card_store(root.path(), &archive)?;
    assert_eq!(compacted.records_before, 3);
    assert_eq!(compacted.records_after, 2);
    assert!(compacted.archived);
    assert_eq!(fs::read_to_string(&archive)?.lines().count(), 3);
    assert_eq!(export_card_store(root.path(), &export, true)?, 2);

    let forgotten = forget_card_store(root.path(), &[first.card_id.clone()], &forget_archive)?;
    assert_eq!(forgotten.removed_card_ids, vec![first.card_id]);
    assert!(forgotten.archived);
    assert_eq!(crate::load_cards(root.path())?.len(), 1);
    assert!(fs::read_to_string(&forget_archive)?.contains("trace-one"));
    Ok(())
}

#[test]
fn apply_maintenance_retires_cards_without_deleting_history() -> Result<()> {
    let root = TempDir::new()?;
    let now = 1_700_000_000_000;
    let mut card = card(
        "trace-negative",
        TaskFamily::Testing,
        FailureClass::ToolFailure,
        "check tool preconditions",
        now,
    );
    card.adoption.feedback = FeedbackState::Negative;
    append_card(root.path(), &card)?;
    let config = EvolutionConfig::default();
    let plan = plan_card_maintenance(&[card], &config, now);
    assert_eq!(apply_card_maintenance(root.path(), &plan)?, 1);
    assert_eq!(
        crate::load_cards(root.path())?[0].status,
        CardStatus::Retired
    );
    Ok(())
}

#[test]
fn evolve_root_reads_local_stores_and_respects_history_limits() -> Result<()> {
    let root = TempDir::new()?;
    let now = 1_700_000_000_000;
    let mut card = card(
        "trace-root",
        TaskFamily::Coding,
        FailureClass::CommandFailure,
        "reproduce the failing command",
        now,
    );
    card.recurrence.count = 3;
    append_card(root.path(), &card)?;
    let report = evolve_root(root.path(), &EvolutionConfig::default(), now)?;
    assert_eq!(report.cards_examined, 1);
    assert!(!report.briefs.is_empty());
    Ok(())
}
