//! Review-only improvement proposals.
//!
//! This module is deliberately separate from the active card store. A proposal
//! can describe a card, Skill, repository guidance patch, configuration
//! change, or diagnostic issue, but it has no runtime authority. Milestone 6
//! owns the promotion state machine; until then this is an evidence-linked,
//! validated artifact that a human can inspect and evaluate.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use anyhow::Context;
use anyhow::Result;
use anyhow::ensure;
use serde::Deserialize;
use serde::Serialize;
use sha2::Digest;
use sha2::Sha256;

use crate::CandidateStatus;
use crate::EVALUATION_RESULT_SCHEMA_VERSION;
use crate::EvaluationResult;
use crate::EvaluationSplit;
use crate::ExperienceCandidate;
use crate::ExperienceCard;
use crate::ExperienceScope;
use crate::TaskFamily;

/// Version of the review-only improvement proposal contract.
pub const IMPROVEMENT_CANDIDATE_SCHEMA_VERSION: u32 = 1;

const MAX_CANDIDATE_ID_CHARS: usize = 128;
const MAX_TARGET_NAME_CHARS: usize = 256;
const MAX_EVIDENCE: usize = 32;
const MAX_VALIDATION_REQUIREMENTS: usize = 16;
const MAX_TEXT_CHARS: usize = 2048;
const MAX_DIFF_CHARS: usize = 32 * 1024;
const MAX_COMMAND_CHARS: usize = 1024;
const MAX_PATH_CHARS: usize = 1024;
const MAX_DAYS_TO_EXPIRY: i64 = 180;
const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// Kind of artifact that the proposer is asking a reviewer to consider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImprovementCandidateType {
    ExperienceCardUpdate,
    FocusedSkillChange,
    AgentsMdPatch,
    PromptExtensionConfigChange,
    DiagnosticToolingIssue,
}

impl ImprovementCandidateType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ExperienceCardUpdate => "experience_card_update",
            Self::FocusedSkillChange => "focused_skill_change",
            Self::AgentsMdPatch => "agents_md_patch",
            Self::PromptExtensionConfigChange => "prompt_extension_config_change",
            Self::DiagnosticToolingIssue => "diagnostic_tooling_issue",
        }
    }
}

/// Milestone 5 has one state. Promotion states belong to Milestone 6.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImprovementCandidateStatus {
    Proposed,
}

/// A bounded reference to evidence stored in another local artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImprovementEvidenceKind {
    ExperienceCandidate,
    ExperienceCard,
    TraceBundle,
    EvaluationManifest,
    EvaluationScenario,
}

impl ImprovementEvidenceKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ExperienceCandidate => "experience_candidate",
            Self::ExperienceCard => "experience_card",
            Self::TraceBundle => "trace_bundle",
            Self::EvaluationManifest => "evaluation_manifest",
            Self::EvaluationScenario => "evaluation_scenario",
        }
    }
}

/// A validation check required before a reviewer can consider promotion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImprovementValidationKind {
    Command,
    EvaluationScenario,
    ManualReview,
}

/// Task and repository scope of a proposal. Workspace identity is opaque.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImprovementTarget {
    pub task_family: TaskFamily,
    #[serde(default)]
    pub scope: ExperienceScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_name: Option<String>,
}

/// One evidence reference. The ID is opaque and is interpreted by its kind.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ImprovementEvidenceReference {
    pub kind: ImprovementEvidenceKind,
    pub id: String,
}

/// A command, evaluation scenario, or explicit manual review required for the
/// proposal. Commands are recorded as text for review; this module never runs
/// them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImprovementValidationRequirement {
    pub check_id: String,
    pub kind: ImprovementValidationKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evaluation_scenario_id: Option<String>,
    pub expected: String,
}

/// Review and expiry metadata for a proposal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImprovementExpiry {
    pub review_after_unix_ms: i64,
    pub expires_at_unix_ms: i64,
}

/// Explicit rollback intent. It is descriptive until Milestone 6 supplies a
/// promotion and rollback control plane.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImprovementRollback {
    pub strategy: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_version: Option<String>,
}

