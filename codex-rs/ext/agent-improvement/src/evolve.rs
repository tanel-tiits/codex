//! Deterministic slow-loop aggregation for long-horizon agent improvement.
//!
//! This module deliberately does not call a model, replay a live task, or
//! mutate active runtime context. It reads already-derived cards, review
//! candidates, and explicit experiment outcomes, then emits bounded,
//! evidence-linked experiment briefs. A separate human-controlled workflow
//! decides whether any brief becomes a Milestone 5/6 proposal.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::AttributionState;
use crate::CandidateStatus;
use crate::EvaluationSplit;
use crate::ExperienceCandidate;
use crate::ExperienceCard;
use crate::ExperienceScope;
use crate::FailureClass;
use crate::FeedbackState;
use crate::ImprovementCandidateType;
use crate::ImprovementTarget;
use crate::ModelFamily;
use crate::TaskFamily;
use crate::ToolchainFamily;
use crate::load_candidates;
use crate::store::{
    CardStoreRewriteReport, compact_cards, export_cards, forget_cards, load_cards, retire_cards,
};

/// Version of the deterministic experiment-brief contract.
pub const EVOLUTION_SCHEMA_VERSION: u32 = 1;
/// Version of the append-only experiment-outcome contract.
pub const EVOLUTION_HISTORY_SCHEMA_VERSION: u32 = 1;
/// Append-only outcome history under an agent-improvement root.
pub const EVOLUTION_HISTORY_FILE_NAME: &str = "evolution-history.jsonl";

const MAX_HISTORY_STORE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_HISTORY_LINE_BYTES: usize = 16 * 1024;
const MAX_HISTORY_EVENTS: usize = 4096;
const MAX_BRIEF_ID_CHARS: usize = 128;
const MAX_FINGERPRINT_CHARS: usize = 128;
const MAX_ACTOR_CHARS: usize = 128;
const MAX_REASON_CHARS: usize = 512;
const MAX_BRIEFS: usize = 64;
const MAX_EVIDENCE_IDS: usize = 16;
const MAX_HYPOTHESES: usize = 6;
const MAX_CANDIDATE_CHANGES: usize = 6;
const MAX_STALE_DAYS: u32 = 3650;
const MAX_EXAMINED_RECORDS: usize = 4096;
const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// Historical pattern that caused the slow loop to create an experiment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvolutionSignalKind {
    RecurringFailureSignature,
    RepeatedVerificationOmission,
    RetrievalNotFollowed,
    ConflictingRecommendations,
    CoverageGap,
}

impl EvolutionSignalKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RecurringFailureSignature => "recurring_failure_signature",
            Self::RepeatedVerificationOmission => "repeated_verification_omission",
            Self::RetrievalNotFollowed => "retrieval_not_followed",
            Self::ConflictingRecommendations => "conflicting_recommendations",
            Self::CoverageGap => "coverage_gap",
        }
    }
}

/// Controlled explanation to test in a future held-out experiment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvolutionHypothesis {
    StaleGuidance,
    ScopeTooBroad,
    MissingVerification,
    RecommendationNotFollowed,
    ConflictingGuidance,
    ToolPrecondition,
    UncoveredTaskFamily,
}

impl EvolutionHypothesis {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::StaleGuidance => "stale_guidance",
            Self::ScopeTooBroad => "scope_too_broad",
            Self::MissingVerification => "missing_verification",
            Self::RecommendationNotFollowed => "recommendation_not_followed",
            Self::ConflictingGuidance => "conflicting_guidance",
            Self::ToolPrecondition => "tool_precondition",
            Self::UncoveredTaskFamily => "uncovered_task_family",
        }
    }
}

/// Bounded change suggestion carried by an experiment brief. It is not an
/// instruction to edit a file and it cannot be promoted by this module.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvolutionCandidateChangeKind {
    RefineCardScope,
    AddVerificationStep,
    CheckToolPreconditions,
    RetireConflictingCard,
    CreateCoverageExperiment,
}

impl EvolutionCandidateChangeKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RefineCardScope => "refine_card_scope",
            Self::AddVerificationStep => "add_verification_step",
            Self::CheckToolPreconditions => "check_tool_preconditions",
            Self::RetireConflictingCard => "retire_conflicting_card",
            Self::CreateCoverageExperiment => "create_coverage_experiment",
        }
    }
}

/// Metric that a future paired held-out evaluation must measure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvolutionMetric {
    TaskSuccess,
    VerificationSuccess,
    RecommendationFollowed,
}

impl EvolutionMetric {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TaskSuccess => "task_success",
            Self::VerificationSuccess => "verification_success",
            Self::RecommendationFollowed => "recommendation_followed",
        }
    }
}

/// Deterministic selection rule for constructing a future held-out set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvolutionEvaluationSelection {
    MatchingTaskFamilyAndScope,
    MatchingFailureSignature,
    MatchingTaskFamilyWithoutCardExposure,
}

impl EvolutionEvaluationSelection {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MatchingTaskFamilyAndScope => "matching_task_family_and_scope",
            Self::MatchingFailureSignature => "matching_failure_signature",
            Self::MatchingTaskFamilyWithoutCardExposure => {
                "matching_task_family_without_card_exposure"
            }
        }
    }
}

/// Lifecycle status of a generated experiment brief.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExperimentBriefStatus {
    Proposed,
    Suppressed,
}

/// Outcome recorded after a brief has been evaluated through the separate
/// offline evaluation/promotion workflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvolutionOutcome {
    Improved,
    NoImprovement,
    Regression,
    Inconclusive,
    Retired,
}

impl EvolutionOutcome {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Improved => "improved",
            Self::NoImprovement => "no_improvement",
            Self::Regression => "regression",
            Self::Inconclusive => "inconclusive",
            Self::Retired => "retired",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "improved" => Some(Self::Improved),
            "no_improvement" => Some(Self::NoImprovement),
            "regression" => Some(Self::Regression),
            "inconclusive" => Some(Self::Inconclusive),
            "retired" => Some(Self::Retired),
            _ => None,
        }
    }

    fn counts_as_failed_attempt(self) -> bool {
        matches!(self, Self::NoImprovement | Self::Regression)
    }
}

