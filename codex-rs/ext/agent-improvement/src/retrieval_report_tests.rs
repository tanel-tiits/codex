use pretty_assertions::assert_eq;

use super::intervention::InterventionRetrievalContext;
use super::retrieval_report::{
    INTERVENTION_RETRIEVAL_CASES_SCHEMA_VERSION, InterventionRetrievalCase,
    InterventionRetrievalCaseSet, compare_retrieval_policies,
};
use super::schema::{AgentImprovementEvidence, ExperienceCard, FailureClass, TaskFamily};

fn card(trace_id: &str, family: TaskFamily, recommendation: &str) -> ExperienceCard {
    ExperienceCard::new(
        trace_id,
        1_000,
        family,
        FailureClass::Completed,
        AgentImprovementEvidence::default(),
        vec!["parser".to_string()],
        recommendation,
        vec!["run the focused verification".to_string()],
    )
}

#[test]
fn retrieval_case_sets_are_versioned_and_reject_duplicates() {
    let case = InterventionRetrievalCase {
        case_id: "case-1".to_string(),
        query: "fix parser".to_string(),
        context: InterventionRetrievalContext::from_query("fix parser", None),
        relevant_card_ids: Vec::new(),
    };
    let cases = InterventionRetrievalCaseSet {
        schema_version: INTERVENTION_RETRIEVAL_CASES_SCHEMA_VERSION,
        cases: vec![case.clone(), case],
    };
    assert!(cases.validate().is_err());
}

#[test]
fn retrieval_report_is_aggregate_only_and_exposes_scoped_precision_signals() {
    let relevant = card(
        "trace-relevant",
        TaskFamily::Coding,
        "Run the parser verification before changing more code.",
    );
    let mut generic = card(
        "trace-generic",
        TaskFamily::General,
        "Use a focused verification check.",
    );
    let generic_intervention = generic.intervention.as_mut().expect("generic projection");
    generic_intervention.scope.task_family = None;
    generic_intervention.scope.failure_class = None;
    generic_intervention.scope.failure_signature = None;
    generic_intervention.scope.topic_hashes.clear();
    generic_intervention.trigger = "generic:verification".to_string();
    assert!(generic_intervention.is_bounded());

    let cases = InterventionRetrievalCaseSet {
        schema_version: INTERVENTION_RETRIEVAL_CASES_SCHEMA_VERSION,
        cases: vec![
            InterventionRetrievalCase {
                case_id: "case-parser".to_string(),
                query: "PRIVATE_PROMPT_PARSER_42 fix parser".to_string(),
                context: InterventionRetrievalContext::from_query("fix parser", None),
                relevant_card_ids: vec![relevant.card_id.clone()],
            },
            InterventionRetrievalCase {
                case_id: "case-review".to_string(),
                query: "review documentation".to_string(),
                context: InterventionRetrievalContext::from_query("review documentation", None),
                relevant_card_ids: Vec::new(),
            },
        ],
    };
    let report = compare_retrieval_policies(&[relevant, generic], &cases, 2, 1_200, 480, 0)
        .expect("compare retrieval policies");

    assert_eq!(report.case_count, 2);
    assert_eq!(report.cases_with_relevant_cards, 1);
    assert_eq!(report.scoped_relevant_count, 1);
    assert_eq!(report.scoped_coverage_bps, 10_000);
    assert_eq!(report.scoped_generic_fallback_count, 1);
    assert_eq!(report.scoped_generic_fallback_irrelevant_count, 1);
    assert_eq!(report.scoped_irrelevant_count, 1);
    let encoded = serde_json::to_string(&report).expect("serialize report");
    assert!(!encoded.contains("PRIVATE_PROMPT_PARSER_42"));
    assert!(!encoded.contains("trace-relevant"));
}