/// Proposed content for one supported artifact lane.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum ImprovementContent {
    ExperienceCardUpdate {
        card: ExperienceCard,
    },
    FocusedSkillChange {
        skill_name: String,
        diff: String,
    },
    AgentsMdPatch {
        relative_path: String,
        diff: String,
    },
    PromptExtensionConfigChange {
        setting: String,
        value: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        diff: Option<String>,
    },
    DiagnosticToolingIssue {
        title: String,
        description: String,
        suggested_fix: String,
    },
}

impl ImprovementContent {
    pub const fn candidate_type(&self) -> ImprovementCandidateType {
        match self {
            Self::ExperienceCardUpdate { .. } => ImprovementCandidateType::ExperienceCardUpdate,
            Self::FocusedSkillChange { .. } => ImprovementCandidateType::FocusedSkillChange,
            Self::AgentsMdPatch { .. } => ImprovementCandidateType::AgentsMdPatch,
            Self::PromptExtensionConfigChange { .. } => {
                ImprovementCandidateType::PromptExtensionConfigChange
            }
            Self::DiagnosticToolingIssue { .. } => ImprovementCandidateType::DiagnosticToolingIssue,
        }
    }
}

/// A review-only, evidence-linked improvement artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImprovementCandidate {
    pub schema_version: u32,
    pub candidate_id: String,
    pub candidate_type: ImprovementCandidateType,
    pub status: ImprovementCandidateStatus,
    pub created_at_unix_ms: i64,
    pub target: ImprovementTarget,
    pub evidence: Vec<ImprovementEvidenceReference>,
    pub rationale: String,
    pub expected_benefit: String,
    #[serde(default)]
    pub expected_costs: Vec<String>,
    #[serde(default)]
    pub risks: Vec<String>,
    pub content: ImprovementContent,
    pub validation: Vec<ImprovementValidationRequirement>,
    pub expiry: ImprovementExpiry,
    pub rollback: ImprovementRollback,
}

impl ImprovementCandidate {
    /// Drafts the first supported proposal: an active-card update that still
    /// requires validation and explicit review before it can affect retrieval.
    pub fn from_experience_candidate(
        candidate: &ExperienceCandidate,
        evaluation_scenario_id: impl Into<String>,
    ) -> Result<Self> {
        ensure!(
            candidate.status == CandidateStatus::Pending,
            "only pending experience candidates can become proposals"
        );
        ensure!(
            candidate.is_bounded(),
            "experience candidate is malformed or outside bounded limits"
        );
        let evaluation_scenario_id = evaluation_scenario_id.into();
        validate_identifier(&evaluation_scenario_id, "evaluation_scenario_id")?;

        let card = candidate.card.clone();
        let candidate_id = proposal_id(candidate, &evaluation_scenario_id);
        ensure!(
            candidate.created_at_unix_ms >= 0,
            "experience candidate timestamp cannot be negative"
        );
        let created_at = candidate.created_at_unix_ms;
        let target = ImprovementTarget {
            task_family: card.task_family,
            scope: card.scope.clone(),
            target_name: Some(card.card_id.clone()),
        };
        let proposal = Self {
            schema_version: IMPROVEMENT_CANDIDATE_SCHEMA_VERSION,
            candidate_id,
            candidate_type: ImprovementCandidateType::ExperienceCardUpdate,
            status: ImprovementCandidateStatus::Proposed,
            created_at_unix_ms: created_at,
            target,
            evidence: vec![
                ImprovementEvidenceReference {
                    kind: ImprovementEvidenceKind::ExperienceCandidate,
                    id: candidate.candidate_id.clone(),
                },
                ImprovementEvidenceReference {
                    kind: ImprovementEvidenceKind::ExperienceCard,
                    id: card.card_id.clone(),
                },
                ImprovementEvidenceReference {
                    kind: ImprovementEvidenceKind::TraceBundle,
                    id: candidate.source_trace_id.clone(),
                },
                ImprovementEvidenceReference {
                    kind: ImprovementEvidenceKind::EvaluationScenario,
                    id: evaluation_scenario_id.clone(),
                },
            ],
            rationale: format!(
                "Review whether the observed {} lesson for {} should be retained as scoped advisory context.",
                card.failure_class.as_str(),
                card.task_family.as_str()
            ),
            expected_benefit:
                "Improve retrieval of a bounded, previously observed workflow lesson on matching tasks."
                    .to_string(),
            expected_costs: vec![
                "Small additional retrieved context when the task and scope match.".to_string(),
                "Human review and a paired evaluation run are required before activation.".to_string(),
            ],
            risks: vec![
                "The lesson may be stale, over-generalized, or not followed by the agent.".to_string(),
                "A retrieved advisory card must not change instruction precedence or permissions."
                    .to_string(),
            ],
            content: ImprovementContent::ExperienceCardUpdate { card },
            validation: vec![ImprovementValidationRequirement {
                check_id: "held_out_paired_evaluation".to_string(),
                kind: ImprovementValidationKind::EvaluationScenario,
                command: None,
                evaluation_scenario_id: Some(evaluation_scenario_id),
                expected: "Compare baseline and treatment on the declared held-out scenario without changing policy or execution configuration.".to_string(),
            }],
            expiry: ImprovementExpiry {
                review_after_unix_ms: created_at.saturating_add(7 * DAY_MS),
                expires_at_unix_ms: created_at.saturating_add(30 * DAY_MS),
            },
            rollback: ImprovementRollback {
                strategy:
                    "Reject the proposal or restore the previous active card version; do not rewrite unrelated cards."
                        .to_string(),
                previous_version: Some(candidate.card.card_id.clone()),
            },
        };
        proposal.validate()?;
        Ok(proposal)
    }