/// Bounded slow-loop configuration. These limits apply to one local run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvolutionConfig {
    #[serde(default = "default_max_cards_examined")]
    pub max_cards_examined: usize,
    #[serde(default = "default_max_candidates_examined")]
    pub max_candidates_examined: usize,
    #[serde(default = "default_max_briefs")]
    pub max_briefs: usize,
    #[serde(default = "default_min_recurrence")]
    pub min_recurrence: u32,
    #[serde(default = "default_min_exposure")]
    pub min_exposure: u32,
    #[serde(default = "default_stale_after_days")]
    pub stale_after_days: u32,
    #[serde(default = "default_max_active_cards")]
    pub max_active_cards: usize,
    #[serde(default = "default_max_failed_attempts")]
    pub max_failed_attempts: u32,
    #[serde(default = "default_minimum_evaluation_scenarios")]
    pub minimum_evaluation_scenarios: u32,
}

impl Default for EvolutionConfig {
    fn default() -> Self {
        Self {
            max_cards_examined: default_max_cards_examined(),
            max_candidates_examined: default_max_candidates_examined(),
            max_briefs: default_max_briefs(),
            min_recurrence: default_min_recurrence(),
            min_exposure: default_min_exposure(),
            stale_after_days: default_stale_after_days(),
            max_active_cards: default_max_active_cards(),
            max_failed_attempts: default_max_failed_attempts(),
            minimum_evaluation_scenarios: default_minimum_evaluation_scenarios(),
        }
    }
}

impl EvolutionConfig {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            (1..=MAX_EXAMINED_RECORDS).contains(&self.max_cards_examined),
            "max_cards_examined is outside the supported bound"
        );
        ensure!(
            (1..=MAX_EXAMINED_RECORDS).contains(&self.max_candidates_examined),
            "max_candidates_examined is outside the supported bound"
        );
        ensure!(
            (1..=MAX_BRIEFS).contains(&self.max_briefs),
            "max_briefs is outside the supported bound"
        );
        ensure!(
            self.min_recurrence >= 2,
            "min_recurrence must be at least 2"
        );
        ensure!(self.min_exposure >= 1, "min_exposure must be at least 1");
        ensure!(
            (1..=MAX_STALE_DAYS).contains(&self.stale_after_days),
            "stale_after_days is outside the supported bound"
        );
        ensure!(
            (1..=MAX_EXAMINED_RECORDS).contains(&self.max_active_cards),
            "max_active_cards is outside the supported bound"
        );
        ensure!(
            (1..=32).contains(&self.max_failed_attempts),
            "max_failed_attempts is outside the supported bound"
        );
        ensure!(
            (1..=64).contains(&self.minimum_evaluation_scenarios),
            "minimum_evaluation_scenarios is outside the supported bound"
        );
        Ok(())
    }
}

const fn default_max_cards_examined() -> usize {
    4096
}

const fn default_max_candidates_examined() -> usize {
    4096
}

const fn default_max_briefs() -> usize {
    16
}

const fn default_min_recurrence() -> u32 {
    2
}

const fn default_min_exposure() -> u32 {
    3
}

const fn default_stale_after_days() -> u32 {
    180
}

const fn default_max_active_cards() -> usize {
    256
}

const fn default_max_failed_attempts() -> u32 {
    3
}

const fn default_minimum_evaluation_scenarios() -> u32 {
    2
}

/// Provenance and aggregate counts retained in a brief.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvolutionEvidence {
    pub card_ids: Vec<String>,
    pub candidate_ids: Vec<String>,
    pub source_trace_ids: Vec<String>,
    pub observation_count: u32,
    pub exposure_count: u32,
    pub not_followed_count: u32,
    pub conflicting_card_count: u32,
}

/// The future held-out task slice that a brief asks a reviewer to assemble.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvolutionEvaluationSet {
    pub split: EvaluationSplit,
    pub task_family: TaskFamily,
    pub scope: ExperienceScope,
    pub minimum_scenarios: u32,
    pub selection: EvolutionEvaluationSelection,
}

/// One bounded proposed change in a brief. All changes currently use the
/// review-only experience-card lane; program-level changes remain Milestone 8.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvolutionCandidateChange {
    pub kind: EvolutionCandidateChangeKind,
    pub lane: ImprovementCandidateType,
    pub rationale: String,
}

/// Stagnation information reconstructed from explicit experiment outcomes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvolutionStagnation {
    pub attempt_count: u32,
    pub failed_attempt_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_outcome: Option<EvolutionOutcome>,
    pub suppressed: bool,
}

/// One explainable, review-only experiment proposal emitted by the slow loop.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExperimentBrief {
    pub schema_version: u32,
    pub brief_id: String,
    pub fingerprint: String,
    pub created_at_unix_ms: i64,
    pub status: ExperimentBriefStatus,
    pub signal: EvolutionSignalKind,
    pub priority: u16,
    pub target: ImprovementTarget,
    pub problem: String,
    pub evidence: EvolutionEvidence,
    pub competing_hypotheses: Vec<EvolutionHypothesis>,
    pub candidate_changes: Vec<EvolutionCandidateChange>,
    pub expected_metric: EvolutionMetric,
    pub evaluation_set: EvolutionEvaluationSet,
    pub stagnation: EvolutionStagnation,
}

/// One explicit human/evaluator outcome for a generated experiment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvolutionHistoryEvent {
    pub schema_version: u32,
    pub event_id: String,
    pub brief_id: String,
    pub brief_fingerprint: String,
    pub occurred_at_unix_ms: i64,
    pub actor: String,
    pub outcome: EvolutionOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evaluation_result_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_success_delta_bps: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification_delta_bps: Option<i32>,
    pub reason: String,
}

