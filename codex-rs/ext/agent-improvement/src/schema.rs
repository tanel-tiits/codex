use std::collections::BTreeSet;

use serde::Deserialize;
use serde::Serialize;
use sha2::Digest;
use sha2::Sha256;

use crate::intervention::Intervention;

/// Current on-disk experience-card schema.
pub const EXPERIENCE_CARD_SCHEMA_VERSION: u32 = 2;
pub(crate) const LEGACY_EXPERIENCE_CARD_SCHEMA_VERSION: u32 = 1;

pub(crate) const MAX_CARD_SIGNALS: usize = 8;
pub(crate) const MAX_ACTIVITY_LABELS: usize = 8;
pub(crate) const MAX_TOPIC_HASHES: usize = 16;
const MAX_SOURCE_TRACE_IDS: usize = 8;
const MAX_CARD_LABEL_CHARS: usize = 64;
const MAX_RECOMMENDATION_CHARS: usize = 512;
const MAX_SOURCE_TRACE_ID_CHARS: usize = 128;
const MAX_LESSON_ID_CHARS: usize = 128;

const TOPIC_STOP_WORDS: &[&str] = &[
    "a",
    "an",
    "add",
    "after",
    "and",
    "are",
    "as",
    "at",
    "be",
    "by",
    "change",
    "check",
    "continue",
    "create",
    "for",
    "fix",
    "from",
    "help",
    "implement",
    "implementation",
    "in",
    "investigate",
    "is",
    "it",
    "make",
    "need",
    "of",
    "on",
    "or",
    "please",
    "remove",
    "review",
    "run",
    "test",
    "tests",
    "that",
    "the",
    "this",
    "to",
    "update",
    "with",
    "write",
];

/// Coarse task family used for conservative lexical retrieval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskFamily {
    Coding,
    Testing,
    Review,
    Planning,
    Investigation,
    Documentation,
    General,
}

impl TaskFamily {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Coding => "coding",
            Self::Testing => "testing",
            Self::Review => "review",
            Self::Planning => "planning",
            Self::Investigation => "investigation",
            Self::Documentation => "documentation",
            Self::General => "general",
        }
    }

    /// Conservatively classifies text without retaining the text itself.
    pub fn from_text(text: &str) -> Self {
        let text = text.to_ascii_lowercase();
        if has_any(
            &text,
            &[
                "test",
                "pytest",
                "jest",
                "vitest",
                "cargo test",
                "unit test",
            ],
        ) {
            return Self::Testing;
        }
        if has_any(&text, &["review", "code review", "pull request", "diff"]) {
            return Self::Review;
        }
        if has_any(&text, &["plan", "planning", "roadmap", "design"]) {
            return Self::Planning;
        }
        if has_any(&text, &["readme", "documentation", "document", "docs"]) {
            return Self::Documentation;
        }
        if has_any(
            &text,
            &[
                "investigate",
                "investigation",
                "debug",
                "diagnose",
                "root cause",
            ],
        ) {
            return Self::Investigation;
        }
        if has_any(
            &text,
            &[
                "code",
                "coding",
                "implement",
                "implementation",
                "fix",
                "bug",
                "refactor",
                "feature",
            ],
        ) {
            return Self::Coding;
        }
        Self::General
    }
}

/// Deterministic coarse outcome class for one reduced rollout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureClass {
    Completed,
    CommandFailure,
    ToolFailure,
    ModelFailure,
    PolicyBlocked,
    Cancelled,
    IncompleteVerification,
    Unknown,
}

impl FailureClass {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::CommandFailure => "command_failure",
            Self::ToolFailure => "tool_failure",
            Self::ModelFailure => "model_failure",
            Self::PolicyBlocked => "policy_blocked",
            Self::Cancelled => "cancelled",
            Self::IncompleteVerification => "incomplete_verification",
            Self::Unknown => "unknown",
        }
    }
}

/// Lifecycle state for a reviewed card. Retired cards remain in the append-only
/// store for provenance but are excluded from retrieval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CardStatus {
    #[default]
    Active,
    Retired,
}

impl CardStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Retired => "retired",
        }
    }
}

/// Confidence in a derived lesson. Direct runtime observations are stronger
/// than hypotheses and neither is treated as policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConfidenceLevel {
    #[default]
    Unknown,
    Low,
    Medium,
    High,
}

impl ConfidenceLevel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

/// Operation class in the bounded failure signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OperationClass {
    #[default]
    Unknown,
    None,
    Command,
    Tool,
    Inference,
    Verification,
    Policy,
    Mixed,
}

impl OperationClass {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::None => "none",
            Self::Command => "command",
            Self::Tool => "tool",
            Self::Inference => "inference",
            Self::Verification => "verification",
            Self::Policy => "policy",
            Self::Mixed => "mixed",
        }
    }
}

/// Controlled runtime/tool family. Names and arguments are intentionally not
/// persisted because they can contain arbitrary user or repository data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ToolFamily {
    #[default]
    Unknown,
    None,
    ExecCommand,
    WriteStdin,
    ApplyPatch,
    Mcp,
    Web,
    ImageGeneration,
    Agent,
    CodeMode,
    Other,
}

impl ToolFamily {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::None => "none",
            Self::ExecCommand => "exec_command",
            Self::WriteStdin => "write_stdin",
            Self::ApplyPatch => "apply_patch",
            Self::Mcp => "mcp",
            Self::Web => "web",
            Self::ImageGeneration => "image_generation",
            Self::Agent => "agent",
            Self::CodeMode => "code_mode",
            Self::Other => "other",
        }
    }
}