    /// Validates structure, content, safety boundaries, and the required
    /// concrete validation plan without touching the filesystem.
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == IMPROVEMENT_CANDIDATE_SCHEMA_VERSION,
            "unsupported improvement candidate schema {}",
            self.schema_version
        );
        validate_identifier(&self.candidate_id, "candidate_id")?;
        ensure!(
            self.status == ImprovementCandidateStatus::Proposed,
            "unsupported Milestone 5 improvement candidate status"
        );
        ensure!(
            self.created_at_unix_ms >= 0,
            "created_at_unix_ms cannot be negative"
        );
        ensure!(
            self.target.scope.is_bounded(),
            "improvement target scope is outside bounded limits"
        );
        if let Some(target_name) = &self.target.target_name {
            validate_text(target_name, MAX_TARGET_NAME_CHARS, "target.target_name")?;
        }
        ensure!(
            self.evidence.len() <= MAX_EVIDENCE && !self.evidence.is_empty(),
            "improvement candidate must contain between 1 and {MAX_EVIDENCE} evidence references"
        );
        let mut evidence = BTreeSet::new();
        for reference in &self.evidence {
            validate_identifier(&reference.id, "evidence.id")?;
            ensure!(
                evidence.insert(reference),
                "duplicate improvement evidence reference"
            );
        }

        validate_text(&self.rationale, MAX_TEXT_CHARS, "rationale")?;
        validate_text(&self.expected_benefit, MAX_TEXT_CHARS, "expected_benefit")?;
        validate_nonempty_text_list(&self.expected_costs, "expected_costs")?;
        validate_nonempty_text_list(&self.risks, "risks")?;
        validate_content(&self.content, &self.target)?;
        ensure!(
            self.content.candidate_type() == self.candidate_type,
            "candidate_type does not match proposed content"
        );
        validate_validation_requirements(&self.validation)?;
        ensure!(
            self.validation.iter().any(|requirement| {
                requirement.command.is_some() || requirement.evaluation_scenario_id.is_some()
            }),
            "improvement candidate requires a concrete command or evaluation scenario"
        );
        ensure!(
            self.expiry.review_after_unix_ms >= self.created_at_unix_ms,
            "review expiry precedes proposal creation"
        );
        ensure!(
            self.expiry.expires_at_unix_ms > self.expiry.review_after_unix_ms,
            "proposal expiry must follow review_after"
        );
        ensure!(
            self.expiry
                .expires_at_unix_ms
                .saturating_sub(self.created_at_unix_ms)
                <= MAX_DAYS_TO_EXPIRY * DAY_MS,
            "proposal expiry exceeds the bounded review horizon"
        );
        validate_text(&self.rollback.strategy, MAX_TEXT_CHARS, "rollback.strategy")?;
        if let Some(previous_version) = &self.rollback.previous_version {
            validate_identifier(previous_version, "rollback.previous_version")?;
        }
        Ok(())
    }

    /// Validates that every referenced artifact is present in a local evidence
    /// catalog. It still does not run commands or apply any proposal content.
    pub fn validate_against(&self, catalog: &ImprovementEvidenceCatalog) -> Result<()> {
        self.validate()?;
        for reference in &self.evidence {
            ensure!(
                catalog.contains(reference),
                "missing improvement evidence {}:{}",
                reference.kind.as_str(),
                reference.id
            );
        }
        for requirement in &self.validation {
            if let Some(scenario_id) = &requirement.evaluation_scenario_id {
                let reference = ImprovementEvidenceReference {
                    kind: ImprovementEvidenceKind::EvaluationScenario,
                    id: scenario_id.clone(),
                };
                ensure!(
                    catalog.contains(&reference),
                    "missing evaluation scenario evidence {scenario_id}"
                );
                ensure!(
                    catalog.is_held_out_evaluation_scenario(scenario_id),
                    "evaluation scenario evidence {scenario_id} is not held out"
                );
            }
        }
        Ok(())
    }
}

