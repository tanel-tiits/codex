//! Manual promotion, rollback, and policy control for improvement proposals.
//!
//! The proposal artifact is deliberately kept separate from this module. A
//! proposal is inert until a human walks it through the append-only
//! transition log. Runtime retrieval treats a promoted card as active only
//! when both the card store and the promotion log agree, so a partial write
//! fails closed instead of silently changing agent behavior.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::CardStatus;
use crate::EVALUATION_RESULT_SCHEMA_VERSION;
use crate::EvaluationResult;
use crate::EvaluationScenarioResult;
use crate::EvaluationSplit;
use crate::ExperienceCard;
use crate::ExperienceScope;
use crate::ImprovementCandidate;
use crate::ImprovementContent;
use crate::ImprovementEvidenceCatalog;
use crate::ImprovementEvidenceKind;
use crate::ImprovementEvidenceReference;
use crate::ImprovementTarget;
use crate::TaskFamily;
use crate::store::{append_card, load_cards, retire_card};

/// Version of the append-only promotion event contract.
pub const PROMOTION_EVENT_SCHEMA_VERSION: u32 = 1;
/// Version of the policy contract.
pub const PROMOTION_POLICY_SCHEMA_VERSION: u32 = 1;
/// Append-only lifecycle event store under an agent-improvement root.
pub const PROMOTION_EVENT_STORE_FILE_NAME: &str = "promotion-events.jsonl";
/// Optional policy file under an agent-improvement root.
pub const PROMOTION_POLICY_FILE_NAME: &str = "promotion-policy.json";

const MAX_EVENT_STORE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_EVENT_LINE_BYTES: usize = 64 * 1024;
const MAX_EVENTS: usize = 8192;
const MAX_ID_CHARS: usize = 128;
const MAX_ACTOR_CHARS: usize = 128;
const MAX_REASON_CHARS: usize = 2048;
const MAX_EVIDENCE: usize = 64;
const MAX_POLICY_SCOPES: usize = 128;
const MAX_POLICY_TTL_DAYS: u32 = 180;
const MAX_ACTIVE_IMPROVEMENTS: usize = 1024;
const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// Lifecycle state for one proposal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImprovementState {
    Proposed,
    Validated,
    Evaluated,
    Accepted,
    Active,
    Retired,
    Rejected,
}

impl ImprovementState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Proposed => "proposed",
            Self::Validated => "validated",
            Self::Evaluated => "evaluated",
            Self::Accepted => "accepted",
            Self::Active => "active",
            Self::Retired => "retired",
            Self::Rejected => "rejected",
        }
    }
}

/// Human-triggered operation that produced a lifecycle event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromotionTransition {
    Validate,
    Evaluate,
    Accept,
    Promote,
    Rollback,
    Retire,
    Reject,
}

impl PromotionTransition {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Validate => "validate",
            Self::Evaluate => "evaluate",
            Self::Accept => "accept",
            Self::Promote => "promote",
            Self::Rollback => "rollback",
            Self::Retire => "retire",
            Self::Reject => "reject",
        }
    }
}

/// Redacted evaluation evidence retained with an accepted or active proposal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromotionEvaluation {
    pub result_manifest_id: String,
    pub result_fingerprint: String,
    pub evaluated_at_unix_ms: i64,
    pub scenario_ids: Vec<String>,
    pub held_out_scenario_count: u32,
    pub paired_count: u32,
    pub baseline_success_count: u32,
    pub treatment_success_count: u32,
    pub regression_free: bool,
}

/// One append-only lifecycle event. It contains identifiers and bounded
/// summaries, never prompts, commands, paths, or trace payloads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromotionEvent {
    pub schema_version: u32,
    pub event_id: String,
    pub candidate_id: String,
    pub candidate_fingerprint: String,
    pub proposal_created_at_unix_ms: i64,
    pub from_state: ImprovementState,
    pub to_state: ImprovementState,
    pub transition: PromotionTransition,
    pub actor: String,
    pub occurred_at_unix_ms: i64,
    pub target: ImprovementTarget,
    pub evidence: Vec<ImprovementEvidenceReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evaluation: Option<PromotionEvaluation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_version: Option<String>,
    pub expires_at_unix_ms: i64,
    pub reason: String,
}

/// A rebuildable projection of the promotion event log. No index file is
/// authoritative; loading this value always replays the append-only log.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PromotionState {
    records: BTreeMap<String, PromotionStatus>,
}

impl PromotionState {
    /// Returns the status for a candidate, if it has emitted a transition.
    pub fn get(&self, candidate_id: &str) -> Option<&PromotionStatus> {
        self.records.get(candidate_id)
    }

    /// Returns all records, sorted by candidate ID for stable inspection.
    pub fn records(&self) -> impl Iterator<Item = &PromotionStatus> {
        self.records.values()
    }

    /// Returns active records whose task family and scope match the target.
    pub fn active_for_target(&self, target: &ImprovementTarget) -> Vec<&PromotionStatus> {
        self.records
            .values()
            .filter(|status| {
                status.state == ImprovementState::Active
                    && status.target.task_family == target.task_family
                    && status.target.scope == target.scope
            })
            .collect()
    }

    fn status_for_proposal(
        &self,
        proposal: &ImprovementCandidate,
        fingerprint: &str,
    ) -> Result<PromotionStatus> {
        if let Some(status) = self.records.get(&proposal.candidate_id) {
            ensure!(
                status.candidate_fingerprint == fingerprint,
                "candidate {} is associated with a different proposal fingerprint",
                proposal.candidate_id
            );
            ensure!(
                status.target == proposal.target,
                "candidate {} is associated with a different promotion scope",
                proposal.candidate_id
            );
            return Ok(status.clone());
        }

        Ok(PromotionStatus::proposed(proposal, fingerprint.to_string()))
    }
}

/// Rebuildable status and provenance for one proposal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromotionStatus {
    pub candidate_id: String,
    pub candidate_fingerprint: String,
    pub state: ImprovementState,
    pub target: ImprovementTarget,
    pub proposal_created_at_unix_ms: i64,
    pub updated_at_unix_ms: i64,
    pub expires_at_unix_ms: i64,
    pub evidence: Vec<ImprovementEvidenceReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evaluation: Option<PromotionEvaluation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_event_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_actor: Option<String>,
}

