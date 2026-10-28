//! Versioned benchmark contracts and offline evaluator qualification.
//!
//! Benchmark manifests are private inputs. They intentionally keep the task
//! prompt in a separate field from the expected contract, reference solution,
//! fixture worktrees, and evaluator commands. The only value that should be
//! passed to a model is [`BenchmarkTask::model_visible_context`].

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::CheckResult;
use crate::EVALUATOR_VERSION;
use crate::EvaluationBudget;
use crate::EvaluatorKind;
use crate::EvaluatorSpec;

/// Version of the private benchmark manifest contract.
pub const BENCHMARK_MANIFEST_SCHEMA_VERSION: u32 = 1;
/// Version of the evaluator qualification report contract.
pub const BENCHMARK_QUALIFICATION_REPORT_SCHEMA_VERSION: u32 = 1;
/// Version of the private baseline-observation input contract.
pub const BENCHMARK_BASELINE_INPUT_SCHEMA_VERSION: u32 = 1;
/// Version of the redacted baseline calibration report contract.
pub const BENCHMARK_BASELINE_REPORT_SCHEMA_VERSION: u32 = 1;
/// Evaluator version used by benchmark qualification and live results.
pub const BENCHMARK_EVALUATOR_VERSION: u32 = EVALUATOR_VERSION;
/// Minimum task count for the practical-value benchmark.
pub const MIN_BENCHMARK_TASKS: usize = 32;
/// Maximum task count for the practical-value benchmark.
pub const MAX_BENCHMARK_TASKS: usize = 48;
/// Minimum number of independently labeled task families.
pub const MIN_BENCHMARK_FAMILIES: usize = 4;
/// Lower bound of the calibrated baseline success range, in basis points.
pub const MIN_CALIBRATED_SUCCESS_BPS: u32 = 4_000;
/// Upper bound of the calibrated baseline success range, in basis points.
pub const MAX_CALIBRATED_SUCCESS_BPS: u32 = 7_500;
/// Success rates above this value are classified as a ceiling test.
pub const CEILING_SUCCESS_BPS: u32 = 8_500;
/// Success rates below this value are classified as too difficult.
pub const TOO_HARD_SUCCESS_BPS: u32 = 2_500;

const MAX_BENCHMARK_ID_CHARS: usize = 128;
const MAX_BENCHMARK_VERSION_CHARS: usize = 64;
const MAX_TASK_PROMPT_CHARS: usize = 32 * 1024;
const MAX_CONTRACT_SUMMARY_CHARS: usize = 2 * 1024;
const MAX_PRIVATE_MARKERS: usize = 16;
const MAX_PRIVATE_MARKER_CHARS: usize = 256;
const MAX_FIXTURES_PER_TASK: usize = 16;
const MAX_BASELINE_OBSERVATIONS: usize = MAX_BENCHMARK_TASKS;

/// Task families used by the practical-value benchmark.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BenchmarkTaskFamily {
    ParsingEscaping,
    StatefulBehavior,
    ConfigurationCli,
    TestDebugRecovery,
    RepositoryIntegration,
}

impl BenchmarkTaskFamily {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ParsingEscaping => "parsing_escaping",
            Self::StatefulBehavior => "stateful_behavior",
            Self::ConfigurationCli => "configuration_cli",
            Self::TestDebugRecovery => "test_debug_recovery",
            Self::RepositoryIntegration => "repository_integration",
        }
    }
}

/// Human review metadata for task difficulty. It is never included in model
/// context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BenchmarkDifficulty {
    Easy,
    Medium,
    Hard,
}

/// Controlled failure motif used for analysis, never as treatment context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BenchmarkFailureMotif {
    Ordering,
    Escaping,
    StateTransition,
    ConfigurationContract,
    CliContract,
    VerificationRecovery,
    RepositoryIntegration,
    UnrelatedChange,
}

/// Fixture classes used to qualify one task's evaluator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BenchmarkFixtureKind {
    Good,
    NoOp,
    Incomplete,
    TargetedMutant,
}

/// The contract and private markers used to detect accidental prompt leakage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BenchmarkExpectedContract {
    pub contract_id: String,
    pub summary: String,
    /// Strings that must never occur in the model-visible prompt. These are
    /// deliberately explicit because a private expected answer may be too
    /// large or structured to compare as one string.
    #[serde(default)]
    pub private_markers: Vec<String>,
}

/// Opaque reference to a known-good implementation. The solution contents
/// are never part of the model-visible task projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BenchmarkReferenceSolution {
    pub solution_id: String,
    pub path: PathBuf,
    pub fingerprint: String,
}

/// One evaluator qualification worktree. The worktree contains a good,
/// incomplete, no-op, or deliberately mutated implementation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BenchmarkFixture {
    pub fixture_id: String,
    pub kind: BenchmarkFixtureKind,
    pub worktree: PathBuf,
    /// For targeted mutants, these checks must fail. For no-op and incomplete
    /// fixtures an empty list means that at least one evaluator check must
    /// fail.
    #[serde(default)]
    pub expected_failed_check_ids: Vec<String>,
}