/// An in-memory index of evidence IDs. It contains identifiers only, never
/// prompts, command tokens, paths, or trace payloads.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImprovementEvidenceCatalog {
    references: BTreeSet<ImprovementEvidenceReference>,
    held_out_evaluation_scenarios: BTreeSet<String>,
}

impl ImprovementEvidenceCatalog {
    pub fn from_candidates(candidates: &[ExperienceCandidate]) -> Self {
        let mut catalog = Self::default();
        for candidate in candidates {
            catalog.add(ImprovementEvidenceReference {
                kind: ImprovementEvidenceKind::ExperienceCandidate,
                id: candidate.candidate_id.clone(),
            });
            catalog.add(ImprovementEvidenceReference {
                kind: ImprovementEvidenceKind::ExperienceCard,
                id: candidate.card.card_id.clone(),
            });
            catalog.add(ImprovementEvidenceReference {
                kind: ImprovementEvidenceKind::TraceBundle,
                id: candidate.source_trace_id.clone(),
            });
        }
        catalog
    }

    /// Adds the public evidence IDs exposed by an offline evaluation result.
    pub fn add_evaluation_result(&mut self, result: &EvaluationResult) -> Result<()> {
        ensure!(
            result.schema_version == EVALUATION_RESULT_SCHEMA_VERSION,
            "unsupported evaluation result schema {}",
            result.schema_version
        );
        validate_identifier(&result.manifest_id, "evaluation.manifest_id")?;
        self.add_evaluation_manifest(&result.manifest_id);
        for scenario in &result.scenarios {
            validate_identifier(&scenario.scenario_id, "evaluation.scenario_id")?;
            if scenario.split == EvaluationSplit::HeldOut {
                self.add_held_out_evaluation_scenario(&scenario.scenario_id);
            } else {
                self.add_evaluation_scenario(&scenario.scenario_id);
            }
            if let Some(trace_id) = &scenario.baseline.trace_id {
                validate_identifier(trace_id, "evaluation.baseline.trace_id")?;
                self.add_trace_bundle(trace_id);
            }
            if let Some(trace_id) = &scenario.treatment.trace_id {
                validate_identifier(trace_id, "evaluation.treatment.trace_id")?;
                self.add_trace_bundle(trace_id);
            }
        }
        Ok(())
    }

    pub fn add_evaluation_manifest(&mut self, manifest_id: impl Into<String>) {
        self.add(ImprovementEvidenceReference {
            kind: ImprovementEvidenceKind::EvaluationManifest,
            id: manifest_id.into(),
        });
    }

    pub fn add_evaluation_scenario(&mut self, scenario_id: impl Into<String>) {
        self.add(ImprovementEvidenceReference {
            kind: ImprovementEvidenceKind::EvaluationScenario,
            id: scenario_id.into(),
        });
    }

    pub fn add_held_out_evaluation_scenario(&mut self, scenario_id: impl Into<String>) {
        let scenario_id = scenario_id.into();
        self.add_evaluation_scenario(scenario_id.clone());
        self.held_out_evaluation_scenarios.insert(scenario_id);
    }

