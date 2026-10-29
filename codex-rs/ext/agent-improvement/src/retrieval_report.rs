//! Offline comparison of broad historical retrieval and scoped interventions.
//!
//! The case set is a private input. Reports contain aggregate counts only, so
//! task prompts, case IDs, card IDs, and advisory text cannot cross the
//! reporting boundary.

use std::collections::BTreeSet;

use anyhow::{Result, ensure};
use codex_utils_string::approx_token_count;
use serde::{Deserialize, Serialize};

use crate::intervention::{InterventionRetrievalContext, render_intervention, select_intervention};
use crate::schema::ExperienceCard;
use crate::store::{
    ExperienceRetrievalContext, render_cards, select_relevant_with_context_and_min_score,
};

/// Version of the private retrieval-case input contract.
pub const INTERVENTION_RETRIEVAL_CASES_SCHEMA_VERSION: u32 = 1;
/// Version of the aggregate retrieval comparison report.
pub const INTERVENTION_RETRIEVAL_REPORT_SCHEMA_VERSION: u32 = 1;
/// Maximum number of private cases accepted by one offline comparison.
pub const MAX_INTERVENTION_RETRIEVAL_CASES: usize = 4096;

const MAX_CASE_ID_CHARS: usize = 128;
const MAX_QUERY_CHARS: usize = 32 * 1024;
const MAX_RELEVANT_CARD_IDS: usize = 16;

/// One private retrieval case. The query and expected card IDs are never
/// copied into [`InterventionRetrievalReport`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterventionRetrievalCase {
    pub case_id: String,
    pub query: String,
    pub context: InterventionRetrievalContext,
    #[serde(default)]
    pub relevant_card_ids: Vec<String>,
}

/// Private, versioned input for the offline retrieval comparison.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterventionRetrievalCaseSet {
    pub schema_version: u32,
    pub cases: Vec<InterventionRetrievalCase>,
}

impl InterventionRetrievalCaseSet {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == INTERVENTION_RETRIEVAL_CASES_SCHEMA_VERSION,
            "unsupported intervention retrieval case schema {}",
            self.schema_version
        );
        ensure!(
            !self.cases.is_empty() && self.cases.len() <= MAX_INTERVENTION_RETRIEVAL_CASES,
            "retrieval case set must contain between 1 and {} cases",
            MAX_INTERVENTION_RETRIEVAL_CASES
        );
        let mut case_ids = BTreeSet::new();
        for case in &self.cases {
            ensure!(
                is_safe_case_id(&case.case_id),
                "retrieval case ID is empty, oversized, or unsafe"
            );
            ensure!(
                case_ids.insert(case.case_id.clone()),
                "duplicate retrieval case ID"
            );
            ensure!(
                !case.query.trim().is_empty() && case.query.chars().count() <= MAX_QUERY_CHARS,
                "retrieval case query is empty or oversized"
            );
            ensure!(
                case.context.is_bounded(),
                "retrieval case {} contains an invalid opaque context key",
                case.case_id
            );
            ensure!(
                case.relevant_card_ids.len() <= MAX_RELEVANT_CARD_IDS
                    && case.relevant_card_ids.iter().all(|id| is_safe_case_id(id)),
                "retrieval case {} contains invalid expected card IDs",
                case.case_id
            );
        }
        Ok(())
    }
}

/// Aggregate comparison of the legacy and scoped retrieval lanes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterventionRetrievalReport {
    pub schema_version: u32,
    pub case_count: u32,
    pub cases_with_relevant_cards: u32,
    pub max_cards: u32,
    pub max_context_tokens: u32,
    pub intervention_context_tokens: u32,
    pub min_relevance_score: u32,
    pub legacy_selected_count: u32,
    pub legacy_relevant_count: u32,
    pub legacy_irrelevant_count: u32,
    pub legacy_precision_bps: u32,
    pub legacy_coverage_bps: u32,
    pub legacy_context_tokens: u64,
    pub scoped_selected_count: u32,
    pub scoped_relevant_count: u32,
    pub scoped_irrelevant_count: u32,
    pub scoped_precision_bps: u32,
    pub scoped_coverage_bps: u32,
    pub scoped_generic_fallback_count: u32,
    pub scoped_generic_fallback_irrelevant_count: u32,
    pub scoped_context_tokens: u64,
}

