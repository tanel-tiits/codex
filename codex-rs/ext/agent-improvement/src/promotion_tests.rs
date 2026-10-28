use std::collections::BTreeMap;
use std::fs;

use pretty_assertions::assert_eq;
use tempfile::tempdir;

use crate::AgentImprovementEvidence;
use crate::AttributionAssessment;
use crate::CardStatus;
use crate::EVALUATION_RESULT_SCHEMA_VERSION;
use crate::EXPERIENCE_CARD_SCHEMA_VERSION;
use crate::EvaluationAggregate;
use crate::EvaluationConditionProvenance;
use crate::EvaluationConditionResult;
use crate::EvaluationMemoryProvenance;
use crate::EvaluationResult;
use crate::EvaluationRetrievalResult;
use crate::EvaluationScenarioResult;
use crate::EvaluationSplit;
use crate::EvaluationStatus;
use crate::EvaluatorKind;
use crate::EvaluatorResult;
use crate::EvaluatorStatus;
use crate::ExperienceCard;
use crate::ExperienceScope;
use crate::FailureClass;
use crate::IMPROVEMENT_CANDIDATE_SCHEMA_VERSION;
use crate::ImprovementCandidate;
use crate::ImprovementCandidateType;
use crate::ImprovementContent;
use crate::ImprovementEvidenceCatalog;
use crate::ImprovementState;
use crate::MetricDelta;
use crate::PROMOTION_EVENT_STORE_FILE_NAME;
use crate::PromotionPolicy;
use crate::PromotionPolicyScope;
use crate::TaskFamily;
use crate::accept_proposal;
use crate::candidate::{CandidateTrigger, ExperienceCandidate};
use crate::evaluate_proposal;
use crate::load_cards;
use crate::load_promotion_state;
use crate::load_runtime_cards;
use crate::promote_proposal;
use crate::promote_proposal_with_policy;
use crate::promotion_events_path;
use crate::promotion_policy_path;
use crate::promotion_status;
use crate::reject_proposal;
use crate::retire_proposal;
use crate::rollback_proposal;
use crate::validate_proposal;

const CREATED_AT: i64 = 1_000;
const NOW: i64 = 2_000;

fn card(trace_id: &str, task_family: TaskFamily) -> ExperienceCard {
    ExperienceCard::new(
        trace_id,
        CREATED_AT,
        task_family,
        FailureClass::CommandFailure,
        AgentImprovementEvidence::default(),
        vec!["command_failed".to_string()],
        "Reproduce the first failing command before changing more files.",
        vec!["run the focused verification command".to_string()],
    )
}

fn candidate(trace_id: &str, task_family: TaskFamily) -> ExperienceCandidate {
    ExperienceCandidate::from_card(
        card(trace_id, task_family),
        CREATED_AT,
        CandidateTrigger::ThreadStop,
        Some("turn-promotion".to_string()),
    )
}

fn proposal(trace_id: &str, scenario_id: &str) -> (ExperienceCandidate, ImprovementCandidate) {
    let candidate = candidate(trace_id, TaskFamily::Coding);
    let proposal = ImprovementCandidate::from_experience_candidate(&candidate, scenario_id)
        .expect("valid card proposal");
    (candidate, proposal)
}

fn condition(enabled: bool, task_success: bool, trace_id: &str) -> EvaluationConditionResult {
    EvaluationConditionResult {
        provenance: EvaluationConditionProvenance {
            execution_config_fingerprint: "execution-v1".to_string(),
            policy_fingerprint: "policy-v1".to_string(),
            worktree_fingerprint: "worktree-v1".to_string(),
            memory: EvaluationMemoryProvenance {
                enabled,
                card_set_fingerprint: if enabled {
                    "cards-treatment".to_string()
                } else {
                    "cards-none".to_string()
                },
                retrieval_policy_fingerprint: "retrieval-v1".to_string(),
                card_schema_version: EXPERIENCE_CARD_SCHEMA_VERSION,
            },
        },
        retrieval: EvaluationRetrievalResult::default(),
        trace_id: Some(trace_id.to_string()),
        status: EvaluationStatus::Evaluated,
        error_code: None,
        metrics: None,
        evaluator: EvaluatorResult {
            status: if task_success {
                EvaluatorStatus::Passed
            } else {
                EvaluatorStatus::Failed
            },
            task_success: Some(task_success),
            verification_success: Some(task_success),
            checks: Vec::new(),
            judge_score_bps: None,
        },
    }
}

