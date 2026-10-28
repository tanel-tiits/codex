use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::fs::OpenOptions;
use std::io;
use std::io::BufRead;
use std::io::BufReader;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use codex_utils_string::truncate_middle_with_token_budget;
use serde::Serialize;
use serde_json::from_str;

use crate::TaskFamily;
use crate::schema::{
    AttributionState, CardStatus, ConfidenceLevel, ExperienceCard, FailureClass, FeedbackState,
    ModelFamily, ToolFamily, ToolchainFamily, topic_hashes_for_text,
};

const MAX_STORE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_CARD_LINE_BYTES: usize = 32 * 1024;
const MAX_LOADED_CARDS: usize = 4096;
const MILLIS_PER_DAY: i64 = 24 * 60 * 60 * 1000;
const SEARCH_STOP_WORDS: &[&str] = &[
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
    "fix",
    "for",
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

/// Append-only store file name under the configured agent-improvement root.
pub const STORE_FILE_NAME: &str = "cards.jsonl";

/// Structured filters used by retrieval. Every field is a bounded category or
/// opaque key; no prompt, command, model identifier, or path is retained.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExperienceRetrievalContext {
    pub workspace_key: Option<String>,
    pub task_family: Option<TaskFamily>,
    pub failure_class: Option<FailureClass>,
    pub tool_family: Option<ToolFamily>,
    pub toolchain_family: Option<ToolchainFamily>,
    pub model_family: Option<ModelFamily>,
}

impl ExperienceRetrievalContext {
    /// Derives conservative structured filters from the current task text.
    pub fn from_query(query: &str, workspace_path: Option<&str>) -> Self {
        Self {
            workspace_key: workspace_path.map(crate::workspace_scope_key),
            task_family: Some(TaskFamily::from_text(query)),
            failure_class: failure_class_for_query(query),
            tool_family: tool_family_for_query(query),
            toolchain_family: toolchain_family_for_query(query),
            model_family: None,
        }
    }
}

/// Returns the card store path for a Codex state directory.
pub fn cards_path(root: &Path) -> PathBuf {
    root.join(STORE_FILE_NAME)
}

/// Loads valid cards and skips malformed lines as a recoverable local-state issue.
///
/// The file is append-only, so the latest record for a card ID wins. Records
/// from schema v1 are normalized to v2 before they are exposed. Recurring
/// records with the same lesson identity are merged so old stores converge in
/// memory even before a future compaction pass is added.
pub fn load_cards(root: &Path) -> io::Result<Vec<ExperienceCard>> {
    let path = cards_path(root);
    let file = match File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    if file.metadata()?.len() > MAX_STORE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "agent-improvement card store exceeds the size limit",
        ));
    }

    let mut latest_by_card_id = BTreeMap::new();
    let mut skipped_count = 0usize;
    for line in BufReader::new(file).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if line.len() > MAX_CARD_LINE_BYTES {
            skipped_count += 1;
            continue;
        }
        match from_str::<ExperienceCard>(&line)
            .ok()
            .and_then(ExperienceCard::normalize_for_load)
        {
            Some(card) if card.is_bounded() => {
                if latest_by_card_id.len() < MAX_LOADED_CARDS
                    || latest_by_card_id.contains_key(&card.card_id)
                {
                    latest_by_card_id.insert(card.card_id.clone(), card);
                } else {
                    skipped_count += 1;
                }
            }
            Some(_) | None => skipped_count += 1,
        }
    }

    let mut merged_by_lesson = BTreeMap::new();
    for card in latest_by_card_id.into_values() {
        merged_by_lesson
            .entry(card.lesson_id.clone())
            .and_modify(|existing: &mut ExperienceCard| {
                *existing = existing.merge_with(&card);
            })
            .or_insert(card);
    }
    let cards = merged_by_lesson.into_values().collect::<Vec<_>>();

    if skipped_count > 0 {
        tracing::warn!(
            skipped_count,
            "skipped invalid, unknown-schema, or oversized agent-improvement experience cards"
        );
    }

    Ok(cards)
}

