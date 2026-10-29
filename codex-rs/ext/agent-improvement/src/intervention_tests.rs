use pretty_assertions::assert_eq;

use super::intervention::{
    InterventionRetrievalContext, MAX_INTERVENTION_CONTEXT_TOKENS, MIN_INTERVENTION_CONTEXT_TOKENS,
    render_intervention, select_intervention,
};
use super::schema::{
    AgentImprovementEvidence, ExperienceCard, FailureClass, FailureSignature, OperationClass,
    TaskFamily, ToolFamily, VerificationState, workspace_scope_key,
};

fn card(
    trace_id: &str,
    family: TaskFamily,
    failure_class: FailureClass,
    recommendation: &str,
    verification: &[&str],
) -> ExperienceCard {
    ExperienceCard::new(
        trace_id,
        1_000,
        family,
        failure_class,
        AgentImprovementEvidence::default(),
        vec!["parser".to_string()],
        recommendation,
        verification
            .iter()
            .map(|value| (*value).to_string())
            .collect(),
    )
}

#[test]
fn intervention_projection_is_versioned_bounded_and_redacted() {
    let safe_card = card(
        "trace-safe",
        TaskFamily::Coding,
        FailureClass::CommandFailure,
        "Reproduce the first failing command.",
        &["run the focused verification"],
    );
    let intervention = safe_card
        .intervention
        .clone()
        .expect("new cards receive an intervention projection");
    assert_eq!(intervention.schema_version, 1);
    assert!(intervention.is_bounded());
    let encoded = serde_json::to_string(&intervention).expect("serialize intervention");
    assert!(!encoded.contains("stdout"));
    assert!(!encoded.contains("/Users/"));
    assert!(!encoded.contains("parser"));

    let unsafe_card = card(
        "trace-unsafe",
        TaskFamily::Coding,
        FailureClass::CommandFailure,
        "Read /Users/private/secret.txt before continuing.",
        &["verify"],
    );
    assert!(unsafe_card.intervention.is_none());

    let diagnostic_card = card(
        "trace-diagnostic",
        TaskFamily::General,
        FailureClass::Unknown,
        "Use a bounded check.",
        &["verify"],
    );
    assert!(diagnostic_card.intervention.is_none());

    let overbroad_card = card(
        "trace-overbroad",
        TaskFamily::Coding,
        FailureClass::CommandFailure,
        "First step. Second step. Third step.",
        &["verify"],
    );
    assert!(overbroad_card.intervention.is_none());
}

#[test]
fn retrieval_prefers_exact_project_failure_then_family_then_generic() {
    let mut family = card(
        "trace-family",
        TaskFamily::Coding,
        FailureClass::Completed,
        "Use the smallest verifiable change.",
        &["run the focused verification"],
    )
    .intervention
    .expect("family intervention");
    family.scope.failure_class = None;
    family.scope.failure_signature = None;
    family.trigger = "coding:family".to_string();

    let mut failure = card(
        "trace-failure",
        TaskFamily::Coding,
        FailureClass::CommandFailure,
        "Reproduce the command failure before editing more files.",
        &["run the focused verification"],
    )
    .intervention
    .expect("failure intervention");
    failure.scope.workspace_key = None;
    failure.trigger = "coding:command_failure".to_string();

    let mut exact = failure.clone();
    exact.intervention_id = "intervention_exact".to_string();
    exact.scope.workspace_key = Some(workspace_scope_key("/workspace/project"));
    exact.trigger = "coding:command_failure:project".to_string();

    let context = InterventionRetrievalContext::from_query(
        "fix the parser command failed in cargo",
        Some("/workspace/project"),
    );
    assert_eq!(
        select_intervention(&[family, failure, exact.clone()], &context),
        Some(exact.clone())
    );

    let other_workspace = InterventionRetrievalContext::from_query(
        "fix the parser command failed in cargo",
        Some("/workspace/other"),
    );
    let mut family_fallback = card(
        "trace-family-2",
        TaskFamily::Coding,
        FailureClass::Completed,
        "Use the smallest verifiable change.",
        &["verify"],
    )
    .intervention
    .expect("family intervention");
    family_fallback.scope.failure_class = None;
    family_fallback.scope.failure_signature = None;
    let selected = select_intervention(&[family_fallback, exact], &other_workspace)
        .expect("family fallback should remain available");
    assert_eq!(selected.scope_label(), "task family");
}

#[test]
fn signature_matching_and_unrelated_tasks_are_conservative() {
    let mut intervention = card(
        "trace-rust",
        TaskFamily::Coding,
        FailureClass::CommandFailure,
        "Reproduce the first failing command.",
        &["run the focused verification"],
    )
    .intervention
    .expect("failure intervention");
    intervention.scope.workspace_key = Some(workspace_scope_key("/workspace/project"));
    intervention.scope.failure_signature = Some(FailureSignature {
        operation_class: OperationClass::Command,
        tool_family: ToolFamily::ExecCommand,
        verification_state: VerificationState::Failed,
        ..Default::default()
    });

    let unrelated = InterventionRetrievalContext::from_query(
        "review the documentation",
        Some("/workspace/project"),
    );
    assert!(select_intervention(&[intervention.clone()], &unrelated).is_none());

    let matching = InterventionRetrievalContext {
        workspace_key: Some(workspace_scope_key("/workspace/project")),
        task_family: TaskFamily::Coding,
        failure_class: Some(FailureClass::CommandFailure),
        failure_signature: Some(FailureSignature {
            operation_class: OperationClass::Command,
            tool_family: ToolFamily::ExecCommand,
            verification_state: VerificationState::Failed,
            ..Default::default()
        }),
        topic_hashes: Vec::new(),
    };
    assert!(select_intervention(&[intervention], &matching).is_some());
}

#[test]
fn rendering_is_advisory_and_bounded_to_the_intervention_lane() {
    let intervention = card(
        "trace-render",
        TaskFamily::Testing,
        FailureClass::IncompleteVerification,
        "Run the focused test command before changing unrelated files.",
        &["the focused verification passes"],
    )
    .intervention
    .expect("render intervention");
    let rendered = render_intervention(&intervention, MAX_INTERVENTION_CONTEXT_TOKENS);
    assert!(rendered.contains("Historical advisory intervention"));
    assert!(rendered.contains("Action:"));
    assert!(rendered.contains("Verification:"));
    assert!(!rendered.contains(&intervention.evidence.source_card_ids[0]));
    let (bounded, token_count) = codex_utils_string::truncate_middle_with_token_budget(
        &rendered,
        MAX_INTERVENTION_CONTEXT_TOKENS,
    );
    assert!(!bounded.is_empty());
    assert!(token_count.is_none_or(|count| { count <= MAX_INTERVENTION_CONTEXT_TOKENS as u64 }));
    let short = render_intervention(&intervention, MIN_INTERVENTION_CONTEXT_TOKENS);
    assert!(!short.is_empty());
}