impl PromotionStatus {
    fn proposed(proposal: &ImprovementCandidate, fingerprint: String) -> Self {
        Self {
            candidate_id: proposal.candidate_id.clone(),
            candidate_fingerprint: fingerprint,
            state: ImprovementState::Proposed,
            target: proposal.target.clone(),
            proposal_created_at_unix_ms: proposal.created_at_unix_ms,
            updated_at_unix_ms: proposal.created_at_unix_ms,
            expires_at_unix_ms: proposal.expiry.expires_at_unix_ms,
            evidence: proposal.evidence.clone(),
            evaluation: None,
            previous_version: proposal.rollback.previous_version.clone(),
            last_event_id: None,
            last_actor: None,
        }
    }

    /// Returns the promoted card ID when this status represents the card lane.
    pub fn active_card_id(&self) -> Option<&str> {
        (self.state == ImprovementState::Active)
            .then_some(self.target.target_name.as_deref())
            .flatten()
    }
}

/// Global and scoped controls for manual promotion and runtime retrieval.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromotionPolicy {
    pub schema_version: u32,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub kill_switch: bool,
    #[serde(default = "default_ttl_days")]
    pub default_ttl_days: u32,
    #[serde(default = "default_max_active")]
    pub default_max_active: usize,
    #[serde(default)]
    pub scopes: Vec<PromotionPolicyScope>,
}

impl Default for PromotionPolicy {
    fn default() -> Self {
        Self {
            schema_version: PROMOTION_POLICY_SCHEMA_VERSION,
            enabled: true,
            kill_switch: false,
            default_ttl_days: default_ttl_days(),
            default_max_active: default_max_active(),
            scopes: Vec::new(),
        }
    }
}

impl PromotionPolicy {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == PROMOTION_POLICY_SCHEMA_VERSION,
            "unsupported promotion policy schema {}",
            self.schema_version
        );
        ensure!(
            (1..=MAX_POLICY_TTL_DAYS).contains(&self.default_ttl_days),
            "promotion policy default TTL must be between 1 and {MAX_POLICY_TTL_DAYS} days"
        );
        ensure!(
            (1..=MAX_ACTIVE_IMPROVEMENTS).contains(&self.default_max_active),
            "promotion policy default max_active is outside the supported range"
        );
        ensure!(
            self.scopes.len() <= MAX_POLICY_SCOPES,
            "promotion policy contains too many scopes"
        );
        for scope in &self.scopes {
            ensure!(
                scope.scope.is_bounded(),
                "promotion policy scope is outside bounded limits"
            );
            if let Some(ttl_days) = scope.ttl_days {
                ensure!(
                    (1..=MAX_POLICY_TTL_DAYS).contains(&ttl_days),
                    "promotion policy scope TTL is outside the supported range"
                );
            }
            if let Some(max_active) = scope.max_active {
                ensure!(
                    (1..=MAX_ACTIVE_IMPROVEMENTS).contains(&max_active),
                    "promotion policy scope max_active is outside the supported range"
                );
            }
        }
        Ok(())
    }

    /// Resolves the most specific matching policy. When an allowlist is
    /// present, an unmatched repository/task family is disabled.
    pub fn decision_for(&self, target: &ImprovementTarget) -> PromotionPolicyDecision {
        let default_decision = PromotionPolicyDecision {
            enabled: self.enabled,
            kill_switch: self.kill_switch,
            ttl_days: self.default_ttl_days,
            max_active: self.default_max_active,
        };
        if self.kill_switch || !self.enabled {
            return default_decision;
        }
        if self.scopes.is_empty() {
            return default_decision;
        }

        let Some(scope) = self
            .scopes
            .iter()
            .filter(|scope| scope.matches(target))
            .max_by_key(|scope| scope.specificity())
        else {
            return PromotionPolicyDecision {
                enabled: false,
                kill_switch: false,
                ttl_days: self.default_ttl_days,
                max_active: self.default_max_active,
            };
        };

        PromotionPolicyDecision {
            enabled: scope.enabled,
            kill_switch: scope.kill_switch,
            ttl_days: scope.ttl_days.unwrap_or(self.default_ttl_days),
            max_active: scope.max_active.unwrap_or(self.default_max_active),
        }
    }
}

fn default_true() -> bool {
    true
}

const fn default_ttl_days() -> u32 {
    30
}

const fn default_max_active() -> usize {
    32
}

/// One repository/task-family policy override.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromotionPolicyScope {
    #[serde(default)]
    pub scope: ExperienceScope,
    #[serde(default)]
    pub task_family: Option<TaskFamily>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub kill_switch: bool,
    #[serde(default)]
    pub ttl_days: Option<u32>,
    #[serde(default)]
    pub max_active: Option<usize>,
}

impl PromotionPolicyScope {
    fn matches(&self, target: &ImprovementTarget) -> bool {
        self.task_family
            .is_none_or(|task_family| task_family == target.task_family)
            && self
                .scope
                .workspace_key
                .as_ref()
                .is_none_or(|key| target.scope.workspace_key.as_ref() == Some(key))
            && self
                .scope
                .toolchain_family
                .is_none_or(|family| target.scope.toolchain_family == Some(family))
            && self
                .scope
                .model_family
                .is_none_or(|family| target.scope.model_family == Some(family))
    }

    fn specificity(&self) -> usize {
        usize::from(self.task_family.is_some())
            + usize::from(self.scope.workspace_key.is_some())
            + usize::from(self.scope.toolchain_family.is_some())
            + usize::from(self.scope.model_family.is_some())
    }
}

/// Resolved policy values used by promotion and runtime filtering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PromotionPolicyDecision {
    pub enabled: bool,
    pub kill_switch: bool,
    pub ttl_days: u32,
    pub max_active: usize,
}