fn result(
    scenario_id: &str,
    task_family: TaskFamily,
    baseline: bool,
    treatment: bool,
) -> EvaluationResult {
    EvaluationResult {
        schema_version: EVALUATION_RESULT_SCHEMA_VERSION,
        manifest_id: "manifest-promotion".to_string(),
        evaluated_at_unix_ms: 1_500,
        scenario_count: 1,
        all: EvaluationAggregate::default(),
        held_out: EvaluationAggregate::default(),
        by_task_family: BTreeMap::new(),
        scenarios: vec![EvaluationScenarioResult {
            scenario_id: scenario_id.to_string(),
            task_family,
            split: EvaluationSplit::HeldOut,
            task_fingerprint: "task-v1".to_string(),
            repository_snapshot_fingerprint: "repo-v1".to_string(),
            evaluator: EvaluatorKind::HumanJudge,
            baseline: condition(baseline, baseline, "trace-baseline"),
            treatment: condition(true, treatment, "trace-treatment"),
            delta: MetricDelta {
                task_success: Some(i8::from(treatment) - i8::from(baseline)),
                verification_success: Some(i8::from(treatment) - i8::from(baseline)),
                input_tokens: None,
                output_tokens: None,
                reasoning_output_tokens: None,
                wall_time_ms: None,
                tool_calls: None,
                terminal_operations: None,
                retry_count: None,
                repeated_failure_count: None,
                policy_event_count: None,
            },
            attribution: AttributionAssessment::ExposedAndOutcomeImproved,
        }],
    }
}

fn catalog(
    candidate: &ExperienceCandidate,
    evaluation: &EvaluationResult,
) -> ImprovementEvidenceCatalog {
    let mut catalog = ImprovementEvidenceCatalog::from_candidates(std::slice::from_ref(candidate));
    catalog
        .add_evaluation_result(evaluation)
        .expect("evaluation evidence is valid");
    catalog
}

fn advance_to_accepted(
    root: &std::path::Path,
    candidate: &ExperienceCandidate,
    proposal: &ImprovementCandidate,
    evaluation: &EvaluationResult,
) {
    let catalog = catalog(candidate, evaluation);
    validate_proposal(root, proposal, "reviewer", NOW, Some(&catalog)).expect("validate");
    evaluate_proposal(
        root,
        proposal,
        std::slice::from_ref(evaluation),
        &catalog,
        "evaluator",
        NOW + 1,
    )
    .expect("evaluate");
    accept_proposal(root, proposal, "human", NOW + 2).expect("accept");
}

#[test]
fn full_promotion_lifecycle_is_scoped_reversible_and_recoverable() {
    let root = tempdir().expect("temporary promotion root");
    let (candidate, proposal) = proposal("trace-promote", "scenario-promote");
    let evaluation = result("scenario-promote", TaskFamily::Coding, false, true);

    advance_to_accepted(root.path(), &candidate, &proposal, &evaluation);
    let active =
        promote_proposal(root.path(), &proposal, "human", NOW + 3).expect("promote card proposal");
    assert_eq!(active.state, ImprovementState::Active);
    assert_eq!(active.last_actor.as_deref(), Some("human"));
    assert!(active.evaluation.is_some());
    assert_eq!(load_cards(root.path()).expect("load cards").len(), 1);
    assert_eq!(
        load_runtime_cards(root.path(), NOW + 3)
            .expect("load runtime cards")
            .len(),
        1
    );
    assert!(
        load_runtime_cards(root.path(), active.expires_at_unix_ms)
            .expect("load expired runtime cards")
            .is_empty()
    );

    // There is no authoritative index to go stale: replaying the log alone
    // reconstructs the same active state after an interrupted index update.
    let rebuilt = load_promotion_state(root.path()).expect("rebuild promotion state");
    assert_eq!(rebuilt.get(&proposal.candidate_id), Some(&active));
    assert!(!root.path().join("promotion-index.json").exists());

    let retired =
        rollback_proposal(root.path(), &proposal, "human", NOW + 4).expect("rollback active card");
    assert_eq!(retired.state, ImprovementState::Retired);
    assert!(
        load_runtime_cards(root.path(), NOW + 4)
            .expect("load runtime cards after rollback")
            .is_empty()
    );
    assert_eq!(
        load_cards(root.path()).expect("load cards after rollback")[0].status,
        CardStatus::Retired
    );
    assert_eq!(
        fs::read_to_string(promotion_events_path(root.path()))
            .expect("read event log")
            .lines()
            .count(),
        5
    );
}