    pub fn add_trace_bundle(&mut self, trace_id: impl Into<String>) {
        self.add(ImprovementEvidenceReference {
            kind: ImprovementEvidenceKind::TraceBundle,
            id: trace_id.into(),
        });
    }

    pub fn contains(&self, reference: &ImprovementEvidenceReference) -> bool {
        self.references.contains(reference)
    }

    pub fn is_held_out_evaluation_scenario(&self, scenario_id: &str) -> bool {
        self.held_out_evaluation_scenarios.contains(scenario_id)
    }

    fn add(&mut self, reference: ImprovementEvidenceReference) {
        self.references.insert(reference);
    }
}

/// Loads and validates one proposal artifact.
pub fn load_improvement_candidate(path: impl AsRef<Path>) -> Result<ImprovementCandidate> {
    let path = path.as_ref();
    let candidate: ImprovementCandidate = serde_json::from_reader(
        fs::File::open(path)
            .with_context(|| format!("open improvement candidate {}", path.display()))?,
    )
    .with_context(|| format!("parse improvement candidate {}", path.display()))?;
    candidate.validate()?;
    Ok(candidate)
}

/// Validates and writes one proposal artifact. This function does not update
/// the active card store or any other Codex configuration.
pub fn write_improvement_candidate(
    candidate: &ImprovementCandidate,
    path: impl AsRef<Path>,
) -> Result<()> {
    candidate.validate()?;
    let path = path.as_ref();
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "create improvement candidate directory {}",
                parent.display()
            )
        })?;
    }
    let data = serde_json::to_vec_pretty(candidate).context("serialize improvement candidate")?;
    fs::write(path, data)
        .with_context(|| format!("write improvement candidate {}", path.display()))?;
    Ok(())
}

fn proposal_id(candidate: &ExperienceCandidate, evaluation_scenario_id: &str) -> String {
    let identity = format!(
        "{}\u{1f}{}\u{1f}{}",
        candidate.candidate_id, candidate.card.card_id, evaluation_scenario_id
    );
    let digest = Sha256::digest(identity.as_bytes());
    let suffix = digest[..12]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("improvement_{suffix}")
}

fn validate_content(content: &ImprovementContent, target: &ImprovementTarget) -> Result<()> {
    match content {
        ImprovementContent::ExperienceCardUpdate { card } => {
            ensure!(card.is_bounded(), "proposed experience card is malformed");
            ensure!(
                card.task_family == target.task_family,
                "card task family does not match proposal target"
            );
            ensure!(
                card.scope == target.scope,
                "card scope does not match proposal target"
            );
            validate_card_text(card)?;
            if let Some(target_name) = &target.target_name {
                ensure!(
                    target_name == &card.card_id,
                    "card target name does not match proposed card"
                );
            }
        }
        ImprovementContent::FocusedSkillChange { skill_name, diff } => {
            validate_identifier(skill_name, "skill_name")?;
            ensure!(
                !skill_name.contains('/') && !skill_name.contains('\\'),
                "skill name cannot contain a path separator"
            );
            validate_diff(diff, "Skill diff")?;
            ensure!(
                target.target_name.as_deref() == Some(skill_name.as_str()),
                "Skill target name does not match proposed Skill"
            );
        }
        ImprovementContent::AgentsMdPatch {
            relative_path,
            diff,
        } => {
            validate_text(relative_path, MAX_PATH_CHARS, "AGENTS.md relative path")?;
            let path = Path::new(relative_path);
            ensure!(!path.is_absolute(), "AGENTS.md patch path must be relative");
            ensure!(
                !relative_path.split(['/', '\\']).any(|part| part == ".."),
                "AGENTS.md patch path cannot contain parent traversal"
            );
            ensure!(
                relative_path.ends_with("AGENTS.md"),
                "AGENTS.md patch must target an AGENTS.md file"
            );
            validate_diff(diff, "AGENTS.md diff")?;
            if let Some(target_name) = &target.target_name {
                ensure!(
                    target_name == relative_path,
                    "AGENTS.md target name does not match patch path"
                );
            }
        }
        ImprovementContent::PromptExtensionConfigChange {
            setting,
            value,
            diff,
        } => {
            validate_config_setting(setting)?;
            validate_text(value, MAX_TEXT_CHARS, "configuration value")?;
            reject_protected_instruction(value, "configuration value")?;
            if let Some(diff) = diff {
                validate_diff(diff, "configuration diff")?;
            }
            if let Some(target_name) = &target.target_name {
                ensure!(
                    target_name == setting,
                    "configuration target name does not match setting"
                );
            }
        }
        ImprovementContent::DiagnosticToolingIssue {
            title,
            description,
            suggested_fix,
        } => {
            validate_text(title, MAX_TEXT_CHARS, "diagnostic title")?;
            validate_text(description, MAX_DIFF_CHARS, "diagnostic description")?;
            validate_text(suggested_fix, MAX_DIFF_CHARS, "diagnostic suggested fix")?;
        }
    }
    Ok(())
}