/// Loads and validates the event log, rebuilding the active state projection.
pub fn load_promotion_state(root: &Path) -> Result<PromotionState> {
    let path = promotion_events_path(root);
    let file = match File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(PromotionState::default());
        }
        Err(error) => {
            return Err(error)
                .with_context(|| format!("open promotion event store {}", path.display()));
        }
    };
    ensure!(
        file.metadata()?.len() <= MAX_EVENT_STORE_BYTES,
        "promotion event store exceeds the size limit"
    );

    let mut state = PromotionState::default();
    let mut event_count = 0usize;
    let mut event_ids = BTreeSet::new();
    for line in BufReader::new(file).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        event_count = event_count.saturating_add(1);
        ensure!(
            event_count <= MAX_EVENTS,
            "promotion event store has too many events"
        );
        ensure!(
            line.len() <= MAX_EVENT_LINE_BYTES,
            "promotion event exceeds the line size limit"
        );
        let event: PromotionEvent = serde_json::from_str(&line)
            .with_context(|| format!("parse promotion event in {}", path.display()))?;
        event.validate()?;
        ensure!(
            event_ids.insert(event.event_id.clone()),
            "duplicate promotion event ID"
        );
        apply_event(&mut state, &event)?;
    }
    Ok(state)
}

/// Returns the event-log path for a promotion root.
pub fn promotion_events_path(root: &Path) -> PathBuf {
    root.join(PROMOTION_EVENT_STORE_FILE_NAME)
}

/// Returns the policy path for a promotion root.
pub fn promotion_policy_path(root: &Path) -> PathBuf {
    root.join(PROMOTION_POLICY_FILE_NAME)
}

/// Loads an explicitly named policy file. Missing explicit files are errors.
pub fn load_promotion_policy(path: &Path) -> Result<PromotionPolicy> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("read promotion policy metadata {}", path.display()))?;
    ensure!(
        !metadata.file_type().is_symlink() && metadata.is_file(),
        "promotion policy must be a regular file, not a symlink or directory"
    );
    ensure!(
        metadata.len() <= 256 * 1024,
        "promotion policy exceeds the size limit"
    );
    let policy: PromotionPolicy = serde_json::from_reader(
        File::open(path).with_context(|| format!("open promotion policy {}", path.display()))?,
    )
    .with_context(|| format!("parse promotion policy {}", path.display()))?;
    policy.validate()?;
    Ok(policy)
}

/// Loads a root policy, using the safe, manual-promotion defaults when no
/// policy file exists. A malformed existing policy fails closed.
pub fn load_promotion_policy_for_root(root: &Path) -> Result<PromotionPolicy> {
    let path = promotion_policy_path(root);
    match fs::symlink_metadata(&path) {
        Ok(_) => load_promotion_policy(&path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(PromotionPolicy::default())
        }
        Err(error) => {
            Err(error).with_context(|| format!("read promotion policy metadata {}", path.display()))
        }
    }
}

/// Writes a validated policy file. This is a configuration operation, not a
/// promotion operation; it never changes active cards by itself.
pub fn write_promotion_policy(policy: &PromotionPolicy, path: &Path) -> Result<()> {
    policy.validate()?;
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("create promotion policy directory {}", parent.display()))?;
    }
    let data = serde_json::to_vec_pretty(policy).context("serialize promotion policy")?;
    fs::write(path, data).with_context(|| format!("write promotion policy {}", path.display()))?;
    Ok(())
}

/// Returns the current status for a proposal, including the implicit proposed
/// state before its first transition.
pub fn promotion_status(root: &Path, proposal: &ImprovementCandidate) -> Result<PromotionStatus> {
    proposal.validate()?;
    let fingerprint = proposal_fingerprint(proposal)?;
    let state = load_promotion_state(root)?;
    state.status_for_proposal(proposal, &fingerprint)
}

/// Records the explicit validation transition.
pub fn validate_proposal(
    root: &Path,
    proposal: &ImprovementCandidate,
    actor: &str,
    now_unix_ms: i64,
    catalog: Option<&ImprovementEvidenceCatalog>,
) -> Result<PromotionStatus> {
    proposal.validate()?;
    if let Some(catalog) = catalog {
        proposal.validate_against(catalog)?;
    }
    let state = load_promotion_state(root)?;
    let fingerprint = proposal_fingerprint(proposal)?;
    let status = state.status_for_proposal(proposal, &fingerprint)?;
    if status.state == ImprovementState::Validated {
        return Ok(status);
    }
    ensure!(
        status.state == ImprovementState::Proposed,
        "proposal {} cannot be validated from {}",
        proposal.candidate_id,
        status.state.as_str()
    );
    ensure_not_expired(proposal, now_unix_ms)?;
    let event = event_for_transition(
        proposal,
        &status,
        PromotionTransition::Validate,
        TransitionInput {
            actor: actor.to_string(),
            now_unix_ms,
            evidence: proposal.evidence.clone(),
            evaluation: None,
            previous_version: None,
            expires_at_unix_ms: proposal.expiry.expires_at_unix_ms,
            reason: "proposal structure and safety boundaries validated".to_string(),
        },
    )?;
    append_event(root, &event)?;
    promotion_status(root, proposal)
}

