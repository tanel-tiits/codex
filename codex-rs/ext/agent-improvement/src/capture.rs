use std::io;
use std::path::Path;

use codex_rollout_trace::replay_bundle;

use crate::CandidateTrigger;
use crate::ExperienceCandidate;
use crate::append_candidate;
use crate::summarize_rollout;

/// Result of reducing one automatically observed rollout into the review queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CandidateCaptureOutcome {
    Added(ExperienceCandidate),
    AlreadyPresent,
    SkippedNoEvidence,
}

/// Replays one local trace and appends only its bounded derived candidate.
pub(crate) fn capture_candidate_from_bundle(
    bundle_dir: &Path,
    store_root: &Path,
    trigger: CandidateTrigger,
    source_turn_id: Option<String>,
    max_candidates: usize,
) -> io::Result<CandidateCaptureOutcome> {
    let trace = replay_bundle(bundle_dir).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("replay rollout trace for agent-improvement candidate: {error:#}"),
        )
    })?;
    let card = summarize_rollout(&trace);
    if !has_observable_evidence(&card) {
        return Ok(CandidateCaptureOutcome::SkippedNoEvidence);
    }

    let candidate = ExperienceCandidate::from_card(
        card.clone(),
        card.created_at_unix_ms,
        trigger,
        source_turn_id,
    );
    if append_candidate(store_root, &candidate, max_candidates)? {
        Ok(CandidateCaptureOutcome::Added(candidate))
    } else {
        Ok(CandidateCaptureOutcome::AlreadyPresent)
    }
}

fn has_observable_evidence(card: &crate::ExperienceCard) -> bool {
    let evidence = &card.evidence;
    evidence.turn_count > 0
        || evidence.inference_count > 0
        || evidence.tool_count > 0
        || evidence.terminal_operation_count > 0
}
