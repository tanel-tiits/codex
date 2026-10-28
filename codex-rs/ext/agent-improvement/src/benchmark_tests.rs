use std::fs;
use std::path::Path;

use anyhow::Result;
use tempfile::TempDir;

use super::*;
use crate::CheckResult;
use crate::VerificationCheck;

fn evaluator(task_id: &str) -> crate::EvaluatorSpec {
    crate::EvaluatorSpec {
        kind: crate::EvaluatorKind::PostRunVerification,
        checks: vec![VerificationCheck {
            check_id: "contract".to_string(),
            command_tokens: vec!["check-fixture".to_string(), task_id.to_string()],
            expected_exit_code: 0,
            preflight_command_tokens: Some(vec![
                "check-fixture".to_string(),
                "--preflight".to_string(),
            ]),
            preflight_expected_exit_code: 0,
        }],
        require_all_checks: true,
    }
}

fn manifest(root: &Path) -> BenchmarkManifest {
    let families = [
        BenchmarkTaskFamily::ParsingEscaping,
        BenchmarkTaskFamily::StatefulBehavior,
        BenchmarkTaskFamily::ConfigurationCli,
        BenchmarkTaskFamily::TestDebugRecovery,
        BenchmarkTaskFamily::RepositoryIntegration,
    ];
    let tasks = (0..32)
        .map(|index| {
            let task_id = format!("task-{index:02}");
            let task_root = root.join(&task_id);
            let reference_path = root.join("references").join(&task_id);
            let fixture = |kind: BenchmarkFixtureKind| {
                let name = match kind {
                    BenchmarkFixtureKind::Good => "good",
                    BenchmarkFixtureKind::NoOp => "no-op",
                    BenchmarkFixtureKind::Incomplete => "incomplete",
                    BenchmarkFixtureKind::TargetedMutant => "mutant-ordering",
                };
                BenchmarkFixture {
                    fixture_id: format!("{task_id}-{name}"),
                    kind,
                    worktree: task_root.join(name),
                    expected_failed_check_ids: if kind == BenchmarkFixtureKind::TargetedMutant {
                        vec!["contract".to_string()]
                    } else {
                        Vec::new()
                    },
                }
            };
            BenchmarkTask {
                task_id: task_id.clone(),
                task_fingerprint: format!("task-fingerprint-{index:02}"),
                family: families[index % families.len()],
                difficulty: match index % 3 {
                    0 => BenchmarkDifficulty::Easy,
                    1 => BenchmarkDifficulty::Medium,
                    _ => BenchmarkDifficulty::Hard,
                },
                failure_motif: BenchmarkFailureMotif::Ordering,
                seed_commit: "commit-1".to_string(),
                prompt: format!("Implement the public behavior for {task_id} and run its tests."),
                expected_contract: BenchmarkExpectedContract {
                    contract_id: format!("contract-{task_id}"),
                    summary: "Private expected behavior is kept outside model context.".to_string(),
                    private_markers: vec![format!("PRIVATE_EXPECTED_{index:02}")],
                },
                reference_solution: BenchmarkReferenceSolution {
                    solution_id: format!("solution-{task_id}"),
                    path: reference_path,
                    fingerprint: format!("solution-fingerprint-{index:02}"),
                },
                evaluator: evaluator(&task_id),
                budget: crate::EvaluationBudget::default(),
                fixtures: vec![
                    fixture(BenchmarkFixtureKind::Good),
                    fixture(BenchmarkFixtureKind::NoOp),
                    fixture(BenchmarkFixtureKind::Incomplete),
                    fixture(BenchmarkFixtureKind::TargetedMutant),
                ],
            }
        })
        .collect();
    BenchmarkManifest {
        schema_version: BENCHMARK_MANIFEST_SCHEMA_VERSION,
        benchmark_id: "benchmark-v1".to_string(),
        benchmark_version: "8a3-v1".to_string(),
        source_commit: "commit-1".to_string(),
        primary_checkout: Some(root.join("primary")),
        tasks,
    }
}

fn evidence_for(
    manifest: &BenchmarkManifest,
    mutant_passes: bool,
) -> Vec<FixtureExecutionEvidence> {
    let fingerprint = manifest.evaluator_fingerprint();
    manifest
        .tasks
        .iter()
        .flat_map(|task| {
            task.fixtures
                .iter()
                .map(|fixture| FixtureExecutionEvidence {
                    task_id: task.task_id.clone(),
                    fixture_id: fixture.fixture_id.clone(),
                    evaluator_version: EVALUATOR_VERSION,
                    evaluator_fingerprint: fingerprint.clone(),
                    preflight_passed: true,
                    status: FixtureExecutionStatus::Completed,
                    checks: task
                        .evaluator
                        .checks
                        .iter()
                        .map(|check| {
                            let success = match fixture.kind {
                                BenchmarkFixtureKind::Good => true,
                                BenchmarkFixtureKind::NoOp | BenchmarkFixtureKind::Incomplete => {
                                    false
                                }
                                BenchmarkFixtureKind::TargetedMutant => {
                                    mutant_passes
                                        || !fixture
                                            .expected_failed_check_ids
                                            .iter()
                                            .any(|check_id| check_id == &check.check_id)
                                }
                            };
                            CheckResult {
                                check_id: check.check_id.clone(),
                                success,
                            }
                        })
                        .collect(),
                })
        })
        .collect()
}

