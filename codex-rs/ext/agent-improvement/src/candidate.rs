use serde::Deserialize;
use serde::Serialize;
use sha2::Digest;
use sha2::Sha256;

use crate::ExperienceCard;

/// Version of the on-disk candidate-review schema.
pub const EXPERIENCE_CANDIDATE_SCHEMA_VERSION: u32 = 1;

const MAX_CANDIDATE_ID_CHARS: usize = 128;
const MAX_SOURCE_TRACE_ID_CHARS: usize = 128;
const MAX_SOURCE_TURN_ID_CHARS: usize = 128;

/// Human-review state for one automatically captured candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateStatus {
    Pending,
    Accepted,
    Rejected,
    Expired,
}

impl CandidateStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
            Self::Expired => "expired",
        }
    }
}

/// Lifecycle event that caused Codex to consider a candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateTrigger {
    ThreadStop,
    TurnStop,
    TurnError,
    TurnAbort,
}

impl CandidateTrigger {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ThreadStop => "thread_stop",
            Self::TurnStop => "turn_stop",
            Self::TurnError => "turn_error",
            Self::TurnAbort => "turn_abort",
        }
    }
}

/// A pending, reviewable projection of one observed rollout.
///
/// Candidates are intentionally separate from `ExperienceCard`s consumed by
/// prompt retrieval. A pending candidate has no runtime authority and is never
/// injected into a turn until a user explicitly accepts it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExperienceCandidate {
    pub schema_version: u32,
    pub candidate_id: String,
    pub source_trace_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_turn_id: Option<String>,
    pub created_at_unix_ms: i64,
    pub trigger: CandidateTrigger,
    pub status: CandidateStatus,
    pub card: ExperienceCard,
}

impl ExperienceCandidate {
    /// Creates a deterministic pending candidate from a derived card.
    pub fn from_card(
        card: ExperienceCard,
        created_at_unix_ms: i64,
        trigger: CandidateTrigger,
        source_turn_id: Option<String>,
    ) -> Self {
        let source_trace_id = bounded_text(card.source_trace_id.clone(), MAX_SOURCE_TRACE_ID_CHARS);
        let source_turn_id = source_turn_id
            .map(|turn_id| bounded_text(turn_id, MAX_SOURCE_TURN_ID_CHARS))
            .filter(|turn_id| !turn_id.is_empty());
        let stable_identity = format!(
            "{}\u{1f}{}\u{1f}{}\u{1f}{}",
            source_trace_id,
            source_turn_id.as_deref().unwrap_or(""),
            trigger.as_str(),
            card.card_id,
        );
        let digest = Sha256::digest(stable_identity.as_bytes());
        let candidate_suffix = digest[..12]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();

        Self {
            schema_version: EXPERIENCE_CANDIDATE_SCHEMA_VERSION,
            candidate_id: format!("candidate_{candidate_suffix}"),
            source_trace_id,
            source_turn_id,
            created_at_unix_ms,
            trigger,
            status: CandidateStatus::Pending,
            card,
        }
    }

    pub(crate) fn with_status(&self, status: CandidateStatus) -> Self {
        let mut candidate = self.clone();
        candidate.status = status;
        candidate
    }

    pub(crate) fn is_bounded(&self) -> bool {
        self.schema_version == EXPERIENCE_CANDIDATE_SCHEMA_VERSION
            && is_safe_text(&self.candidate_id, MAX_CANDIDATE_ID_CHARS)
            && is_safe_text(&self.source_trace_id, MAX_SOURCE_TRACE_ID_CHARS)
            && self
                .source_turn_id
                .as_deref()
                .is_none_or(|turn_id| is_safe_text(turn_id, MAX_SOURCE_TURN_ID_CHARS))
            && self.card.is_bounded()
            && self.card.source_trace_id == self.source_trace_id
    }
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
