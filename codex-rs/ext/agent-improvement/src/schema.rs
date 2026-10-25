use std::collections::BTreeSet;

use serde::Deserialize;
use serde::Serialize;
use sha2::Digest;
use sha2::Sha256;

/// Version of the on-disk experience-card schema.
pub const EXPERIENCE_CARD_SCHEMA_VERSION: u32 = 1;

pub(crate) const MAX_CARD_SIGNALS: usize = 8;
pub(crate) const MAX_ACTIVITY_LABELS: usize = 8;
pub(crate) const MAX_TOPIC_HASHES: usize = 16;
const MAX_CARD_LABEL_CHARS: usize = 64;
const MAX_RECOMMENDATION_CHARS: usize = 512;
const MAX_SOURCE_TRACE_ID_CHARS: usize = 128;

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
    pub token_usage: TokenUsageSummary,
}

/// A small token-usage projection that avoids carrying provider payloads.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsageSummary {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_output_tokens: u64,
}

/// A derived, reviewable unit of historical execution experience.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExperienceCard {
    pub schema_version: u32,
    pub card_id: String,
    pub source_trace_id: String,
    pub created_at_unix_ms: i64,
    pub task_family: TaskFamily,
    pub failure_class: FailureClass,
    pub evidence: AgentImprovementEvidence,
    /// Opaque hashes of a small set of normalized task terms used for retrieval.
    /// Raw prompt terms are never persisted in this field.
    #[serde(default)]
    pub topic_hashes: Vec<String>,
    /// Fixed, bounded runtime categories such as `exec_command` or `mcp`.
    #[serde(default)]
    pub activity_labels: Vec<String>,
    /// Short, bounded labels such as `verification_missing` or `policy_boundary_observed`.
    pub signals: Vec<String>,
    pub recommendation: String,
    pub verification: Vec<String>,
}

impl ExperienceCard {
    /// Builds a card with a deterministic ID derived from non-secret fields.
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
        let stable_identity = format!(
            "{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}",
            source_trace_id,
            task_family.as_str(),
            failure_class.as_str(),
            topic_hashes.join("\u{1f}"),
            activity_labels.join("\u{1f}"),
            signals.join("\u{1f}"),
            verification.join("\u{1f}"),
        );
        let digest = Sha256::digest(stable_identity.as_bytes());
        let card_id = digest[..12]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();

        Self {
            schema_version: EXPERIENCE_CARD_SCHEMA_VERSION,
            card_id: format!("card_{card_id}"),
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
        }
    }

    pub(crate) fn searchable_text(&self) -> String {
        format!(
            "{} {} {} {} {} {}",
            self.task_family.as_str(),
            self.failure_class.as_str(),
            self.activity_labels.join(" "),
            self.signals.join(" "),
            self.recommendation,
            self.verification.join(" "),
        )
    }

    pub(crate) fn is_bounded(&self) -> bool {
        self.schema_version == EXPERIENCE_CARD_SCHEMA_VERSION
            && is_safe_text(&self.card_id, MAX_SOURCE_TRACE_ID_CHARS)
            && is_safe_text(&self.source_trace_id, MAX_SOURCE_TRACE_ID_CHARS)
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
    }
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