#[test]
fn manifest_requires_a_real_task_pack_and_keeps_private_fields_out_of_context() -> Result<()> {
    let temp = TempDir::new()?;
    let manifest = manifest(temp.path());
    manifest.validate()?;
    assert_eq!(manifest.tasks.len(), MIN_BENCHMARK_TASKS);
    assert!(manifest.validation_report().valid);

    let visible = serde_json::to_string(&manifest.model_visible_tasks())?;
    assert!(!visible.contains("PRIVATE_EXPECTED"));
    assert!(!visible.contains("check-fixture"));
    assert!(!visible.contains("solution-fingerprint"));
    assert!(!visible.contains("mutant-ordering"));
    assert!(visible.contains("Implement the public behavior"));
    Ok(())
}

#[test]
fn manifest_rejects_duplicate_tasks_bad_commits_missing_fixtures_and_overlap() -> Result<()> {
    let temp = TempDir::new()?;
    let valid = manifest(temp.path());

    let mut duplicate = valid.clone();
    duplicate.tasks[1].task_id = duplicate.tasks[0].task_id.clone();
    assert!(duplicate.validate().is_err());

    let mut bad_commit = valid.clone();
    bad_commit.tasks[0].seed_commit = "other-commit".to_string();
    assert!(bad_commit.validate().is_err());

    let mut overlap = valid.clone();
    overlap.tasks[0].fixtures[0].worktree = temp.path().join("primary");
    assert!(overlap.validate().is_err());

    let mut reference_overlap = valid.clone();
    reference_overlap.tasks[0].reference_solution.path =
        reference_overlap.tasks[1].reference_solution.path.clone();
    assert!(reference_overlap.validate().is_err());

    let mut missing_preflight = valid;
    missing_preflight.tasks[0].evaluator.checks[0].preflight_command_tokens = None;
    assert!(missing_preflight.validate().is_err());

    let mut unsafe_task_id = manifest(temp.path());
    unsafe_task_id.tasks[0].task_id = "../outside".to_string();
    assert!(unsafe_task_id.validate().is_err());

    let mut unsafe_fixture_id = manifest(temp.path());
    unsafe_fixture_id.tasks[0].fixtures[0].fixture_id = "../outside".to_string();
    assert!(unsafe_fixture_id.validate().is_err());

    let mut leaked_prompt = manifest(temp.path());
    leaked_prompt.tasks[0]
        .prompt
        .push_str(" PRIVATE_EXPECTED_00");
    assert!(leaked_prompt.validate().is_err());

    let mut leaked_task_id = manifest(temp.path());
    leaked_task_id.tasks[0].task_id = "PRIVATE_EXPECTED_00".to_string();
    assert!(leaked_task_id.validate().is_err());
    Ok(())
}

#[test]
fn qualification_passes_good_fixtures_and_detects_all_negative_variants() -> Result<()> {
    let temp = TempDir::new()?;
    let manifest = manifest(temp.path());
    let report = build_qualification_report(&manifest, &evidence_for(&manifest, false))?;
    assert!(report.qualified);
    assert_eq!(report.good_fixture_count, 32);
    assert_eq!(report.good_fixture_pass_count, 32);
    assert_eq!(report.negative_fixture_count, 96);
    assert_eq!(report.negative_fixture_detected_count, 96);
    assert_eq!(report.targeted_mutant_count, 32);
    assert_eq!(report.targeted_mutant_detected_count, 32);
    assert!(report.no_model_execution);
    Ok(())
}

#[test]
fn qualification_rejects_false_positive_mutants_and_bad_or_missing_evidence() -> Result<()> {
    let temp = TempDir::new()?;
    let manifest = manifest(temp.path());
    let false_positive = build_qualification_report(&manifest, &evidence_for(&manifest, true))?;
    assert!(!false_positive.qualified);
    assert_eq!(false_positive.targeted_mutant_detected_count, 0);

    let mut missing = evidence_for(&manifest, false);
    missing.pop();
    assert!(build_qualification_report(&manifest, &missing).is_err());

    let mut missing_check = evidence_for(&manifest, false);
    missing_check[0].checks.pop();
    assert!(build_qualification_report(&manifest, &missing_check).is_err());

    let mut mismatched = evidence_for(&manifest, false);
    mismatched[0].evaluator_version += 1;
    assert!(build_qualification_report(&manifest, &mismatched).is_err());

    let mut incomplete = evidence_for(&manifest, false);
    incomplete[0].status = FixtureExecutionStatus::TimedOut;
    let report = build_qualification_report(&manifest, &incomplete)?;
    assert!(!report.qualified);
    assert_eq!(report.execution_failure_count, 1);
    Ok(())
}