/// Slow-loop maintenance recommendations. The plan is inert until a user
/// invokes the explicit maintenance command with `--apply`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardMaintenancePlan {
    pub active_card_count: usize,
    pub stale_card_ids: Vec<String>,
    pub negative_feedback_card_ids: Vec<String>,
    pub retention_candidate_ids: Vec<String>,
}

/// Complete output of one deterministic aggregation pass.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvolutionReport {
    pub schema_version: u32,
    pub generated_at_unix_ms: i64,
    pub cards_examined: usize,
    pub candidates_examined: usize,
    pub history_events_examined: usize,
    pub briefs: Vec<ExperimentBrief>,
    pub maintenance: CardMaintenancePlan,
}

/// Returns the append-only history path for a slow-loop root.
pub fn evolution_history_path(root: &Path) -> PathBuf {
    root.join(EVOLUTION_HISTORY_FILE_NAME)
}

/// Loads explicit experiment outcomes. A malformed history fails closed rather
/// than silently treating an untrusted experiment as fresh.
pub fn load_evolution_history(root: &Path) -> Result<Vec<EvolutionHistoryEvent>> {
    let path = evolution_history_path(root);
    let file = match File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("open evolution history {}", path.display()));
        }
    };
    ensure!(
        file.metadata()?.len() <= MAX_HISTORY_STORE_BYTES,
        "evolution history exceeds the size limit"
    );
    let mut events = Vec::new();
    let mut event_ids = BTreeSet::new();
    for line in BufReader::new(file).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        ensure!(
            line.len() <= MAX_HISTORY_LINE_BYTES,
            "evolution history event exceeds the line size limit"
        );
        ensure!(
            events.len() < MAX_HISTORY_EVENTS,
            "evolution history contains too many events"
        );
        let event: EvolutionHistoryEvent = serde_json::from_str(&line)
            .with_context(|| format!("parse evolution history event in {}", path.display()))?;
        validate_history_event(&event)?;
        ensure!(
            event_ids.insert(event.event_id.clone()),
            "duplicate evolution history event ID"
        );
        events.push(event);
    }
    Ok(events)
}