/// A private benchmark task. Only `task_id` and `prompt` are model-visible.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BenchmarkTask {
    pub task_id: String,
    pub task_fingerprint: String,
    pub family: BenchmarkTaskFamily,
    pub difficulty: BenchmarkDifficulty,
    pub failure_motif: BenchmarkFailureMotif,
    pub seed_commit: String,
    /// The only task text eligible for model context.
    pub prompt: String,
    pub expected_contract: BenchmarkExpectedContract,
    pub reference_solution: BenchmarkReferenceSolution,
    /// Evaluator commands and preflights are private manifest data.
    pub evaluator: EvaluatorSpec,
    #[serde(default)]
    pub budget: EvaluationBudget,
    pub fixtures: Vec<BenchmarkFixture>,
}

/// The deliberately small model-visible task projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelVisibleBenchmarkTask {
    pub task_id: String,
    pub prompt: String,
}

impl BenchmarkTask {
    /// Project this private task into the only structure that may be passed to
    /// a model runner.
    pub fn model_visible_context(&self) -> ModelVisibleBenchmarkTask {
        ModelVisibleBenchmarkTask {
            task_id: self.task_id.clone(),
            prompt: self.prompt.clone(),
        }
    }
}

/// Private, versioned benchmark task pack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BenchmarkManifest {
    pub schema_version: u32,
    pub benchmark_id: String,
    pub benchmark_version: String,
    pub source_commit: String,
    #[serde(default)]
    pub primary_checkout: Option<PathBuf>,
    pub tasks: Vec<BenchmarkTask>,
}

impl BenchmarkManifest {
    /// Validate the benchmark before any fixture command or model process is
    /// launched.
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == BENCHMARK_MANIFEST_SCHEMA_VERSION,
            "unsupported benchmark manifest schema {}",
            self.schema_version
        );
        validate_id(&self.benchmark_id, "benchmark_id", MAX_BENCHMARK_ID_CHARS)?;
        validate_id(
            &self.benchmark_version,
            "benchmark_version",
            MAX_BENCHMARK_VERSION_CHARS,
        )?;
        validate_id(&self.source_commit, "source_commit", MAX_BENCHMARK_ID_CHARS)?;
        ensure!(
            (MIN_BENCHMARK_TASKS..=MAX_BENCHMARK_TASKS).contains(&self.tasks.len()),
            "benchmark must contain between {} and {} tasks",
            MIN_BENCHMARK_TASKS,
            MAX_BENCHMARK_TASKS
        );

        let primary = self
            .primary_checkout
            .as_ref()
            .map(|path| path_identity(path));
        let mut task_ids = BTreeSet::new();
        let mut task_fingerprints = BTreeSet::new();
        let mut fixture_ids = BTreeSet::new();
        let mut private_worktrees: Vec<(String, PathBuf)> = Vec::new();
        let mut family_coverage =
            BTreeMap::<BenchmarkTaskFamily, BTreeSet<BenchmarkFixtureKind>>::new();

        for task in &self.tasks {
            validate_task(task, &self.source_commit)?;
            ensure!(
                task_ids.insert(task.task_id.clone()),
                "duplicate benchmark task {}",
                task.task_id
            );
            ensure!(
                task_fingerprints.insert(task.task_fingerprint.clone()),
                "duplicate benchmark task fingerprint for {}",
                task.task_id
            );

            let reference_identity = path_identity(&task.reference_solution.path);
            if let Some(primary) = &primary {
                ensure!(
                    !paths_overlap(primary, &reference_identity),
                    "reference solution {} overlaps the primary checkout",
                    task.reference_solution.solution_id
                );
            }
            for (other_id, other) in &private_worktrees {
                ensure!(
                    !paths_overlap(other, &reference_identity),
                    "reference solution {} overlaps {}",
                    task.reference_solution.solution_id,
                    other_id
                );
            }
            private_worktrees.push((
                format!("reference:{}", task.reference_solution.solution_id),
                reference_identity,
            ));

            let coverage = family_coverage.entry(task.family).or_default();
            for fixture in &task.fixtures {
                ensure!(
                    fixture_ids.insert(fixture.fixture_id.clone()),
                    "duplicate benchmark fixture {}",
                    fixture.fixture_id
                );
                coverage.insert(fixture.kind);
                let identity = path_identity(&fixture.worktree);
                if let Some(primary) = &primary {
                    ensure!(
                        !paths_overlap(primary, &identity),
                        "benchmark fixture {} overlaps the primary checkout",
                        fixture.fixture_id
                    );
                }
                for (other_id, other) in &private_worktrees {
                    ensure!(
                        !paths_overlap(other, &identity),
                        "benchmark fixture {} overlaps fixture {}",
                        fixture.fixture_id,
                        other_id
                    );
                }
                private_worktrees.push((format!("fixture:{}", fixture.fixture_id), identity));
            }
        }

        ensure!(
            family_coverage.len() >= MIN_BENCHMARK_FAMILIES,
            "benchmark must cover at least {} task families",
            MIN_BENCHMARK_FAMILIES
        );
        for (family, kinds) in family_coverage {
            ensure!(
                kinds.contains(&BenchmarkFixtureKind::Good),
                "task family {} has no good evaluator fixture",
                family.as_str()
            );
            ensure!(
                kinds.iter().any(|kind| matches!(
                    kind,
                    BenchmarkFixtureKind::NoOp | BenchmarkFixtureKind::Incomplete
                )),
                "task family {} has no no-op or incomplete evaluator fixture",
                family.as_str()
            );
            ensure!(
                kinds.contains(&BenchmarkFixtureKind::TargetedMutant),
                "task family {} has no targeted-mutant evaluator fixture",
                family.as_str()
            );
        }
        Ok(())
    }

    /// Stable opaque identity for all evaluator definitions in this pack.
    /// Commands, expected answers, and reference paths are included only as
    /// hash input and never returned in the fingerprint's clear text.
    pub fn evaluator_fingerprint(&self) -> String {
        let mut definitions = self
            .tasks
            .iter()
            .map(|task| (&task.task_id, &task.evaluator))
            .collect::<Vec<_>>();
        definitions.sort_by(|left, right| left.0.cmp(right.0));
        let bytes = serde_json::to_vec(&definitions).unwrap_or_default();
        fingerprint_bytes("benchmark-evaluator-v1", &bytes)
    }

    /// The model-visible projection of the complete pack. This intentionally
    /// excludes labels, expected contracts, references, evaluator commands,
    /// budgets, and fixture metadata.
    pub fn model_visible_tasks(&self) -> Vec<ModelVisibleBenchmarkTask> {
        self.tasks
            .iter()
            .map(BenchmarkTask::model_visible_context)
            .collect()
    }

    /// A redacted structural report suitable for operator inspection.
    pub fn validation_report(&self) -> BenchmarkValidationReport {
        let mut family_counts = BTreeMap::new();
        let mut fixture_count = 0_u32;
        for task in &self.tasks {
            *family_counts
                .entry(task.family.as_str().to_string())
                .or_insert(0) += 1;
            fixture_count = fixture_count.saturating_add(task.fixtures.len() as u32);
        }
        BenchmarkValidationReport {
            schema_version: BENCHMARK_QUALIFICATION_REPORT_SCHEMA_VERSION,
            benchmark_id: self.benchmark_id.clone(),
            benchmark_version: self.benchmark_version.clone(),
            evaluator_version: EVALUATOR_VERSION,
            evaluator_fingerprint: self.evaluator_fingerprint(),
            source_commit_fingerprint: fingerprint_bytes(
                "benchmark-source-commit-v1",
                self.source_commit.as_bytes(),
            ),
            task_count: self.tasks.len() as u32,
            family_counts,
            fixture_count,
            model_visible_task_count: self.tasks.len() as u32,
            valid: true,
        }
    }
}

