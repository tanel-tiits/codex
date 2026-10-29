//! Deterministic, paired evaluation of memory-assisted Codex rollouts.
//!
//! The evaluator consumes two already-recorded rollout bundles per scenario.
//! It deliberately does not launch a model, write a checkout, or read an
//! active card store. That makes the evaluation path suitable for offline CI
//! and prevents an evaluation command from inheriting live approval,
//! credential, or sandbox state. The Milestone 8a live bridge can produce the
//! bundles consumed here under a separately reviewed execution policy.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use anyhow::Context;
use anyhow::Result;
use anyhow::bail;
use anyhow::ensure;
use codex_rollout_trace::ConversationRole;
use codex_rollout_trace::RolloutStatus;
use codex_rollout_trace::RolloutTrace;
use codex_rollout_trace::TerminalRequest;
use codex_rollout_trace::ToolCallSummary;
use codex_rollout_trace::replay_bundle;
use serde::Deserialize;
use serde::Serialize;

use crate::AttributionState;
use crate::FailureClass;
use crate::FailureSignature;
use crate::InterventionTelemetryAggregate;
use crate::InterventionTelemetryContext;
use crate::InterventionTelemetryEvidence;
use crate::InterventionTelemetryResult;
use crate::MAX_INTERVENTION_SIGNATURES;
use crate::TaskFamily;
use crate::TokenUsageSummary;
use crate::derive_intervention_telemetry;
use crate::summarize_rollout;

/// Version of the local scenario manifest contract.
pub const EVALUATION_MANIFEST_SCHEMA_VERSION: u32 = 2;
/// Version of the aggregate result contract.
pub const EVALUATION_RESULT_SCHEMA_VERSION: u32 = 5;
/// Version of the deterministic evaluator contract included in every result.
pub const EVALUATOR_VERSION: u32 = 2;

const MAX_SCENARIOS: usize = 512;
const MAX_CHECKS: usize = 16;
const MAX_ID_CHARS: usize = 128;
const MAX_FINGERPRINT_CHARS: usize = 256;
const MAX_COMMAND_TOKEN_CHARS: usize = 128;
const DEFAULT_MAX_TRACE_BYTES: u64 = 64 * 1024 * 1024;
const DEFAULT_MAX_TOKENS: u64 = 2_000_000;
const DEFAULT_MAX_WALL_TIME_MS: u64 = 30 * 60 * 1000;
const DEFAULT_MAX_TOOL_CALLS: u32 = 512;
const DEFAULT_MAX_TERMINAL_OPERATIONS: u32 = 512;

/// Split used to prevent tuning and reporting on the same task instances.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvaluationSplit {
    Train,
    Validation,
    HeldOut,
}

impl EvaluationSplit {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Train => "train",
            Self::Validation => "validation",
            Self::HeldOut => "held_out",
        }
    }
}

/// Evaluator used for one scenario pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvaluatorKind {
    /// Treat a completed, non-cancelled trace as a successful task outcome.
    TraceOutcome,
    /// Match declared command tokens and require their declared exit code.
    Verification,
    /// Consume bounded checks executed by the evaluator after the agent run.
    PostRunVerification,
    /// Consume a human label supplied in the private manifest.
    HumanJudge,
    /// Consume an explicitly recorded model-judge label; no model is called.
    ModelJudge,
}

impl EvaluatorKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TraceOutcome => "trace_outcome",
            Self::Verification => "verification",
            Self::PostRunVerification => "post_run_verification",
            Self::HumanJudge => "human_judge",
            Self::ModelJudge => "model_judge",
        }
    }
}

/// A task prompt or an opaque reference to a task kept outside aggregate
/// results. `fingerprint` must be stable for the paired baseline/treatment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvaluationTask {
    pub task_id: String,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub reference: Option<String>,
    pub fingerprint: String,
}

/// Repository identity and the primary checkout that must remain untouched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvaluationRepository {
    pub snapshot_fingerprint: String,
    #[serde(default)]
    pub primary_checkout: Option<PathBuf>,
}

/// Memory provenance for one condition. The card-store path is used only for
/// isolation checks and is intentionally omitted from published results.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvaluationMemory {
    pub enabled: bool,
    pub card_set_fingerprint: String,
    pub retrieval_policy_fingerprint: String,
    pub card_schema_version: u32,
    #[serde(default)]
    pub card_store_path: Option<PathBuf>,
}

/// One recorded condition in a paired scenario.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvaluationCondition {
    pub trace_bundle: PathBuf,
    pub worktree: PathBuf,
    pub worktree_fingerprint: String,
    pub execution_config_fingerprint: String,
    pub policy_fingerprint: String,
    pub memory: EvaluationMemory,
    /// Private retrieval evidence; raw card IDs never enter the redacted result.
    #[serde(default)]
    pub retrieval: EvaluationRetrievalEvidence,
    /// Private context and event chain for intervention attribution. It is
    /// never copied into the aggregate evaluation result.
    #[serde(default)]
    pub intervention_telemetry: Option<InterventionTelemetryEvidence>,
    #[serde(default)]
    pub judge: Option<JudgeLabel>,
    /// Private evidence produced by evaluator-side post-run checks.
    #[serde(default)]
    pub post_run: Option<PostRunEvidence>,
}

/// Private label supplied by a human or an offline judge. It is never inferred
/// from exposure and is not required for deterministic verification evaluators.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JudgeLabel {
    pub label_id: String,
    #[serde(default)]
    pub task_success: Option<bool>,
    #[serde(default)]
    pub verification_success: Option<bool>,
    #[serde(default)]
    pub score_bps: Option<u16>,
}

/// One deterministic verification command specification. The raw token list
/// stays in the local manifest; result output contains only `check_id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationCheck {
    pub check_id: String,
    pub command_tokens: Vec<String>,
    #[serde(default)]
    pub expected_exit_code: i32,
    /// Optional evaluator self-check executed before any model run. Live
    /// post-run plans must provide this to validate the check harness.
    #[serde(default)]
    pub preflight_command_tokens: Option<Vec<String>>,
    #[serde(default)]
    pub preflight_expected_exit_code: i32,
}

/// Private evidence captured by the live runner about card selection. IDs are
/// retained only in the private manifest and are never written to results.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct EvaluationRetrievalEvidence {
    #[serde(default)]
    pub selected_card_ids: Vec<String>,
    #[serde(default)]
    pub exposure_count: u64,
    /// Private injection evidence for the intervention lane.
    #[serde(default)]
    pub injection_count: u64,
}

/// Redacted retrieval evidence published alongside condition metrics.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct EvaluationRetrievalResult {
    pub selected_card_count: u32,
    pub exposure_count: u64,
    #[serde(default)]
    pub selection_fingerprint: Option<String>,
}

/// Results from evaluator-side commands executed after a child run exits.
/// Command tokens and command output remain private to the evaluation
/// manifest and runner; only these bounded check results are published.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PostRunEvidence {
    pub checks: Vec<CheckResult>,
}

/// Evaluator settings for one scenario.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvaluatorSpec {
    pub kind: EvaluatorKind,
    #[serde(default)]
    pub checks: Vec<VerificationCheck>,
    /// For multiple checks, require all (`true`) or at least one (`false`).
    #[serde(default = "default_require_all_checks")]
    pub require_all_checks: bool,
}