/// Coarse exit/result bucket, without copying exit text or output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExitBucket {
    #[default]
    Unknown,
    Success,
    NonZero,
    Cancelled,
    Blocked,
    Unavailable,
}

impl ExitBucket {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Success => "success",
            Self::NonZero => "non_zero",
            Self::Cancelled => "cancelled",
            Self::Blocked => "blocked",
            Self::Unavailable => "unavailable",
        }
    }
}

/// Coarse reason for a permission or sandbox boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PolicyReason {
    #[default]
    NotObserved,
    ApprovalRequired,
    SandboxRestricted,
    PermissionDenied,
    Other,
}

impl PolicyReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotObserved => "not_observed",
            Self::ApprovalRequired => "approval_required",
            Self::SandboxRestricted => "sandbox_restricted",
            Self::PermissionDenied => "permission_denied",
            Self::Other => "other",
        }
    }
}

/// Verification state derived from observed test-like operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum VerificationState {
    #[default]
    Unknown,
    Passed,
    Failed,
    Missing,
    NotApplicable,
}

impl VerificationState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::Missing => "missing",
            Self::NotApplicable => "not_applicable",
        }
    }
}

/// Coarse toolchain family inferred from command names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ToolchainFamily {
    #[default]
    Unknown,
    Rust,
    Node,
    Python,
    Go,
    Dotnet,
    Java,
    Ruby,
    Other,
}

impl ToolchainFamily {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Rust => "rust",
            Self::Node => "node",
            Self::Python => "python",
            Self::Go => "go",
            Self::Dotnet => "dotnet",
            Self::Java => "java",
            Self::Ruby => "ruby",
            Self::Other => "other",
        }
    }
}

/// Coarse model family. The concrete model identifier is never stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ModelFamily {
    #[default]
    Unknown,
    OpenAi,
    Anthropic,
    Google,
    Local,
    Other,
}

impl ModelFamily {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::OpenAi => "open_ai",
            Self::Anthropic => "anthropic",
            Self::Google => "google",
            Self::Local => "local",
            Self::Other => "other",
        }
    }
}

/// Directly observed runtime fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceFact {
    RolloutCompleted,
    TurnActivityObserved,
    TerminalActivityObserved,
    CommandFailed,
    ToolFailed,
    InferenceFailed,
    VerificationObserved,
    VerificationMissing,
    PolicyBoundaryObserved,
    Cancelled,
    NoRuntimeEvidence,
    RecoveryObserved,
}

impl EvidenceFact {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RolloutCompleted => "rollout_completed",
            Self::TurnActivityObserved => "turn_activity_observed",
            Self::TerminalActivityObserved => "terminal_activity_observed",
            Self::CommandFailed => "command_failed",
            Self::ToolFailed => "tool_failed",
            Self::InferenceFailed => "inference_failed",
            Self::VerificationObserved => "verification_observed",
            Self::VerificationMissing => "verification_missing",
            Self::PolicyBoundaryObserved => "policy_boundary_observed",
            Self::Cancelled => "cancelled",
            Self::NoRuntimeEvidence => "no_runtime_evidence",
            Self::RecoveryObserved => "recovery_observed",
        }
    }
}

/// One bounded step in the observed rollout sequence. This is a category-only
/// projection; it never stores event IDs, commands, output, or timestamps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservedStep {
    Turn,
    Inference,
    Tool,
    TerminalOperation,
    VerificationAttempt,
    PolicyBoundary,
    Recovery,
}

impl ObservedStep {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Turn => "turn",
            Self::Inference => "inference",
            Self::Tool => "tool",
            Self::TerminalOperation => "terminal_operation",
            Self::VerificationAttempt => "verification_attempt",
            Self::PolicyBoundary => "policy_boundary",
            Self::Recovery => "recovery",
        }
    }
}

/// Lower-confidence explanation suggested by observed facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HypothesisKind {
    LikelyCommandEnvironmentIssue,
    LikelyToolPrecondition,
    LikelyModelBoundary,
    LikelyPolicyConstraint,
    LikelyMissingVerification,
    Unknown,
}

impl HypothesisKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LikelyCommandEnvironmentIssue => "likely_command_environment_issue",
            Self::LikelyToolPrecondition => "likely_tool_precondition",
            Self::LikelyModelBoundary => "likely_model_boundary",
            Self::LikelyPolicyConstraint => "likely_policy_constraint",
            Self::LikelyMissingVerification => "likely_missing_verification",
            Self::Unknown => "unknown",
        }
    }
}

/// Controlled intervention/recommendation label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterventionKind {
    ReproduceFirstFailingCommand,
    CheckToolPreconditions,
    ReduceToVerifiableStep,
    RequestRequiredAuthority,
    ResumeFromCheckpoint,
    RunTargetedTests,
    PreserveWorkflow,
    InspectReducedTrace,
}

impl InterventionKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ReproduceFirstFailingCommand => "reproduce_first_failing_command",
            Self::CheckToolPreconditions => "check_tool_preconditions",
            Self::ReduceToVerifiableStep => "reduce_to_verifiable_step",
            Self::RequestRequiredAuthority => "request_required_authority",
            Self::ResumeFromCheckpoint => "resume_from_checkpoint",
            Self::RunTargetedTests => "run_targeted_tests",
            Self::PreserveWorkflow => "preserve_observed_workflow",
            Self::InspectReducedTrace => "inspect_reduced_trace",
        }
    }
}

/// Coarse observed outcome, distinct from a hypothesis or recommendation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeKind {
    Completed,
    FailureObserved,
    VerificationPassed,
    VerificationMissing,
    PolicyBlocked,
    Cancelled,
    Unknown,
}