#[test]
fn promotion_requires_real_held_out_evidence_and_rejects_regression() {
    let root = tempdir().expect("temporary promotion root");
    let (candidate, proposal) = proposal("trace-gates", "scenario-gates");
    let evaluation = result("scenario-gates", TaskFamily::Coding, false, true);

    validate_proposal(root.path(), &proposal, "reviewer", NOW, None).expect("structural validate");
    let empty_catalog = ImprovementEvidenceCatalog::default();
    assert!(
        evaluate_proposal(
            root.path(),
            &proposal,
            std::slice::from_ref(&evaluation),
            &empty_catalog,
            "evaluator",
            NOW + 1,
        )
        .is_err()
    );

    let regression = result("scenario-gates", TaskFamily::Coding, true, false);
    let regression_catalog = catalog(&candidate, &regression);
    assert!(
        evaluate_proposal(
            root.path(),
            &proposal,
            std::slice::from_ref(&regression),
            &regression_catalog,
            "evaluator",
            NOW + 1,
        )
        .is_err()
    );
    assert_eq!(
        promotion_status(root.path(), &proposal)
            .expect("status after rejected evaluation")
            .state,
        ImprovementState::Validated
    );
}

#[test]
fn promotion_gates_expiry_schema_and_scope_mismatch() {
    let root = tempdir().expect("temporary promotion root");
    let (candidate, mut proposal) = proposal("trace-policy", "scenario-policy");
    let evaluation = result("scenario-policy", TaskFamily::Coding, false, true);
    let catalog = catalog(&candidate, &evaluation);

    let mut expired = proposal.clone();
    expired.expiry.expires_at_unix_ms = NOW;
    assert!(validate_proposal(root.path(), &expired, "reviewer", NOW, Some(&catalog)).is_err());

    let mut stale = proposal.clone();
    stale.schema_version = IMPROVEMENT_CANDIDATE_SCHEMA_VERSION + 1;
    assert!(validate_proposal(root.path(), &stale, "reviewer", NOW, Some(&catalog)).is_err());

    advance_to_accepted(root.path(), &candidate, &proposal, &evaluation);
    let mismatched_policy = PromotionPolicy {
        scopes: vec![PromotionPolicyScope {
            scope: ExperienceScope::for_workspace_path("/different/repository"),
            task_family: Some(TaskFamily::Coding),
            enabled: true,
            kill_switch: false,
            ttl_days: None,
            max_active: None,
        }],
        ..PromotionPolicy::default()
    };
    assert!(
        promote_proposal_with_policy(root.path(), &proposal, "human", NOW + 3, &mismatched_policy,)
            .is_err()
    );
    let kill_switch_policy = PromotionPolicy {
        kill_switch: true,
        ..PromotionPolicy::default()
    };
    assert!(
        promote_proposal_with_policy(
            root.path(),
            &proposal,
            "human",
            NOW + 3,
            &kill_switch_policy,
        )
        .is_err()
    );
    assert!(!root.path().join("cards.jsonl").exists());

    proposal.target.scope = ExperienceScope::for_workspace_path("/different/repository");
    assert!(proposal.validate().is_err());
}

#[test]
fn non_card_lanes_cannot_be_activated_by_the_card_control_plane() {
    let root = tempdir().expect("temporary promotion root");
    let (candidate, mut proposal) = proposal("trace-lane", "scenario-lane");
    proposal.candidate_type = ImprovementCandidateType::FocusedSkillChange;
    proposal.target.target_name = Some("focused-review".to_string());
    proposal.content = ImprovementContent::FocusedSkillChange {
        skill_name: "focused-review".to_string(),
        diff: "Require a focused regression check.".to_string(),
    };
    proposal.validate().expect("valid focused skill proposal");
    let evaluation = result("scenario-lane", TaskFamily::Coding, false, true);
    advance_to_accepted(root.path(), &candidate, &proposal, &evaluation);
    let error = promote_proposal(root.path(), &proposal, "human", NOW + 3)
        .expect_err("non-card lane must stay on normal diff review");
    assert!(error.to_string().contains("cannot be promoted"));
    assert!(root.path().join(PROMOTION_EVENT_STORE_FILE_NAME).exists());
    assert_eq!(
        promotion_status(root.path(), &proposal)
            .expect("status after blocked promotion")
            .state,
        ImprovementState::Accepted
    );
    assert!(!root.path().join("cards.jsonl").exists());
}