/// Appends one card, merging a recurring lesson when its content identity is
/// already present. A changed record with the same card ID is treated as an
/// append-only update for adoption or lifecycle metadata.
pub fn append_card(root: &Path, card: &ExperienceCard) -> io::Result<bool> {
    if !card.is_bounded() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "agent-improvement card exceeds the schema bounds",
        ));
    }
    let existing = load_cards(root)?;
    let record = if let Some(previous) = existing
        .iter()
        .find(|existing| existing.card_id == card.card_id)
    {
        if previous == card {
            return Ok(false);
        }
        card.clone()
    } else if let Some(previous) = existing
        .iter()
        .find(|existing| existing.lesson_id == card.lesson_id)
    {
        let merged = previous.merge_with(card);
        if &merged == previous {
            return Ok(false);
        }
        merged
    } else {
        card.clone()
    };
    if !record.is_bounded() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "merged agent-improvement card exceeds the schema bounds",
        ));
    }
    append_record(root, &record)
}

/// Marks one card retired. Retired records remain auditable in the append-only
/// store but are excluded from future retrieval.
pub fn retire_card(root: &Path, card_id: &str) -> io::Result<bool> {
    let cards = load_cards(root)?;
    let Some(card) = cards.iter().find(|card| card.card_id == card_id) else {
        return Ok(false);
    };
    if card.status == CardStatus::Retired {
        return Ok(false);
    }
    append_card(root, &card.with_status(CardStatus::Retired))
}

/// Retires a bounded set of cards in one maintenance pass. Retired records
/// remain auditable in the append-only store but are excluded from retrieval.
pub fn retire_cards(root: &Path, card_ids: &[String]) -> io::Result<usize> {
    let wanted = card_ids.iter().collect::<BTreeSet<_>>();
    if wanted.is_empty() {
        return Ok(0);
    }
    let cards = load_cards(root)?;
    let mut retired = 0;
    for card in cards
        .iter()
        .filter(|card| wanted.contains(&card.card_id) && card.status == CardStatus::Active)
    {
        append_record(root, &card.with_status(CardStatus::Retired))?;
        retired += 1;
    }
    Ok(retired)
}

/// Summary returned by an explicit card-store rewrite operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CardStoreRewriteReport {
    pub records_before: usize,
    pub records_after: usize,
    pub archived: bool,
    pub removed_card_ids: Vec<String>,
}

/// Exports the canonical, merged card view as JSONL. The active store is not
/// modified; retired cards are omitted unless `include_retired` is true.
pub fn export_cards(root: &Path, output: &Path, include_retired: bool) -> io::Result<usize> {
    let store_path = cards_path(root);
    ensure_distinct_store_path(&store_path, output)?;
    let cards = load_cards(root)?;
    let selected = cards
        .into_iter()
        .filter(|card| include_retired || card.status == CardStatus::Active)
        .collect::<Vec<_>>();
    write_card_snapshot(output, &selected, false)?;
    Ok(selected.len())
}

/// Compacts the append-only store into its canonical merged representation.
/// The original bytes must be archived first so this operation remains
/// recoverable. This is intentionally a slow, explicit maintenance operation.
pub fn compact_cards(root: &Path, archive: &Path) -> io::Result<CardStoreRewriteReport> {
    let store_path = cards_path(root);
    let records_before = count_non_empty_records(&store_path)?;
    if records_before == 0 {
        return Ok(CardStoreRewriteReport {
            records_before: 0,
            records_after: 0,
            archived: false,
            removed_card_ids: Vec::new(),
        });
    }
    ensure_distinct_store_path(&store_path, archive)?;
    archive_store(&store_path, archive)?;
    let cards = load_cards(root)?;
    write_card_snapshot(&store_path, &cards, true)?;
    Ok(CardStoreRewriteReport {
        records_before,
        records_after: cards.len(),
        archived: true,
        removed_card_ids: Vec::new(),
    })
}