#[test]
fn calibration_reports_task_family_success_verification_and_cost_ranges() -> Result<()> {
    let temp = TempDir::new()?;
    let manifest = manifest(temp.path());
    let observations = manifest
        .tasks
        .iter()
        .enumerate()
        .map(|(index, task)| BenchmarkBaselineObservation {
            task_id: task.task_id.clone(),
            success: index < 18,
            verification_complete: index % 4 != 0,
            input_tokens: 100 + index as u64,
            output_tokens: 20,
            wall_time_ms: 1_000 + index as u64,
        })
        .collect();
    let input = BenchmarkBaselineInput {
        schema_version: BENCHMARK_BASELINE_INPUT_SCHEMA_VERSION,
        benchmark_id: manifest.benchmark_id.clone(),
        source_commit: manifest.source_commit.clone(),
        evaluator_version: EVALUATOR_VERSION,
        evaluator_fingerprint: manifest.evaluator_fingerprint(),
        retrieval_enabled: false,
        observations,
    };
    let report = calibrate_baseline(&manifest, &input)?;
    assert_eq!(report.success_count, 18);
    assert_eq!(report.success_rate_bps, 5_625);
    assert_eq!(report.status, BaselineCalibrationStatus::Qualified);
    assert!(report.qualified);
    assert_eq!(report.by_family.len(), 5);
    assert_eq!(report.all.token_distribution.sample_count, 32);
    assert_eq!(report.tasks.len(), 32);

    let mut ceiling = input.clone();
    ceiling
        .observations
        .iter_mut()
        .for_each(|observation| observation.success = true);
    assert_eq!(
        calibrate_baseline(&manifest, &ceiling)?.status,
        BaselineCalibrationStatus::Ceiling
    );

    let mut too_hard = input;
    too_hard
        .observations
        .iter_mut()
        .for_each(|observation| observation.success = false);
    assert_eq!(
        calibrate_baseline(&manifest, &too_hard)?.status,
        BaselineCalibrationStatus::TooHard
    );

    let mut treatment_input = too_hard;
    treatment_input.retrieval_enabled = true;
    assert!(calibrate_baseline(&manifest, &treatment_input).is_err());
    Ok(())
}

#[test]
fn evaluator_fingerprint_changes_when_a_private_check_changes() -> Result<()> {
    let temp = TempDir::new()?;
    let manifest = manifest(temp.path());
    let first = manifest.evaluator_fingerprint();
    let mut changed = manifest;
    changed.tasks[0].evaluator.checks[0]
        .command_tokens
        .push("--strict".to_string());
    assert_ne!(first, changed.evaluator_fingerprint());
    Ok(())
}

#[test]
fn targeted_mutants_must_fail_the_declared_check_not_an_unrelated_check() -> Result<()> {
    let temp = TempDir::new()?;
    let mut manifest = manifest(temp.path());
    for task in &mut manifest.tasks {
        task.evaluator.checks.push(VerificationCheck {
            check_id: "ordering".to_string(),
            command_tokens: vec!["check-ordering".to_string()],
            expected_exit_code: 0,
            preflight_command_tokens: Some(vec![
                "check-fixture".to_string(),
                "--preflight".to_string(),
            ]),
            preflight_expected_exit_code: 0,
        });
        for fixture in &mut task.fixtures {
            if fixture.kind == BenchmarkFixtureKind::TargetedMutant {
                fixture.expected_failed_check_ids = vec!["ordering".to_string()];
            }
        }
    }
    manifest.validate()?;
    let mut evidence = evidence_for(&manifest, false);
    let wrong_check = evidence
        .iter_mut()
        .find(|item| item.fixture_id == "task-00-mutant-ordering")
        .expect("targeted mutant evidence");
    wrong_check.checks = vec![
        CheckResult {
            check_id: "contract".to_string(),
            success: false,
        },
        CheckResult {
            check_id: "ordering".to_string(),
            success: true,
        },
    ];
    let report = build_qualification_report(&manifest, &evidence)?;
    assert!(!report.qualified);
    assert_eq!(report.targeted_mutant_detected_count, 31);
    Ok(())
}

#[test]
fn private_reference_paths_are_not_in_model_projection() -> Result<()> {
    let temp = TempDir::new()?;
    let manifest = manifest(temp.path());
    let visible = serde_json::to_string(&manifest.model_visible_tasks())?;
    assert!(!visible.contains(&temp.path().display().to_string()));
    assert!(!visible.contains("references"));
    assert!(!visible.contains("expected_contract"));
    assert!(!visible.contains("evaluator"));
    fs::create_dir_all(temp.path().join("primary"))?;
    Ok(())
}