/// Redacted structural validation report. It contains no prompts, command
/// tokens, reference paths, or expected-answer text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BenchmarkValidationReport {
    pub schema_version: u32,
    pub benchmark_id: String,
    pub benchmark_version: String,
    pub evaluator_version: u32,
    pub evaluator_fingerprint: String,
    pub source_commit_fingerprint: String,
    pub task_count: u32,
    pub family_counts: BTreeMap<String, u32>,
    pub fixture_count: u32,
    pub model_visible_task_count: u32,
    pub valid: bool,
}

/// The outcome of a fixture command execution. A command failing is not a
/// harness error; a spawn or timeout failure is kept distinct.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FixtureExecutionStatus {
    Completed,
    TimedOut,
    SpawnFailed,
    FixtureUnavailable,
    PreflightFailed,
}

/// Private evidence returned by the qualification runner for one fixture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FixtureExecutionEvidence {
    pub task_id: String,
    pub fixture_id: String,
    pub evaluator_version: u32,
    pub evaluator_fingerprint: String,
    pub preflight_passed: bool,
    pub status: FixtureExecutionStatus,
    pub checks: Vec<CheckResult>,
}

/// Redacted qualification result for one fixture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FixtureQualificationResult {
    pub task_id: String,
    pub fixture_id: String,
    pub kind: BenchmarkFixtureKind,
    pub status: FixtureExecutionStatus,
    pub preflight_passed: bool,
    pub checks: Vec<CheckResult>,
    pub passed: bool,
    pub expected_failure_detected: bool,
}

/// Per-task qualification summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BenchmarkTaskQualificationResult {
    pub task_id: String,
    pub family: BenchmarkTaskFamily,
    pub fixture_count: u32,
    pub passed_fixture_count: u32,
    pub good_fixture_count: u32,
    pub good_fixture_pass_count: u32,
    pub negative_fixture_count: u32,
    pub negative_fixture_detected_count: u32,
    pub targeted_mutant_count: u32,
    pub targeted_mutant_detected_count: u32,
    pub qualified: bool,
}