/// Evaluates the required held-out scenarios and records a redacted summary.
pub fn evaluate_proposal(
    root: &Path,
    proposal: &ImprovementCandidate,
    results: &[EvaluationResult],
    catalog: &ImprovementEvidenceCatalog,
    actor: &str,
    now_unix_ms: i64,
) -> Result<PromotionStatus> {
    proposal.validate_against(catalog)?;
    let state = load_promotion_state(root)?;
    let fingerprint = proposal_fingerprint(proposal)?;
    let status = state.status_for_proposal(proposal, &fingerprint)?;
    if status.state == ImprovementState::Evaluated {
        return Ok(status);
    }
    ensure!(
        status.state == ImprovementState::Validated,
        "proposal {} must be validated before evaluation",
        proposal.candidate_id
    );
    ensure_not_expired(proposal, now_unix_ms)?;

    let scenario_ids = proposal
        .validation
        .iter()
        .filter_map(|requirement| requirement.evaluation_scenario_id.as_deref())
        .collect::<BTreeSet<_>>();
    ensure!(
        !scenario_ids.is_empty(),
        "proposal {} has no evaluation scenario requirement",
        proposal.candidate_id
    );

    let mut selected = Vec::with_capacity(scenario_ids.len());
    for scenario_id in &scenario_ids {
        let matches = results
            .iter()
            .flat_map(|result| result.scenarios.iter())
            .filter(|scenario| scenario.scenario_id == *scenario_id)
            .collect::<Vec<_>>();
        ensure!(
            matches.len() == 1,
            "evaluation scenario {scenario_id} must occur exactly once in the supplied results"
        );
        let scenario = matches[0];
        validate_evaluation_scenario(proposal, scenario)?;
        selected.push(scenario);
    }

    let first_result = results.first().ok_or_else(|| {
        anyhow::anyhow!("at least one evaluation result is required for promotion")
    })?;
    let result_manifest_id = first_result.manifest_id.clone();
    ensure!(
        results
            .iter()
            .all(|result| result.schema_version == EVALUATION_RESULT_SCHEMA_VERSION),
        "evaluation result schema is stale or unsupported"
    );
    ensure!(
        results
            .iter()
            .all(|result| result.manifest_id == result_manifest_id),
        "evaluation results must belong to one manifest"
    );
    let evaluated_at_unix_ms = results
        .iter()
        .map(|result| result.evaluated_at_unix_ms)
        .min()
        .unwrap_or_default();
    ensure!(
        evaluated_at_unix_ms >= proposal.created_at_unix_ms,
        "evaluation evidence predates the proposal"
    );
    let result_fingerprint = fingerprint_results(results)?;
    let evaluation = PromotionEvaluation {
        result_manifest_id: result_manifest_id.clone(),
        result_fingerprint,
        evaluated_at_unix_ms,
        scenario_ids: scenario_ids.iter().map(|id| (*id).to_string()).collect(),
        held_out_scenario_count: u32::try_from(selected.len()).unwrap_or(u32::MAX),
        paired_count: u32::try_from(
            selected
                .iter()
                .filter(|scenario| {
                    scenario.baseline.evaluator.task_success.is_some()
                        && scenario.treatment.evaluator.task_success.is_some()
                })
                .count(),
        )
        .unwrap_or(u32::MAX),
        baseline_success_count: u32::try_from(
            selected
                .iter()
                .filter(|scenario| scenario.baseline.evaluator.task_success == Some(true))
                .count(),
        )
        .unwrap_or(u32::MAX),
        treatment_success_count: u32::try_from(
            selected
                .iter()
                .filter(|scenario| scenario.treatment.evaluator.task_success == Some(true))
                .count(),
        )
        .unwrap_or(u32::MAX),
        regression_free: true,
    };

    let mut evidence = proposal.evidence.clone();
    evidence.push(ImprovementEvidenceReference {
        kind: ImprovementEvidenceKind::EvaluationManifest,
        id: result_manifest_id,
    });
    for scenario in &selected {
        evidence.push(ImprovementEvidenceReference {
            kind: ImprovementEvidenceKind::EvaluationScenario,
            id: scenario.scenario_id.clone(),
        });
        if let Some(trace_id) = &scenario.baseline.trace_id {
            evidence.push(ImprovementEvidenceReference {
                kind: ImprovementEvidenceKind::TraceBundle,
                id: trace_id.clone(),
            });
        }
        if let Some(trace_id) = &scenario.treatment.trace_id {
            evidence.push(ImprovementEvidenceReference {
                kind: ImprovementEvidenceKind::TraceBundle,
                id: trace_id.clone(),
            });
        }
    }
    deduplicate_evidence(&mut evidence);

    let event = event_for_transition(
        proposal,
        &status,
        PromotionTransition::Evaluate,
        TransitionInput {
            actor: actor.to_string(),
            now_unix_ms,
            evidence,
            evaluation: Some(evaluation),
            previous_version: status.previous_version.clone(),
            expires_at_unix_ms: proposal.expiry.expires_at_unix_ms,
            reason: "held-out paired evaluation completed without a measured regression"
                .to_string(),
        },
    )?;
    append_event(root, &event)?;
    promotion_status(root, proposal)
}

/// Records the explicit human acceptance decision.
pub fn accept_proposal(
    root: &Path,
    proposal: &ImprovementCandidate,
    actor: &str,
    now_unix_ms: i64,
) -> Result<PromotionStatus> {
    let status = require_current_state(root, proposal, ImprovementState::Evaluated)?;
    ensure_not_expired(proposal, now_unix_ms)?;
    let event = event_for_transition(
        proposal,
        &status,
        PromotionTransition::Accept,
        TransitionInput {
            actor: actor.to_string(),
            now_unix_ms,
            evidence: status.evidence.clone(),
            evaluation: status.evaluation.clone(),
            previous_version: status.previous_version.clone(),
            expires_at_unix_ms: status.expires_at_unix_ms,
            reason: "human reviewer accepted the evaluated proposal".to_string(),
        },
    )?;
    append_event(root, &event)?;
    promotion_status(root, proposal)
}

/// Activates a card proposal after policy and evaluation gates pass.
pub fn promote_proposal(
    root: &Path,
    proposal: &ImprovementCandidate,
    actor: &str,
    now_unix_ms: i64,
) -> Result<PromotionStatus> {
    let policy = load_promotion_policy_for_root(root)?;
    promote_proposal_with_policy(root, proposal, actor, now_unix_ms, &policy)
}