/// Permanently removes explicitly named cards from the active store after
/// archiving the pre-rewrite bytes. This is the only card-store operation that
/// deletes durable card records; callers must provide an archive destination.
pub fn forget_cards(
    root: &Path,
    card_ids: &[String],
    archive: &Path,
) -> io::Result<CardStoreRewriteReport> {
    let store_path = cards_path(root);
    let records_before = count_non_empty_records(&store_path)?;
    if records_before == 0 {
        return Ok(CardStoreRewriteReport {
            records_before: 0,
            records_after: 0,
            archived: false,
            removed_card_ids: Vec::new(),
        });
    }
    let wanted = card_ids.iter().cloned().collect::<BTreeSet<_>>();
    if wanted.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "at least one card ID is required for explicit forgetting",
        ));
    }
    ensure_distinct_store_path(&store_path, archive)?;
    let cards = load_cards(root)?;
    let removed_card_ids = cards
        .iter()
        .filter(|card| wanted.contains(&card.card_id))
        .map(|card| card.card_id.clone())
        .collect::<Vec<_>>();
    if removed_card_ids.is_empty() {
        return Ok(CardStoreRewriteReport {
            records_before,
            records_after: cards.len(),
            archived: false,
            removed_card_ids,
        });
    }
    archive_store(&store_path, archive)?;
    let retained = cards
        .into_iter()
        .filter(|card| !wanted.contains(&card.card_id))
        .collect::<Vec<_>>();
    write_card_snapshot(&store_path, &retained, true)?;
    Ok(CardStoreRewriteReport {
        records_before,
        records_after: retained.len(),
        archived: true,
        removed_card_ids,
    })
}

/// Records that cards were shown to the model. This is exposure evidence only;
/// it does not claim that the recommendation was followed or caused an outcome.
pub fn record_card_exposure(root: &Path, card_ids: &[String]) -> io::Result<usize> {
    let timestamp = now_unix_ms();
    let wanted = card_ids.iter().collect::<BTreeSet<_>>();
    if wanted.is_empty() {
        return Ok(0);
    }
    let cards = load_cards(root)?;
    let mut updated = 0;
    for card in cards
        .iter()
        .filter(|card| wanted.contains(&card.card_id) && card.status == CardStatus::Active)
    {
        if append_card(root, &card.with_exposure(timestamp))? {
            updated += 1;
        }
    }
    Ok(updated)
}

/// Records explicit human feedback without treating it as causal evaluation.
pub fn record_card_feedback(
    root: &Path,
    card_id: &str,
    feedback: FeedbackState,
) -> io::Result<bool> {
    let cards = load_cards(root)?;
    let Some(card) = cards.iter().find(|card| card.card_id == card_id) else {
        return Ok(false);
    };
    append_card(root, &card.with_feedback(feedback))
}

/// Records descriptive outcome attribution. Unknown is the safe value when a
/// later result cannot be linked to a retrieved card.
pub fn record_card_outcome(
    root: &Path,
    card_id: &str,
    followed: AttributionState,
    verified: AttributionState,
) -> io::Result<bool> {
    let cards = load_cards(root)?;
    let Some(card) = cards.iter().find(|card| card.card_id == card_id) else {
        return Ok(false);
    };
    append_card(root, &card.with_attribution(followed, verified))
}

/// Selects cards using deterministic lexical overlap and stable tie-breaking.
/// This compatibility wrapper infers structured filters from the query.
pub fn select_relevant(
    cards: &[ExperienceCard],
    query: &str,
    max_cards: usize,
) -> Vec<ExperienceCard> {
    let context = ExperienceRetrievalContext::from_query(query, None);
    select_relevant_with_context(cards, query, &context, max_cards)
}

/// Selects active cards using structured scope/signature matches, evidence
/// quality, confidence, recurrence, and recency. Unknown scope is retained as
/// a fallback; an explicitly mismatched workspace is filtered out.
pub fn select_relevant_with_context(
    cards: &[ExperienceCard],
    query: &str,
    context: &ExperienceRetrievalContext,
    max_cards: usize,
) -> Vec<ExperienceCard> {
    select_relevant_at(cards, query, context, max_cards, now_unix_ms())
}