/// Compare retrieval policies without a model call or prompt-visible output.
pub fn compare_retrieval_policies(
    cards: &[ExperienceCard],
    cases: &InterventionRetrievalCaseSet,
    max_cards: usize,
    max_context_tokens: usize,
    intervention_context_tokens: usize,
    min_relevance_score: usize,
) -> Result<InterventionRetrievalReport> {
    cases.validate()?;
    ensure!(max_cards > 0, "max_cards must be positive");
    ensure!(
        max_context_tokens > 0,
        "max_context_tokens must be positive"
    );
    ensure!(
        (crate::MIN_INTERVENTION_CONTEXT_TOKENS..=crate::MAX_INTERVENTION_CONTEXT_TOKENS)
            .contains(&intervention_context_tokens),
        "intervention_context_tokens must be between {} and {}",
        crate::MIN_INTERVENTION_CONTEXT_TOKENS,
        crate::MAX_INTERVENTION_CONTEXT_TOKENS
    );

    let mut legacy_selected_count = 0_u32;
    let mut legacy_relevant_count = 0_u32;
    let mut legacy_context_tokens = 0_u64;
    let mut scoped_selected_count = 0_u32;
    let mut scoped_relevant_count = 0_u32;
    let mut scoped_generic_fallback_count = 0_u32;
    let mut scoped_generic_fallback_irrelevant_count = 0_u32;
    let mut scoped_context_tokens = 0_u64;
    let mut cases_with_relevant_cards = 0_u32;
    let mut legacy_covered_cases = 0_u32;
    let mut scoped_covered_cases = 0_u32;
    let interventions = cards
        .iter()
        .filter_map(|card| card.intervention.clone())
        .collect::<Vec<_>>();

    for case in &cases.cases {
        let relevant = case.relevant_card_ids.iter().collect::<BTreeSet<_>>();
        if !relevant.is_empty() {
            cases_with_relevant_cards = cases_with_relevant_cards.saturating_add(1);
        }

        let legacy_context = broad_context(&case.context);
        let legacy_selected = select_relevant_with_context_and_min_score(
            cards,
            &case.query,
            &legacy_context,
            max_cards,
            min_relevance_score,
        );
        let legacy_hits = legacy_selected
            .iter()
            .filter(|card| relevant.contains(&card.card_id))
            .count();
        legacy_selected_count = legacy_selected_count.saturating_add(legacy_selected.len() as u32);
        legacy_relevant_count = legacy_relevant_count.saturating_add(legacy_hits as u32);
        if legacy_hits > 0 {
            legacy_covered_cases = legacy_covered_cases.saturating_add(1);
        }
        legacy_context_tokens = legacy_context_tokens.saturating_add(approx_token_count(
            &render_cards(&legacy_selected, max_context_tokens),
        ) as u64);

        if let Some(selected) = select_intervention(&interventions, &case.context) {
            scoped_selected_count = scoped_selected_count.saturating_add(1);
            let scoped_hit = selected
                .evidence
                .source_card_ids
                .iter()
                .any(|card_id| relevant.contains(card_id));
            if scoped_hit {
                scoped_relevant_count = scoped_relevant_count.saturating_add(1);
                scoped_covered_cases = scoped_covered_cases.saturating_add(1);
            }
            if selected.scope_label() == "generic fallback" {
                scoped_generic_fallback_count = scoped_generic_fallback_count.saturating_add(1);
                if !scoped_hit {
                    scoped_generic_fallback_irrelevant_count =
                        scoped_generic_fallback_irrelevant_count.saturating_add(1);
                }
            }
            scoped_context_tokens = scoped_context_tokens.saturating_add(approx_token_count(
                &render_intervention(&selected, intervention_context_tokens),
            ) as u64);
        }
    }

    let legacy_irrelevant_count = legacy_selected_count.saturating_sub(legacy_relevant_count);
    let scoped_irrelevant_count = scoped_selected_count.saturating_sub(scoped_relevant_count);
    Ok(InterventionRetrievalReport {
        schema_version: INTERVENTION_RETRIEVAL_REPORT_SCHEMA_VERSION,
        case_count: cases.cases.len() as u32,
        cases_with_relevant_cards,
        max_cards: max_cards.try_into().unwrap_or(u32::MAX),
        max_context_tokens: max_context_tokens.try_into().unwrap_or(u32::MAX),
        intervention_context_tokens: intervention_context_tokens.try_into().unwrap_or(u32::MAX),
        min_relevance_score: min_relevance_score.try_into().unwrap_or(u32::MAX),
        legacy_selected_count,
        legacy_relevant_count,
        legacy_irrelevant_count,
        legacy_precision_bps: ratio_bps(legacy_relevant_count, legacy_selected_count),
        legacy_coverage_bps: ratio_bps(legacy_covered_cases, cases_with_relevant_cards),
        legacy_context_tokens,
        scoped_selected_count,
        scoped_relevant_count,
        scoped_irrelevant_count,
        scoped_precision_bps: ratio_bps(scoped_relevant_count, scoped_selected_count),
        scoped_coverage_bps: ratio_bps(scoped_covered_cases, cases_with_relevant_cards),
        scoped_generic_fallback_count,
        scoped_generic_fallback_irrelevant_count,
        scoped_context_tokens,
    })
}

fn broad_context(context: &InterventionRetrievalContext) -> ExperienceRetrievalContext {
    ExperienceRetrievalContext {
        workspace_key: context.workspace_key.clone(),
        task_family: Some(context.task_family),
        failure_class: context.failure_class,
        tool_family: context
            .failure_signature
            .map(|signature| signature.tool_family),
        toolchain_family: None,
        model_family: None,
    }
}

fn ratio_bps(numerator: u32, denominator: u32) -> u32 {
    if denominator == 0 {
        0
    } else {
        numerator.saturating_mul(10_000) / denominator
    }
}

fn is_safe_case_id(value: &str) -> bool {
    !value.is_empty()
        && value.chars().count() <= MAX_CASE_ID_CHARS
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
}