/// Versioned, no-model evaluator qualification report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BenchmarkQualificationReport {
    pub schema_version: u32,
    pub benchmark_id: String,
    pub benchmark_version: String,
    pub evaluator_version: u32,
    pub evaluator_fingerprint: String,
    pub task_count: u32,
    pub fixture_count: u32,
    pub preflight_count: u32,
    pub preflight_pass_count: u32,
    pub good_fixture_count: u32,
    pub good_fixture_pass_count: u32,
    pub negative_fixture_count: u32,
    pub negative_fixture_detected_count: u32,
    pub targeted_mutant_count: u32,
    pub targeted_mutant_detected_count: u32,
    pub execution_failure_count: u32,
    pub no_model_execution: bool,
    pub qualified: bool,
    pub tasks: Vec<BenchmarkTaskQualificationResult>,
    pub fixtures: Vec<FixtureQualificationResult>,
}

/// Build a qualification report from runner evidence. This function is
/// deliberately pure: it cannot launch commands or models.
pub fn build_qualification_report(
    manifest: &BenchmarkManifest,
    evidence: &[FixtureExecutionEvidence],
) -> Result<BenchmarkQualificationReport> {
    manifest.validate()?;
    let expected_fingerprint = manifest.evaluator_fingerprint();
    let mut fixtures_by_id = BTreeMap::new();
    let mut expected_fixtures = BTreeMap::new();
    for task in &manifest.tasks {
        for fixture in &task.fixtures {
            expected_fixtures.insert(fixture.fixture_id.clone(), (task, fixture));
        }
    }
    for item in evidence {
        ensure!(
            item.evaluator_version == EVALUATOR_VERSION,
            "fixture {} uses evaluator version {}, expected {}",
            item.fixture_id,
            item.evaluator_version,
            EVALUATOR_VERSION
        );
        ensure!(
            item.evaluator_fingerprint == expected_fingerprint,
            "fixture {} evaluator fingerprint does not match the benchmark",
            item.fixture_id
        );
        ensure!(
            fixtures_by_id
                .insert(item.fixture_id.clone(), item)
                .is_none(),
            "duplicate fixture evidence {}",
            item.fixture_id
        );
    }
    ensure!(
        fixtures_by_id.len() == expected_fixtures.len(),
        "qualification evidence covers {} fixtures, expected {}",
        fixtures_by_id.len(),
        expected_fixtures.len()
    );

    let mut fixture_results = Vec::with_capacity(expected_fixtures.len());
    let mut task_results = Vec::with_capacity(manifest.tasks.len());
    let mut preflight_count = 0_u32;
    let mut preflight_pass_count = 0_u32;
    let mut good_fixture_count = 0_u32;
    let mut good_fixture_pass_count = 0_u32;
    let mut negative_fixture_count = 0_u32;
    let mut negative_fixture_detected_count = 0_u32;
    let mut targeted_mutant_count = 0_u32;
    let mut targeted_mutant_detected_count = 0_u32;
    let mut execution_failure_count = 0_u32;

    for task in &manifest.tasks {
        let mut task_summary = BenchmarkTaskQualificationResult {
            task_id: task.task_id.clone(),
            family: task.family,
            fixture_count: 0,
            passed_fixture_count: 0,
            good_fixture_count: 0,
            good_fixture_pass_count: 0,
            negative_fixture_count: 0,
            negative_fixture_detected_count: 0,
            targeted_mutant_count: 0,
            targeted_mutant_detected_count: 0,
            qualified: true,
        };
        for fixture in &task.fixtures {
            let item = expected_fixtures
                .get(&fixture.fixture_id)
                .and_then(|_| fixtures_by_id.get(&fixture.fixture_id))
                .ok_or_else(|| {
                    anyhow::anyhow!("missing fixture evidence {}", fixture.fixture_id)
                })?;
            ensure!(
                item.task_id == task.task_id,
                "fixture {} evidence belongs to a different task",
                fixture.fixture_id
            );
            let result = assess_fixture(task, fixture, item)?;
            preflight_count = preflight_count.saturating_add(1);
            if item.preflight_passed {
                preflight_pass_count = preflight_pass_count.saturating_add(1);
            }
            if !matches!(item.status, FixtureExecutionStatus::Completed) {
                execution_failure_count = execution_failure_count.saturating_add(1);
            }
            task_summary.fixture_count = task_summary.fixture_count.saturating_add(1);
            if result.passed {
                task_summary.passed_fixture_count =
                    task_summary.passed_fixture_count.saturating_add(1);
            } else {
                task_summary.qualified = false;
            }
            match fixture.kind {
                BenchmarkFixtureKind::Good => {
                    good_fixture_count = good_fixture_count.saturating_add(1);
                    task_summary.good_fixture_count =
                        task_summary.good_fixture_count.saturating_add(1);
                    if result.passed {
                        good_fixture_pass_count = good_fixture_pass_count.saturating_add(1);
                        task_summary.good_fixture_pass_count =
                            task_summary.good_fixture_pass_count.saturating_add(1);
                    }
                }
                BenchmarkFixtureKind::NoOp
                | BenchmarkFixtureKind::Incomplete
                | BenchmarkFixtureKind::TargetedMutant => {
                    negative_fixture_count = negative_fixture_count.saturating_add(1);
                    task_summary.negative_fixture_count =
                        task_summary.negative_fixture_count.saturating_add(1);
                    if result.expected_failure_detected {
                        negative_fixture_detected_count =
                            negative_fixture_detected_count.saturating_add(1);
                        task_summary.negative_fixture_detected_count = task_summary
                            .negative_fixture_detected_count
                            .saturating_add(1);
                    } else {
                        task_summary.qualified = false;
                    }
                    if matches!(fixture.kind, BenchmarkFixtureKind::TargetedMutant) {
                        targeted_mutant_count = targeted_mutant_count.saturating_add(1);
                        task_summary.targeted_mutant_count =
                            task_summary.targeted_mutant_count.saturating_add(1);
                        if result.expected_failure_detected {
                            targeted_mutant_detected_count =
                                targeted_mutant_detected_count.saturating_add(1);
                            task_summary.targeted_mutant_detected_count = task_summary
                                .targeted_mutant_detected_count
                                .saturating_add(1);
                        }
                    }
                }
            }
            fixture_results.push(result);
        }
        task_results.push(task_summary);
    }

    let qualified = preflight_count > 0
        && preflight_count == preflight_pass_count
        && good_fixture_count == good_fixture_pass_count
        && negative_fixture_count == negative_fixture_detected_count
        && targeted_mutant_count == targeted_mutant_detected_count
        && execution_failure_count == 0
        && task_results.iter().all(|task| task.qualified);
    Ok(BenchmarkQualificationReport {
        schema_version: BENCHMARK_QUALIFICATION_REPORT_SCHEMA_VERSION,
        benchmark_id: manifest.benchmark_id.clone(),
        benchmark_version: manifest.benchmark_version.clone(),
        evaluator_version: EVALUATOR_VERSION,
        evaluator_fingerprint: expected_fingerprint,
        task_count: manifest.tasks.len() as u32,
        fixture_count: fixture_results.len() as u32,
        preflight_count,
        preflight_pass_count,
        good_fixture_count,
        good_fixture_pass_count,
        negative_fixture_count,
        negative_fixture_detected_count,
        targeted_mutant_count,
        targeted_mutant_detected_count,
        execution_failure_count,
        no_model_execution: true,
        qualified,
        tasks: task_results,
        fixtures: fixture_results,
    })
}