pub(crate) fn select_relevant_at(
    cards: &[ExperienceCard],
    query: &str,
    context: &ExperienceRetrievalContext,
    max_cards: usize,
    now_unix_ms: i64,
) -> Vec<ExperienceCard> {
    if max_cards == 0 || cards.is_empty() {
        return Vec::new();
    }

    let query_tokens = tokenize(query);
    let query_topic_hashes = topic_hashes_for_text(query);
    let mut ranked = cards
        .iter()
        .filter_map(|card| {
            if card.status == CardStatus::Retired || !workspace_matches(card, context) {
                return None;
            }
            let searchable_tokens = tokenize(&card.searchable_text());
            let lexical_score = query_tokens
                .iter()
                .filter(|token| searchable_tokens.contains(*token))
                .count();
            let topic_score = query_topic_hashes
                .iter()
                .filter(|hash| card.topic_hashes.iter().any(|candidate| candidate == *hash))
                .count();
            let structured_score = structured_match_score(card, context);
            if lexical_score == 0 && topic_score == 0 && structured_score == 0 {
                return None;
            }
            let evidence_quality = if !card.evidence_profile.facts.is_empty() {
                2u8
            } else if !card.evidence_profile.hypotheses.is_empty() {
                0u8
            } else {
                1u8
            };
            let scope_score = scope_match_score(card, context);
            let recency_score = recency_score(card.recurrence.last_seen_at_unix_ms, now_unix_ms);
            Some((
                scope_score,
                structured_score,
                evidence_quality,
                confidence_score(card.confidence),
                lexical_score,
                topic_score,
                card.recurrence.count.min(8),
                recency_score,
                card,
            ))
        })
        .collect::<Vec<_>>();

    ranked.sort_by(
        |(
            left_scope,
            left_structured,
            left_evidence,
            left_confidence,
            left_lexical,
            left_topic,
            left_recurrence,
            left_recency,
            left,
        ),
         (
            right_scope,
            right_structured,
            right_evidence,
            right_confidence,
            right_lexical,
            right_topic,
            right_recurrence,
            right_recency,
            right,
        )| {
            right_scope
                .cmp(left_scope)
                .then_with(|| right_structured.cmp(left_structured))
                .then_with(|| right_evidence.cmp(left_evidence))
                .then_with(|| right_confidence.cmp(left_confidence))
                .then_with(|| right_lexical.cmp(left_lexical))
                .then_with(|| right_topic.cmp(left_topic))
                .then_with(|| right_recurrence.cmp(left_recurrence))
                .then_with(|| right_recency.cmp(left_recency))
                .then_with(|| right.created_at_unix_ms.cmp(&left.created_at_unix_ms))
                .then_with(|| left.card_id.cmp(&right.card_id))
        },
    );

    let mut selected = Vec::new();
    let mut selected_conflicts = BTreeMap::<String, String>::new();
    for (_, _, _, _, _, _, _, _, card) in ranked {
        let conflict_key = card.conflict_key();
        if let Some(selected_lesson) = selected_conflicts.get(&conflict_key)
            && selected_lesson != &card.lesson_id
        {
            continue;
        }
        selected_conflicts
            .entry(conflict_key)
            .or_insert_with(|| card.lesson_id.clone());
        selected.push(card.clone());
        if selected.len() == max_cards {
            break;
        }
    }
    selected
}

/// Renders selected cards into a bounded, human-readable advisory block.
pub fn render_cards(cards: &[ExperienceCard], max_context_tokens: usize) -> String {
    if cards.is_empty() || max_context_tokens == 0 {
        return String::new();
    }

    let rendered = cards.iter().map(render_card).collect::<Vec<_>>().join("\n");
    truncate_middle_with_token_budget(&rendered, max_context_tokens).0
}