impl EvaluatorSpec {
    /// Stable evaluator implementation version recorded with live results.
    pub const fn version(&self) -> u32 {
        EVALUATOR_VERSION
    }

    /// Stable opaque identity for this evaluator definition. The command
    /// tokens are hash input only and never appear in published results.
    pub fn fingerprint(&self) -> String {
        crate::fingerprint_descriptor(
            "evaluator-definition-v1",
            &serde_json::to_string(self).unwrap_or_default(),
        )
    }
}

fn default_require_all_checks() -> bool {
    true
}

/// Resource limits applied independently to each baseline/treatment trace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvaluationBudget {
    #[serde(default = "default_max_trace_bytes")]
    pub max_trace_bytes: u64,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u64,
    #[serde(default = "default_max_wall_time_ms")]
    pub max_wall_time_ms: u64,
    #[serde(default = "default_max_tool_calls")]
    pub max_tool_calls: u32,
    #[serde(default = "default_max_terminal_operations")]
    pub max_terminal_operations: u32,
    #[serde(default = "default_max_post_run_time_ms")]
    pub max_post_run_time_ms: u64,
}

impl Default for EvaluationBudget {
    fn default() -> Self {
        Self {
            max_trace_bytes: default_max_trace_bytes(),
            max_tokens: default_max_tokens(),
            max_wall_time_ms: default_max_wall_time_ms(),
            max_tool_calls: default_max_tool_calls(),
            max_terminal_operations: default_max_terminal_operations(),
            max_post_run_time_ms: default_max_post_run_time_ms(),
        }
    }
}

const fn default_max_trace_bytes() -> u64 {
    DEFAULT_MAX_TRACE_BYTES
}

const fn default_max_tokens() -> u64 {
    DEFAULT_MAX_TOKENS
}

const fn default_max_wall_time_ms() -> u64 {
    DEFAULT_MAX_WALL_TIME_MS
}

const fn default_max_tool_calls() -> u32 {
    DEFAULT_MAX_TOOL_CALLS
}

const fn default_max_terminal_operations() -> u32 {
    DEFAULT_MAX_TERMINAL_OPERATIONS
}

const fn default_max_post_run_time_ms() -> u64 {
    2 * 60 * 1000
}

/// Explicit evidence about whether the assisted condition exposed and used a
/// card. This remains descriptive evidence; it is not a causal guarantee.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AttributionEvidence {
    #[serde(default)]
    pub exposure: AttributionState,
    #[serde(default)]
    pub followed: AttributionState,
    #[serde(default)]
    pub evidence_id: Option<String>,
}

/// One matched baseline/treatment task instance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvaluationScenario {
    pub scenario_id: String,
    pub task_family: TaskFamily,
    pub split: EvaluationSplit,
    pub task: EvaluationTask,
    pub repository: EvaluationRepository,
    pub baseline: EvaluationCondition,
    pub treatment: EvaluationCondition,
    pub evaluator: EvaluatorSpec,
    #[serde(default)]
    pub budget: EvaluationBudget,
    #[serde(default)]
    pub attribution: AttributionEvidence,
}

/// Local, private scenario set. Prompts, paths, command tokens, and judge
/// labels belong here; aggregate results intentionally omit them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvaluationManifest {
    pub schema_version: u32,
    pub manifest_id: String,
    pub scenarios: Vec<EvaluationScenario>,
}

impl EvaluationManifest {
    /// Validates pairing, isolation, version identity, and bounded inputs.
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == EVALUATION_MANIFEST_SCHEMA_VERSION,
            "unsupported evaluation manifest schema {}",
            self.schema_version
        );
        ensure_id(&self.manifest_id, "manifest_id")?;
        ensure!(
            !self.scenarios.is_empty(),
            "evaluation manifest has no scenarios"
        );
        ensure!(
            self.scenarios.len() <= MAX_SCENARIOS,
            "evaluation manifest contains too many scenarios"
        );
        ensure!(
            self.scenarios
                .iter()
                .any(|scenario| scenario.split == EvaluationSplit::HeldOut),
            "evaluation manifest must contain at least one held-out scenario"
        );

        let mut scenario_ids = BTreeSet::new();
        for scenario in &self.scenarios {
            ensure!(
                scenario_ids.insert(scenario.scenario_id.clone()),
                "duplicate evaluation scenario {}",
                scenario.scenario_id
            );
            validate_scenario(scenario)?;
        }
        Ok(())
    }
}

fn validate_scenario(scenario: &EvaluationScenario) -> Result<()> {
    ensure_id(&scenario.scenario_id, "scenario_id")?;
    ensure_id(&scenario.task.task_id, "task.task_id")?;
    ensure_fingerprint(&scenario.task.fingerprint, "task.fingerprint")?;
    ensure!(
        scenario
            .task
            .prompt
            .as_ref()
            .is_some_and(|prompt| !prompt.trim().is_empty())
            ^ scenario
                .task
                .reference
                .as_ref()
                .is_some_and(|reference| !reference.trim().is_empty()),
        "task must contain exactly one non-empty prompt or reference"
    );
    ensure_fingerprint(
        &scenario.repository.snapshot_fingerprint,
        "repository.snapshot_fingerprint",
    )?;
    validate_condition(&scenario.baseline, "baseline")?;
    validate_condition(&scenario.treatment, "treatment")?;
    for (label, condition) in [
        ("baseline", &scenario.baseline),
        ("treatment", &scenario.treatment),
    ] {
        if let Some(telemetry) = &condition.intervention_telemetry {
            ensure!(
                telemetry.context.task_id == scenario.task.task_id,
                "{label}.intervention_telemetry task ID does not match scenario task"
            );
        }
    }

    ensure!(
        !scenario.baseline.memory.enabled,
        "baseline memory must be disabled"
    );
    ensure!(
        scenario.treatment.memory.enabled,
        "treatment memory must be enabled"
    );
    ensure!(
        scenario.baseline.execution_config_fingerprint
            == scenario.treatment.execution_config_fingerprint,
        "baseline and treatment execution config fingerprints differ"
    );
    ensure!(
        scenario.baseline.policy_fingerprint == scenario.treatment.policy_fingerprint,
        "baseline and treatment policy fingerprints differ"
    );
    ensure!(
        scenario.baseline.worktree_fingerprint == scenario.treatment.worktree_fingerprint,
        "baseline and treatment worktree fingerprints differ"
    );
    ensure!(
        scenario.baseline.memory.retrieval_policy_fingerprint
            == scenario.treatment.memory.retrieval_policy_fingerprint,
        "baseline and treatment retrieval policy fingerprints differ"
    );
    ensure!(
        scenario.baseline.memory.card_schema_version
            == scenario.treatment.memory.card_schema_version,
        "baseline and treatment card schema versions differ"
    );

    let baseline_worktree = path_identity(&scenario.baseline.worktree);
    let treatment_worktree = path_identity(&scenario.treatment.worktree);
    ensure!(
        !paths_overlap(&baseline_worktree, &treatment_worktree),
        "baseline and treatment worktrees overlap"
    );
    let baseline_trace = path_identity(&scenario.baseline.trace_bundle);
    let treatment_trace = path_identity(&scenario.treatment.trace_bundle);
    ensure!(
        !paths_overlap(&baseline_trace, &treatment_trace),
        "baseline and treatment trace bundles overlap"
    );
    if let Some(primary) = &scenario.repository.primary_checkout {
        let primary = path_identity(primary);
        ensure!(
            !paths_overlap(&primary, &baseline_worktree)
                && !paths_overlap(&primary, &treatment_worktree),
            "evaluation worktree overlaps the primary checkout"
        );
    }

    validate_evaluator(&scenario.evaluator)?;
    validate_budget(&scenario.budget)?;
    if let Some(evidence_id) = &scenario.attribution.evidence_id {
        ensure_id(evidence_id, "attribution.evidence_id")?;
    }
    Ok(())
}