fn assess_fixture(
    task: &BenchmarkTask,
    fixture: &BenchmarkFixture,
    evidence: &FixtureExecutionEvidence,
) -> Result<FixtureQualificationResult> {
    let expected_check_ids = task
        .evaluator
        .checks
        .iter()
        .map(|check| check.check_id.as_str())
        .collect::<BTreeSet<_>>();
    let actual_check_ids = evidence
        .checks
        .iter()
        .map(|check| check.check_id.as_str())
        .collect::<BTreeSet<_>>();
    ensure!(
        expected_check_ids == actual_check_ids,
        "fixture {} check evidence does not match evaluator checks",
        fixture.fixture_id
    );
    ensure!(
        evidence.checks.len() == expected_check_ids.len(),
        "fixture {} contains duplicate check evidence",
        fixture.fixture_id
    );
    let check_success = evidence
        .checks
        .iter()
        .map(|check| (check.check_id.as_str(), check.success))
        .collect::<BTreeMap<_, _>>();
    let all_checks_pass = expected_check_ids
        .iter()
        .all(|check_id| check_success.get(check_id).copied().unwrap_or(false));
    let expected_failure_detected = match fixture.kind {
        BenchmarkFixtureKind::Good => false,
        BenchmarkFixtureKind::NoOp | BenchmarkFixtureKind::Incomplete => {
            evidence.checks.iter().any(|check| !check.success)
        }
        BenchmarkFixtureKind::TargetedMutant => {
            fixture.expected_failed_check_ids.iter().all(|check_id| {
                !check_success
                    .get(check_id.as_str())
                    .copied()
                    .unwrap_or(true)
            })
        }
    };
    let passed = evidence.preflight_passed
        && matches!(evidence.status, FixtureExecutionStatus::Completed)
        && match fixture.kind {
            BenchmarkFixtureKind::Good => all_checks_pass,
            _ => expected_failure_detected,
        };
    Ok(FixtureQualificationResult {
        task_id: task.task_id.clone(),
        fixture_id: fixture.fixture_id.clone(),
        kind: fixture.kind,
        status: evidence.status,
        preflight_passed: evidence.preflight_passed,
        checks: evidence.checks.clone(),
        passed,
        expected_failure_detected,
    })
}

/// Private input containing baseline outcomes collected with retrieval
/// disabled. It may contain task-level cost data, but never task prompts or
/// evaluator commands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BenchmarkBaselineInput {
    pub schema_version: u32,
    pub benchmark_id: String,
    pub source_commit: String,
    pub evaluator_version: u32,
    pub evaluator_fingerprint: String,
    /// Must remain false for 8a.3 calibration.
    pub retrieval_enabled: bool,
    pub observations: Vec<BenchmarkBaselineObservation>,
}

/// One retrieval-disabled baseline observation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BenchmarkBaselineObservation {
    pub task_id: String,
    pub success: bool,
    pub verification_complete: bool,
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub wall_time_ms: u64,
}

