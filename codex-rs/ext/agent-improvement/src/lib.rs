//! Privacy-bounded local experience cards for agent improvement.
//!
//! The first layer of this crate is deliberately independent of the Codex
//! runtime. It stores derived, reviewable evidence rather than replaying raw
//! prompts, model responses, paths, or terminal output into future turns.

mod candidate;
mod candidates;
mod capture;
mod eval;
mod extension;
mod improvement;
mod promotion;
mod schema;
mod store;
mod summarize;

pub use candidate::{
    CandidateStatus, CandidateTrigger, EXPERIENCE_CANDIDATE_SCHEMA_VERSION, ExperienceCandidate,
};
pub use candidates::{
    CANDIDATE_STORE_FILE_NAME, append_candidate, candidates_path, find_candidate, load_candidates,
    update_candidate_status,
};
pub use eval::{
    AttributionAssessment, AttributionEvidence, CheckResult, EVALUATION_MANIFEST_SCHEMA_VERSION,
    EVALUATION_RESULT_SCHEMA_VERSION, EvaluationAggregate, EvaluationBudget, EvaluationCondition,
    EvaluationConditionProvenance, EvaluationConditionResult, EvaluationErrorCode,
    EvaluationManifest, EvaluationMemory, EvaluationMemoryProvenance, EvaluationMetrics,
    EvaluationRepository, EvaluationResult, EvaluationScenario, EvaluationScenarioResult,
    EvaluationSplit, EvaluationStatus, EvaluationTask, EvaluatorKind, EvaluatorResult,
    EvaluatorSpec, EvaluatorStatus, JudgeLabel, MetricDelta, VerificationCheck, evaluate_manifest,
    evaluate_manifest_at, load_evaluation_manifest, write_evaluation_result,
};
pub use extension::{AgentImprovementExtensionConfig, install};
pub use improvement::{
    IMPROVEMENT_CANDIDATE_SCHEMA_VERSION, ImprovementCandidate, ImprovementCandidateStatus,
    ImprovementCandidateType, ImprovementContent, ImprovementEvidenceCatalog,
    ImprovementEvidenceKind, ImprovementEvidenceReference, ImprovementExpiry, ImprovementRollback,
    ImprovementTarget, ImprovementValidationKind, ImprovementValidationRequirement,
    load_improvement_candidate, write_improvement_candidate,
};
pub use promotion::{
    ImprovementState, PROMOTION_EVENT_SCHEMA_VERSION, PROMOTION_EVENT_STORE_FILE_NAME,
    PROMOTION_POLICY_FILE_NAME, PROMOTION_POLICY_SCHEMA_VERSION, PromotionEvaluation,
    PromotionEvent, PromotionPolicy, PromotionPolicyDecision, PromotionPolicyScope, PromotionState,
    PromotionStatus, PromotionTransition, accept_proposal, current_unix_ms, evaluate_proposal,
    filter_runtime_cards, load_promotion_policy, load_promotion_policy_for_root,
    load_promotion_state, load_runtime_cards, promote_proposal, promote_proposal_with_policy,
    promotion_events_path, promotion_policy_path, promotion_status, reject_proposal,
    retire_proposal, rollback_proposal, validate_proposal, write_promotion_policy,
};
pub use schema::{
    AdoptionEvidence, AgentImprovementEvidence, AttributionState, CardStatus, ConfidenceLevel,
    EXPERIENCE_CARD_SCHEMA_VERSION, EvidenceFact, EvidenceProfile, ExitBucket, ExperienceCard,
    ExperienceScope, FailureClass, FailureSignature, FeedbackState, HypothesisKind,
    InterventionKind, ModelFamily, ObservedStep, OperationClass, OutcomeKind, PolicyReason,
    Recurrence, TaskFamily, TokenUsageSummary, ToolFamily, ToolchainFamily, VerificationState,
    workspace_scope_key,
};
pub use store::{
    ExperienceRetrievalContext, STORE_FILE_NAME, append_card, cards_path, load_cards,
    record_card_exposure, record_card_feedback, record_card_outcome, render_cards, retire_card,
    select_relevant, select_relevant_with_context,
};
pub use summarize::summarize_rollout;

#[cfg(test)]
mod candidate_tests;

#[cfg(test)]
mod candidates_tests;

#[cfg(test)]
mod schema_tests;

#[cfg(test)]
mod store_tests;

#[cfg(test)]
mod summarize_tests;

#[cfg(test)]
mod extension_tests;