/// Variant used by the CLI when a policy file is explicitly supplied.
pub fn promote_proposal_with_policy(
    root: &Path,
    proposal: &ImprovementCandidate,
    actor: &str,
    now_unix_ms: i64,
    policy: &PromotionPolicy,
) -> Result<PromotionStatus> {
    proposal.validate()?;
    policy.validate()?;
    let state = load_promotion_state(root)?;
    let fingerprint = proposal_fingerprint(proposal)?;
    let status = state.status_for_proposal(proposal, &fingerprint)?;
    if status.state == ImprovementState::Active {
        return Ok(status);
    }
    ensure!(
        status.state == ImprovementState::Accepted,
        "proposal {} must be accepted before promotion",
        proposal.candidate_id
    );
    ensure_not_expired(proposal, now_unix_ms)?;
    let evaluation = status
        .evaluation
        .clone()
        .ok_or_else(|| anyhow::anyhow!("promotion requires recorded evaluation evidence"))?;
    ensure!(
        evaluation.regression_free,
        "promotion is blocked because evaluation recorded a regression"
    );
    let decision = policy.decision_for(&proposal.target);
    ensure!(
        decision.enabled && !decision.kill_switch,
        "promotion policy disables this repository/task-family scope"
    );
    let active_count = state.active_for_target(&proposal.target).len();
    ensure!(
        active_count < decision.max_active,
        "promotion policy maximum active improvements reached for this scope"
    );

    let ImprovementContent::ExperienceCardUpdate { card } = &proposal.content else {
        bail!(
            "{} lane cannot be promoted by this control plane; review and apply it as a normal diff",
            proposal.candidate_type.as_str()
        );
    };
    ensure!(
        card.status == CardStatus::Active,
        "a retired experience card cannot be promoted"
    );
    let effective_expiry = proposal
        .expiry
        .expires_at_unix_ms
        .min(now_unix_ms.saturating_add(i64::from(decision.ttl_days).saturating_mul(DAY_MS)));
    ensure!(
        effective_expiry > now_unix_ms,
        "promotion policy TTL has already expired"
    );

    let previous_version = status
        .previous_version
        .clone()
        .filter(|version| version != &card.card_id)
        .or_else(|| find_previous_active_card(root, proposal, &card.card_id));
    let event = event_for_transition(
        proposal,
        &status,
        PromotionTransition::Promote,
        TransitionInput {
            actor: actor.to_string(),
            now_unix_ms,
            evidence: status.evidence.clone(),
            evaluation: Some(evaluation),
            previous_version,
            expires_at_unix_ms: effective_expiry,
            reason: "human-approved card proposal activated for scoped retrieval".to_string(),
        },
    )?;
    append_event(root, &event)?;
    if let Err(error) = append_card(root, card) {
        // The event is authoritative for runtime filtering. Compensate with a
        // retirement event when possible so a failed card write remains
        // visibly and operationally disabled on the next retry.
        if let Ok(active_status) = promotion_status(root, proposal)
            && active_status.state == ImprovementState::Active
            && let Ok(retire_event) = event_for_transition(
                proposal,
                &active_status,
                PromotionTransition::Retire,
                TransitionInput {
                    actor: actor.to_string(),
                    now_unix_ms,
                    evidence: active_status.evidence.clone(),
                    evaluation: active_status.evaluation.clone(),
                    previous_version: active_status.previous_version.clone(),
                    expires_at_unix_ms: active_status.expires_at_unix_ms,
                    reason: "card store write failed during promotion; automatically disabled"
                        .to_string(),
                },
            )
        {
            let _ = append_event(root, &retire_event);
        }
        return Err(error).context("write promoted experience card");
    }
    promotion_status(root, proposal)
}

/// Rolls back an active card and restores the previous active card when one
/// was recorded. Unrelated cards and event history are left untouched.
pub fn rollback_proposal(
    root: &Path,
    proposal: &ImprovementCandidate,
    actor: &str,
    now_unix_ms: i64,
) -> Result<PromotionStatus> {
    let status = require_current_state(root, proposal, ImprovementState::Active)?;
    let event = event_for_transition(
        proposal,
        &status,
        PromotionTransition::Rollback,
        TransitionInput {
            actor: actor.to_string(),
            now_unix_ms,
            evidence: status.evidence.clone(),
            evaluation: status.evaluation.clone(),
            previous_version: status.previous_version.clone(),
            expires_at_unix_ms: status.expires_at_unix_ms,
            reason: "human-requested rollback of the active improvement".to_string(),
        },
    )?;
    append_event(root, &event)?;

    if let ImprovementContent::ExperienceCardUpdate { card } = &proposal.content {
        retire_card(root, &card.card_id).context("retire rolled-back experience card")?;
        if let Some(previous_version) = &status.previous_version
            && previous_version != &card.card_id
            && let Some(previous) = load_cards(root)?
                .into_iter()
                .find(|candidate| candidate.card_id == *previous_version)
        {
            append_card(root, &previous.with_status(CardStatus::Active))
                .context("restore previous experience card")?;
        }
    }
    promotion_status(root, proposal)
}

/// Retires an active proposal without changing unrelated state.
pub fn retire_proposal(
    root: &Path,
    proposal: &ImprovementCandidate,
    actor: &str,
    now_unix_ms: i64,
) -> Result<PromotionStatus> {
    let status = require_current_state(root, proposal, ImprovementState::Active)?;
    let event = event_for_transition(
        proposal,
        &status,
        PromotionTransition::Retire,
        TransitionInput {
            actor: actor.to_string(),
            now_unix_ms,
            evidence: status.evidence.clone(),
            evaluation: status.evaluation.clone(),
            previous_version: status.previous_version.clone(),
            expires_at_unix_ms: status.expires_at_unix_ms,
            reason: "human-requested retirement of the active improvement".to_string(),
        },
    )?;
    append_event(root, &event)?;
    if let ImprovementContent::ExperienceCardUpdate { card } = &proposal.content {
        retire_card(root, &card.card_id).context("retire experience card")?;
    }
    promotion_status(root, proposal)
}

