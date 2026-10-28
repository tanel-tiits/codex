//! Private inputs and bounded fingerprints for live paired evaluation.
//!
//! The live runner itself belongs to the CLI because it must launch a separate
//! Codex process. This module owns the private plan contract and the small
//! pieces of deterministic identity shared by the runner and the offline
//! evaluator. Private plans may contain prompts and filesystem paths; they
//! must never be published as aggregate evaluation results.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::EvaluationBudget;
use crate::EvaluationResult;
use crate::EvaluationSplit;
use crate::EvaluationTask;
use crate::EvaluatorSpec;
use crate::EvolutionReport;
use crate::ExperienceCard;
use crate::ExperimentBrief;
use crate::TaskFamily;

/// Version of the private live-evaluation plan contract.
pub const LIVE_EVALUATION_PLAN_SCHEMA_VERSION: u32 = 1;

/// Environment variable used by an isolated child run to select a temporary
/// agent-improvement store instead of the user's normal store.
pub const AGENT_IMPROVEMENT_ROOT_ENV: &str = "CODEX_AGENT_IMPROVEMENT_ROOT";

/// Environment variable used by an isolated child run to preserve the source
/// workspace identity while its files live in a temporary worktree.
pub const AGENT_IMPROVEMENT_WORKSPACE_PATH_ENV: &str = "CODEX_AGENT_IMPROVEMENT_WORKSPACE_PATH";

const MAX_LIVE_SCENARIOS: usize = 64;
const MAX_LIVE_PLAN_ID_CHARS: usize = 128;
const MAX_LIVE_PROMPT_CHARS: usize = 32 * 1024;
const MAX_LIVE_WALL_TIME_MS: u64 = 2 * 60 * 60 * 1000;

/// Private task set used to run one experiment brief against held-out tasks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveEvaluationPlan {
    pub schema_version: u32,
    pub plan_id: String,
    pub brief_id: String,
    pub brief_fingerprint: String,
    /// The clean source checkout from which isolated evaluation worktrees are
    /// created. This path is private plan input and never enters results.
    pub primary_checkout: PathBuf,
    pub scenarios: Vec<LiveEvaluationScenario>,
}

/// One private held-out task to execute in both conditions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveEvaluationScenario {
    pub scenario_id: String,
    pub task_family: TaskFamily,
    pub split: EvaluationSplit,
    pub task: EvaluationTask,
    pub evaluator: EvaluatorSpec,
    #[serde(default)]
    pub budget: EvaluationBudget,
}

impl LiveEvaluationPlan {
    /// Validate all bounded private inputs before creating worktrees or
    /// launching a model process.
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == LIVE_EVALUATION_PLAN_SCHEMA_VERSION,
            "unsupported live evaluation plan schema {}",
            self.schema_version
        );
        validate_live_id(&self.plan_id, "plan_id")?;
        validate_live_id(&self.brief_id, "brief_id")?;
        validate_live_id(&self.brief_fingerprint, "brief_fingerprint")?;
        ensure!(
            !self.primary_checkout.as_os_str().is_empty(),
            "primary_checkout is empty"
        );
        ensure!(
            !self.scenarios.is_empty(),
            "live evaluation plan has no scenarios"
        );
        ensure!(
            self.scenarios.len() <= MAX_LIVE_SCENARIOS,
            "live evaluation plan contains too many scenarios"
        );

        let mut scenario_ids = std::collections::BTreeSet::new();
        for scenario in &self.scenarios {
            validate_live_path_component(&scenario.scenario_id, "scenario_id")?;
            ensure!(
                scenario_ids.insert(scenario.scenario_id.clone()),
                "duplicate live evaluation scenario {}",
                scenario.scenario_id
            );
            ensure!(
                scenario.split == EvaluationSplit::HeldOut,
                "live evaluation scenario {} is not held out",
                scenario.scenario_id
            );
            validate_live_task(&scenario.task)?;
            crate::eval::validate_evaluator(&scenario.evaluator)?;
            crate::eval::validate_budget(&scenario.budget)?;
            ensure!(
                scenario.budget.max_wall_time_ms <= MAX_LIVE_WALL_TIME_MS,
                "live evaluation scenario {} exceeds the two-hour wall-time limit",
                scenario.scenario_id
            );
        }
        Ok(())
    }

    /// Convert private live tasks into the existing evaluator's scenario
    /// shape after the runner has populated both condition paths.
    pub fn scenario_count(&self) -> usize {
        self.scenarios.len()
    }
}