fn render_card(card: &ExperienceCard) -> String {
    let activity = render_labels(&card.activity_labels);
    let signals = render_labels(&card.signals);
    let verification = render_labels(&card.verification);
    let facts = card
        .evidence_profile
        .facts
        .iter()
        .map(|fact| fact.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let hypotheses = card
        .evidence_profile
        .hypotheses
        .iter()
        .map(|hypothesis| hypothesis.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let interventions = card
        .evidence_profile
        .interventions
        .iter()
        .map(|intervention| intervention.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let outcomes = card
        .evidence_profile
        .outcomes
        .iter()
        .map(|outcome| outcome.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let observed_sequence = card
        .evidence_profile
        .observed_sequence
        .iter()
        .map(|step| step.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let toolchain = card
        .scope
        .toolchain_family
        .map(ToolchainFamily::as_str)
        .unwrap_or("unknown");
    let model = card
        .scope
        .model_family
        .map(ModelFamily::as_str)
        .unwrap_or("unknown");
    format!(
        "- task={} outcome={} signature={}/{}/{}/{} toolchain={} model={} confidence={} recurrence={} sequence={} facts={} hypotheses={} interventions={} outcomes={} activity={} signals={} recommendation={} verification={} exposure={} feedback={} followed={} verified={}",
        card.task_family.as_str(),
        card.failure_class.as_str(),
        card.failure_signature.operation_class.as_str(),
        card.failure_signature.tool_family.as_str(),
        card.failure_signature.exit_bucket.as_str(),
        card.failure_signature.verification_state.as_str(),
        toolchain,
        model,
        card.confidence.as_str(),
        card.recurrence.count,
        if observed_sequence.is_empty() {
            "none"
        } else {
            &observed_sequence
        },
        if facts.is_empty() { "none" } else { &facts },
        if hypotheses.is_empty() {
            "none"
        } else {
            &hypotheses
        },
        if interventions.is_empty() {
            "none"
        } else {
            &interventions
        },
        if outcomes.is_empty() {
            "none"
        } else {
            &outcomes
        },
        activity,
        signals,
        render_text(&card.recommendation, 512),
        verification,
        card.adoption.exposure_count,
        card.adoption.feedback.as_str(),
        card.adoption.followed.as_str(),
        card.adoption.verified.as_str(),
    )
}

fn render_labels(labels: &[String]) -> String {
    if labels.is_empty() {
        "none".to_string()
    } else {
        labels
            .iter()
            .map(|label| render_text(label, 64))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn append_record(root: &Path, card: &ExperienceCard) -> io::Result<bool> {
    std::fs::create_dir_all(root)?;
    let encoded = serde_json::to_vec(card)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let current_size = match std::fs::metadata(cards_path(root)) {
        Ok(metadata) => metadata.len(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => 0,
        Err(error) => return Err(error),
    };
    if current_size.saturating_add(encoded.len() as u64 + 1) > MAX_STORE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "agent-improvement card store would exceed the size limit",
        ));
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(cards_path(root))?;
    let mut record = encoded;
    record.push(b'\n');
    file.write_all(&record)?;
    Ok(true)
}

fn count_non_empty_records(path: &Path) -> io::Result<usize> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error),
    };
    BufReader::new(file)
        .lines()
        .try_fold(0usize, |count, line| {
            let line = line?;
            Ok::<_, io::Error>(count + usize::from(!line.trim().is_empty()))
        })
}

fn ensure_distinct_store_path(store_path: &Path, output: &Path) -> io::Result<()> {
    if normalized_path(store_path) == normalized_path(output) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "maintenance output must not overwrite the active card store",
        ));
    }
    Ok(())
}

fn normalized_path(path: &Path) -> PathBuf {
    if let Ok(path) = std::fs::canonicalize(path) {
        return path;
    }
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

fn archive_store(store_path: &Path, archive: &Path) -> io::Result<()> {
    ensure_distinct_store_path(store_path, archive)?;
    let data = std::fs::read(store_path)?;
    if let Some(parent) = archive
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(archive)?;
    file.write_all(&data)?;
    Ok(())
}

fn write_card_snapshot(
    path: &Path,
    cards: &[ExperienceCard],
    replace_existing: bool,
) -> io::Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    for card in cards {
        if !card.is_bounded() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "cannot write an unbounded experience card",
            ));
        }
    }
    let temp_path = path.with_extension(format!("jsonl.tmp-{}", std::process::id()));
    if temp_path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "card-store maintenance temporary file already exists",
        ));
    }
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)?;
        let mut total_bytes = 0u64;
        for card in cards {
            let mut record = serde_json::to_vec(card)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            record.push(b'\n');
            total_bytes = total_bytes.saturating_add(record.len() as u64);
            if total_bytes > MAX_STORE_BYTES {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "compacted agent-improvement card store exceeds the size limit",
                ));
            }
            file.write_all(&record)?;
        }
        file.sync_all()?;
        if replace_existing && path.exists() {
            let metadata = std::fs::symlink_metadata(path)?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "active card store must be a regular file",
                ));
            }
        }
        std::fs::rename(&temp_path, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp_path);
    }
    result
}

