//! Privacy-bounded local experience cards for agent improvement.
//!
//! The first layer of this crate is deliberately independent of the Codex
//! runtime. It stores derived, reviewable evidence rather than replaying raw
//! prompts, model responses, paths, or terminal output into future turns.

mod candidate;
mod candidates;
mod capture;
mod extension;
mod schema;
mod store;
mod summarize;

pub use candidate::{
    CandidateStatus, CandidateTrigger, EXPERIENCE_CANDIDATE_SCHEMA_VERSION, ExperienceCandidate,
};
pub use candidates::{
    CANDIDATE_STORE_FILE_NAME, append_candidate, candidates_path, find_candidate, load_candidates,
    update_candidate_status,
};
pub use extension::{AgentImprovementExtensionConfig, install};
pub use schema::{
    AgentImprovementEvidence, EXPERIENCE_CARD_SCHEMA_VERSION, ExperienceCard, FailureClass,
    TaskFamily, TokenUsageSummary,
};
pub use store::{
    STORE_FILE_NAME, append_card, cards_path, load_cards, render_cards, select_relevant,
};
pub use summarize::summarize_rollout;

#[cfg(test)]
mod candidate_tests;

#[cfg(test)]
mod candidates_tests;

#[cfg(test)]
mod schema_tests;

#[cfg(test)]
mod store_tests;

#[cfg(test)]
mod summarize_tests;

#[cfg(test)]
mod extension_tests;