/// Load and validate a private live-evaluation plan.
pub fn load_live_evaluation_plan(path: impl AsRef<Path>) -> Result<LiveEvaluationPlan> {
    let path = path.as_ref();
    let plan: LiveEvaluationPlan = serde_json::from_reader(
        fs::File::open(path)
            .with_context(|| format!("open live evaluation plan {}", path.display()))?,
    )
    .with_context(|| format!("parse live evaluation plan {}", path.display()))?;
    plan.validate()?;
    Ok(plan)
}

/// Load one brief either from a single `ExperimentBrief` file or from the
/// `EvolutionReport` emitted by `evolve run`.
pub fn load_experiment_brief(path: impl AsRef<Path>, brief_id: &str) -> Result<ExperimentBrief> {
    let path = path.as_ref();
    let value: serde_json::Value = serde_json::from_reader(
        fs::File::open(path)
            .with_context(|| format!("open experiment brief {}", path.display()))?,
    )
    .with_context(|| format!("parse experiment brief {}", path.display()))?;
    let brief = if value.get("briefs").is_some() {
        let report: EvolutionReport = serde_json::from_value(value)
            .with_context(|| format!("parse evolution report {}", path.display()))?;
        report
            .briefs
            .into_iter()
            .find(|brief| brief.brief_id == brief_id)
            .ok_or_else(|| anyhow::anyhow!("brief {brief_id} was not found in evolution report"))?
    } else {
        serde_json::from_value::<ExperimentBrief>(value)
            .with_context(|| format!("parse experiment brief {}", path.display()))?
    };
    ensure!(
        brief.brief_id == brief_id,
        "brief file contains {}, expected {}",
        brief.brief_id,
        brief_id
    );
    crate::evolve::validate_brief(&brief)?;
    Ok(brief)
}

/// Stable identity for a card set used by a live treatment condition.
pub fn fingerprint_card_set(cards: &[ExperienceCard]) -> String {
    let mut cards = cards.to_vec();
    cards.sort_by(|left, right| left.card_id.cmp(&right.card_id));
    fingerprint_bytes(
        "card-set-v1",
        &serde_json::to_vec(&cards).unwrap_or_default(),
    )
}

/// Stable identity for a redacted evaluation result.
pub fn fingerprint_evaluation_result(result: &EvaluationResult) -> Result<String> {
    Ok(fingerprint_bytes(
        "evaluation-result-v1",
        &serde_json::to_vec(result)?,
    ))
}

/// Stable identity for an execution or policy descriptor that must be equal
/// between paired conditions.
pub fn fingerprint_descriptor(namespace: &str, descriptor: &str) -> String {
    fingerprint_bytes(namespace, descriptor.as_bytes())
}

fn validate_live_task(task: &EvaluationTask) -> Result<()> {
    validate_live_id(&task.task_id, "task.task_id")?;
    validate_live_id(&task.fingerprint, "task.fingerprint")?;
    let prompt = task
        .prompt
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("live task {} has no prompt", task.task_id))?;
    ensure!(!prompt.trim().is_empty(), "live task prompt is empty");
    ensure!(
        prompt.chars().count() <= MAX_LIVE_PROMPT_CHARS,
        "live task prompt is too long"
    );
    ensure!(
        !prompt.chars().any(|character| character == '\0'),
        "live task prompt contains a NUL character"
    );
    ensure!(
        task.reference.is_none(),
        "live task {} cannot use a reference instead of a prompt",
        task.task_id
    );
    Ok(())
}