fn workspace_matches(card: &ExperienceCard, context: &ExperienceRetrievalContext) -> bool {
    match (&context.workspace_key, &card.scope.workspace_key) {
        (Some(expected), Some(actual)) => expected == actual,
        _ => true,
    }
}

fn scope_match_score(card: &ExperienceCard, context: &ExperienceRetrievalContext) -> u8 {
    match (&context.workspace_key, &card.scope.workspace_key) {
        (Some(expected), Some(actual)) if expected == actual => 3,
        (Some(_), Some(_)) => 0,
        (Some(_), None) => 1,
        (None, _) => 0,
    }
}

fn structured_match_score(card: &ExperienceCard, context: &ExperienceRetrievalContext) -> u8 {
    let mut score = 0;
    if context.task_family == Some(card.task_family) {
        score += 3;
    }
    if context.failure_class == Some(card.failure_class) {
        score += 3;
    }
    if context.tool_family == Some(card.failure_signature.tool_family) {
        score += 2;
    }
    if context.toolchain_family == card.scope.toolchain_family && context.toolchain_family.is_some()
    {
        score += 2;
    }
    if context.model_family == card.scope.model_family && context.model_family.is_some() {
        score += 1;
    }
    score
}

fn confidence_score(confidence: ConfidenceLevel) -> u8 {
    match confidence {
        ConfidenceLevel::Unknown => 0,
        ConfidenceLevel::Low => 1,
        ConfidenceLevel::Medium => 2,
        ConfidenceLevel::High => 3,
    }
}

fn recency_score(last_seen_at_unix_ms: i64, now_unix_ms: i64) -> u8 {
    let age = now_unix_ms.saturating_sub(last_seen_at_unix_ms).max(0);
    if age <= 7 * MILLIS_PER_DAY {
        3
    } else if age <= 30 * MILLIS_PER_DAY {
        2
    } else if age <= 180 * MILLIS_PER_DAY {
        1
    } else {
        0
    }
}

fn failure_class_for_query(query: &str) -> Option<FailureClass> {
    let query = query.to_ascii_lowercase();
    if contains_any(&query, &["permission", "approval", "sandbox", "authority"]) {
        Some(FailureClass::PolicyBlocked)
    } else if contains_any(&query, &["verification", "test failed", "tests failed"]) {
        Some(FailureClass::IncompleteVerification)
    } else if contains_any(
        &query,
        &["command failed", "exit code", "nonzero", "non-zero"],
    ) {
        Some(FailureClass::CommandFailure)
    } else if contains_any(&query, &["tool failed", "tool error", "mcp error"]) {
        Some(FailureClass::ToolFailure)
    } else {
        None
    }
}

fn tool_family_for_query(query: &str) -> Option<ToolFamily> {
    let query = query.to_ascii_lowercase();
    if contains_any(
        &query,
        &["exec_command", "shell", "terminal", "cargo", "npm"],
    ) {
        Some(ToolFamily::ExecCommand)
    } else if query.contains("apply_patch") || query.contains("patch") {
        Some(ToolFamily::ApplyPatch)
    } else if query.contains("mcp") {
        Some(ToolFamily::Mcp)
    } else {
        None
    }
}

fn toolchain_family_for_query(query: &str) -> Option<ToolchainFamily> {
    let query = query.to_ascii_lowercase();
    if contains_any(&query, &["cargo", "rustc", "rustfmt"]) {
        Some(ToolchainFamily::Rust)
    } else if contains_any(&query, &["npm", "node", "jest", "vitest", "yarn"]) {
        Some(ToolchainFamily::Node)
    } else if contains_any(&query, &["python", "pytest", "pip"]) {
        Some(ToolchainFamily::Python)
    } else if contains_any(&query, &["go test", "go build", "golang"]) {
        Some(ToolchainFamily::Go)
    } else {
        None
    }
}

fn contains_any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| text.contains(needle))
}

fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or_default()
}

fn render_text(value: &str, max_chars: usize) -> String {
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
        .collect()
}

fn tokenize(text: &str) -> BTreeSet<String> {
    text.split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_ascii_lowercase)
        .filter(|token| !SEARCH_STOP_WORDS.contains(&token.as_str()))
        .collect()
}