impl OutcomeKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::FailureObserved => "failure_observed",
            Self::VerificationPassed => "verification_passed",
            Self::VerificationMissing => "verification_missing",
            Self::PolicyBlocked => "policy_blocked",
            Self::Cancelled => "cancelled",
            Self::Unknown => "unknown",
        }
    }
}

/// Explicit user feedback about a card's usefulness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackState {
    #[default]
    Unknown,
    Positive,
    Negative,
    Mixed,
}

impl FeedbackState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Positive => "positive",
            Self::Negative => "negative",
            Self::Mixed => "mixed",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "unknown" => Some(Self::Unknown),
            "positive" => Some(Self::Positive),
            "negative" => Some(Self::Negative),
            "mixed" => Some(Self::Mixed),
            _ => None,
        }
    }
}

/// Attribution state. Unknown is the safe default when the trace cannot link
/// a later result to a retrieved recommendation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AttributionState {
    #[default]
    Unknown,
    Yes,
    No,
    Mixed,
}

impl AttributionState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Yes => "yes",
            Self::No => "no",
            Self::Mixed => "mixed",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "unknown" => Some(Self::Unknown),
            "yes" => Some(Self::Yes),
            "no" => Some(Self::No),
            "mixed" => Some(Self::Mixed),
            _ => None,
        }
    }
}

/// Bounded evidence counters copied from a reduced trace.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentImprovementEvidence {
    pub turn_count: u32,
    pub inference_count: u32,
    pub failed_inference_count: u32,
    pub tool_count: u32,
    pub failed_tool_count: u32,
    pub terminal_operation_count: u32,
    pub failed_terminal_operation_count: u32,
    pub verification_step_count: u32,
    #[serde(default)]
    pub verification_attempt_count: u32,
    #[serde(default)]
    pub failed_verification_count: u32,
    #[serde(default)]
    pub token_usage: TokenUsageSummary,
}

/// A small token-usage projection that avoids carrying provider payloads.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsageSummary {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_output_tokens: u64,
}

/// Controlled facts, hypotheses, interventions, and outcomes for one lesson.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceProfile {
    #[serde(default)]
    pub observed_sequence: Vec<ObservedStep>,
    #[serde(default)]
    pub facts: Vec<EvidenceFact>,
    #[serde(default)]
    pub hypotheses: Vec<HypothesisKind>,
    #[serde(default)]
    pub interventions: Vec<InterventionKind>,
    #[serde(default)]
    pub outcomes: Vec<OutcomeKind>,
}

impl EvidenceProfile {
    fn is_bounded(&self) -> bool {
        self.observed_sequence.len() <= MAX_CARD_SIGNALS
            && self.facts.len() <= MAX_CARD_SIGNALS
            && self.hypotheses.len() <= MAX_CARD_SIGNALS
            && self.interventions.len() <= MAX_CARD_SIGNALS
            && self.outcomes.len() <= MAX_CARD_SIGNALS
    }

    fn fact_count(&self) -> usize {
        self.facts.len()
    }

    pub(crate) fn from_legacy(
        signals: &[String],
        failure_class: FailureClass,
        verification: &[String],
    ) -> Self {
        let mut profile = Self::default();
        for signal in signals {
            let fact = match signal.as_str() {
                "rollout_completed" => Some(EvidenceFact::RolloutCompleted),
                "turn_activity_observed" => Some(EvidenceFact::TurnActivityObserved),
                "terminal_activity_observed" => Some(EvidenceFact::TerminalActivityObserved),
                "command_failed" => Some(EvidenceFact::CommandFailed),
                "tool_failed" => Some(EvidenceFact::ToolFailed),
                "inference_failed" => Some(EvidenceFact::InferenceFailed),
                "verification_observed" => Some(EvidenceFact::VerificationObserved),
                "verification_missing" => Some(EvidenceFact::VerificationMissing),
                "policy_boundary_observed" => Some(EvidenceFact::PolicyBoundaryObserved),
                "cancelled" => Some(EvidenceFact::Cancelled),
                "no_runtime_evidence" => Some(EvidenceFact::NoRuntimeEvidence),
                _ => None,
            };
            if let Some(fact) = fact {
                push_unique(&mut profile.facts, fact, MAX_CARD_SIGNALS);
            }
        }
        if profile.facts.is_empty() && failure_class == FailureClass::Unknown {
            profile.facts.push(EvidenceFact::NoRuntimeEvidence);
        }
        profile
            .hypotheses
            .push(hypothesis_for_failure(failure_class));
        profile
            .interventions
            .push(intervention_for_failure(failure_class));
        profile.outcomes.push(outcome_for_failure(
            failure_class,
            verification
                .iter()
                .any(|label| label == "test_like_command_succeeded"),
        ));
        profile
    }
}

/// Bounded failure signature used for structured retrieval.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailureSignature {
    #[serde(default)]
    pub operation_class: OperationClass,
    #[serde(default)]
    pub tool_family: ToolFamily,
    #[serde(default)]
    pub exit_bucket: ExitBucket,
    #[serde(default)]
    pub policy_reason: PolicyReason,
    #[serde(default)]
    pub verification_state: VerificationState,
}

impl FailureSignature {
    pub const fn as_identity(self) -> [&'static str; 5] {
        [
            self.operation_class.as_str(),
            self.tool_family.as_str(),
            self.exit_bucket.as_str(),
            self.policy_reason.as_str(),
            self.verification_state.as_str(),
        ]
    }