/// Baseline calibration status used by the 8a.3 exit gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BaselineCalibrationStatus {
    Qualified,
    Ceiling,
    TooHard,
    Borderline,
}

impl BaselineCalibrationStatus {
    pub const fn is_qualified(self) -> bool {
        matches!(self, Self::Qualified)
    }
}

/// Bounded cost distribution used in task and family reports.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct BaselineCostDistribution {
    pub sample_count: u32,
    pub total: u64,
    pub minimum: u64,
    pub median: u64,
    pub maximum: u64,
}

/// Task-level baseline result retained in the private calibration report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BenchmarkBaselineTaskResult {
    pub task_id: String,
    pub family: BenchmarkTaskFamily,
    pub success: bool,
    pub verification_complete: bool,
    pub total_tokens: u64,
    pub wall_time_ms: u64,
}

/// Family-level baseline calibration summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BenchmarkBaselineFamilyResult {
    pub task_count: u32,
    pub success_count: u32,
    pub verification_complete_count: u32,
    pub success_rate_bps: u32,
    pub token_distribution: BaselineCostDistribution,
    pub wall_time_distribution: BaselineCostDistribution,
}

/// Versioned calibration report. This report is evidence for benchmark
/// difficulty only; it is not a treatment result or promotion decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BenchmarkBaselineReport {
    pub schema_version: u32,
    pub benchmark_id: String,
    pub benchmark_version: String,
    pub source_commit_fingerprint: String,
    pub evaluator_version: u32,
    pub evaluator_fingerprint: String,
    pub retrieval_enabled: bool,
    pub task_count: u32,
    pub success_count: u32,
    pub verification_complete_count: u32,
    pub success_rate_bps: u32,
    pub status: BaselineCalibrationStatus,
    pub qualified: bool,
    pub all: BenchmarkBaselineFamilyResult,
    pub by_family: BTreeMap<String, BenchmarkBaselineFamilyResult>,
    pub tasks: Vec<BenchmarkBaselineTaskResult>,
}

/// Calibrate a retrieval-disabled baseline against a validated benchmark.
pub fn calibrate_baseline(
    manifest: &BenchmarkManifest,
    input: &BenchmarkBaselineInput,
) -> Result<BenchmarkBaselineReport> {
    manifest.validate()?;
    ensure!(
        input.schema_version == BENCHMARK_BASELINE_INPUT_SCHEMA_VERSION,
        "unsupported baseline input schema {}",
        input.schema_version
    );
    ensure!(
        input.benchmark_id == manifest.benchmark_id,
        "baseline input belongs to a different benchmark"
    );
    ensure!(
        input.source_commit == manifest.source_commit,
        "baseline source commit does not match the benchmark"
    );
    ensure!(
        input.evaluator_version == EVALUATOR_VERSION,
        "baseline evaluator version does not match the current evaluator"
    );
    ensure!(
        input.evaluator_fingerprint == manifest.evaluator_fingerprint(),
        "baseline evaluator fingerprint does not match the benchmark"
    );
    ensure!(
        !input.retrieval_enabled,
        "baseline calibration requires retrieval to be disabled"
    );
    ensure!(
        input.observations.len() == manifest.tasks.len()
            && input.observations.len() <= MAX_BASELINE_OBSERVATIONS,
        "baseline must contain exactly one observation for every benchmark task"
    );

    let tasks_by_id = manifest
        .tasks
        .iter()
        .map(|task| (task.task_id.as_str(), task))
        .collect::<BTreeMap<_, _>>();
    let mut seen = BTreeSet::new();
    let mut task_results = Vec::with_capacity(input.observations.len());
    for observation in &input.observations {
        let task = tasks_by_id
            .get(observation.task_id.as_str())
            .ok_or_else(|| {
                anyhow::anyhow!("baseline contains unknown task {}", observation.task_id)
            })?;
        ensure!(
            seen.insert(observation.task_id.clone()),
            "duplicate baseline observation {}",
            observation.task_id
        );
        task_results.push(BenchmarkBaselineTaskResult {
            task_id: observation.task_id.clone(),
            family: task.family,
            success: observation.success,
            verification_complete: observation.verification_complete,
            total_tokens: observation
                .input_tokens
                .saturating_add(observation.output_tokens),
            wall_time_ms: observation.wall_time_ms,
        });
    }
    ensure!(
        seen.len() == manifest.tasks.len(),
        "baseline is missing one or more benchmark tasks"
    );
    task_results.sort_by(|left, right| left.task_id.cmp(&right.task_id));

    let all = baseline_family_result(&task_results);
    let status = baseline_status(all.success_rate_bps);
    let mut by_family = BTreeMap::new();
    for family in [
        BenchmarkTaskFamily::ParsingEscaping,
        BenchmarkTaskFamily::StatefulBehavior,
        BenchmarkTaskFamily::ConfigurationCli,
        BenchmarkTaskFamily::TestDebugRecovery,
        BenchmarkTaskFamily::RepositoryIntegration,
    ] {
        let family_tasks = task_results
            .iter()
            .filter(|task| task.family == family)
            .cloned()
            .collect::<Vec<_>>();
        if !family_tasks.is_empty() {
            by_family.insert(
                family.as_str().to_string(),
                baseline_family_result(&family_tasks),
            );
        }
    }
    Ok(BenchmarkBaselineReport {
        schema_version: BENCHMARK_BASELINE_REPORT_SCHEMA_VERSION,
        benchmark_id: manifest.benchmark_id.clone(),
        benchmark_version: manifest.benchmark_version.clone(),
        source_commit_fingerprint: fingerprint_bytes(
            "benchmark-source-commit-v1",
            manifest.source_commit.as_bytes(),
        ),
        evaluator_version: EVALUATOR_VERSION,
        evaluator_fingerprint: manifest.evaluator_fingerprint(),
        retrieval_enabled: false,
        task_count: task_results.len() as u32,
        success_count: all.success_count,
        verification_complete_count: all.verification_complete_count,
        success_rate_bps: all.success_rate_bps,
        qualified: status.is_qualified(),
        status,
        all,
        by_family,
        tasks: task_results,
    })
}