#[test]
fn rollback_restores_the_previous_active_card_without_retiring_unrelated_cards() {
    let root = tempdir().expect("temporary promotion root");
    let (first_candidate, first_proposal) = proposal("trace-first", "scenario-first");
    let first_evaluation = result("scenario-first", TaskFamily::Coding, false, true);
    advance_to_accepted(
        root.path(),
        &first_candidate,
        &first_proposal,
        &first_evaluation,
    );
    promote_proposal(root.path(), &first_proposal, "human", NOW + 3).expect("promote first");

    let (second_candidate, second_proposal) = proposal("trace-second", "scenario-second");
    let second_evaluation = result("scenario-second", TaskFamily::Coding, false, true);
    advance_to_accepted(
        root.path(),
        &second_candidate,
        &second_proposal,
        &second_evaluation,
    );
    let second_active =
        promote_proposal(root.path(), &second_proposal, "human", NOW + 4).expect("promote second");
    assert_eq!(
        second_active.previous_version.as_deref(),
        Some(
            first_proposal
                .target
                .target_name
                .as_deref()
                .expect("first card ID")
        )
    );

    rollback_proposal(root.path(), &second_proposal, "human", NOW + 5).expect("rollback second");
    let runtime_cards = load_runtime_cards(root.path(), NOW + 5).expect("load restored cards");
    assert_eq!(runtime_cards.len(), 1);
    assert_eq!(
        runtime_cards[0].card_id,
        first_proposal.target.target_name.expect("first card ID")
    );
    assert_eq!(
        load_promotion_state(root.path())
            .expect("rebuild state")
            .get(&first_proposal.candidate_id)
            .expect("first status")
            .state,
        ImprovementState::Active
    );
}

#[test]
fn malformed_promotion_store_fails_closed_for_runtime_retrieval() {
    let root = tempdir().expect("temporary promotion root");
    fs::write(promotion_policy_path(root.path()), "not-json\n").expect("write malformed policy");
    assert!(load_runtime_cards(root.path(), NOW).is_err());
    fs::remove_file(promotion_policy_path(root.path())).expect("remove malformed policy");
    fs::write(promotion_events_path(root.path()), "not-json\n").expect("write malformed log");
    assert!(load_promotion_state(root.path()).is_err());
    assert!(load_runtime_cards(root.path(), NOW).is_err());
}

#[test]
fn rejection_and_explicit_retirement_are_append_only_terminal_decisions() {
    let rejected_root = tempdir().expect("temporary rejected root");
    let (_candidate, rejected_proposal) = proposal("trace-rejected", "scenario-rejected");
    let rejected = reject_proposal(
        rejected_root.path(),
        &rejected_proposal,
        "human",
        NOW,
        "held-out task family is no longer in scope",
    )
    .expect("reject proposal");
    assert_eq!(rejected.state, ImprovementState::Rejected);
    assert!(
        reject_proposal(
            rejected_root.path(),
            &rejected_proposal,
            "human",
            NOW + 1,
            "second decision",
        )
        .is_err()
    );
    assert!(!rejected_root.path().join("cards.jsonl").exists());

    let retired_root = tempdir().expect("temporary retired root");
    let (candidate, proposal) = proposal("trace-retired", "scenario-retired");
    let evaluation = result("scenario-retired", TaskFamily::Coding, false, true);
    advance_to_accepted(retired_root.path(), &candidate, &proposal, &evaluation);
    promote_proposal(retired_root.path(), &proposal, "human", NOW + 3).expect("promote");
    let retired =
        retire_proposal(retired_root.path(), &proposal, "human", NOW + 4).expect("retire proposal");
    assert_eq!(retired.state, ImprovementState::Retired);
    assert!(
        load_runtime_cards(retired_root.path(), NOW + 4)
            .expect("load retired runtime cards")
            .is_empty()
    );
}