fn validate_live_id(value: &str, label: &str) -> Result<()> {
    ensure!(!value.trim().is_empty(), "{label} is empty");
    ensure!(
        value.chars().count() <= MAX_LIVE_PLAN_ID_CHARS,
        "{label} is too long"
    );
    ensure!(
        !value
            .chars()
            .any(|character| character.is_control() || matches!(character, '<' | '>')),
        "{label} contains unsafe characters"
    );
    Ok(())
}

fn validate_live_path_component(value: &str, label: &str) -> Result<()> {
    validate_live_id(value, label)?;
    ensure!(
        !value.contains(['/', '\\']) && value != "." && value != "..",
        "{label} must be a single safe path component"
    );
    Ok(())
}

fn fingerprint_bytes(namespace: &str, bytes: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(namespace.as_bytes());
    digest.update([0]);
    digest.update(bytes);
    format!(
        "sha256:{}",
        digest
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EvaluatorKind;
    use crate::ExperienceCard;

    fn plan() -> LiveEvaluationPlan {
        LiveEvaluationPlan {
            schema_version: LIVE_EVALUATION_PLAN_SCHEMA_VERSION,
            plan_id: "plan-1".to_string(),
            brief_id: "experiment-1".to_string(),
            brief_fingerprint: "fingerprint-1".to_string(),
            primary_checkout: PathBuf::from("/private/checkout"),
            scenarios: vec![LiveEvaluationScenario {
                scenario_id: "scenario-1".to_string(),
                task_family: TaskFamily::Coding,
                split: EvaluationSplit::HeldOut,
                task: EvaluationTask {
                    task_id: "task-1".to_string(),
                    prompt: Some("fix the bounded fixture".to_string()),
                    reference: None,
                    fingerprint: "task-fingerprint-1".to_string(),
                },
                evaluator: EvaluatorSpec {
                    kind: EvaluatorKind::TraceOutcome,
                    checks: Vec::new(),
                    require_all_checks: true,
                },
                budget: EvaluationBudget::default(),
            }],
        }
    }

    #[test]
    fn live_plan_requires_held_out_prompt_tasks() {
        let mut plan = plan();
        plan.validate().expect("valid plan");
        plan.scenarios[0].split = EvaluationSplit::Validation;
        assert!(plan.validate().is_err());
        plan.scenarios[0].split = EvaluationSplit::HeldOut;
        plan.scenarios[0].task.prompt = None;
        assert!(plan.validate().is_err());
    }

    #[test]
    fn live_plan_rejects_scenario_path_traversal_and_unbounded_wall_time() {
        let mut plan = plan();
        plan.scenarios[0].scenario_id = "../outside".to_string();
        assert!(plan.validate().is_err());

        plan.scenarios[0].scenario_id = "scenario-1".to_string();
        plan.scenarios[0].budget.max_wall_time_ms = MAX_LIVE_WALL_TIME_MS + 1;
        assert!(plan.validate().is_err());
    }

    #[test]
    fn fingerprints_are_stable_and_card_order_independent() {
        let first = ExperienceCard::new(
            "trace-a",
            100,
            TaskFamily::Coding,
            crate::FailureClass::CommandFailure,
            crate::AgentImprovementEvidence::default(),
            vec!["a".to_string()],
            "lesson a",
            vec!["intervention a".to_string()],
        );
        let second = ExperienceCard::new(
            "trace-b",
            101,
            TaskFamily::Testing,
            crate::FailureClass::IncompleteVerification,
            crate::AgentImprovementEvidence::default(),
            vec!["b".to_string()],
            "lesson b",
            vec!["intervention b".to_string()],
        );
        assert_eq!(
            fingerprint_card_set(&[first.clone(), second.clone()]),
            fingerprint_card_set(&[second, first])
        );
        assert_eq!(
            fingerprint_descriptor("policy-v1", "workspace-write/never"),
            fingerprint_descriptor("policy-v1", "workspace-write/never")
        );
    }
}