    fn from_legacy(
        failure_class: FailureClass,
        activity_labels: &[String],
        evidence: &AgentImprovementEvidence,
        signals: &[String],
    ) -> Self {
        let tool_family = if activity_labels.iter().any(|label| label == "exec_command") {
            ToolFamily::ExecCommand
        } else if activity_labels.iter().any(|label| label == "apply_patch") {
            ToolFamily::ApplyPatch
        } else if activity_labels.iter().any(|label| label == "mcp") {
            ToolFamily::Mcp
        } else {
            ToolFamily::Unknown
        };
        let operation_class = match failure_class {
            FailureClass::CommandFailure => OperationClass::Command,
            FailureClass::ToolFailure => OperationClass::Tool,
            FailureClass::ModelFailure => OperationClass::Inference,
            FailureClass::PolicyBlocked => OperationClass::Policy,
            FailureClass::IncompleteVerification => OperationClass::Verification,
            FailureClass::Completed if evidence.terminal_operation_count > 0 => {
                OperationClass::Command
            }
            FailureClass::Completed => OperationClass::None,
            FailureClass::Cancelled | FailureClass::Unknown => OperationClass::Unknown,
        };
        let policy_reason = if signals
            .iter()
            .any(|signal| signal == "policy_boundary_observed")
        {
            PolicyReason::Other
        } else {
            PolicyReason::default()
        };
        let verification_state = if evidence.failed_verification_count > 0 {
            VerificationState::Failed
        } else if evidence.verification_step_count > 0 {
            VerificationState::Passed
        } else if evidence.verification_attempt_count > 0 {
            VerificationState::Missing
        } else {
            VerificationState::Unknown
        };
        Self {
            operation_class,
            tool_family,
            exit_bucket: match failure_class {
                FailureClass::CommandFailure => ExitBucket::NonZero,
                FailureClass::PolicyBlocked => ExitBucket::Blocked,
                FailureClass::Cancelled => ExitBucket::Cancelled,
                FailureClass::Completed if evidence.verification_step_count > 0 => {
                    ExitBucket::Success
                }
                _ => ExitBucket::Unknown,
            },
            policy_reason,
            verification_state,
        }
    }
}

/// Scope that can safely be used for filtering without persisting paths.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExperienceScope {
    #[serde(default)]
    pub workspace_key: Option<String>,
    #[serde(default)]
    pub toolchain_family: Option<ToolchainFamily>,
    #[serde(default)]
    pub model_family: Option<ModelFamily>,
}

impl ExperienceScope {
    /// Derives an opaque workspace identity. The path itself is never returned
    /// or serialized by this function.
    pub fn for_workspace_path(path: &str) -> Self {
        Self {
            workspace_key: Some(workspace_scope_key(path)),
            ..Self::default()
        }
    }

    pub(crate) fn is_bounded(&self) -> bool {
        self.workspace_key
            .as_deref()
            .is_none_or(is_valid_workspace_key)
    }

    fn as_identity(&self) -> String {
        format!(
            "{}\u{1f}{}\u{1f}{}",
            self.workspace_key.as_deref().unwrap_or("unknown"),
            self.toolchain_family
                .map(ToolchainFamily::as_str)
                .unwrap_or("unknown"),
            self.model_family
                .map(ModelFamily::as_str)
                .unwrap_or("unknown"),
        )
    }
}

/// A bounded recurrence summary for one deduplicated lesson.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recurrence {
    pub count: u32,
    pub first_seen_at_unix_ms: i64,
    pub last_seen_at_unix_ms: i64,
}

/// Descriptive adoption evidence. Unknown is retained when no causal linkage
/// can be established from local traces.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdoptionEvidence {
    #[serde(default)]
    pub exposure_count: u32,
    #[serde(default)]
    pub feedback: FeedbackState,
    #[serde(default)]
    pub followed: AttributionState,
    #[serde(default)]
    pub verified: AttributionState,
    #[serde(default)]
    pub last_exposed_at_unix_ms: Option<i64>,
}

/// A derived, reviewable unit of historical execution experience.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExperienceCard {
    pub schema_version: u32,
    pub card_id: String,
    /// Stable content identity used to merge recurring observations.
    #[serde(default)]
    pub lesson_id: String,
    pub source_trace_id: String,
    /// Bounded provenance for recurring observations of the same lesson.
    #[serde(default)]
    pub source_trace_ids: Vec<String>,
    pub created_at_unix_ms: i64,
    pub task_family: TaskFamily,
    pub failure_class: FailureClass,
    #[serde(default)]
    pub status: CardStatus,
    pub evidence: AgentImprovementEvidence,
    /// Opaque hashes of a small set of normalized task terms used for retrieval.
    /// Raw prompt terms are never persisted in this field.
    #[serde(default)]
    pub topic_hashes: Vec<String>,
    /// Fixed, bounded runtime categories such as `exec_command` or `mcp`.
    #[serde(default)]
    pub activity_labels: Vec<String>,
    /// Short, bounded labels such as `verification_missing` or
    /// `policy_boundary_observed`.
    pub signals: Vec<String>,
    pub recommendation: String,
    pub verification: Vec<String>,
    #[serde(default)]
    pub scope: ExperienceScope,
    #[serde(default)]
    pub failure_signature: FailureSignature,
    #[serde(default)]
    pub evidence_profile: EvidenceProfile,
    #[serde(default)]
    pub confidence: ConfidenceLevel,
    #[serde(default)]
    pub recurrence: Recurrence,
    #[serde(default)]
    pub adoption: AdoptionEvidence,
    /// Optional bounded intervention projection. Legacy cards without this
    /// field remain readable and continue through the legacy retrieval lane.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intervention: Option<Intervention>,
}