/// Records an explicit outcome and returns the append-only event. Re-recording
/// the exact event is idempotent; a different event gets a new deterministic ID.
#[allow(clippy::too_many_arguments)]
pub fn record_evolution_outcome(
    root: &Path,
    brief_id: impl Into<String>,
    brief_fingerprint: impl Into<String>,
    actor: impl Into<String>,
    outcome: EvolutionOutcome,
    evaluation_result_fingerprint: Option<String>,
    task_success_delta_bps: Option<i32>,
    verification_delta_bps: Option<i32>,
    reason: impl Into<String>,
    occurred_at_unix_ms: i64,
) -> Result<EvolutionHistoryEvent> {
    let brief_id = bounded_id(brief_id.into(), MAX_BRIEF_ID_CHARS, "brief_id")?;
    let brief_fingerprint = bounded_id(
        brief_fingerprint.into(),
        MAX_FINGERPRINT_CHARS,
        "brief_fingerprint",
    )?;
    let actor = bounded_id(actor.into(), MAX_ACTOR_CHARS, "actor")?;
    let reason = bounded_text(reason.into(), MAX_REASON_CHARS, "reason")?;
    if let Some(value) = task_success_delta_bps {
        ensure!(
            (-10_000..=10_000).contains(&value),
            "task success delta is outside basis-point bounds"
        );
    }
    if let Some(value) = verification_delta_bps {
        ensure!(
            (-10_000..=10_000).contains(&value),
            "verification delta is outside basis-point bounds"
        );
    }
    let evaluation_result_fingerprint = evaluation_result_fingerprint
        .map(|value| {
            bounded_id(
                value,
                MAX_FINGERPRINT_CHARS,
                "evaluation_result_fingerprint",
            )
        })
        .transpose()?;
    let identity = format!(
        "{brief_id}\u{1f}{brief_fingerprint}\u{1f}{occurred_at_unix_ms}\u{1f}{actor}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{reason}",
        outcome.as_str(),
        evaluation_result_fingerprint.as_deref().unwrap_or(""),
        task_success_delta_bps.unwrap_or_default(),
        verification_delta_bps.unwrap_or_default(),
    );
    let event = EvolutionHistoryEvent {
        schema_version: EVOLUTION_HISTORY_SCHEMA_VERSION,
        event_id: digest_id("evolution_event_", &identity),
        brief_id,
        brief_fingerprint,
        occurred_at_unix_ms,
        actor,
        outcome,
        evaluation_result_fingerprint,
        task_success_delta_bps,
        verification_delta_bps,
        reason,
    };
    validate_history_event(&event)?;

    let existing = load_evolution_history(root)?;
    if let Some(previous) = existing
        .iter()
        .find(|previous| previous.event_id == event.event_id)
    {
        ensure!(previous == &event, "evolution history event ID collision");
        return Ok(event);
    }
    let encoded = serde_json::to_vec(&event).context("serialize evolution history event")?;
    ensure!(
        encoded.len() <= MAX_HISTORY_LINE_BYTES,
        "evolution history event exceeds the line size limit"
    );
    let current_size = match fs::metadata(evolution_history_path(root)) {
        Ok(metadata) => metadata.len(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
        Err(error) => return Err(error).context("read evolution history size"),
    };
    ensure!(
        current_size.saturating_add(encoded.len() as u64 + 1) <= MAX_HISTORY_STORE_BYTES,
        "evolution history would exceed the size limit"
    );
    fs::create_dir_all(root)?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(evolution_history_path(root))?;
    file.write_all(&encoded)?;
    file.write_all(b"\n")?;
    Ok(event)
}

/// Runs the slow loop over a local root without touching the live turn path.
pub fn evolve_root(
    root: &Path,
    config: &EvolutionConfig,
    now_unix_ms: i64,
) -> Result<EvolutionReport> {
    let cards = load_cards(root).context("load experience cards for evolution")?;
    let candidates = load_candidates(root).context("load experience candidates for evolution")?;
    let history = load_evolution_history(root).context("load evolution history")?;
    build_evolution_report(&cards, &candidates, &history, config, now_unix_ms)
}

/// Builds a deterministic report from already-loaded bounded artifacts.
pub fn build_evolution_report(
    cards: &[ExperienceCard],
    candidates: &[ExperienceCandidate],
    history: &[EvolutionHistoryEvent],
    config: &EvolutionConfig,
    now_unix_ms: i64,
) -> Result<EvolutionReport> {
    config.validate()?;
    let mut cards = cards.to_vec();
    cards.sort_by(|left, right| left.card_id.cmp(&right.card_id));
    cards.truncate(config.max_cards_examined);
    let mut candidates = candidates.to_vec();
    candidates.sort_by(|left, right| left.candidate_id.cmp(&right.candidate_id));
    candidates.truncate(config.max_candidates_examined);

    let mut briefs = BTreeMap::<String, ExperimentBrief>::new();
    add_recurring_failure_briefs(
        &mut briefs,
        &cards,
        &candidates,
        history,
        config,
        now_unix_ms,
    )?;
    add_verification_omission_briefs(
        &mut briefs,
        &cards,
        &candidates,
        history,
        config,
        now_unix_ms,
    )?;
    add_not_followed_briefs(
        &mut briefs,
        &cards,
        &candidates,
        history,
        config,
        now_unix_ms,
    )?;
    add_conflict_briefs(
        &mut briefs,
        &cards,
        &candidates,
        history,
        config,
        now_unix_ms,
    )?;
    add_coverage_gap_briefs(
        &mut briefs,
        &cards,
        &candidates,
        history,
        config,
        now_unix_ms,
    )?;

    let mut briefs = briefs.into_values().collect::<Vec<_>>();
    briefs.sort_by(|left, right| {
        right
            .priority
            .cmp(&left.priority)
            .then_with(|| left.status.cmp(&right.status))
            .then_with(|| left.brief_id.cmp(&right.brief_id))
    });
    briefs.truncate(config.max_briefs);

    Ok(EvolutionReport {
        schema_version: EVOLUTION_SCHEMA_VERSION,
        generated_at_unix_ms: now_unix_ms,
        cards_examined: cards.len(),
        candidates_examined: candidates.len(),
        history_events_examined: history.len(),
        briefs,
        maintenance: plan_card_maintenance(&cards, config, now_unix_ms),
    })
}

/// Computes stale, negative-feedback, and retention candidates without
/// changing the card store.
pub fn plan_card_maintenance(
    cards: &[ExperienceCard],
    config: &EvolutionConfig,
    now_unix_ms: i64,
) -> CardMaintenancePlan {
    let active = cards
        .iter()
        .filter(|card| card.status == crate::CardStatus::Active)
        .collect::<Vec<_>>();
    let stale_threshold = i64::from(config.stale_after_days).saturating_mul(DAY_MS);
    let stale_card_ids = active
        .iter()
        .filter(|card| {
            let last_observed = card
                .recurrence
                .last_seen_at_unix_ms
                .max(card.adoption.last_exposed_at_unix_ms.unwrap_or(0));
            now_unix_ms.saturating_sub(last_observed) >= stale_threshold
        })
        .map(|card| card.card_id.clone())
        .collect::<Vec<_>>();
    let negative_feedback_card_ids = active
        .iter()
        .filter(|card| card.adoption.feedback == FeedbackState::Negative)
        .map(|card| card.card_id.clone())
        .collect::<Vec<_>>();

    let mut retention_order = active;
    retention_order.sort_by(|left, right| {
        maintenance_rank(left, now_unix_ms, config.stale_after_days)
            .cmp(&maintenance_rank(
                right,
                now_unix_ms,
                config.stale_after_days,
            ))
            .then_with(|| left.card_id.cmp(&right.card_id))
    });
    let retention_count = retention_order
        .len()
        .saturating_sub(config.max_active_cards);
    let retention_candidate_ids = if retention_count > 0 {
        retention_order
            .into_iter()
            .take(retention_count)
            .map(|card| card.card_id.clone())
            .collect()
    } else {
        Vec::new()
    };

    CardMaintenancePlan {
        active_card_count: cards
            .iter()
            .filter(|card| card.status == crate::CardStatus::Active)
            .count(),
        stale_card_ids,
        negative_feedback_card_ids,
        retention_candidate_ids,
    }
}

/// Applies the non-destructive part of a maintenance plan by retiring cards.
/// Explicit forgetting remains a separate operation requiring an archive.
pub fn apply_card_maintenance(root: &Path, plan: &CardMaintenancePlan) -> Result<usize> {
    let mut ids = BTreeSet::new();
    ids.extend(plan.stale_card_ids.iter().cloned());
    ids.extend(plan.negative_feedback_card_ids.iter().cloned());
    ids.extend(plan.retention_candidate_ids.iter().cloned());
    Ok(retire_cards(root, &ids.into_iter().collect::<Vec<_>>())?)
}

/// Re-exports the explicit store rewrite operations from the slow-loop API.
pub fn export_card_store(root: &Path, output: &Path, include_retired: bool) -> Result<usize> {
    Ok(export_cards(root, output, include_retired)?)
}

/// Compacts the card store after archiving its previous bytes.
pub fn compact_card_store(root: &Path, archive: &Path) -> Result<CardStoreRewriteReport> {
    Ok(compact_cards(root, archive)?)
}

/// Explicitly forgets named cards after archiving the previous bytes.
pub fn forget_card_store(
    root: &Path,
    card_ids: &[String],
    archive: &Path,
) -> Result<CardStoreRewriteReport> {
    Ok(forget_cards(root, card_ids, archive)?)
}

fn add_recurring_failure_briefs(
    briefs: &mut BTreeMap<String, ExperimentBrief>,
    cards: &[ExperienceCard],
    candidates: &[ExperienceCandidate],
    history: &[EvolutionHistoryEvent],
    config: &EvolutionConfig,
    now_unix_ms: i64,
) -> Result<()> {
    let mut groups = BTreeMap::<String, Vec<&ExperienceCard>>::new();
    for card in cards.iter().filter(|card| {
        card.recurrence.count >= config.min_recurrence
            || card.source_trace_ids.len() >= config.min_recurrence as usize
    }) {
        let key = format!(
            "{}|{}|{}|{}|{}",
            EvolutionSignalKind::RecurringFailureSignature.as_str(),
            target_identity(card.task_family, &card.scope),
            card.failure_class.as_str(),
            card.failure_signature
                .as_identity()
                .into_iter()
                .collect::<Vec<_>>()
                .join("|"),
            card.conflict_key(),
        );
        groups.entry(key).or_default().push(card);
    }
    for (key, group) in groups {
        let observation_count = group
            .iter()
            .map(|card| card.recurrence.count)
            .fold(0u32, u32::saturating_add);
        let target = target_for_cards(&group, None);
        let evidence = evidence_for_cards(&group, matching_candidates(candidates, &target));
        let hypotheses = hypotheses_for_cards(&group, &[EvolutionHypothesis::StaleGuidance]);
        let changes = vec![
            change(
                EvolutionCandidateChangeKind::RefineCardScope,
                "Compare a narrower scope against the recurring failure signature.",
            ),
            change(
                EvolutionCandidateChangeKind::CheckToolPreconditions,
                "Test the observed precondition before retaining another recommendation.",
            ),
        ];
        let brief = make_brief(
            key,
            EvolutionSignalKind::RecurringFailureSignature,
            target,
            format!(
                "The {} failure signature recurs in the {} task family.",
                group[0].failure_class.as_str(),
                group[0].task_family.as_str()
            ),
            evidence,
            hypotheses,
            changes,
            EvolutionMetric::TaskSuccess,
            EvolutionEvaluationSelection::MatchingFailureSignature,
            800u16.saturating_add(observation_count.min(200) as u16),
            history,
            config,
            now_unix_ms,
        )?;
        insert_brief(briefs, brief);
    }
    Ok(())
}

fn add_verification_omission_briefs(
    briefs: &mut BTreeMap<String, ExperimentBrief>,
    cards: &[ExperienceCard],
    candidates: &[ExperienceCandidate],
    history: &[EvolutionHistoryEvent],
    config: &EvolutionConfig,
    now_unix_ms: i64,
) -> Result<()> {
    let mut groups = BTreeMap::<String, Vec<&ExperienceCard>>::new();
    for card in cards.iter().filter(|card| has_verification_omission(card)) {
        let key = format!(
            "{}|{}|{}",
            EvolutionSignalKind::RepeatedVerificationOmission.as_str(),
            target_identity(card.task_family, &card.scope),
            card.failure_signature.as_identity().join("|")
        );
        groups.entry(key).or_default().push(card);
    }
    for (key, group) in groups {
        let observation_count = group
            .iter()
            .map(|card| card.recurrence.count)
            .fold(0u32, u32::saturating_add);
        if observation_count < config.min_recurrence {
            continue;
        }
        let target = target_for_cards(&group, None);
        let evidence = evidence_for_cards(&group, matching_candidates(candidates, &target));
        let brief = make_brief(
            key,
            EvolutionSignalKind::RepeatedVerificationOmission,
            target,
            "Repeated observations omit a verification step before the task is treated as complete.",
            evidence,
            vec![
                EvolutionHypothesis::MissingVerification,
                EvolutionHypothesis::StaleGuidance,
            ],
            vec![change(
                EvolutionCandidateChangeKind::AddVerificationStep,
                "Add a targeted held-out verification step without changing permissions or policy.",
            )],
            EvolutionMetric::VerificationSuccess,
            EvolutionEvaluationSelection::MatchingTaskFamilyAndScope,
            850,
            history,
            config,
            now_unix_ms,
        )?;
        insert_brief(briefs, brief);
    }
    Ok(())
}

fn add_not_followed_briefs(
    briefs: &mut BTreeMap<String, ExperimentBrief>,
    cards: &[ExperienceCard],
    candidates: &[ExperienceCandidate],
    history: &[EvolutionHistoryEvent],
    config: &EvolutionConfig,
    now_unix_ms: i64,
) -> Result<()> {
    for card in cards.iter().filter(|card| {
        card.status == crate::CardStatus::Active
            && card.adoption.exposure_count >= config.min_exposure
            && card.adoption.followed == AttributionState::No
    }) {
        let key = format!(
            "{}|{}|{}",
            EvolutionSignalKind::RetrievalNotFollowed.as_str(),
            target_identity(card.task_family, &card.scope),
            card.card_id
        );
        let target = target_for_cards(&[card], Some(card.card_id.clone()));
        let evidence = evidence_for_cards(&[card], matching_candidates(candidates, &target));
        let brief = make_brief(
            key,
            EvolutionSignalKind::RetrievalNotFollowed,
            target,
            "A frequently retrieved card has explicit evidence that its recommendation was not followed.",
            evidence,
            vec![
                EvolutionHypothesis::RecommendationNotFollowed,
                EvolutionHypothesis::ScopeTooBroad,
            ],
            vec![
                change(
                    EvolutionCandidateChangeKind::RefineCardScope,
                    "Test a narrower advisory scope before retaining the same card broadly.",
                ),
                change(
                    EvolutionCandidateChangeKind::RetireConflictingCard,
                    "Retire the card if a held-out comparison does not show useful adoption.",
                ),
            ],
            EvolutionMetric::RecommendationFollowed,
            EvolutionEvaluationSelection::MatchingTaskFamilyWithoutCardExposure,
            700,
            history,
            config,
            now_unix_ms,
        )?;
        insert_brief(briefs, brief);
    }
    Ok(())
}

fn add_conflict_briefs(
    briefs: &mut BTreeMap<String, ExperimentBrief>,
    cards: &[ExperienceCard],
    candidates: &[ExperienceCandidate],
    history: &[EvolutionHistoryEvent],
    config: &EvolutionConfig,
    now_unix_ms: i64,
) -> Result<()> {
    let mut groups = BTreeMap::<String, Vec<&ExperienceCard>>::new();
    for card in cards
        .iter()
        .filter(|card| card.status == crate::CardStatus::Active)
    {
        groups.entry(card.conflict_key()).or_default().push(card);
    }
    for (conflict_key, group) in groups {
        let lesson_ids = group
            .iter()
            .map(|card| card.lesson_id.as_str())
            .collect::<BTreeSet<_>>();
        let recommendations = group
            .iter()
            .map(|card| card.recommendation.as_str())
            .collect::<BTreeSet<_>>();
        if lesson_ids.len() < 2 || recommendations.len() < 2 {
            continue;
        }
        let target = target_for_cards(&group, None);
        let evidence = evidence_for_cards(&group, matching_candidates(candidates, &target));
        let brief = make_brief(
            format!(
                "{}|{}|{}",
                EvolutionSignalKind::ConflictingRecommendations.as_str(),
                target_identity(target.task_family, &target.scope),
                conflict_key
            ),
            EvolutionSignalKind::ConflictingRecommendations,
            target,
            "Multiple active cards make different recommendations for the same scoped failure signature.",
            evidence,
            vec![
                EvolutionHypothesis::ConflictingGuidance,
                EvolutionHypothesis::ScopeTooBroad,
            ],
            vec![
                change(
                    EvolutionCandidateChangeKind::RefineCardScope,
                    "Separate the competing recommendations by evidence-backed scope.",
                ),
                change(
                    EvolutionCandidateChangeKind::RetireConflictingCard,
                    "Retire the weaker version only after a paired held-out comparison.",
                ),
            ],
            EvolutionMetric::TaskSuccess,
            EvolutionEvaluationSelection::MatchingFailureSignature,
            950,
            history,
            config,
            now_unix_ms,
        )?;
        insert_brief(briefs, brief);
    }
    Ok(())
}

fn add_coverage_gap_briefs(
    briefs: &mut BTreeMap<String, ExperimentBrief>,
    cards: &[ExperienceCard],
    candidates: &[ExperienceCandidate],
    history: &[EvolutionHistoryEvent],
    config: &EvolutionConfig,
    now_unix_ms: i64,
) -> Result<()> {
    let mut observed_targets =
        BTreeMap::<String, (TaskFamily, ExperienceScope, Vec<&ExperienceCandidate>)>::new();
    for candidate in candidates.iter().filter(|candidate| {
        matches!(
            candidate.status,
            CandidateStatus::Pending | CandidateStatus::Accepted
        )
    }) {
        let key = target_identity(candidate.card.task_family, &candidate.card.scope);
        observed_targets
            .entry(key)
            .or_insert_with(|| {
                (
                    candidate.card.task_family,
                    candidate.card.scope.clone(),
                    Vec::new(),
                )
            })
            .2
            .push(candidate);
    }
    for (key, (task_family, scope, target_candidates)) in observed_targets {
        let has_useful_coverage = cards.iter().any(|card| {
            card.status == crate::CardStatus::Active
                && card.task_family == task_family
                && card.scope == scope
                && card.confidence != crate::ConfidenceLevel::Unknown
                && card.adoption.feedback != FeedbackState::Negative
        });
        if has_useful_coverage {
            continue;
        }
        let target = ImprovementTarget {
            task_family,
            scope,
            target_name: None,
        };
        let evidence = evidence_for_cards(&[], target_candidates.to_vec());
        let candidate_ids = target_candidates
            .iter()
            .map(|candidate| candidate.candidate_id.clone())
            .collect::<BTreeSet<_>>();
        let mut evidence = evidence;
        evidence.candidate_ids = candidate_ids.into_iter().take(MAX_EVIDENCE_IDS).collect();
        let brief = make_brief(
            format!("{}|{}", EvolutionSignalKind::CoverageGap.as_str(), key),
            EvolutionSignalKind::CoverageGap,
            target,
            "Observed task-family history has no useful active card coverage in this scope.",
            evidence,
            vec![
                EvolutionHypothesis::UncoveredTaskFamily,
                EvolutionHypothesis::StaleGuidance,
            ],
            vec![change(
                EvolutionCandidateChangeKind::CreateCoverageExperiment,
                "Create a narrowly scoped card experiment and evaluate it on held-out tasks.",
            )],
            EvolutionMetric::TaskSuccess,
            EvolutionEvaluationSelection::MatchingTaskFamilyWithoutCardExposure,
            600,
            history,
            config,
            now_unix_ms,
        )?;
        insert_brief(briefs, brief);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn make_brief(
    cluster_key: String,
    signal: EvolutionSignalKind,
    target: ImprovementTarget,
    problem: impl Into<String>,
    evidence: EvolutionEvidence,
    competing_hypotheses: Vec<EvolutionHypothesis>,
    candidate_changes: Vec<EvolutionCandidateChange>,
    expected_metric: EvolutionMetric,
    selection: EvolutionEvaluationSelection,
    base_priority: u16,
    history: &[EvolutionHistoryEvent],
    config: &EvolutionConfig,
    now_unix_ms: i64,
) -> Result<ExperimentBrief> {
    let brief_id = digest_id("experiment_", &cluster_key);
    let fingerprint_identity = format!(
        "{cluster_key}\u{1f}{target_identity}\u{1f}{evidence}\u{1f}{hypotheses}\u{1f}{changes}\u{1f}{}\u{1f}{}",
        expected_metric.as_str(),
        selection.as_str(),
        target_identity = target_identity(target.task_family, &target.scope),
        evidence = serde_json::to_string(&evidence)?,
        hypotheses = serde_json::to_string(&competing_hypotheses)?,
        changes = serde_json::to_string(&candidate_changes)?,
    );
    let fingerprint = digest_id("experiment_fingerprint_", &fingerprint_identity);
    let attempt_events = history
        .iter()
        .filter(|event| event.brief_id == brief_id && event.brief_fingerprint == fingerprint)
        .collect::<Vec<_>>();
    let failed_attempt_count = attempt_events
        .iter()
        .filter(|event| event.outcome.counts_as_failed_attempt())
        .count() as u32;
    let attempt_count = attempt_events.len() as u32;
    let last_outcome = attempt_events
        .iter()
        .max_by_key(|event| (event.occurred_at_unix_ms, event.event_id.as_str()))
        .map(|event| event.outcome);
    let suppressed = failed_attempt_count >= config.max_failed_attempts;
    let priority = if suppressed {
        0
    } else {
        base_priority.saturating_sub((failed_attempt_count as u16).saturating_mul(100))
    };
    let brief = ExperimentBrief {
        schema_version: EVOLUTION_SCHEMA_VERSION,
        brief_id,
        fingerprint,
        created_at_unix_ms: now_unix_ms,
        status: if suppressed {
            ExperimentBriefStatus::Suppressed
        } else {
            ExperimentBriefStatus::Proposed
        },
        signal,
        priority,
        target: target.clone(),
        problem: bounded_text(problem.into(), 1024, "problem")?,
        evidence,
        competing_hypotheses: bounded_unique(competing_hypotheses, MAX_HYPOTHESES),
        candidate_changes: candidate_changes
            .into_iter()
            .take(MAX_CANDIDATE_CHANGES)
            .collect(),
        expected_metric,
        evaluation_set: EvolutionEvaluationSet {
            split: EvaluationSplit::HeldOut,
            task_family: target.task_family,
            scope: target.scope,
            minimum_scenarios: config.minimum_evaluation_scenarios,
            selection,
        },
        stagnation: EvolutionStagnation {
            attempt_count,
            failed_attempt_count,
            last_outcome,
            suppressed,
        },
    };
    validate_brief(&brief)?;
    Ok(brief)
}

fn insert_brief(briefs: &mut BTreeMap<String, ExperimentBrief>, brief: ExperimentBrief) {
    briefs.entry(brief.brief_id.clone()).or_insert(brief);
}

fn target_for_cards(cards: &[&ExperienceCard], target_name: Option<String>) -> ImprovementTarget {
    let Some(first) = cards.first() else {
        return ImprovementTarget {
            task_family: TaskFamily::General,
            scope: ExperienceScope::default(),
            target_name,
        };
    };
    ImprovementTarget {
        task_family: first.task_family,
        scope: first.scope.clone(),
        target_name,
    }
}

fn matching_candidates<'a>(
    candidates: &'a [ExperienceCandidate],
    target: &ImprovementTarget,
) -> Vec<&'a ExperienceCandidate> {
    candidates
        .iter()
        .filter(|candidate| {
            candidate.card.task_family == target.task_family && candidate.card.scope == target.scope
        })
        .collect()
}

fn evidence_for_cards(
    cards: &[&ExperienceCard],
    candidates: Vec<&ExperienceCandidate>,
) -> EvolutionEvidence {
    let mut card_ids = BTreeSet::new();
    let mut candidate_ids = BTreeSet::new();
    let mut source_trace_ids = BTreeSet::new();
    let mut observation_count = 0u32;
    let mut exposure_count = 0u32;
    let mut not_followed_count = 0u32;
    for card in cards {
        card_ids.insert(card.card_id.clone());
        observation_count = observation_count.saturating_add(card.recurrence.count);
        exposure_count = exposure_count.saturating_add(card.adoption.exposure_count);
        if card.adoption.followed == AttributionState::No {
            not_followed_count = not_followed_count.saturating_add(1);
        }
        source_trace_ids.extend(card.source_trace_ids.iter().cloned());
    }
    for candidate in candidates {
        candidate_ids.insert(candidate.candidate_id.clone());
        source_trace_ids.insert(candidate.source_trace_id.clone());
    }
    EvolutionEvidence {
        card_ids: card_ids.into_iter().take(MAX_EVIDENCE_IDS).collect(),
        candidate_ids: candidate_ids.into_iter().take(MAX_EVIDENCE_IDS).collect(),
        source_trace_ids: source_trace_ids
            .into_iter()
            .take(MAX_EVIDENCE_IDS)
            .collect(),
        observation_count,
        exposure_count,
        not_followed_count,
        conflicting_card_count: cards.len().min(u32::MAX as usize) as u32,
    }
}

fn hypotheses_for_cards(
    cards: &[&ExperienceCard],
    fallback: &[EvolutionHypothesis],
) -> Vec<EvolutionHypothesis> {
    let mut hypotheses = BTreeSet::new();
    for card in cards {
        for hypothesis in &card.evidence_profile.hypotheses {
            let hypothesis = match hypothesis {
                crate::HypothesisKind::LikelyMissingVerification => {
                    EvolutionHypothesis::MissingVerification
                }
                crate::HypothesisKind::LikelyToolPrecondition => {
                    EvolutionHypothesis::ToolPrecondition
                }
                crate::HypothesisKind::LikelyCommandEnvironmentIssue
                | crate::HypothesisKind::LikelyModelBoundary
                | crate::HypothesisKind::LikelyPolicyConstraint
                | crate::HypothesisKind::Unknown => EvolutionHypothesis::StaleGuidance,
            };
            hypotheses.insert(hypothesis);
        }
    }
    if hypotheses.is_empty() {
        hypotheses.extend(fallback.iter().copied());
    }
    hypotheses.into_iter().take(MAX_HYPOTHESES).collect()
}

fn change(kind: EvolutionCandidateChangeKind, rationale: &str) -> EvolutionCandidateChange {
    EvolutionCandidateChange {
        kind,
        lane: ImprovementCandidateType::ExperienceCardUpdate,
        rationale: rationale.to_string(),
    }
}

fn has_verification_omission(card: &ExperienceCard) -> bool {
    card.failure_class == FailureClass::IncompleteVerification
        || card.failure_signature.verification_state == crate::VerificationState::Missing
        || card
            .signals
            .iter()
            .any(|signal| signal == "verification_missing")
        || card
            .evidence_profile
            .facts
            .contains(&crate::EvidenceFact::VerificationMissing)
}

fn target_identity(task_family: TaskFamily, scope: &ExperienceScope) -> String {
    format!(
        "{}|{}|{}|{}",
        task_family.as_str(),
        scope.workspace_key.as_deref().unwrap_or("unknown"),
        scope
            .toolchain_family
            .map(ToolchainFamily::as_str)
            .unwrap_or("unknown"),
        scope
            .model_family
            .map(ModelFamily::as_str)
            .unwrap_or("unknown"),
    )
}

fn maintenance_rank(
    card: &ExperienceCard,
    now_unix_ms: i64,
    stale_after_days: u32,
) -> (u8, u8, u8, i64, u32, String) {
    let feedback_rank = match card.adoption.feedback {
        FeedbackState::Negative => 0,
        FeedbackState::Unknown => 1,
        FeedbackState::Mixed => 2,
        FeedbackState::Positive => 3,
    };
    let followed_rank = match card.adoption.followed {
        AttributionState::No => 0,
        AttributionState::Unknown => 1,
        AttributionState::Mixed => 2,
        AttributionState::Yes => 3,
    };
    let stale_rank = u8::from(
        now_unix_ms.saturating_sub(card.recurrence.last_seen_at_unix_ms)
            >= i64::from(stale_after_days).saturating_mul(DAY_MS),
    );
    (
        feedback_rank,
        followed_rank,
        stale_rank,
        card.recurrence.last_seen_at_unix_ms,
        card.recurrence.count,
        card.card_id.clone(),
    )
}

fn validate_brief(brief: &ExperimentBrief) -> Result<()> {
    ensure!(
        brief.schema_version == EVOLUTION_SCHEMA_VERSION,
        "unsupported evolution brief schema"
    );
    validate_safe_id(&brief.brief_id, MAX_BRIEF_ID_CHARS, "brief_id")?;
    validate_safe_id(&brief.fingerprint, MAX_FINGERPRINT_CHARS, "fingerprint")?;
    ensure!(
        !brief.problem.is_empty() && brief.problem.chars().count() <= 1024,
        "brief problem is invalid"
    );
    ensure!(
        brief.evidence.card_ids.len() <= MAX_EVIDENCE_IDS,
        "brief has too many card references"
    );
    ensure!(
        brief.evidence.candidate_ids.len() <= MAX_EVIDENCE_IDS,
        "brief has too many candidate references"
    );
    ensure!(
        brief.evidence.source_trace_ids.len() <= MAX_EVIDENCE_IDS,
        "brief has too many trace references"
    );
    ensure!(
        brief.competing_hypotheses.len() <= MAX_HYPOTHESES,
        "brief has too many hypotheses"
    );
    ensure!(
        brief.candidate_changes.len() <= MAX_CANDIDATE_CHANGES,
        "brief has too many candidate changes"
    );
    for change in &brief.candidate_changes {
        ensure!(
            !change.rationale.is_empty() && change.rationale.chars().count() <= 512,
            "candidate change rationale is invalid"
        );
    }
    ensure!(
        brief.evaluation_set.minimum_scenarios > 0,
        "evaluation set must require a scenario"
    );
    Ok(())
}

fn validate_history_event(event: &EvolutionHistoryEvent) -> Result<()> {
    ensure!(
        event.schema_version == EVOLUTION_HISTORY_SCHEMA_VERSION,
        "unsupported evolution history schema {}",
        event.schema_version
    );
    validate_safe_id(&event.event_id, MAX_FINGERPRINT_CHARS, "event_id")?;
    validate_safe_id(&event.brief_id, MAX_BRIEF_ID_CHARS, "brief_id")?;
    validate_safe_id(
        &event.brief_fingerprint,
        MAX_FINGERPRINT_CHARS,
        "brief_fingerprint",
    )?;
    validate_safe_id(&event.actor, MAX_ACTOR_CHARS, "actor")?;
    ensure!(
        event.reason.chars().count() <= MAX_REASON_CHARS,
        "history reason is too long"
    );
    ensure!(
        !event.reason.chars().any(char::is_control),
        "history reason contains control characters"
    );
    for value in [event.task_success_delta_bps, event.verification_delta_bps]
        .into_iter()
        .flatten()
    {
        ensure!(
            (-10_000..=10_000).contains(&value),
            "history metric delta is outside bounds"
        );
    }
    if let Some(fingerprint) = &event.evaluation_result_fingerprint {
        validate_safe_id(
            fingerprint,
            MAX_FINGERPRINT_CHARS,
            "evaluation_result_fingerprint",
        )?;
    }
    Ok(())
}

fn bounded_unique<T: Copy + Ord>(values: Vec<T>, max_items: usize) -> Vec<T> {
    values
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .take(max_items)
        .collect()
}

fn bounded_id(value: String, max_chars: usize, field: &str) -> Result<String> {
    let value = value.trim().to_string();
    validate_safe_id(&value, max_chars, field)?;
    Ok(value)
}

fn bounded_text(value: String, max_chars: usize, field: &str) -> Result<String> {
    let value = value.trim().to_string();
    ensure!(!value.is_empty(), "{field} must not be empty");
    ensure!(value.chars().count() <= max_chars, "{field} is too long");
    ensure!(
        !value.chars().any(char::is_control),
        "{field} contains control characters"
    );
    Ok(value)
}

fn validate_safe_id(value: &str, max_chars: usize, field: &str) -> Result<()> {
    ensure!(!value.is_empty(), "{field} must not be empty");
    ensure!(value.chars().count() <= max_chars, "{field} is too long");
    ensure!(
        !value
            .chars()
            .any(|character| character.is_control() || matches!(character, '<' | '>')),
        "{field} contains unsafe characters"
    );
    Ok(())
}

fn digest_id(prefix: &str, identity: &str) -> String {
    let digest = Sha256::digest(identity.as_bytes());
    let suffix = digest[..12]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("{prefix}{suffix}")
}
