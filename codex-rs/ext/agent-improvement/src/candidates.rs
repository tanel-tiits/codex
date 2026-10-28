use std::collections::BTreeMap;
use std::fs::File;
use std::fs::OpenOptions;
use std::io;
use std::io::BufRead;
use std::io::BufReader;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::from_str;

use crate::CandidateStatus;
use crate::ExperienceCandidate;

const MAX_CANDIDATE_STORE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_CANDIDATE_LINE_BYTES: usize = 48 * 1024;
const MAX_LOADED_CANDIDATES: usize = 4096;
const MAX_CANDIDATE_RECORDS: usize = 8192;

/// Append-only review-queue file name under the configured agent-improvement root.
pub const CANDIDATE_STORE_FILE_NAME: &str = "candidates.jsonl";

/// Returns the candidate store path for a Codex state directory.
pub fn candidates_path(root: &Path) -> PathBuf {
    root.join(CANDIDATE_STORE_FILE_NAME)
}

/// Loads the latest valid record for each candidate ID.
pub fn load_candidates(root: &Path) -> io::Result<Vec<ExperienceCandidate>> {
    let path = candidates_path(root);
    let file = match File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    if file.metadata()?.len() > MAX_CANDIDATE_STORE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "agent-improvement candidate store exceeds the size limit",
        ));
    }

    let mut candidates = BTreeMap::new();
    let mut skipped_count = 0usize;
    let mut records_seen = 0usize;
    for line in BufReader::new(file).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if records_seen >= MAX_CANDIDATE_RECORDS || line.len() > MAX_CANDIDATE_LINE_BYTES {
            skipped_count = skipped_count.saturating_add(1);
            continue;
        }
        records_seen += 1;
        match from_str::<ExperienceCandidate>(&line)
            .ok()
            .and_then(|mut candidate| {
                candidate.card = candidate.card.normalize_for_load()?;
                Some(candidate)
            }) {
            Some(candidate) if candidate.is_bounded() => {
                if candidates.len() < MAX_LOADED_CANDIDATES
                    || candidates.contains_key(&candidate.candidate_id)
                {
                    candidates.insert(candidate.candidate_id.clone(), candidate);
                } else {
                    skipped_count = skipped_count.saturating_add(1);
                }
            }
            Some(_) | None => skipped_count = skipped_count.saturating_add(1),
        }
    }

    if skipped_count > 0 {
        tracing::warn!(
            skipped_count,
            "skipped invalid or oversized agent-improvement candidates"
        );
    }

    Ok(candidates.into_values().collect())
}

/// Appends a pending candidate unless its deterministic ID or source turn
/// episode already exists.
pub fn append_candidate(
    root: &Path,
    candidate: &ExperienceCandidate,
    max_candidates: usize,
) -> io::Result<bool> {
    if !candidate.is_bounded() || candidate.status != CandidateStatus::Pending {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "only bounded pending candidates may be appended",
        ));
    }

    let existing = load_candidates(root)?;
    if existing
        .iter()
        .any(|existing| same_capture_episode(existing, candidate))
    {
        return Ok(false);
    }
    let pending_count = existing
        .iter()
        .filter(|candidate| candidate.status == CandidateStatus::Pending)
        .count();
    if pending_count >= max_candidates {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "agent-improvement candidate queue is full",
        ));
    }

    append_record(root, candidate)
}

/// Finds one candidate by its stable ID.
pub fn find_candidate(root: &Path, candidate_id: &str) -> io::Result<Option<ExperienceCandidate>> {
    Ok(load_candidates(root)?
        .into_iter()
        .find(|candidate| candidate.candidate_id == candidate_id))
}

/// Moves a pending candidate to a terminal review state.
pub fn update_candidate_status(
    root: &Path,
    candidate_id: &str,
    status: CandidateStatus,
) -> io::Result<Option<ExperienceCandidate>> {
    let Some(candidate) = find_candidate(root, candidate_id)? else {
        return Ok(None);
    };
    if candidate.status == status {
        return Ok(Some(candidate));
    }
    if candidate.status != CandidateStatus::Pending {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "terminal candidate status cannot be changed",
        ));
    }
    if status == CandidateStatus::Pending {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "candidate status updates must be terminal",
        ));
    }

    let updated = candidate.with_status(status);
    append_record(root, &updated)?;
    Ok(Some(updated))
}

fn append_record(root: &Path, candidate: &ExperienceCandidate) -> io::Result<bool> {
    let encoded = serde_json::to_vec(candidate)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if encoded.len() > MAX_CANDIDATE_LINE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "agent-improvement candidate exceeds the line size limit",
        ));
    }
    let current_size = match std::fs::metadata(candidates_path(root)) {
        Ok(metadata) => metadata.len(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => 0,
        Err(error) => return Err(error),
    };
    if current_size.saturating_add(encoded.len() as u64 + 1) > MAX_CANDIDATE_STORE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "agent-improvement candidate store would exceed the size limit",
        ));
    }

    std::fs::create_dir_all(root)?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(candidates_path(root))?;
    // Keep one JSONL record in one append write so concurrent local writers do
    // not interleave the payload and newline into an unparsable record.
    let mut record = encoded;
    record.push(b'\n');
    file.write_all(&record)?;
    Ok(true)
}

fn same_capture_episode(left: &ExperienceCandidate, right: &ExperienceCandidate) -> bool {
    if left.candidate_id == right.candidate_id {
        return true;
    }

    left.source_trace_id == right.source_trace_id
        && left.source_turn_id.is_some()
        && left.source_turn_id == right.source_turn_id
}