fn validate_card_text(card: &ExperienceCard) -> Result<()> {
    validate_identifier(&card.card_id, "card.card_id")?;
    validate_identifier(&card.lesson_id, "card.lesson_id")?;
    validate_identifier(&card.source_trace_id, "card.source_trace_id")?;
    for trace_id in &card.source_trace_ids {
        validate_identifier(trace_id, "card.source_trace_ids")?;
    }
    validate_text(&card.recommendation, 512, "card.recommendation")?;
    validate_text_list(&card.activity_labels, "card.activity_labels")?;
    validate_text_list(&card.signals, "card.signals")?;
    validate_text_list(&card.verification, "card.verification")?;
    Ok(())
}

fn validate_validation_requirements(
    requirements: &[ImprovementValidationRequirement],
) -> Result<()> {
    ensure!(
        !requirements.is_empty() && requirements.len() <= MAX_VALIDATION_REQUIREMENTS,
        "improvement candidate must contain between 1 and {MAX_VALIDATION_REQUIREMENTS} validation requirements"
    );
    let mut check_ids = BTreeSet::new();
    for requirement in requirements {
        validate_identifier(&requirement.check_id, "validation.check_id")?;
        ensure!(
            check_ids.insert(&requirement.check_id),
            "duplicate improvement validation check"
        );
        validate_text(&requirement.expected, MAX_TEXT_CHARS, "validation.expected")?;
        match requirement.kind {
            ImprovementValidationKind::Command => {
                let command = requirement
                    .command
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("command validation is missing command text"))?;
                validate_command(command)?;
                ensure!(
                    requirement.evaluation_scenario_id.is_none(),
                    "command validation cannot also name an evaluation scenario"
                );
            }
            ImprovementValidationKind::EvaluationScenario => {
                let scenario_id = requirement
                    .evaluation_scenario_id
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("scenario validation is missing scenario ID"))?;
                validate_identifier(scenario_id, "validation.evaluation_scenario_id")?;
                ensure!(
                    requirement.command.is_none(),
                    "scenario validation cannot also contain a command"
                );
            }
            ImprovementValidationKind::ManualReview => {
                ensure!(
                    requirement.command.is_none() && requirement.evaluation_scenario_id.is_none(),
                    "manual review validation cannot contain executable evidence"
                );
            }
        }
    }
    Ok(())
}

fn validate_command(command: &str) -> Result<()> {
    validate_text(command, MAX_COMMAND_CHARS, "validation.command")?;
    for marker in [
        "rm -rf",
        "git reset --hard",
        "git checkout --",
        "sudo ",
        "chmod ",
        "curl ",
        "wget ",
        "dangerously-bypass",
        "approval_policy",
        "sandbox_mode",
        "disable sandbox",
    ] {
        ensure!(
            !command.to_ascii_lowercase().contains(marker),
            "validation command contains a forbidden operation"
        );
    }
    ensure!(
        !command
            .chars()
            .any(|character| matches!(character, ';' | '|' | '&' | '<' | '>' | '$')),
        "validation command must not contain shell composition"
    );
    Ok(())
}

fn validate_config_setting(setting: &str) -> Result<()> {
    validate_text(setting, MAX_TARGET_NAME_CHARS, "configuration setting")?;
    ensure!(
        !setting.chars().any(|character| {
            !(character.is_ascii_alphanumeric() || matches!(character, '_' | '.' | '-'))
        }),
        "configuration setting contains unsupported characters"
    );
    reject_protected_instruction(setting, "configuration setting")?;
    Ok(())
}

