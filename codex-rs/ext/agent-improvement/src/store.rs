use std::fs::File;
use std::fs::OpenOptions;
use std::io;
use std::io::BufRead;
use std::io::BufReader;
use std::io::Write;
use std::path::{Path, PathBuf};

use codex_utils_string::truncate_middle_with_token_budget;
use serde_json::from_str;

use crate::ExperienceCard;

const MAX_STORE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_CARD_LINE_BYTES: usize = 32 * 1024;
const MAX_LOADED_CARDS: usize = 4096;
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

/// Returns the card store path for a Codex state directory.
pub fn cards_path(root: &Path) -> PathBuf {
    root.join(STORE_FILE_NAME)
}

/// Loads valid cards and skips malformed lines as a recoverable local-state issue.
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

    let mut cards = Vec::new();
    let mut skipped_count = 0usize;
    for line in BufReader::new(file).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if line.len() > MAX_CARD_LINE_BYTES || cards.len() >= MAX_LOADED_CARDS {
            skipped_count += 1;
            continue;
        }
        match from_str::<ExperienceCard>(&line) {
            Ok(card) if card.is_bounded() => cards.push(card),
            Ok(_) | Err(_) => skipped_count += 1,
        }
    }

    if skipped_count > 0 {
        tracing::warn!(
            skipped_count,
            "skipped invalid or oversized agent-improvement experience cards"
        );
    }

    Ok(cards)
}

/// Appends one card unless the same deterministic card ID is already present.
pub fn append_card(root: &Path, card: &ExperienceCard) -> io::Result<bool> {
    if !card.is_bounded() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "agent-improvement card exceeds the schema bounds",
        ));
    }
    let existing = load_cards(root)?;
    if existing
        .iter()
        .any(|existing| existing.card_id == card.card_id)
    {
        return Ok(false);
    }

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
    file.write_all(&encoded)?;
    file.write_all(b"\n")?;
    Ok(true)
}

/// Selects cards using deterministic lexical overlap and stable tie-breaking.
pub fn select_relevant(
    cards: &[ExperienceCard],
    query: &str,
    max_cards: usize,
) -> Vec<ExperienceCard> {
    if max_cards == 0 || cards.is_empty() {
        return Vec::new();
    }

    let query_tokens = tokenize(query);
    let query_topic_hashes = crate::schema::topic_hashes_for_text(query);
    let mut ranked = cards
        .iter()
        .filter_map(|card| {
            let searchable_tokens = tokenize(&card.searchable_text());
            let lexical_score = query_tokens
                .iter()
                .filter(|token| searchable_tokens.contains(*token))
                .count();
            let topic_score = query_topic_hashes
                .iter()
                .filter(|hash| card.topic_hashes.iter().any(|candidate| candidate == *hash))
                .count();
            let score = lexical_score + topic_score;
            (score > 0).then_some((score, card))
        })
        .collect::<Vec<_>>();

    ranked.sort_by(|(left_score, left), (right_score, right)| {
        right_score
            .cmp(left_score)
            .then_with(|| right.created_at_unix_ms.cmp(&left.created_at_unix_ms))
            .then_with(|| left.card_id.cmp(&right.card_id))
    });

    ranked
        .into_iter()
        .take(max_cards)
        .map(|(_, card)| card.clone())
        .collect()
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
    let activity = if card.activity_labels.is_empty() {
        "none".to_string()
    } else {
        card.activity_labels
            .iter()
            .map(|label| render_text(label, 64))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let signals = if card.signals.is_empty() {
        "none".to_string()
    } else {
        card.signals
            .iter()
            .map(|signal| render_text(signal, 64))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let verification = if card.verification.is_empty() {
        "none".to_string()
    } else {
        card.verification
            .iter()
            .map(|label| render_text(label, 64))
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!(
        "- task={} outcome={} activity={} signals={} recommendation={} verification={}",
        card.task_family.as_str(),
        card.failure_class.as_str(),
        activity,
        signals,
        render_text(&card.recommendation, 512),
        verification,
    )
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

fn tokenize(text: &str) -> std::collections::BTreeSet<String> {
    text.split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_ascii_lowercase)
        .filter(|token| !SEARCH_STOP_WORDS.contains(&token.as_str()))
        .collect()
}