fn validate_condition(condition: &EvaluationCondition, label: &str) -> Result<()> {
    ensure!(
        !condition.trace_bundle.as_os_str().is_empty(),
        "{label} trace bundle is empty"
    );
    ensure!(
        !condition.worktree.as_os_str().is_empty(),
        "{label} worktree is empty"
    );
    ensure_fingerprint(
        &condition.worktree_fingerprint,
        &format!("{label}.worktree_fingerprint"),
    )?;
    ensure_fingerprint(
        &condition.execution_config_fingerprint,
        &format!("{label}.execution_config_fingerprint"),
    )?;
    ensure_fingerprint(
        &condition.policy_fingerprint,
        &format!("{label}.policy_fingerprint"),
    )?;
    ensure_fingerprint(
        &condition.memory.card_set_fingerprint,
        &format!("{label}.memory.card_set_fingerprint"),
    )?;
    ensure_fingerprint(
        &condition.memory.retrieval_policy_fingerprint,
        &format!("{label}.memory.retrieval_policy_fingerprint"),
    )?;
    ensure!(
        condition.retrieval.selected_card_ids.len() <= MAX_CHECKS * 4,
        "{label}.retrieval contains too many selected cards"
    );
    let mut selected_card_ids = BTreeSet::new();
    for card_id in &condition.retrieval.selected_card_ids {
        ensure_id(card_id, &format!("{label}.retrieval.card_id"))?;
        ensure!(
            selected_card_ids.insert(card_id),
            "{label}.retrieval contains duplicate card IDs"
        );
    }
    ensure!(
        condition.retrieval.injection_count <= 1024,
        "{label}.retrieval contains too many injections"
    );
    if let Some(telemetry) = &condition.intervention_telemetry {
        telemetry.validate()?;
        if telemetry.context.selected_card_ids.is_empty() {
            ensure!(
                condition.retrieval.selected_card_ids.len() <= MAX_INTERVENTION_SIGNATURES,
                "{label}.retrieval has too many selected cards to link to intervention telemetry"
            );
        }
    }
    if let Some(judge) = &condition.judge {
        ensure_id(&judge.label_id, &format!("{label}.judge.label_id"))?;
        if let Some(score) = judge.score_bps {
            ensure!(
                score <= 10_000,
                "{label} judge score exceeds 10000 basis points"
            );
        }
    }
    Ok(())
}

pub(crate) fn validate_evaluator(evaluator: &EvaluatorSpec) -> Result<()> {
    if matches!(
        evaluator.kind,
        EvaluatorKind::Verification | EvaluatorKind::PostRunVerification
    ) {
        ensure!(
            !evaluator.checks.is_empty(),
            "verification evaluator has no checks"
        );
    }
    ensure!(
        evaluator.checks.len() <= MAX_CHECKS,
        "verification evaluator contains too many checks"
    );
    let mut check_ids = BTreeSet::new();
    for check in &evaluator.checks {
        ensure_id(&check.check_id, "evaluator.check_id")?;
        ensure!(
            check_ids.insert(check.check_id.clone()),
            "duplicate evaluator check {}",
            check.check_id
        );
        validate_command_tokens(&check.command_tokens, "evaluator check")?;
        if let Some(tokens) = &check.preflight_command_tokens {
            validate_command_tokens(tokens, "evaluator preflight check")?;
        }
    }
    Ok(())
}

fn validate_command_tokens(tokens: &[String], label: &str) -> Result<()> {
    ensure!(!tokens.is_empty(), "{label} has no command tokens");
    ensure!(
        tokens.len() <= 32,
        "{label} contains too many command tokens"
    );
    for token in tokens {
        ensure!(
            !token.trim().is_empty() && token.chars().count() <= MAX_COMMAND_TOKEN_CHARS,
            "{label} command token is empty or too long"
        );
    }
    Ok(())
}

pub(crate) fn validate_budget(budget: &EvaluationBudget) -> Result<()> {
    ensure!(
        budget.max_trace_bytes > 0,
        "max_trace_bytes must be positive"
    );
    ensure!(budget.max_tokens > 0, "max_tokens must be positive");
    ensure!(
        budget.max_wall_time_ms > 0,
        "max_wall_time_ms must be positive"
    );
    ensure!(budget.max_tool_calls > 0, "max_tool_calls must be positive");
    ensure!(
        budget.max_terminal_operations > 0,
        "max_terminal_operations must be positive"
    );
    ensure!(
        budget.max_post_run_time_ms > 0,
        "max_post_run_time_ms must be positive"
    );
    Ok(())
}

fn ensure_id(value: &str, label: &str) -> Result<()> {
    ensure!(
        !value.trim().is_empty() && value.chars().count() <= MAX_ID_CHARS,
        "{label} is empty or too long"
    );
    Ok(())
}

fn ensure_fingerprint(value: &str, label: &str) -> Result<()> {
    ensure!(
        !value.trim().is_empty() && value.chars().count() <= MAX_FINGERPRINT_CHARS,
        "{label} is empty or too long"
    );
    Ok(())
}

/// Load and validate a local scenario manifest.
pub fn load_evaluation_manifest(path: impl AsRef<Path>) -> Result<EvaluationManifest> {
    let path = path.as_ref();
    let manifest: EvaluationManifest = serde_json::from_reader(
        fs::File::open(path)
            .with_context(|| format!("open evaluation manifest {}", path.display()))?,
    )
    .with_context(|| format!("parse evaluation manifest {}", path.display()))?;
    manifest.validate()?;
    Ok(manifest)
}

/// Evaluate a manifest using the current wall clock for result provenance.
pub fn evaluate_manifest(manifest: &EvaluationManifest) -> Result<EvaluationResult> {
    evaluate_manifest_at(manifest, unix_time_ms())
}

/// Deterministic evaluation entry point with an injected timestamp for tests.
pub fn evaluate_manifest_at(
    manifest: &EvaluationManifest,
    evaluated_at_unix_ms: i64,
) -> Result<EvaluationResult> {
    manifest.validate()?;
    let scenarios = manifest
        .scenarios
        .iter()
        .map(evaluate_scenario)
        .collect::<Vec<_>>();
    Ok(EvaluationResult::from_scenarios(
        manifest,
        evaluated_at_unix_ms,
        scenarios,
    ))
}