fn reject_protected_instruction(value: &str, label: &str) -> Result<()> {
    let normalized = value.to_ascii_lowercase();
    let has_absolute_path = normalized.split_whitespace().any(|token| {
        token.starts_with('/')
            || token.starts_with("\\\\")
            || token.starts_with("file://")
            || (token.len() >= 3
                && token.as_bytes()[1] == b':'
                && matches!(token.as_bytes()[2], b'/' | b'\\'))
    });
    ensure!(
        !has_absolute_path,
        "{label} contains an absolute path or file URI"
    );
    for marker in [
        "approval",
        "permission",
        "sandbox",
        "system",
        "developer",
        "policy",
        "credential",
        "authorization",
    ] {
        ensure!(
            !normalized.contains(marker),
            "{label} targets a protected instruction or permission boundary"
        );
    }
    Ok(())
}

fn validate_diff(diff: &str, label: &str) -> Result<()> {
    ensure!(!diff.trim().is_empty(), "{label} is empty");
    ensure!(
        diff.chars().count() <= MAX_DIFF_CHARS,
        "{label} exceeds the bounded size"
    );
    ensure!(
        !diff
            .chars()
            .any(|character| character.is_control() && !matches!(character, '\n' | '\r')),
        "{label} contains unsupported control characters"
    );
    validate_text_safety(diff, label)?;
    reject_protected_instruction(diff, label)?;
    ensure!(
        !diff.to_ascii_lowercase().contains("-----begin "),
        "{label} contains a credential-like block"
    );
    Ok(())
}

fn validate_text_list(values: &[String], label: &str) -> Result<()> {
    ensure!(values.len() <= 16, "{label} contains too many entries");
    for value in values {
        validate_text(value, MAX_TEXT_CHARS, label)?;
    }
    Ok(())
}

fn validate_nonempty_text_list(values: &[String], label: &str) -> Result<()> {
    ensure!(
        !values.is_empty(),
        "{label} must contain at least one entry"
    );
    validate_text_list(values, label)
}

fn validate_text(value: &str, max_chars: usize, label: &str) -> Result<()> {
    ensure!(!value.trim().is_empty(), "{label} is empty");
    ensure!(
        value.chars().count() <= max_chars,
        "{label} exceeds the bounded size"
    );
    ensure!(
        !value.chars().any(char::is_control),
        "{label} contains control characters"
    );
    validate_text_safety(value, label)
}

fn validate_text_safety(value: &str, label: &str) -> Result<()> {
    let normalized = value.to_ascii_lowercase();
    let has_absolute_path = normalized.split_whitespace().any(|token| {
        token.starts_with('/')
            || token.starts_with("\\\\")
            || token.starts_with("file://")
            || (token.len() >= 3
                && token.as_bytes()[1] == b':'
                && matches!(token.as_bytes()[2], b'/' | b'\\'))
    });
    ensure!(
        !has_absolute_path,
        "{label} contains an absolute path or file URI"
    );
    for marker in [
        "ignore previous",
        "ignore all previous",
        "system message",
        "developer instruction",
        "override the user",
        "bypass approval",
        "disable sandbox",
        "grant permission",
        "execute automatically",
        "do not follow",
        "jailbreak",
        "private key",
        "authorization:",
        "bearer ",
        "api_key",
        "apikey",
        "password=",
        "sk-",
        "/users/",
        "/home/",
        "/private/",
        "c:\\users\\",
        "-----begin ",
    ] {
        ensure!(
            !normalized.contains(marker),
            "{label} contains disallowed raw, secret-like, or instruction-like content"
        );
    }
    Ok(())
}

fn validate_identifier(value: &str, label: &str) -> Result<()> {
    validate_text(value, MAX_CANDIDATE_ID_CHARS, label)?;
    ensure!(
        value
            .chars()
            .all(|character| character.is_ascii_alphanumeric()
                || matches!(character, '_' | '-' | '.' | ':')),
        "{label} contains unsupported identifier characters"
    );
    Ok(())
}

#[cfg(test)]
#[path = "improvement_tests.rs"]
mod tests;