fn baseline_family_result(tasks: &[BenchmarkBaselineTaskResult]) -> BenchmarkBaselineFamilyResult {
    let success_count = tasks.iter().filter(|task| task.success).count() as u32;
    let verification_complete_count = tasks
        .iter()
        .filter(|task| task.verification_complete)
        .count() as u32;
    let success_rate_bps = if tasks.is_empty() {
        0
    } else {
        ((success_count as u64 * 10_000) / tasks.len() as u64) as u32
    };
    BenchmarkBaselineFamilyResult {
        task_count: tasks.len() as u32,
        success_count,
        verification_complete_count,
        success_rate_bps,
        token_distribution: distribution(tasks.iter().map(|task| task.total_tokens)),
        wall_time_distribution: distribution(tasks.iter().map(|task| task.wall_time_ms)),
    }
}

fn distribution(values: impl Iterator<Item = u64>) -> BaselineCostDistribution {
    let mut values = values.collect::<Vec<_>>();
    values.sort_unstable();
    let Some(&minimum) = values.first() else {
        return BaselineCostDistribution::default();
    };
    let median = values[(values.len() - 1) / 2];
    BaselineCostDistribution {
        sample_count: values.len() as u32,
        total: values
            .iter()
            .fold(0_u64, |total, value| total.saturating_add(*value)),
        minimum,
        median,
        maximum: *values.last().unwrap_or(&minimum),
    }
}

fn baseline_status(success_rate_bps: u32) -> BaselineCalibrationStatus {
    if (MIN_CALIBRATED_SUCCESS_BPS..=MAX_CALIBRATED_SUCCESS_BPS).contains(&success_rate_bps) {
        BaselineCalibrationStatus::Qualified
    } else if success_rate_bps > CEILING_SUCCESS_BPS {
        BaselineCalibrationStatus::Ceiling
    } else if success_rate_bps < TOO_HARD_SUCCESS_BPS {
        BaselineCalibrationStatus::TooHard
    } else {
        BaselineCalibrationStatus::Borderline
    }
}

/// Load a private benchmark manifest and validate it before use.
pub fn load_benchmark_manifest(path: impl AsRef<Path>) -> Result<BenchmarkManifest> {
    let path = path.as_ref();
    let manifest: BenchmarkManifest = serde_json::from_reader(
        fs::File::open(path)
            .with_context(|| format!("open benchmark manifest {}", path.display()))?,
    )
    .with_context(|| format!("parse benchmark manifest {}", path.display()))?;
    manifest.validate()?;
    Ok(manifest)
}

/// Load private retrieval-disabled baseline observations.
pub fn load_baseline_input(path: impl AsRef<Path>) -> Result<BenchmarkBaselineInput> {
    let path = path.as_ref();
    serde_json::from_reader(
        fs::File::open(path)
            .with_context(|| format!("open benchmark baseline {}", path.display()))?,
    )
    .with_context(|| format!("parse benchmark baseline {}", path.display()))
}