/// Populate the private event chain after evaluation has produced the
/// condition outcomes. Raw event evidence remains in the private manifest;
/// callers must persist that manifest separately from the aggregate result.
pub fn materialize_intervention_telemetry(
    manifest: &mut EvaluationManifest,
    result: &EvaluationResult,
) -> Result<()> {
    ensure!(
        manifest.scenarios.len() == result.scenarios.len(),
        "evaluation result scenario count does not match manifest"
    );
    for (scenario, evaluated) in manifest.scenarios.iter_mut().zip(&result.scenarios) {
        ensure!(
            scenario.scenario_id == evaluated.scenario_id,
            "evaluation result scenario order does not match manifest"
        );
        for (condition, condition_result) in [
            (&mut scenario.baseline, &evaluated.baseline),
            (&mut scenario.treatment, &evaluated.treatment),
        ] {
            let Some(existing) = condition.intervention_telemetry.as_ref() else {
                continue;
            };
            if !existing.events.is_empty() {
                continue;
            }
            let trace = replay_bundle(&condition.trace_bundle).ok();
            let Some(derived) = intervention_evidence_for_condition(
                condition,
                trace.as_ref(),
                condition.post_run.as_ref(),
                condition_result.evaluator.task_success,
            ) else {
                continue;
            };
            if let Some(evidence) = &mut condition.intervention_telemetry {
                evidence.events = derived.events;
            }
        }
    }
    manifest.validate()
}

/// Persist aggregate results without exposing prompts, local paths, command
/// tokens, or judge evidence references.
pub fn write_evaluation_result(
    manifest: &EvaluationManifest,
    result: &EvaluationResult,
    output: impl AsRef<Path>,
) -> Result<()> {
    ensure!(
        result.manifest_id == manifest.manifest_id,
        "evaluation result belongs to a different manifest"
    );
    let output = output.as_ref();
    let output_identity = path_identity(output);
    for protected in protected_paths(manifest) {
        ensure!(
            !paths_overlap(&output_identity, &protected),
            "evaluation output overlaps a protected checkout or card store"
        );
    }
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create evaluation result directory {}", parent.display()))?;
    }
    let bytes = serde_json::to_vec_pretty(result)?;
    fs::write(output, bytes)
        .with_context(|| format!("write evaluation result {}", output.display()))?;
    Ok(())
}

fn protected_paths(manifest: &EvaluationManifest) -> Vec<PathBuf> {
    manifest
        .scenarios
        .iter()
        .flat_map(|scenario| {
            scenario
                .repository
                .primary_checkout
                .iter()
                .chain(std::iter::once(&scenario.baseline.worktree))
                .chain(std::iter::once(&scenario.treatment.worktree))
                .chain(std::iter::once(&scenario.baseline.trace_bundle))
                .chain(std::iter::once(&scenario.treatment.trace_bundle))
                .chain(scenario.baseline.memory.card_store_path.iter())
                .chain(scenario.treatment.memory.card_store_path.iter())
                .map(|path| path_identity(path))
        })
        .collect()
}

fn evaluate_scenario(scenario: &EvaluationScenario) -> EvaluationScenarioResult {
    let baseline = EvaluationConditionResult::from_condition(
        &scenario.baseline,
        evaluate_condition(scenario, &scenario.baseline),
    );
    let treatment = EvaluationConditionResult::from_condition(
        &scenario.treatment,
        evaluate_condition(scenario, &scenario.treatment),
    );
    let delta = MetricDelta::between(&baseline, &treatment);
    let attribution = assess_attribution(
        &scenario.attribution,
        baseline.evaluator.task_success,
        treatment.evaluator.task_success,
    );
    EvaluationScenarioResult {
        scenario_id: scenario.scenario_id.clone(),
        task_family: scenario.task_family,
        split: scenario.split,
        task_fingerprint: scenario.task.fingerprint.clone(),
        repository_snapshot_fingerprint: scenario.repository.snapshot_fingerprint.clone(),
        evaluator: scenario.evaluator.kind,
        evaluator_version: scenario.evaluator.version(),
        evaluator_fingerprint: scenario.evaluator.fingerprint(),
        baseline,
        treatment,
        delta,
        attribution,
    }
}

fn evaluate_condition(
    scenario: &EvaluationScenario,
    condition: &EvaluationCondition,
) -> EvaluatedCondition {
    let trace_size = match bundle_size(&condition.trace_bundle) {
        Ok(size) => size,
        Err(_) => {
            let evaluator = evaluate_with_spec(
                None,
                None,
                &scenario.evaluator,
                condition.judge.as_ref(),
                condition.post_run.as_ref(),
            );
            return EvaluatedCondition {
                trace_id: None,
                metrics: None,
                status: EvaluationStatus::TraceUnavailable,
                intervention: intervention_telemetry_for_condition(
                    condition,
                    None,
                    condition.post_run.as_ref(),
                    evaluator.task_success,
                ),
                evaluator,
                error_code: Some(EvaluationErrorCode::TraceUnavailable),
            };
        }
    };
    if trace_size > scenario.budget.max_trace_bytes {
        let evaluator = if scenario.evaluator.kind == EvaluatorKind::PostRunVerification {
            evaluate_with_spec(
                None,
                None,
                &scenario.evaluator,
                condition.judge.as_ref(),
                condition.post_run.as_ref(),
            )
        } else {
            EvaluatorResult::unavailable()
        };
        let outcome = evaluator.task_success;
        return EvaluatedCondition {
            trace_id: None,
            metrics: None,
            status: EvaluationStatus::CostLimited,
            evaluator,
            intervention: intervention_telemetry_for_condition(
                condition,
                None,
                condition.post_run.as_ref(),
                outcome,
            ),
            error_code: Some(EvaluationErrorCode::TraceTooLarge),
        };
    }

    let trace = match replay_bundle(&condition.trace_bundle) {
        Ok(trace) => trace,
        Err(_) => {
            let evaluator = evaluate_with_spec(
                None,
                None,
                &scenario.evaluator,
                condition.judge.as_ref(),
                condition.post_run.as_ref(),
            );
            return EvaluatedCondition {
                trace_id: None,
                metrics: None,
                status: EvaluationStatus::TraceUnavailable,
                intervention: intervention_telemetry_for_condition(
                    condition,
                    None,
                    condition.post_run.as_ref(),
                    evaluator.task_success,
                ),
                evaluator,
                error_code: Some(EvaluationErrorCode::TraceReplayFailed),
            };
        }
    };
    let metrics = EvaluationMetrics::from_trace(&trace);
    let total_tokens = metrics
        .token_usage
        .input_tokens
        .saturating_add(metrics.token_usage.output_tokens)
        .saturating_add(metrics.token_usage.reasoning_output_tokens);
    if total_tokens > scenario.budget.max_tokens
        || metrics.wall_time_ms.unwrap_or_default() > scenario.budget.max_wall_time_ms
        || metrics.tool_calls > scenario.budget.max_tool_calls
        || metrics.terminal_operations > scenario.budget.max_terminal_operations
    {
        let evaluator = if scenario.evaluator.kind == EvaluatorKind::PostRunVerification {
            evaluate_with_spec(
                Some(&trace),
                Some(&metrics),
                &scenario.evaluator,
                condition.judge.as_ref(),
                condition.post_run.as_ref(),
            )
        } else {
            EvaluatorResult::unavailable()
        };
        let outcome = evaluator.task_success;
        let intervention = intervention_telemetry_for_condition(
            condition,
            Some(&trace),
            condition.post_run.as_ref(),
            outcome,
        );
        let trace_id = trace.trace_id.clone();
        return EvaluatedCondition::with_metrics(
            trace_id,
            metrics,
            EvaluationStatus::CostLimited,
            evaluator,
            intervention,
            Some(EvaluationErrorCode::CostLimitExceeded),
        );
    }

    let evaluator = evaluate_with_spec(
        Some(&trace),
        Some(&metrics),
        &scenario.evaluator,
        condition.judge.as_ref(),
        condition.post_run.as_ref(),
    );
    let (status, error_code) = match evaluator.status {
        EvaluatorStatus::Passed | EvaluatorStatus::Failed => (EvaluationStatus::Evaluated, None),
        EvaluatorStatus::Unavailable => (
            EvaluationStatus::EvaluatorUnavailable,
            Some(EvaluationErrorCode::MissingEvaluatorEvidence),
        ),
        EvaluatorStatus::Error => (
            EvaluationStatus::EvaluatorFailed,
            Some(EvaluationErrorCode::EvaluatorFailed),
        ),
    };
    let intervention = intervention_telemetry_for_condition(
        condition,
        Some(&trace),
        condition.post_run.as_ref(),
        evaluator.task_success,
    );
    EvaluatedCondition::with_metrics(
        trace.trace_id,
        metrics,
        status,
        evaluator,
        intervention,
        error_code,
    )
}