impl ExperienceCard {
    /// Builds a card with deterministic IDs derived from non-secret fields.
    pub fn new(
        source_trace_id: impl Into<String>,
        created_at_unix_ms: i64,
        task_family: TaskFamily,
        failure_class: FailureClass,
        evidence: AgentImprovementEvidence,
        signals: Vec<String>,
        recommendation: impl Into<String>,
        verification: Vec<String>,
    ) -> Self {
        Self::new_with_details(
            source_trace_id,
            created_at_unix_ms,
            task_family,
            failure_class,
            evidence,
            Vec::new(),
            Vec::new(),
            signals,
            recommendation,
            verification,
        )
    }

    pub(crate) fn new_with_details(
        source_trace_id: impl Into<String>,
        created_at_unix_ms: i64,
        task_family: TaskFamily,
        failure_class: FailureClass,
        evidence: AgentImprovementEvidence,
        topic_hashes: Vec<String>,
        activity_labels: Vec<String>,
        signals: Vec<String>,
        recommendation: impl Into<String>,
        verification: Vec<String>,
    ) -> Self {
        let evidence_profile = EvidenceProfile::from_legacy(&signals, failure_class, &verification);
        let failure_signature =
            FailureSignature::from_legacy(failure_class, &activity_labels, &evidence, &signals);
        let confidence = if evidence_profile.fact_count() > 0 {
            ConfidenceLevel::High
        } else if failure_class == FailureClass::Unknown {
            ConfidenceLevel::Unknown
        } else {
            ConfidenceLevel::Medium
        };
        Self::new_with_rich_details(
            source_trace_id,
            created_at_unix_ms,
            task_family,
            failure_class,
            evidence,
            topic_hashes,
            activity_labels,
            signals,
            recommendation,
            verification,
            ExperienceScope::default(),
            failure_signature,
            evidence_profile,
            confidence,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new_with_rich_details(
        source_trace_id: impl Into<String>,
        created_at_unix_ms: i64,
        task_family: TaskFamily,
        failure_class: FailureClass,
        evidence: AgentImprovementEvidence,
        topic_hashes: Vec<String>,
        activity_labels: Vec<String>,
        signals: Vec<String>,
        recommendation: impl Into<String>,
        verification: Vec<String>,
        scope: ExperienceScope,
        failure_signature: FailureSignature,
        evidence_profile: EvidenceProfile,
        confidence: ConfidenceLevel,
    ) -> Self {
        let source_trace_id = bounded_text(source_trace_id.into(), MAX_SOURCE_TRACE_ID_CHARS);
        let topic_hashes = topic_hashes
            .into_iter()
            .filter(|hash| is_valid_topic_hash(hash))
            .take(MAX_TOPIC_HASHES)
            .collect::<Vec<_>>();
        let activity_labels = bounded_labels(activity_labels, MAX_ACTIVITY_LABELS);
        let signals = bounded_labels(signals, MAX_CARD_SIGNALS);
        let recommendation = bounded_text(recommendation.into(), MAX_RECOMMENDATION_CHARS);
        let verification = bounded_labels(verification, MAX_CARD_SIGNALS);
        let content_identity = content_identity(
            task_family,
            &topic_hashes,
            &activity_labels,
            &signals,
            &recommendation,
            &verification,
            scope.clone(),
            failure_signature,
            &evidence_profile,
        );
        let lesson_id = digest_id("lesson_", &content_identity);
        let card_id = digest_id("card_", &format!("{source_trace_id}\u{1f}{lesson_id}"));

        let mut card = Self {
            schema_version: EXPERIENCE_CARD_SCHEMA_VERSION,
            card_id,
            lesson_id,
            source_trace_id: source_trace_id.clone(),
            source_trace_ids: vec![source_trace_id],
            created_at_unix_ms,
            task_family,
            failure_class,
            status: CardStatus::Active,
            evidence,
            topic_hashes,
            activity_labels,
            signals,
            recommendation,
            verification,
            scope,
            failure_signature,
            evidence_profile,
            confidence,
            recurrence: Recurrence {
                count: 1,
                first_seen_at_unix_ms: created_at_unix_ms,
                last_seen_at_unix_ms: created_at_unix_ms,
            },
            adoption: AdoptionEvidence::default(),
            intervention: None,
        };
        card.intervention = Intervention::from_card(&card);
        card
    }

    /// Converts a version 1 card into the richer schema without inventing
    /// unsupported evidence.
    pub(crate) fn normalize_for_load(mut self) -> Option<Self> {
        match self.schema_version {
            LEGACY_EXPERIENCE_CARD_SCHEMA_VERSION => {
                let mut migrated = Self::new_with_details(
                    self.source_trace_id,
                    self.created_at_unix_ms,
                    self.task_family,
                    self.failure_class,
                    self.evidence,
                    self.topic_hashes,
                    self.activity_labels,
                    self.signals,
                    self.recommendation,
                    self.verification,
                );
                migrated.intervention = None;
                Some(migrated)
            }
            EXPERIENCE_CARD_SCHEMA_VERSION => {
                if self.source_trace_ids.is_empty() {
                    self.source_trace_ids.push(self.source_trace_id.clone());
                }
                Some(self)
            }
            _ => None,
        }
    }

    pub(crate) fn searchable_text(&self) -> String {
        format!(
            "{} {} {} {} {} {} {} {} {} {} {} {} {} {}",
            self.task_family.as_str(),
            self.failure_class.as_str(),
            self.failure_signature.operation_class.as_str(),
            self.failure_signature.tool_family.as_str(),
            self.failure_signature.exit_bucket.as_str(),
            self.failure_signature.verification_state.as_str(),
            self.scope
                .toolchain_family
                .map(ToolchainFamily::as_str)
                .unwrap_or("unknown"),
            self.activity_labels.join(" "),
            self.signals.join(" "),
            self.evidence_profile
                .facts
                .iter()
                .map(|fact| fact.as_str())
                .collect::<Vec<_>>()
                .join(" "),
            self.evidence_profile
                .observed_sequence
                .iter()
                .map(|step| step.as_str())
                .collect::<Vec<_>>()
                .join(" "),
            self.evidence_profile
                .interventions
                .iter()
                .map(|intervention| intervention.as_str())
                .collect::<Vec<_>>()
                .join(" "),
            self.recommendation,
            self.verification.join(" "),
        )
    }

    pub(crate) fn conflict_key(&self) -> String {
        let identity = format!(
            "{}\u{1f}{}\u{1f}{}\u{1f}{}",
            self.task_family.as_str(),
            self.scope.as_identity(),
            self.failure_signature
                .as_identity()
                .into_iter()
                .collect::<Vec<_>>()
                .join("\u{1f}"),
            self.failure_class.as_str(),
        );
        digest_id("conflict_", &identity)
    }

    pub(crate) fn is_bounded(&self) -> bool {
        self.schema_version == EXPERIENCE_CARD_SCHEMA_VERSION
            && is_safe_text(&self.card_id, MAX_LESSON_ID_CHARS)
            && is_safe_text(&self.lesson_id, MAX_LESSON_ID_CHARS)
            && is_safe_text(&self.source_trace_id, MAX_SOURCE_TRACE_ID_CHARS)
            && !self.source_trace_ids.is_empty()
            && self.source_trace_ids.len() <= MAX_SOURCE_TRACE_IDS
            && self
                .source_trace_ids
                .iter()
                .all(|trace_id| is_safe_text(trace_id, MAX_SOURCE_TRACE_ID_CHARS))
            && self.source_trace_ids.contains(&self.source_trace_id)
            && self.topic_hashes.len() <= MAX_TOPIC_HASHES
            && self
                .topic_hashes
                .iter()
                .all(|hash| is_valid_topic_hash(hash))
            && self.activity_labels.len() <= MAX_ACTIVITY_LABELS
            && self
                .activity_labels
                .iter()
                .all(|label| is_safe_text(label, MAX_CARD_LABEL_CHARS))
            && self.signals.len() <= MAX_CARD_SIGNALS
            && self
                .signals
                .iter()
                .all(|signal| is_safe_text(signal, MAX_CARD_LABEL_CHARS))
            && is_safe_text(&self.recommendation, MAX_RECOMMENDATION_CHARS)
            && self.verification.len() <= MAX_CARD_SIGNALS
            && self
                .verification
                .iter()
                .all(|label| is_safe_text(label, MAX_CARD_LABEL_CHARS))
            && self.scope.is_bounded()
            && self.evidence_profile.is_bounded()
            && self.recurrence.count > 0
            && self
                .intervention
                .as_ref()
                .is_none_or(Intervention::is_bounded)
    }

    pub(crate) fn merge_with(&self, incoming: &Self) -> Self {
        let mut merged = self.clone();
        if self.status == CardStatus::Retired || incoming.status == CardStatus::Retired {
            merged.status = CardStatus::Retired;
        }
        merged.created_at_unix_ms = self.created_at_unix_ms.min(incoming.created_at_unix_ms);
        merged.source_trace_ids = merge_bounded(
            &self.source_trace_ids,
            &incoming.source_trace_ids,
            MAX_SOURCE_TRACE_IDS,
        );
        merged.evidence = merge_evidence(&self.evidence, &incoming.evidence);
        merged.signals = merge_bounded(&self.signals, &incoming.signals, MAX_CARD_SIGNALS);
        merged.activity_labels = merge_bounded(
            &self.activity_labels,
            &incoming.activity_labels,
            MAX_ACTIVITY_LABELS,
        );
        merged.verification =
            merge_bounded(&self.verification, &incoming.verification, MAX_CARD_SIGNALS);
        merged.evidence_profile = merge_profile(&self.evidence_profile, &incoming.evidence_profile);
        merged.confidence = self.confidence.max(incoming.confidence);
        merged.recurrence = Recurrence {
            count: self
                .recurrence
                .count
                .saturating_add(incoming.recurrence.count),
            first_seen_at_unix_ms: self
                .recurrence
                .first_seen_at_unix_ms
                .min(incoming.recurrence.first_seen_at_unix_ms),
            last_seen_at_unix_ms: self
                .recurrence
                .last_seen_at_unix_ms
                .max(incoming.recurrence.last_seen_at_unix_ms),
        };
        merged.adoption = merge_adoption(&self.adoption, &incoming.adoption);
        if self.intervention.is_some() || incoming.intervention.is_some() {
            merged.intervention = Intervention::from_card(&merged);
        }
        merged
    }

    pub(crate) fn with_status(&self, status: CardStatus) -> Self {
        let mut updated = self.clone();
        updated.status = status;
        updated
    }

    pub(crate) fn with_exposure(&self, timestamp_unix_ms: i64) -> Self {
        let mut updated = self.clone();
        updated.adoption.exposure_count = updated.adoption.exposure_count.saturating_add(1);
        updated.adoption.last_exposed_at_unix_ms = Some(
            updated
                .adoption
                .last_exposed_at_unix_ms
                .unwrap_or(timestamp_unix_ms)
                .max(timestamp_unix_ms),
        );
        updated
    }

    pub(crate) fn with_feedback(&self, feedback: FeedbackState) -> Self {
        let mut updated = self.clone();
        updated.adoption.feedback = merge_feedback(updated.adoption.feedback, feedback);
        updated
    }

    pub(crate) fn with_attribution(
        &self,
        followed: AttributionState,
        verified: AttributionState,
    ) -> Self {
        let mut updated = self.clone();
        updated.adoption.followed = merge_attribution(updated.adoption.followed, followed);
        updated.adoption.verified = merge_attribution(updated.adoption.verified, verified);
        updated
    }
}

/// Returns an opaque, deterministic workspace key.
pub fn workspace_scope_key(path: &str) -> String {
    digest_id("workspace_", path)
}

/// Returns opaque retrieval keys for normalized terms in a task description.
/// The result is bounded and deliberately omits common instruction words.
pub(crate) fn topic_hashes_for_text(text: &str) -> Vec<String> {
    let mut hashes = BTreeSet::new();
    for token in text
        .split(|character: char| !character.is_ascii_alphanumeric())
        .map(str::to_ascii_lowercase)
        .filter(|token| token.len() >= 3)
        .filter(|token| !TOPIC_STOP_WORDS.contains(&token.as_str()))
    {
        hashes.insert(hash_topic_token(&token));
        if hashes.len() == MAX_TOPIC_HASHES {
            break;
        }
    }
    hashes.into_iter().collect()
}

pub(crate) fn hash_topic_token(token: &str) -> String {
    let digest = Sha256::digest(format!("codex-agent-improvement-topic-v1:{token}").as_bytes());
    let encoded = digest[..12]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("topic_{encoded}")
}

fn content_identity(
    task_family: TaskFamily,
    topic_hashes: &[String],
    activity_labels: &[String],
    signals: &[String],
    recommendation: &str,
    verification: &[String],
    scope: ExperienceScope,
    failure_signature: FailureSignature,
    evidence_profile: &EvidenceProfile,
) -> String {
    [
        "lesson-v2".to_string(),
        task_family.as_str().to_string(),
        topic_hashes.join("\u{1f}"),
        activity_labels.join("\u{1f}"),
        signals.join("\u{1f}"),
        recommendation.to_string(),
        verification.join("\u{1f}"),
        scope.as_identity(),
        failure_signature.as_identity().join("\u{1f}"),
        profile_identity(evidence_profile),
    ]
    .join("\u{1f}")
}

fn profile_identity(profile: &EvidenceProfile) -> String {
    [
        profile
            .observed_sequence
            .iter()
            .map(|step| step.as_str())
            .collect::<Vec<_>>()
            .join("\u{1f}"),
        profile
            .facts
            .iter()
            .map(|fact| fact.as_str())
            .collect::<Vec<_>>()
            .join("\u{1f}"),
        profile
            .hypotheses
            .iter()
            .map(|hypothesis| hypothesis.as_str())
            .collect::<Vec<_>>()
            .join("\u{1f}"),
        profile
            .interventions
            .iter()
            .map(|intervention| intervention.as_str())
            .collect::<Vec<_>>()
            .join("\u{1f}"),
        profile
            .outcomes
            .iter()
            .map(|outcome| outcome.as_str())
            .collect::<Vec<_>>()
            .join("\u{1f}"),
    ]
    .join("\u{1e}")
}

fn digest_id(prefix: &str, identity: &str) -> String {
    let digest = Sha256::digest(identity.as_bytes());
    let suffix = digest[..12]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("{prefix}{suffix}")
}

fn bounded_labels(values: Vec<String>, max_items: usize) -> Vec<String> {
    values
        .into_iter()
        .map(|value| bounded_text(value, MAX_CARD_LABEL_CHARS))
        .filter(|value| !value.is_empty())
        .take(max_items)
        .collect()
}

fn bounded_text(value: String, max_chars: usize) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_control() || matches!(character, '<' | '>') {
                ' '
            } else {
                character
            }
        })
        .take(max_chars)
        .collect::<String>()
        .trim()
        .to_string()
}