/// Rejects a proposal before activation.
pub fn reject_proposal(
    root: &Path,
    proposal: &ImprovementCandidate,
    actor: &str,
    now_unix_ms: i64,
    reason: impl Into<String>,
) -> Result<PromotionStatus> {
    proposal.validate()?;
    let state = load_promotion_state(root)?;
    let fingerprint = proposal_fingerprint(proposal)?;
    let status = state.status_for_proposal(proposal, &fingerprint)?;
    ensure!(
        matches!(
            status.state,
            ImprovementState::Proposed
                | ImprovementState::Validated
                | ImprovementState::Evaluated
                | ImprovementState::Accepted
        ),
        "proposal {} cannot be rejected from {}",
        proposal.candidate_id,
        status.state.as_str()
    );
    let event = event_for_transition(
        proposal,
        &status,
        PromotionTransition::Reject,
        TransitionInput {
            actor: actor.to_string(),
            now_unix_ms,
            evidence: status.evidence.clone(),
            evaluation: status.evaluation.clone(),
            previous_version: status.previous_version.clone(),
            expires_at_unix_ms: status.expires_at_unix_ms,
            reason: reason.into(),
        },
    )?;
    append_event(root, &event)?;
    promotion_status(root, proposal)
}

/// Loads cards through the promotion policy and event log. Legacy active
/// cards remain available; cards tied to this state machine require an active
/// event, a non-expired status, and a matching policy decision.
pub fn load_runtime_cards(root: &Path, now_unix_ms: i64) -> Result<Vec<ExperienceCard>> {
    let cards = load_cards(root).context("load experience cards for runtime retrieval")?;
    filter_runtime_cards(root, &cards, now_unix_ms)
}

/// Applies fail-closed policy and promotion-state filtering to already loaded
/// cards. This is separated for extension and unit-test use.
pub fn filter_runtime_cards(
    root: &Path,
    cards: &[ExperienceCard],
    now_unix_ms: i64,
) -> Result<Vec<ExperienceCard>> {
    let policy = load_promotion_policy_for_root(root)?;
    policy.validate()?;
    let state = load_promotion_state(root)?;
    let mut filtered = Vec::new();
    for card in cards {
        if card.status == CardStatus::Retired {
            continue;
        }
        let target = ImprovementTarget {
            task_family: card.task_family,
            scope: card.scope.clone(),
            target_name: Some(card.card_id.clone()),
        };
        let decision = policy.decision_for(&target);
        if !decision.enabled || decision.kill_switch {
            continue;
        }
        if let Some(status) = state
            .records()
            .find(|status| status.target.target_name.as_deref() == Some(card.card_id.as_str()))
        {
            if status.state != ImprovementState::Active {
                continue;
            }
            let policy_expiry = status
                .updated_at_unix_ms
                .saturating_add(i64::from(decision.ttl_days).saturating_mul(DAY_MS));
            if now_unix_ms >= status.expires_at_unix_ms.min(policy_expiry) {
                continue;
            }
        }
        filtered.push(card.clone());
    }
    Ok(filtered)
}

/// Current wall-clock time used by CLI and runtime integration.
pub fn current_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or_default()
}

fn require_current_state(
    root: &Path,
    proposal: &ImprovementCandidate,
    expected: ImprovementState,
) -> Result<PromotionStatus> {
    proposal.validate()?;
    let status = promotion_status(root, proposal)?;
    ensure!(
        status.state == expected,
        "proposal {} must be {} before this operation (current state: {})",
        proposal.candidate_id,
        expected.as_str(),
        status.state.as_str()
    );
    Ok(status)
}

fn ensure_not_expired(proposal: &ImprovementCandidate, now_unix_ms: i64) -> Result<()> {
    ensure!(now_unix_ms >= 0, "promotion timestamp cannot be negative");
    ensure!(
        now_unix_ms < proposal.expiry.expires_at_unix_ms,
        "proposal {} has expired",
        proposal.candidate_id
    );
    Ok(())
}

struct TransitionInput {
    actor: String,
    now_unix_ms: i64,
    evidence: Vec<ImprovementEvidenceReference>,
    evaluation: Option<PromotionEvaluation>,
    previous_version: Option<String>,
    expires_at_unix_ms: i64,
    reason: String,
}

fn event_for_transition(
    proposal: &ImprovementCandidate,
    status: &PromotionStatus,
    transition: PromotionTransition,
    mut input: TransitionInput,
) -> Result<PromotionEvent> {
    validate_actor(&input.actor)?;
    ensure!(
        input.now_unix_ms >= 0,
        "promotion timestamp cannot be negative"
    );
    ensure!(
        input.expires_at_unix_ms >= 0,
        "promotion expiry cannot be negative"
    );
    deduplicate_evidence(&mut input.evidence);
    let candidate_fingerprint = proposal_fingerprint(proposal)?;
    let to_state = transition_to_state(transition, status.state)?;
    let identity = format!(
        "{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}",
        proposal.candidate_id,
        status.state.as_str(),
        to_state.as_str(),
        input.actor.as_str(),
        input.now_unix_ms,
        input.reason.as_str()
    );
    let event_id = digest_id("promotion_", &identity);
    let event = PromotionEvent {
        schema_version: PROMOTION_EVENT_SCHEMA_VERSION,
        event_id,
        candidate_id: proposal.candidate_id.clone(),
        candidate_fingerprint,
        proposal_created_at_unix_ms: proposal.created_at_unix_ms,
        from_state: status.state,
        to_state,
        transition,
        actor: input.actor,
        occurred_at_unix_ms: input.now_unix_ms,
        target: proposal.target.clone(),
        evidence: input.evidence,
        evaluation: input.evaluation,
        previous_version: input.previous_version,
        expires_at_unix_ms: input.expires_at_unix_ms,
        reason: input.reason,
    };
    event.validate()?;
    Ok(event)
}

fn transition_to_state(
    transition: PromotionTransition,
    from_state: ImprovementState,
) -> Result<ImprovementState> {
    let to_state = match transition {
        PromotionTransition::Validate if from_state == ImprovementState::Proposed => {
            ImprovementState::Validated
        }
        PromotionTransition::Evaluate if from_state == ImprovementState::Validated => {
            ImprovementState::Evaluated
        }
        PromotionTransition::Accept if from_state == ImprovementState::Evaluated => {
            ImprovementState::Accepted
        }
        PromotionTransition::Promote if from_state == ImprovementState::Accepted => {
            ImprovementState::Active
        }
        PromotionTransition::Rollback | PromotionTransition::Retire
            if from_state == ImprovementState::Active =>
        {
            ImprovementState::Retired
        }
        PromotionTransition::Reject
            if matches!(
                from_state,
                ImprovementState::Proposed
                    | ImprovementState::Validated
                    | ImprovementState::Evaluated
                    | ImprovementState::Accepted
            ) =>
        {
            ImprovementState::Rejected
        }
        _ => bail!(
            "transition {} is not allowed from {}",
            transition.as_str(),
            from_state.as_str()
        ),
    };
    Ok(to_state)
}