fn intervention_telemetry_for_condition(
    condition: &EvaluationCondition,
    trace: Option<&RolloutTrace>,
    post_run: Option<&PostRunEvidence>,
    outcome: Option<bool>,
) -> InterventionTelemetryResult {
    let Some(evidence) = intervention_evidence_for_condition(condition, trace, post_run, outcome)
    else {
        return InterventionTelemetryResult::without_intervention(outcome);
    };
    evidence.to_result()
}

fn intervention_evidence_for_condition(
    condition: &EvaluationCondition,
    trace: Option<&RolloutTrace>,
    post_run: Option<&PostRunEvidence>,
    outcome: Option<bool>,
) -> Option<InterventionTelemetryEvidence> {
    let evidence = condition.intervention_telemetry.as_ref()?;
    if !evidence.events.is_empty() {
        return Some(evidence.clone());
    }
    let selected_card_count = condition
        .retrieval
        .selected_card_ids
        .len()
        .max(evidence.context.selected_card_ids.len());
    let injection_count = condition
        .retrieval
        .injection_count
        .max(condition.retrieval.exposure_count)
        .max(u64::from(evidence.context.injection_count));
    let mut context = evidence.context.clone();
    if context.selected_card_ids.is_empty()
        && condition.retrieval.selected_card_ids.len() <= MAX_INTERVENTION_SIGNATURES
    {
        context.selected_card_ids = condition.retrieval.selected_card_ids.clone();
    }
    if context.injection_count == 0 {
        context.injection_count = u32::try_from(injection_count).unwrap_or(u32::MAX);
    }
    Some(derive_intervention_telemetry(
        &context,
        selected_card_count,
        injection_count,
        trace,
        independent_verification_passed(&context, post_run),
        telemetry_outcome(trace, outcome),
    ))
}

fn independent_verification_passed(
    context: &InterventionTelemetryContext,
    post_run: Option<&PostRunEvidence>,
) -> Option<bool> {
    let post_run = post_run?;
    let checks = if context.independent_verification_check_ids.is_empty() {
        post_run.checks.iter().collect::<Vec<_>>()
    } else {
        let checks = context
            .independent_verification_check_ids
            .iter()
            .filter_map(|check_id| {
                post_run
                    .checks
                    .iter()
                    .find(|check| &check.check_id == check_id)
            })
            .collect::<Vec<_>>();
        if checks.len() != context.independent_verification_check_ids.len() {
            return None;
        }
        checks
    };
    (!checks.is_empty()).then(|| checks.iter().all(|check| check.success))
}

fn telemetry_outcome(trace: Option<&RolloutTrace>, outcome: Option<bool>) -> Option<bool> {
    if trace.is_some_and(|trace| {
        matches!(
            trace.status,
            RolloutStatus::Aborted | RolloutStatus::Running
        )
    }) {
        None
    } else {
        outcome
    }
}

fn evaluate_with_spec(
    trace: Option<&RolloutTrace>,
    metrics: Option<&EvaluationMetrics>,
    evaluator: &EvaluatorSpec,
    judge: Option<&JudgeLabel>,
    post_run: Option<&PostRunEvidence>,
) -> EvaluatorResult {
    match evaluator.kind {
        EvaluatorKind::TraceOutcome => {
            let Some(metrics) = metrics else {
                return EvaluatorResult::unavailable();
            };
            let task_success = Some(metrics.trace_status == TraceStatus::Completed);
            let verification_success = (metrics.verification_attempts > 0)
                .then_some(metrics.failed_verification_attempts == 0);
            EvaluatorResult {
                status: if task_success == Some(true) {
                    EvaluatorStatus::Passed
                } else {
                    EvaluatorStatus::Failed
                },
                task_success,
                verification_success,
                checks: Vec::new(),
                judge_score_bps: None,
            }
        }
        EvaluatorKind::Verification => {
            let Some(trace) = trace else {
                return EvaluatorResult::unavailable();
            };
            let checks = evaluator
                .checks
                .iter()
                .map(|check| evaluate_check(trace, check))
                .collect::<Vec<_>>();
            let task_success = if evaluator.require_all_checks {
                checks.iter().all(|check| check.success)
            } else {
                checks.iter().any(|check| check.success)
            };
            EvaluatorResult {
                status: if task_success {
                    EvaluatorStatus::Passed
                } else {
                    EvaluatorStatus::Failed
                },
                task_success: Some(task_success),
                verification_success: Some(task_success),
                checks,
                judge_score_bps: None,
            }
        }
        EvaluatorKind::PostRunVerification => {
            let Some(post_run) = post_run else {
                return EvaluatorResult::unavailable();
            };
            let Some(checks) = evaluator
                .checks
                .iter()
                .map(|expected| {
                    post_run
                        .checks
                        .iter()
                        .find(|observed| observed.check_id == expected.check_id)
                        .cloned()
                })
                .collect::<Option<Vec<_>>>()
            else {
                return EvaluatorResult::error();
            };
            let task_success = if evaluator.require_all_checks {
                checks.iter().all(|check| check.success)
            } else {
                checks.iter().any(|check| check.success)
            };
            EvaluatorResult {
                status: if task_success {
                    EvaluatorStatus::Passed
                } else {
                    EvaluatorStatus::Failed
                },
                task_success: Some(task_success),
                verification_success: Some(task_success),
                checks,
                judge_score_bps: None,
            }
        }
        EvaluatorKind::HumanJudge | EvaluatorKind::ModelJudge => {
            let Some(judge) = judge else {
                return EvaluatorResult::unavailable();
            };
            let Some(task_success) = judge.task_success else {
                return EvaluatorResult {
                    status: EvaluatorStatus::Unavailable,
                    task_success: None,
                    verification_success: judge.verification_success,
                    checks: Vec::new(),
                    judge_score_bps: judge.score_bps,
                };
            };
            EvaluatorResult {
                status: if task_success {
                    EvaluatorStatus::Passed
                } else {
                    EvaluatorStatus::Failed
                },
                task_success: Some(task_success),
                verification_success: judge.verification_success,
                checks: Vec::new(),
                judge_score_bps: judge.score_bps,
            }
        }
    }
}