fn is_safe_text(value: &str, max_chars: usize) -> bool {
    !value.is_empty()
        && value.chars().count() <= max_chars
        && !value
            .chars()
            .any(|character| character.is_control() || matches!(character, '<' | '>'))
}

fn is_valid_topic_hash(value: &str) -> bool {
    value.len() == "topic_".len() + 24
        && value.starts_with("topic_")
        && value["topic_".len()..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
}

fn is_valid_workspace_key(value: &str) -> bool {
    value.len() == "workspace_".len() + 24
        && value.starts_with("workspace_")
        && value["workspace_".len()..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
}

fn merge_bounded<T: Clone + PartialEq>(left: &[T], right: &[T], max_items: usize) -> Vec<T> {
    let mut merged = left.to_vec();
    for value in right {
        push_unique(&mut merged, value.clone(), max_items);
    }
    merged
}

fn push_unique<T: PartialEq>(values: &mut Vec<T>, value: T, max_items: usize) {
    if values.len() < max_items && !values.iter().any(|existing| existing == &value) {
        values.push(value);
    }
}

fn merge_evidence(
    left: &AgentImprovementEvidence,
    right: &AgentImprovementEvidence,
) -> AgentImprovementEvidence {
    AgentImprovementEvidence {
        turn_count: left.turn_count.saturating_add(right.turn_count),
        inference_count: left.inference_count.saturating_add(right.inference_count),
        failed_inference_count: left
            .failed_inference_count
            .saturating_add(right.failed_inference_count),
        tool_count: left.tool_count.saturating_add(right.tool_count),
        failed_tool_count: left
            .failed_tool_count
            .saturating_add(right.failed_tool_count),
        terminal_operation_count: left
            .terminal_operation_count
            .saturating_add(right.terminal_operation_count),
        failed_terminal_operation_count: left
            .failed_terminal_operation_count
            .saturating_add(right.failed_terminal_operation_count),
        verification_step_count: left
            .verification_step_count
            .saturating_add(right.verification_step_count),
        verification_attempt_count: left
            .verification_attempt_count
            .saturating_add(right.verification_attempt_count),
        failed_verification_count: left
            .failed_verification_count
            .saturating_add(right.failed_verification_count),
        token_usage: TokenUsageSummary {
            input_tokens: left
                .token_usage
                .input_tokens
                .saturating_add(right.token_usage.input_tokens),
            output_tokens: left
                .token_usage
                .output_tokens
                .saturating_add(right.token_usage.output_tokens),
            reasoning_output_tokens: left
                .token_usage
                .reasoning_output_tokens
                .saturating_add(right.token_usage.reasoning_output_tokens),
        },
    }
}

fn merge_profile(left: &EvidenceProfile, right: &EvidenceProfile) -> EvidenceProfile {
    EvidenceProfile {
        observed_sequence: merge_bounded(
            &left.observed_sequence,
            &right.observed_sequence,
            MAX_CARD_SIGNALS,
        ),
        facts: merge_bounded(&left.facts, &right.facts, MAX_CARD_SIGNALS),
        hypotheses: merge_bounded(&left.hypotheses, &right.hypotheses, MAX_CARD_SIGNALS),
        interventions: merge_bounded(&left.interventions, &right.interventions, MAX_CARD_SIGNALS),
        outcomes: merge_bounded(&left.outcomes, &right.outcomes, MAX_CARD_SIGNALS),
    }
}

fn merge_adoption(left: &AdoptionEvidence, right: &AdoptionEvidence) -> AdoptionEvidence {
    AdoptionEvidence {
        exposure_count: left.exposure_count.saturating_add(right.exposure_count),
        feedback: merge_feedback(left.feedback, right.feedback),
        followed: merge_attribution(left.followed, right.followed),
        verified: merge_attribution(left.verified, right.verified),
        last_exposed_at_unix_ms: match (left.last_exposed_at_unix_ms, right.last_exposed_at_unix_ms)
        {
            (Some(left), Some(right)) => Some(left.max(right)),
            (Some(value), None) | (None, Some(value)) => Some(value),
            (None, None) => None,
        },
    }
}

fn merge_feedback(left: FeedbackState, right: FeedbackState) -> FeedbackState {
    match (left, right) {
        (FeedbackState::Unknown, value) | (value, FeedbackState::Unknown) => value,
        (left, right) if left == right => left,
        _ => FeedbackState::Mixed,
    }
}

fn merge_attribution(left: AttributionState, right: AttributionState) -> AttributionState {
    match (left, right) {
        (AttributionState::Unknown, value) | (value, AttributionState::Unknown) => value,
        (left, right) if left == right => left,
        _ => AttributionState::Mixed,
    }
}

fn hypothesis_for_failure(failure_class: FailureClass) -> HypothesisKind {
    match failure_class {
        FailureClass::CommandFailure => HypothesisKind::LikelyCommandEnvironmentIssue,
        FailureClass::ToolFailure => HypothesisKind::LikelyToolPrecondition,
        FailureClass::ModelFailure => HypothesisKind::LikelyModelBoundary,
        FailureClass::PolicyBlocked => HypothesisKind::LikelyPolicyConstraint,
        FailureClass::IncompleteVerification => HypothesisKind::LikelyMissingVerification,
        FailureClass::Completed | FailureClass::Cancelled | FailureClass::Unknown => {
            HypothesisKind::Unknown
        }
    }
}

fn intervention_for_failure(failure_class: FailureClass) -> InterventionKind {
    match failure_class {
        FailureClass::CommandFailure => InterventionKind::ReproduceFirstFailingCommand,
        FailureClass::ToolFailure => InterventionKind::CheckToolPreconditions,
        FailureClass::ModelFailure => InterventionKind::ReduceToVerifiableStep,
        FailureClass::PolicyBlocked => InterventionKind::RequestRequiredAuthority,
        FailureClass::Cancelled => InterventionKind::ResumeFromCheckpoint,
        FailureClass::IncompleteVerification => InterventionKind::RunTargetedTests,
        FailureClass::Completed => InterventionKind::PreserveWorkflow,
        FailureClass::Unknown => InterventionKind::InspectReducedTrace,
    }
}

fn outcome_for_failure(failure_class: FailureClass, verification_passed: bool) -> OutcomeKind {
    if verification_passed {
        return OutcomeKind::VerificationPassed;
    }
    match failure_class {
        FailureClass::Completed => OutcomeKind::Completed,
        FailureClass::PolicyBlocked => OutcomeKind::PolicyBlocked,
        FailureClass::Cancelled => OutcomeKind::Cancelled,
        FailureClass::IncompleteVerification => OutcomeKind::VerificationMissing,
        FailureClass::Unknown => OutcomeKind::Unknown,
        _ => OutcomeKind::FailureObserved,
    }
}

fn has_any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| {
        if needle.contains(' ') {
            text.contains(needle)
        } else {
            text.split(|character: char| !character.is_ascii_alphanumeric())
                .any(|token| token == *needle)
        }
    })
}