fn append_event(root: &Path, event: &PromotionEvent) -> Result<()> {
    event.validate()?;
    let encoded = serde_json::to_vec(event).context("serialize promotion event")?;
    ensure!(
        encoded.len() <= MAX_EVENT_LINE_BYTES,
        "promotion event exceeds the line size limit"
    );
    let path = promotion_events_path(root);
    let current_size = match fs::metadata(&path) {
        Ok(metadata) => metadata.len(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
        Err(error) => {
            return Err(error).with_context(|| format!("read promotion store {}", path.display()));
        }
    };
    ensure!(
        current_size.saturating_add(encoded.len() as u64 + 1) <= MAX_EVENT_STORE_BYTES,
        "promotion event store would exceed the size limit"
    );
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create promotion event directory {}", parent.display()))?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("open promotion event store {}", path.display()))?;
    let mut record = encoded;
    record.push(b'\n');
    file.write_all(&record)
        .with_context(|| format!("append promotion event {}", path.display()))?;
    file.sync_data()
        .with_context(|| format!("sync promotion event {}", path.display()))?;
    Ok(())
}

impl PromotionEvent {
    fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == PROMOTION_EVENT_SCHEMA_VERSION,
            "unsupported promotion event schema {}",
            self.schema_version
        );
        validate_identifier(&self.event_id, "promotion event_id")?;
        validate_identifier(&self.candidate_id, "promotion candidate_id")?;
        ensure!(
            is_fingerprint(&self.candidate_fingerprint),
            "promotion candidate fingerprint is malformed"
        );
        ensure!(
            self.proposal_created_at_unix_ms >= 0,
            "promotion proposal timestamp cannot be negative"
        );
        ensure!(
            self.occurred_at_unix_ms >= 0,
            "promotion event timestamp cannot be negative"
        );
        validate_actor(&self.actor)?;
        validate_target(&self.target)?;
        ensure!(
            self.evidence.len() <= MAX_EVIDENCE,
            "promotion event contains too much evidence"
        );
        let mut evidence = BTreeSet::new();
        for reference in &self.evidence {
            validate_identifier(&reference.id, "promotion evidence ID")?;
            ensure!(
                evidence.insert(reference),
                "duplicate promotion evidence reference"
            );
        }
        if let Some(evaluation) = &self.evaluation {
            validate_evaluation(evaluation)?;
        }
        if let Some(previous_version) = &self.previous_version {
            validate_identifier(previous_version, "promotion previous version")?;
        }
        ensure!(
            self.expires_at_unix_ms >= 0,
            "promotion expiry cannot be negative"
        );
        validate_text(&self.reason, MAX_REASON_CHARS, "promotion reason")?;
        let expected = transition_to_state(self.transition, self.from_state)?;
        ensure!(
            self.to_state == expected,
            "promotion event state does not match its transition"
        );
        if self.to_state == ImprovementState::Evaluated || self.to_state == ImprovementState::Active
        {
            ensure!(
                self.evaluation.is_some(),
                "evaluated and active promotion events require evaluation evidence"
            );
        }
        Ok(())
    }
}

fn apply_event(state: &mut PromotionState, event: &PromotionEvent) -> Result<()> {
    let current = state
        .records
        .get(&event.candidate_id)
        .map_or(ImprovementState::Proposed, |status| status.state);
    ensure!(
        current == event.from_state,
        "promotion event {} expects {}, log state is {}",
        event.event_id,
        event.from_state.as_str(),
        current.as_str()
    );
    let candidate_fingerprint = event.candidate_fingerprint.clone();
    let status = state
        .records
        .entry(event.candidate_id.clone())
        .or_insert_with(|| PromotionStatus {
            candidate_id: event.candidate_id.clone(),
            candidate_fingerprint: candidate_fingerprint.clone(),
            state: ImprovementState::Proposed,
            target: event.target.clone(),
            proposal_created_at_unix_ms: event.proposal_created_at_unix_ms,
            updated_at_unix_ms: event.occurred_at_unix_ms,
            expires_at_unix_ms: event.expires_at_unix_ms,
            evidence: Vec::new(),
            evaluation: None,
            previous_version: None,
            last_event_id: None,
            last_actor: None,
        });
    ensure!(
        status.candidate_fingerprint == candidate_fingerprint,
        "promotion log changes candidate fingerprint for {}",
        event.candidate_id
    );
    ensure!(
        status.target == event.target,
        "promotion log changes target scope for {}",
        event.candidate_id
    );
    ensure!(
        status.proposal_created_at_unix_ms == event.proposal_created_at_unix_ms,
        "promotion log changes proposal creation time for {}",
        event.candidate_id
    );
    status.state = event.to_state;
    status.updated_at_unix_ms = event.occurred_at_unix_ms;
    status.expires_at_unix_ms = event.expires_at_unix_ms;
    status.evidence.extend(event.evidence.clone());
    deduplicate_evidence(&mut status.evidence);
    if event.evaluation.is_some() {
        status.evaluation = event.evaluation.clone();
    }
    if event.previous_version.is_some() {
        status.previous_version = event.previous_version.clone();
    }
    status.last_event_id = Some(event.event_id.clone());
    status.last_actor = Some(event.actor.clone());
    Ok(())
}