fn evaluate_check(trace: &RolloutTrace, check: &VerificationCheck) -> CheckResult {
    let expected_tokens = check
        .command_tokens
        .iter()
        .map(|token| token.to_ascii_lowercase())
        .collect::<Vec<_>>();
    let success = trace.terminal_operations.values().any(|operation| {
        let TerminalRequest::ExecCommand {
            display_command, ..
        } = &operation.request
        else {
            return false;
        };
        let command = display_command.to_ascii_lowercase();
        let tokens_match = expected_tokens.iter().all(|token| command.contains(token));
        tokens_match
            && operation
                .result
                .as_ref()
                .and_then(|result| result.exit_code)
                == Some(check.expected_exit_code)
    });
    CheckResult {
        check_id: check.check_id.clone(),
        success,
    }
}

#[derive(Debug, Clone)]
struct EvaluatedCondition {
    trace_id: Option<String>,
    metrics: Option<EvaluationMetrics>,
    status: EvaluationStatus,
    evaluator: EvaluatorResult,
    intervention: InterventionTelemetryResult,
    error_code: Option<EvaluationErrorCode>,
}

impl EvaluatedCondition {
    fn with_metrics(
        trace_id: String,
        metrics: EvaluationMetrics,
        status: EvaluationStatus,
        evaluator: EvaluatorResult,
        intervention: InterventionTelemetryResult,
        error_code: Option<EvaluationErrorCode>,
    ) -> Self {
        Self {
            trace_id: Some(trace_id),
            metrics: Some(metrics),
            status,
            evaluator,
            intervention,
            error_code,
        }
    }
}

/// Result status for one recorded condition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvaluationStatus {
    Evaluated,
    CostLimited,
    TraceUnavailable,
    EvaluatorUnavailable,
    EvaluatorFailed,
}

/// Stable error category without leaking local paths or evaluator text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvaluationErrorCode {
    TraceUnavailable,
    TraceTooLarge,
    TraceReplayFailed,
    CostLimitExceeded,
    MissingEvaluatorEvidence,
    EvaluatorFailed,
}

/// Bounded runtime metrics derived from a reduced rollout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvaluationMetrics {
    pub trace_status: TraceStatus,
    pub policy_status: PolicyStatus,
    pub failure_class: FailureClass,
    pub failure_signature: FailureSignature,
    pub tool_calls: u32,
    pub terminal_operations: u32,
    pub retry_count: u32,
    pub repeated_failure_count: u32,
    pub verification_attempts: u32,
    pub failed_verification_attempts: u32,
    pub incomplete_verification: bool,
    pub token_usage: TokenUsageSummary,
    pub wall_time_ms: Option<u64>,
    pub cancelled: bool,
    pub policy_event_count: u32,
    pub user_correction_count: Option<u32>,
}

impl EvaluationMetrics {
    fn from_trace(trace: &RolloutTrace) -> Self {
        let card = summarize_rollout(trace);
        let evidence = &card.evidence;
        let failed_operations = evidence
            .failed_terminal_operation_count
            .saturating_add(evidence.failed_tool_count)
            .saturating_add(evidence.failed_inference_count);
        let retry_count = failed_operations.saturating_sub(1);
        let repeated_failure_count = evidence.failed_terminal_operation_count.saturating_sub(1);
        let wall_time_ms = trace.ended_at_unix_ms.and_then(|ended| {
            ended
                .checked_sub(trace.started_at_unix_ms)
                .and_then(|value| u64::try_from(value).ok())
        });
        let policy_event_count = policy_event_count(trace);
        Self {
            trace_status: match trace.status {
                RolloutStatus::Completed => TraceStatus::Completed,
                RolloutStatus::Failed => TraceStatus::Failed,
                RolloutStatus::Aborted => TraceStatus::Aborted,
                RolloutStatus::Running => TraceStatus::Running,
            },
            policy_status: if policy_event_count > 0 {
                PolicyStatus::BoundaryObserved
            } else {
                PolicyStatus::Clean
            },
            failure_class: card.failure_class,
            failure_signature: card.failure_signature,
            tool_calls: bounded_u32(trace.tool_calls.len()),
            terminal_operations: bounded_u32(trace.terminal_operations.len()),
            retry_count,
            repeated_failure_count,
            verification_attempts: evidence.verification_attempt_count,
            failed_verification_attempts: evidence.failed_verification_count,
            incomplete_verification: card.failure_class == FailureClass::IncompleteVerification,
            token_usage: evidence.token_usage.clone(),
            wall_time_ms,
            cancelled: card.failure_class == FailureClass::Cancelled
                || trace.status == RolloutStatus::Aborted,
            policy_event_count,
            user_correction_count: root_user_correction_count(trace),
        }
    }
}

/// Result of one deterministic verification check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckResult {
    pub check_id: String,
    pub success: bool,
}

/// Evaluator outcome. Missing judge labels remain unavailable rather than
/// being silently interpreted as failure or success.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvaluatorResult {
    pub status: EvaluatorStatus,
    pub task_success: Option<bool>,
    pub verification_success: Option<bool>,
    pub checks: Vec<CheckResult>,
    pub judge_score_bps: Option<u16>,
}

impl EvaluatorResult {
    fn unavailable() -> Self {
        Self {
            status: EvaluatorStatus::Unavailable,
            task_success: None,
            verification_success: None,
            checks: Vec::new(),
            judge_score_bps: None,
        }
    }

    fn error() -> Self {
        Self {
            status: EvaluatorStatus::Error,
            task_success: None,
            verification_success: None,
            checks: Vec::new(),
            judge_score_bps: None,
        }
    }
}

/// Evaluator status independent from trace availability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvaluatorStatus {
    Passed,
    Failed,
    Unavailable,
    Error,
}

/// Completion state of the recorded child rollout. This is independent from
/// the coding/task evaluator result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TraceStatus {
    Completed,
    Failed,
    Aborted,
    Running,
}

/// Whether the trace observed a permission or sandbox boundary event. A
/// boundary is diagnostic metadata and does not by itself make the task fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyStatus {
    Clean,
    BoundaryObserved,
}

/// Provenance retained in a result without local paths or raw card IDs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvaluationConditionProvenance {
    pub execution_config_fingerprint: String,
    pub policy_fingerprint: String,
    pub worktree_fingerprint: String,
    pub memory: EvaluationMemoryProvenance,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvaluationMemoryProvenance {
    pub enabled: bool,
    pub card_set_fingerprint: String,
    pub retrieval_policy_fingerprint: String,
    pub card_schema_version: u32,
}

/// Public result for one condition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvaluationConditionResult {
    pub provenance: EvaluationConditionProvenance,
    #[serde(default)]
    pub retrieval: EvaluationRetrievalResult,
    #[serde(default)]
    pub intervention: InterventionTelemetryResult,
    pub trace_id: Option<String>,
    pub status: EvaluationStatus,
    pub error_code: Option<EvaluationErrorCode>,
    pub metrics: Option<EvaluationMetrics>,
    pub evaluator: EvaluatorResult,
}