fn validate_task(task: &BenchmarkTask, source_commit: &str) -> Result<()> {
    validate_path_component(&task.task_id, "task_id", MAX_BENCHMARK_ID_CHARS)?;
    validate_id(
        &task.task_fingerprint,
        "task_fingerprint",
        MAX_BENCHMARK_ID_CHARS,
    )?;
    ensure!(
        task.seed_commit == source_commit,
        "task {} seed commit does not match the benchmark source commit",
        task.task_id
    );
    ensure!(
        !task.prompt.trim().is_empty(),
        "task {} prompt is empty",
        task.task_id
    );
    ensure!(
        task.prompt.chars().count() <= MAX_TASK_PROMPT_CHARS,
        "task {} prompt is too long",
        task.task_id
    );
    ensure!(
        !task.prompt.contains('\0'),
        "task {} prompt contains a NUL character",
        task.task_id
    );
    validate_id(
        &task.expected_contract.contract_id,
        "expected_contract.contract_id",
        MAX_BENCHMARK_ID_CHARS,
    )?;
    ensure!(
        !task.expected_contract.summary.trim().is_empty()
            && task.expected_contract.summary.chars().count() <= MAX_CONTRACT_SUMMARY_CHARS,
        "task {} expected contract summary is empty or too long",
        task.task_id
    );
    ensure!(
        task.expected_contract.private_markers.len() <= MAX_PRIVATE_MARKERS,
        "task {} contains too many private prompt markers",
        task.task_id
    );
    for marker in &task.expected_contract.private_markers {
        ensure!(
            !marker.is_empty() && marker.chars().count() <= MAX_PRIVATE_MARKER_CHARS,
            "task {} has an invalid private prompt marker",
            task.task_id
        );
        ensure!(
            !task.prompt.contains(marker),
            "task {} prompt contains a private expected-answer marker",
            task.task_id
        );
        ensure!(
            !task.task_id.contains(marker),
            "task {} identifier contains a private expected-answer marker",
            task.task_id
        );
    }
    validate_id(
        &task.reference_solution.solution_id,
        "reference_solution.solution_id",
        MAX_BENCHMARK_ID_CHARS,
    )?;
    ensure!(
        !task.reference_solution.path.as_os_str().is_empty(),
        "task {} reference solution path is empty",
        task.task_id
    );
    validate_id(
        &task.reference_solution.fingerprint,
        "reference_solution.fingerprint",
        MAX_BENCHMARK_ID_CHARS,
    )?;
    ensure!(
        task.evaluator.kind == EvaluatorKind::PostRunVerification,
        "task {} must use post_run_verification for fixture qualification",
        task.task_id
    );
    crate::eval::validate_evaluator(&task.evaluator)?;
    for check in &task.evaluator.checks {
        ensure!(
            check.preflight_command_tokens.is_some(),
            "task {} evaluator check {} has no preflight",
            task.task_id,
            check.check_id
        );
    }
    crate::eval::validate_budget(&task.budget)?;
    ensure!(
        !task.fixtures.is_empty() && task.fixtures.len() <= MAX_FIXTURES_PER_TASK,
        "task {} has an invalid fixture count",
        task.task_id
    );
    let check_ids = task
        .evaluator
        .checks
        .iter()
        .map(|check| check.check_id.as_str())
        .collect::<BTreeSet<_>>();
    for fixture in &task.fixtures {
        validate_path_component(&fixture.fixture_id, "fixture_id", MAX_BENCHMARK_ID_CHARS)?;
        ensure!(
            !fixture.worktree.as_os_str().is_empty(),
            "fixture {} worktree is empty",
            fixture.fixture_id
        );
        let mut expected_ids = BTreeSet::new();
        for check_id in &fixture.expected_failed_check_ids {
            ensure!(
                check_ids.contains(check_id.as_str()),
                "fixture {} references unknown failed check {}",
                fixture.fixture_id,
                check_id
            );
            ensure!(
                expected_ids.insert(check_id),
                "fixture {} contains duplicate failed check {}",
                fixture.fixture_id,
                check_id
            );
        }
        if fixture.kind == BenchmarkFixtureKind::Good {
            ensure!(
                fixture.expected_failed_check_ids.is_empty(),
                "good fixture {} declares failed checks",
                fixture.fixture_id
            );
        }
        if fixture.kind == BenchmarkFixtureKind::TargetedMutant {
            ensure!(
                !fixture.expected_failed_check_ids.is_empty(),
                "targeted mutant {} declares no target check",
                fixture.fixture_id
            );
        }
    }
    Ok(())
}

fn validate_id(value: &str, label: &str, max_chars: usize) -> Result<()> {
    ensure!(!value.trim().is_empty(), "{label} is empty");
    ensure!(value.chars().count() <= max_chars, "{label} is too long");
    ensure!(
        !value.chars().any(|character| character.is_control()),
        "{label} contains a control character"
    );
    Ok(())
}

fn validate_path_component(value: &str, label: &str, max_chars: usize) -> Result<()> {
    validate_id(value, label, max_chars)?;
    ensure!(
        !value.contains(['/', '\\']) && value != "." && value != "..",
        "{label} must be a single safe path component"
    );
    Ok(())
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
    let mut missing = Vec::new();
    let mut existing = absolute.as_path();
    while !existing.exists() {
        let Some(name) = existing.file_name() else {
            return absolute;
        };
        missing.push(name.to_os_string());
        let Some(parent) = existing.parent() else {
            return absolute;
        };
        existing = parent;
    }
    let mut identity = fs::canonicalize(existing).unwrap_or_else(|_| existing.to_path_buf());
    for component in missing.iter().rev() {
        identity.push(component);
    }
    identity
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    left == right || left.starts_with(right) || right.starts_with(left)
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

/// Helper for callers that need a protected-path check before writing a
/// report. Prompts and evaluator commands are intentionally not returned.
pub fn benchmark_protected_paths(manifest: &BenchmarkManifest) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(primary) = &manifest.primary_checkout {
        paths.push(path_identity(primary));
    }
    for task in &manifest.tasks {
        paths.push(path_identity(&task.reference_solution.path));
        for fixture in &task.fixtures {
            paths.push(path_identity(&fixture.worktree));
        }
    }
    paths
}

#[cfg(test)]
#[path = "benchmark_tests.rs"]
mod tests;