fn validate_evaluation_scenario(
    proposal: &ImprovementCandidate,
    scenario: &EvaluationScenarioResult,
) -> Result<()> {
    ensure!(
        scenario.split == EvaluationSplit::HeldOut,
        "promotion evaluation scenario {} is not held out",
        scenario.scenario_id
    );
    ensure!(
        scenario.task_family == proposal.target.task_family,
        "evaluation scenario {} has task family {}, expected {}",
        scenario.scenario_id,
        scenario.task_family.as_str(),
        proposal.target.task_family.as_str()
    );
    ensure!(
        scenario.baseline.status == crate::EvaluationStatus::Evaluated
            && scenario.treatment.status == crate::EvaluationStatus::Evaluated,
        "evaluation scenario {} has unavailable evaluator evidence",
        scenario.scenario_id
    );
    ensure!(
        scenario.baseline.evaluator.task_success.is_some()
            && scenario.treatment.evaluator.task_success.is_some(),
        "evaluation scenario {} is missing task-success evidence",
        scenario.scenario_id
    );
    ensure!(
        scenario.delta.task_success.is_none_or(|delta| delta >= 0),
        "evaluation scenario {} regressed task success",
        scenario.scenario_id
    );
    ensure!(
        scenario
            .delta
            .verification_success
            .is_none_or(|delta| delta >= 0),
        "evaluation scenario {} regressed verification success",
        scenario.scenario_id
    );
    ensure!(
        scenario.baseline.provenance.execution_config_fingerprint
            == scenario.treatment.provenance.execution_config_fingerprint,
        "evaluation scenario {} changed execution configuration between conditions",
        scenario.scenario_id
    );
    ensure!(
        scenario.baseline.provenance.policy_fingerprint
            == scenario.treatment.provenance.policy_fingerprint,
        "evaluation scenario {} changed policy between conditions",
        scenario.scenario_id
    );
    ensure!(
        !scenario.baseline.provenance.memory.enabled
            && scenario.treatment.provenance.memory.enabled,
        "evaluation scenario {} is not a baseline versus memory-assisted pair",
        scenario.scenario_id
    );
    if let (Some(baseline), Some(treatment)) = (
        scenario.baseline.metrics.as_ref(),
        scenario.treatment.metrics.as_ref(),
    ) {
        ensure!(
            treatment.policy_event_count <= baseline.policy_event_count,
            "evaluation scenario {} introduced additional policy-boundary events",
            scenario.scenario_id
        );
    }
    Ok(())
}

fn validate_evaluation(evaluation: &PromotionEvaluation) -> Result<()> {
    validate_identifier(&evaluation.result_manifest_id, "evaluation manifest ID")?;
    ensure!(
        is_fingerprint(&evaluation.result_fingerprint),
        "evaluation result fingerprint is malformed"
    );
    ensure!(
        evaluation.evaluated_at_unix_ms >= 0,
        "evaluation timestamp cannot be negative"
    );
    ensure!(
        !evaluation.scenario_ids.is_empty() && evaluation.scenario_ids.len() <= MAX_EVIDENCE,
        "evaluation must contain a bounded held-out scenario set"
    );
    let mut scenario_ids = BTreeSet::new();
    for scenario_id in &evaluation.scenario_ids {
        validate_identifier(scenario_id, "evaluation scenario ID")?;
        ensure!(
            scenario_ids.insert(scenario_id),
            "duplicate evaluation scenario ID"
        );
    }
    ensure!(
        evaluation.held_out_scenario_count > 0
            && evaluation.paired_count > 0
            && evaluation.paired_count <= evaluation.held_out_scenario_count,
        "evaluation does not contain a paired held-out result"
    );
    Ok(())
}

fn find_previous_active_card(
    root: &Path,
    proposal: &ImprovementCandidate,
    current_card_id: &str,
) -> Option<String> {
    let ImprovementContent::ExperienceCardUpdate {
        card: proposed_card,
    } = &proposal.content
    else {
        return None;
    };
    let conflict_key = proposed_card.conflict_key();
    load_cards(root)
        .ok()?
        .into_iter()
        .filter(|card| {
            card.status == CardStatus::Active
                && card.card_id != current_card_id
                && card.conflict_key() == conflict_key
        })
        .max_by(|left, right| {
            left.created_at_unix_ms
                .cmp(&right.created_at_unix_ms)
                .then_with(|| left.card_id.cmp(&right.card_id))
        })
        .map(|card| card.card_id)
}

fn fingerprint_results(results: &[EvaluationResult]) -> Result<String> {
    let encoded = serde_json::to_vec(results).context("serialize evaluation results")?;
    Ok(hex_digest(&encoded))
}

fn proposal_fingerprint(proposal: &ImprovementCandidate) -> Result<String> {
    let encoded = serde_json::to_vec(proposal).context("serialize improvement proposal")?;
    Ok(hex_digest(&encoded))
}

fn hex_digest(value: &[u8]) -> String {
    Sha256::digest(value)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn digest_id(prefix: &str, identity: &str) -> String {
    format!("{prefix}{}", &hex_digest(identity.as_bytes())[..24])
}

fn deduplicate_evidence(evidence: &mut Vec<ImprovementEvidenceReference>) {
    let mut unique = BTreeSet::new();
    evidence.retain(|reference| unique.insert(reference.clone()));
}

fn validate_target(target: &ImprovementTarget) -> Result<()> {
    ensure!(
        target.scope.is_bounded(),
        "promotion target scope is outside bounded limits"
    );
    if let Some(target_name) = &target.target_name {
        validate_identifier(target_name, "promotion target name")?;
    }
    Ok(())
}

fn validate_actor(actor: &str) -> Result<()> {
    validate_text(actor, MAX_ACTOR_CHARS, "promotion actor")
}

fn validate_identifier(value: &str, label: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value.chars().count() <= MAX_ID_CHARS
            && value
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || "_.:-".contains(character)),
        "{label} is not a bounded identifier"
    );
    Ok(())
}

fn validate_text(value: &str, max_chars: usize, label: &str) -> Result<()> {
    ensure!(!value.trim().is_empty(), "{label} is empty");
    ensure!(
        value.chars().count() <= max_chars,
        "{label} exceeds the size limit"
    );
    ensure!(
        !value
            .chars()
            .any(|character| character.is_control() || matches!(character, '<' | '>')),
        "{label} contains unsupported characters"
    );
    Ok(())
}

fn is_fingerprint(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
#[path = "promotion_tests.rs"]
mod tests;