impl EvaluationConditionResult {
    fn from_condition(condition: &EvaluationCondition, evaluated: EvaluatedCondition) -> Self {
        let mut selected_card_ids = condition.retrieval.selected_card_ids.clone();
        selected_card_ids.sort();
        Self {
            provenance: EvaluationConditionProvenance {
                execution_config_fingerprint: condition.execution_config_fingerprint.clone(),
                policy_fingerprint: condition.policy_fingerprint.clone(),
                worktree_fingerprint: condition.worktree_fingerprint.clone(),
                memory: EvaluationMemoryProvenance {
                    enabled: condition.memory.enabled,
                    card_set_fingerprint: condition.memory.card_set_fingerprint.clone(),
                    retrieval_policy_fingerprint: condition
                        .memory
                        .retrieval_policy_fingerprint
                        .clone(),
                    card_schema_version: condition.memory.card_schema_version,
                },
            },
            retrieval: EvaluationRetrievalResult {
                selected_card_count: condition
                    .retrieval
                    .selected_card_ids
                    .len()
                    .min(u32::MAX as usize) as u32,
                exposure_count: condition.retrieval.exposure_count,
                selection_fingerprint: (!selected_card_ids.is_empty()).then(|| {
                    crate::fingerprint_descriptor(
                        "retrieval-selection-v1",
                        &selected_card_ids.join("\0"),
                    )
                }),
            },
            intervention: evaluated.intervention,
            trace_id: evaluated.trace_id,
            status: evaluated.status,
            error_code: evaluated.error_code,
            metrics: evaluated.metrics,
            evaluator: evaluated.evaluator,
        }
    }
}

/// Signed treatment-minus-baseline changes. `None` means one side did not
/// produce the metric, so it must not be treated as zero.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetricDelta {
    pub task_success: Option<i8>,
    pub verification_success: Option<i8>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub reasoning_output_tokens: Option<i64>,
    pub wall_time_ms: Option<i64>,
    pub tool_calls: Option<i64>,
    pub terminal_operations: Option<i64>,
    pub retry_count: Option<i64>,
    pub repeated_failure_count: Option<i64>,
    pub policy_event_count: Option<i64>,
}

impl MetricDelta {
    fn between(
        baseline: &EvaluationConditionResult,
        treatment: &EvaluationConditionResult,
    ) -> Self {
        Self {
            task_success: bool_delta(
                baseline.evaluator.task_success,
                treatment.evaluator.task_success,
            ),
            verification_success: bool_delta(
                baseline.evaluator.verification_success,
                treatment.evaluator.verification_success,
            ),
            input_tokens: metric_delta(
                baseline
                    .metrics
                    .as_ref()
                    .map(|metrics| metrics.token_usage.input_tokens),
                treatment
                    .metrics
                    .as_ref()
                    .map(|metrics| metrics.token_usage.input_tokens),
            ),
            output_tokens: metric_delta(
                baseline
                    .metrics
                    .as_ref()
                    .map(|metrics| metrics.token_usage.output_tokens),
                treatment
                    .metrics
                    .as_ref()
                    .map(|metrics| metrics.token_usage.output_tokens),
            ),
            reasoning_output_tokens: metric_delta(
                baseline
                    .metrics
                    .as_ref()
                    .map(|metrics| metrics.token_usage.reasoning_output_tokens),
                treatment
                    .metrics
                    .as_ref()
                    .map(|metrics| metrics.token_usage.reasoning_output_tokens),
            ),
            wall_time_ms: metric_delta(
                baseline
                    .metrics
                    .as_ref()
                    .and_then(|metrics| metrics.wall_time_ms),
                treatment
                    .metrics
                    .as_ref()
                    .and_then(|metrics| metrics.wall_time_ms),
            ),
            tool_calls: metric_delta(
                baseline
                    .metrics
                    .as_ref()
                    .map(|metrics| u64::from(metrics.tool_calls)),
                treatment
                    .metrics
                    .as_ref()
                    .map(|metrics| u64::from(metrics.tool_calls)),
            ),
            terminal_operations: metric_delta(
                baseline
                    .metrics
                    .as_ref()
                    .map(|metrics| u64::from(metrics.terminal_operations)),
                treatment
                    .metrics
                    .as_ref()
                    .map(|metrics| u64::from(metrics.terminal_operations)),
            ),
            retry_count: metric_delta(
                baseline
                    .metrics
                    .as_ref()
                    .map(|metrics| u64::from(metrics.retry_count)),
                treatment
                    .metrics
                    .as_ref()
                    .map(|metrics| u64::from(metrics.retry_count)),
            ),
            repeated_failure_count: metric_delta(
                baseline
                    .metrics
                    .as_ref()
                    .map(|metrics| u64::from(metrics.repeated_failure_count)),
                treatment
                    .metrics
                    .as_ref()
                    .map(|metrics| u64::from(metrics.repeated_failure_count)),
            ),
            policy_event_count: metric_delta(
                baseline
                    .metrics
                    .as_ref()
                    .map(|metrics| u64::from(metrics.policy_event_count)),
                treatment
                    .metrics
                    .as_ref()
                    .map(|metrics| u64::from(metrics.policy_event_count)),
            ),
        }
    }
}

fn bool_delta(baseline: Option<bool>, treatment: Option<bool>) -> Option<i8> {
    let baseline = if baseline? { 1 } else { 0 };
    let treatment = if treatment? { 1 } else { 0 };
    Some(treatment - baseline)
}

fn metric_delta<T>(baseline: Option<T>, treatment: Option<T>) -> Option<i64>
where
    T: Into<i128> + Copy,
{
    let baseline = baseline?.into();
    let treatment = treatment?.into();
    i64::try_from(treatment - baseline).ok()
}

/// Conservative classification of how much can be said about card usage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttributionAssessment {
    NotExposed,
    ExposedNotFollowed,
    ExposedAndOutcomeImproved,
    ExposedAndOutcomeUnchanged,
    ImprovedWithoutMemoryEvidence,
    Inconclusive,
}

fn assess_attribution(
    evidence: &AttributionEvidence,
    baseline_success: Option<bool>,
    treatment_success: Option<bool>,
) -> AttributionAssessment {
    match evidence.exposure {
        AttributionState::No => {
            if baseline_success == Some(false) && treatment_success == Some(true) {
                AttributionAssessment::ImprovedWithoutMemoryEvidence
            } else {
                AttributionAssessment::NotExposed
            }
        }
        AttributionState::Yes => match evidence.followed {
            AttributionState::No => AttributionAssessment::ExposedNotFollowed,
            AttributionState::Yes => match (baseline_success, treatment_success) {
                (Some(false), Some(true)) => AttributionAssessment::ExposedAndOutcomeImproved,
                (Some(left), Some(right)) if left == right => {
                    AttributionAssessment::ExposedAndOutcomeUnchanged
                }
                _ => AttributionAssessment::Inconclusive,
            },
            AttributionState::Unknown | AttributionState::Mixed => {
                AttributionAssessment::Inconclusive
            }
        },
        AttributionState::Unknown | AttributionState::Mixed => {
            if baseline_success == Some(false) && treatment_success == Some(true) {
                AttributionAssessment::ImprovedWithoutMemoryEvidence
            } else {
                AttributionAssessment::Inconclusive
            }
        }
    }
}

/// Result for one paired scenario.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvaluationScenarioResult {
    pub scenario_id: String,
    pub task_family: TaskFamily,
    pub split: EvaluationSplit,
    pub task_fingerprint: String,
    pub repository_snapshot_fingerprint: String,
    pub evaluator: EvaluatorKind,
    #[serde(default)]
    pub evaluator_version: u32,
    #[serde(default)]
    pub evaluator_fingerprint: String,
    pub baseline: EvaluationConditionResult,
    pub treatment: EvaluationConditionResult,
    pub delta: MetricDelta,
    pub attribution: AttributionAssessment,
}

/// Aggregates for one split or the full manifest.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct EvaluationAggregate {
    pub scenario_count: u32,
    pub paired_count: u32,
    pub baseline_success_count: u32,
    pub treatment_success_count: u32,
    pub baseline_verification_success_count: u32,
    pub treatment_verification_success_count: u32,
    pub baseline_incomplete_verification_count: u32,
    pub treatment_incomplete_verification_count: u32,
    pub baseline_tokens: TokenUsageSummary,
    pub treatment_tokens: TokenUsageSummary,
    pub baseline_wall_time_ms: u64,
    pub treatment_wall_time_ms: u64,
    #[serde(default)]
    pub baseline_intervention: InterventionTelemetryAggregate,
    #[serde(default)]
    pub treatment_intervention: InterventionTelemetryAggregate,
}

impl EvaluationAggregate {
    fn add(&mut self, result: &EvaluationScenarioResult) {
        self.scenario_count = self.scenario_count.saturating_add(1);
        if result.baseline.metrics.is_some() && result.treatment.metrics.is_some() {
            self.paired_count = self.paired_count.saturating_add(1);
        }
        add_condition_aggregate(
            &mut self.baseline_success_count,
            &mut self.baseline_verification_success_count,
            &mut self.baseline_incomplete_verification_count,
            &mut self.baseline_tokens,
            &mut self.baseline_wall_time_ms,
            &result.baseline,
        );
        add_condition_aggregate(
            &mut self.treatment_success_count,
            &mut self.treatment_verification_success_count,
            &mut self.treatment_incomplete_verification_count,
            &mut self.treatment_tokens,
            &mut self.treatment_wall_time_ms,
            &result.treatment,
        );
        self.baseline_intervention
            .add(&result.baseline.intervention);
        self.treatment_intervention
            .add(&result.treatment.intervention);
    }
}

fn add_condition_aggregate(
    success_count: &mut u32,
    verification_success_count: &mut u32,
    incomplete_verification_count: &mut u32,
    tokens: &mut TokenUsageSummary,
    wall_time_ms: &mut u64,
    condition: &EvaluationConditionResult,
) {
    if condition.evaluator.task_success == Some(true) {
        *success_count = success_count.saturating_add(1);
    }
    if condition.evaluator.verification_success == Some(true) {
        *verification_success_count = verification_success_count.saturating_add(1);
    }
    if condition
        .metrics
        .as_ref()
        .is_some_and(|metrics| metrics.incomplete_verification)
    {
        *incomplete_verification_count = incomplete_verification_count.saturating_add(1);
    }
    if let Some(metrics) = &condition.metrics {
        tokens.input_tokens = tokens
            .input_tokens
            .saturating_add(metrics.token_usage.input_tokens);
        tokens.output_tokens = tokens
            .output_tokens
            .saturating_add(metrics.token_usage.output_tokens);
        tokens.reasoning_output_tokens = tokens
            .reasoning_output_tokens
            .saturating_add(metrics.token_usage.reasoning_output_tokens);
        *wall_time_ms = wall_time_ms.saturating_add(metrics.wall_time_ms.unwrap_or_default());
    }
}

/// Complete aggregate result. Scenario details retain exact opaque version
/// fingerprints while omitting private task, trace, and filesystem inputs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvaluationResult {
    pub schema_version: u32,
    pub manifest_id: String,
    pub evaluated_at_unix_ms: i64,
    pub scenario_count: u32,
    pub all: EvaluationAggregate,
    pub held_out: EvaluationAggregate,
    pub by_task_family: BTreeMap<String, EvaluationAggregate>,
    pub scenarios: Vec<EvaluationScenarioResult>,
}

impl EvaluationResult {
    fn from_scenarios(
        manifest: &EvaluationManifest,
        evaluated_at_unix_ms: i64,
        scenarios: Vec<EvaluationScenarioResult>,
    ) -> Self {
        let mut all = EvaluationAggregate::default();
        let mut held_out = EvaluationAggregate::default();
        let mut by_task_family = BTreeMap::<String, EvaluationAggregate>::new();
        for scenario in &scenarios {
            all.add(scenario);
            if scenario.split == EvaluationSplit::HeldOut {
                held_out.add(scenario);
            }
            by_task_family
                .entry(scenario.task_family.as_str().to_string())
                .or_default()
                .add(scenario);
        }
        Self {
            schema_version: EVALUATION_RESULT_SCHEMA_VERSION,
            manifest_id: manifest.manifest_id.clone(),
            evaluated_at_unix_ms,
            scenario_count: bounded_u32(scenarios.len()),
            all,
            held_out,
            by_task_family,
            scenarios,
        }
    }
}

fn bundle_size(path: &Path) -> Result<u64> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("read trace bundle metadata {}", path.display()))?;
    if metadata.file_type().is_symlink() {
        bail!("trace bundle cannot be a symlink");
    }
    if metadata.is_file() {
        return Ok(metadata.len());
    }
    ensure!(metadata.is_dir(), "trace bundle is not a directory");
    let mut total = 0_u64;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let size = bundle_size(&entry.path())?;
        total = total.saturating_add(size);
    }
    Ok(total)
}

fn policy_event_count(trace: &RolloutTrace) -> u32 {
    let terminal_events = trace
        .terminal_operations
        .values()
        .filter(|operation| {
            operation.result.as_ref().is_some_and(|result| {
                contains_policy_marker(&result.stdout)
                    || contains_policy_marker(&result.stderr)
                    || result
                        .formatted_output
                        .as_deref()
                        .is_some_and(contains_policy_marker)
            })
        })
        .count();
    let tool_events = trace
        .tool_calls
        .values()
        .filter(|tool| match &tool.summary {
            ToolCallSummary::Generic {
                input_preview,
                output_preview,
                ..
            } => {
                input_preview.as_deref().is_some_and(contains_policy_marker)
                    || output_preview
                        .as_deref()
                        .is_some_and(contains_policy_marker)
            }
            _ => false,
        })
        .count();
    bounded_u32(terminal_events.saturating_add(tool_events))
}

fn contains_policy_marker(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    [
        "approval required",
        "permission denied",
        "operation not permitted",
        "not allowed",
        "sandbox",
        "policy blocked",
        "approval denied",
        "rejected",
    ]
    .iter()
    .any(|marker| text.contains(marker))
}

fn root_user_correction_count(trace: &RolloutTrace) -> Option<u32> {
    let thread = trace.threads.get(&trace.root_thread_id)?;
    let user_messages = thread
        .conversation_item_ids
        .iter()
        .filter_map(|item_id| trace.conversation_items.get(item_id))
        .filter(|item| item.role == ConversationRole::User)
        .count();
    Some(bounded_u32(user_messages.saturating_sub(1)))
}

fn bounded_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

fn path_identity(path: &Path) -> PathBuf {
    if let Ok(canonical) = fs::canonicalize(path) {
        return canonical;
    }
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    let mut missing_components = Vec::new();
    let mut existing = absolute.as_path();
    while !existing.exists() {
        if let Some(name) = existing.file_name() {
            missing_components.push(name.to_os_string());
        }
        let Some(parent) = existing.parent() else {
            return absolute;
        };
        existing = parent;
    }
    let mut identity = fs::canonicalize(existing).unwrap_or_else(|_| existing.to_path_buf());
    for component in missing_components.iter().rev() {
        identity.push(component);
    }
    identity
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    left == right || left.starts_with(right) || right.starts_with(left)
}

fn unix_time_ms() -> i64 {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}

#[cfg(test)]
#[path = "eval_tests.rs"]
mod tests;
