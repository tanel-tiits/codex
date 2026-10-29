//! Private intervention telemetry with aggregate-only public results.
//!
//! The private context may contain card IDs and bounded command signatures so
//! the harness can distinguish an attempted action from a mere exposure. The
//! public result contains only counts and coarse attribution states.

use codex_rollout_trace::{RolloutTrace, TerminalRequest, ToolCallKind};
use serde::{Deserialize, Serialize};

use crate::AttributionState;
use crate::OperationClass;
use crate::ToolFamily;

/// Version of the private intervention telemetry contract.
pub const INTERVENTION_TELEMETRY_SCHEMA_VERSION: u32 = 1;
/// Maximum number of signatures in one private telemetry context.
pub const MAX_INTERVENTION_SIGNATURES: usize = 8;
/// Maximum number of events in one private intervention event chain.
pub const MAX_INTERVENTION_EVENTS: usize = 6;
const MAX_TELEMETRY_ID_CHARS: usize = 128;
const MAX_TELEMETRY_FINGERPRINT_CHARS: usize = 256;
const MAX_COMMAND_TOKENS: usize = 16;
const MAX_COMMAND_TOKEN_CHARS: usize = 128;
const MAX_EVENT_COUNT: u32 = 1024;

/// Separate event kinds retained in the private event chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterventionEventKind {
    Selected,
    Injected,
    ActionAttempted,
    AgentVerificationAttempted,
    IndependentVerificationPassed,
    Outcome,
}

/// A bounded structured action signature. Command tokens are private input and
/// never enter aggregate results.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterventionActionSignature {
    pub operation_class: OperationClass,
    pub tool_family: ToolFamily,
    #[serde(default)]
    pub command_tokens: Vec<String>,
}

impl InterventionActionSignature {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.command_tokens.len() <= MAX_COMMAND_TOKENS,
            "intervention action signature contains too many command tokens"
        );
        for token in &self.command_tokens {
            anyhow::ensure!(
                !token.trim().is_empty()
                    && token.chars().count() <= MAX_COMMAND_TOKEN_CHARS
                    && !token.chars().any(char::is_control),
                "intervention action signature contains an invalid command token"
            );
        }
        anyhow::ensure!(
            self.tool_family != ToolFamily::Unknown,
            "intervention action signature must name a tool family"
        );
        Ok(())
    }
}

/// Private per-task context linking intervention evidence without publishing
/// raw card identity or action text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterventionTelemetryContext {
    pub schema_version: u32,
    pub experiment_id: String,
    pub task_id: String,
    pub card_fingerprint: String,
    pub trigger_id: String,
    #[serde(default)]
    pub selected_card_ids: Vec<String>,
    #[serde(default)]
    pub injection_count: u32,
    #[serde(default)]
    pub action_signatures: Vec<InterventionActionSignature>,
    #[serde(default)]
    pub agent_verification_signatures: Vec<InterventionActionSignature>,
    #[serde(default)]
    pub independent_verification_check_ids: Vec<String>,
    /// Optional private review annotation when trace evidence cannot identify
    /// an action attempt. Unknown remains the default.
    #[serde(default)]
    pub reviewed_action_attempted: AttributionState,
}

impl InterventionTelemetryContext {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.schema_version == INTERVENTION_TELEMETRY_SCHEMA_VERSION,
            "unsupported intervention telemetry schema {}",
            self.schema_version
        );
        ensure_safe_id(&self.experiment_id, "experiment_id")?;
        ensure_safe_id(&self.task_id, "task_id")?;
        ensure_safe_fingerprint(&self.card_fingerprint, "card_fingerprint")?;
        ensure_safe_id(&self.trigger_id, "trigger_id")?;
        anyhow::ensure!(
            self.selected_card_ids.len() <= MAX_INTERVENTION_SIGNATURES,
            "intervention telemetry contains too many selected cards"
        );
        let mut selected_card_ids = std::collections::BTreeSet::new();
        for card_id in &self.selected_card_ids {
            ensure_safe_id(card_id, "selected_card_id")?;
            anyhow::ensure!(
                selected_card_ids.insert(card_id),
                "intervention telemetry contains duplicate selected card IDs"
            );
        }
        anyhow::ensure!(
            self.injection_count <= MAX_EVENT_COUNT,
            "intervention telemetry injection count is too large"
        );
        anyhow::ensure!(
            self.action_signatures.len() <= MAX_INTERVENTION_SIGNATURES,
            "intervention telemetry contains too many action signatures"
        );
        anyhow::ensure!(
            self.agent_verification_signatures.len() <= MAX_INTERVENTION_SIGNATURES,
            "intervention telemetry contains too many verification signatures"
        );
        anyhow::ensure!(
            self.independent_verification_check_ids.len() <= MAX_INTERVENTION_SIGNATURES,
            "intervention telemetry contains too many independent checks"
        );
        for check_id in &self.independent_verification_check_ids {
            ensure_safe_id(check_id, "independent_verification_check_id")?;
        }
        for signature in self
            .action_signatures
            .iter()
            .chain(self.agent_verification_signatures.iter())
        {
            signature.validate()?;
        }
        Ok(())
    }
}

/// One private event in the intervention chain. Its context remains in the
/// private manifest; the public result is created by [`to_result`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterventionTelemetryEvent {
    pub kind: InterventionEventKind,
    pub state: AttributionState,
    pub count: u32,
    #[serde(default)]
    pub outcome_success: Option<bool>,
}

impl InterventionTelemetryEvent {
    fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.count <= MAX_EVENT_COUNT,
            "intervention telemetry event count is too large"
        );
        if self.kind != InterventionEventKind::Outcome {
            anyhow::ensure!(
                self.outcome_success.is_none(),
                "only outcome telemetry events may include outcome success"
            );
        }
        if self.state == AttributionState::Yes {
            anyhow::ensure!(
                self.count > 0,
                "a positive intervention telemetry event must have a count"
            );
        }
        if self.state == AttributionState::No {
            anyhow::ensure!(
                self.count == 0,
                "a negative intervention telemetry event cannot have a count"
            );
        }
        Ok(())
    }
}

/// Private event chain stored with a local evaluation manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterventionTelemetryEvidence {
    pub context: InterventionTelemetryContext,
    #[serde(default)]
    pub events: Vec<InterventionTelemetryEvent>,
}

impl InterventionTelemetryEvidence {
    pub fn validate(&self) -> anyhow::Result<()> {
        self.context.validate()?;
        anyhow::ensure!(
            self.events.len() <= MAX_INTERVENTION_EVENTS,
            "intervention telemetry contains too many events"
        );
        let mut kinds = std::collections::BTreeSet::new();
        for event in &self.events {
            anyhow::ensure!(
                kinds.insert(event.kind as u8),
                "intervention telemetry contains duplicate event kinds"
            );
            event.validate()?;
        }
        Ok(())
    }

    pub fn to_result(&self) -> InterventionTelemetryResult {
        let mut result = InterventionTelemetryResult::default();
        for event in &self.events {
            match event.kind {
                InterventionEventKind::Selected => {
                    result.selected_card_count = event.count;
                    result.selected = event.state;
                }
                InterventionEventKind::Injected => {
                    result.injection_count = event.count;
                    result.injected = event.state;
                }
                InterventionEventKind::ActionAttempted => {
                    result.action_attempt_count = event.count;
                    result.action_attempted = event.state;
                }
                InterventionEventKind::AgentVerificationAttempted => {
                    result.agent_verification_attempt_count = event.count;
                    result.agent_verification_attempted = event.state;
                }
                InterventionEventKind::IndependentVerificationPassed => {
                    result.independent_verification_pass_count = event.count;
                    result.independent_verification_passed = event.state;
                }
                InterventionEventKind::Outcome => {
                    result.outcome_count = event.count;
                    result.outcome_observed = event.state;
                    result.outcome_success = event
                        .outcome_success
                        .map(|success| {
                            if success {
                                AttributionState::Yes
                            } else {
                                AttributionState::No
                            }
                        })
                        .unwrap_or(AttributionState::Unknown);
                }
            }
            if event.state == AttributionState::Unknown {
                result.unknown_event_count = result.unknown_event_count.saturating_add(1);
            }
        }
        result
    }
}

/// Aggregate-only telemetry included in public evaluation results.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct InterventionTelemetryResult {
    pub selected_card_count: u32,
    pub injection_count: u32,
    pub action_attempt_count: u32,
    pub agent_verification_attempt_count: u32,
    pub independent_verification_pass_count: u32,
    pub outcome_count: u32,
    pub outcome_success: AttributionState,
    pub selected: AttributionState,
    pub injected: AttributionState,
    pub action_attempted: AttributionState,
    pub agent_verification_attempted: AttributionState,
    pub independent_verification_passed: AttributionState,
    pub outcome_observed: AttributionState,
    pub unknown_event_count: u32,
}

impl InterventionTelemetryResult {
    pub fn without_intervention(outcome_observed: Option<bool>) -> Self {
        Self {
            selected: AttributionState::No,
            injected: AttributionState::No,
            outcome_count: u32::from(outcome_observed.is_some()),
            outcome_observed: outcome_observed
                .map(|_| AttributionState::Yes)
                .unwrap_or(AttributionState::Unknown),
            outcome_success: outcome_observed
                .map(|success| {
                    if success {
                        AttributionState::Yes
                    } else {
                        AttributionState::No
                    }
                })
                .unwrap_or(AttributionState::Unknown),
            ..Self::default()
        }
    }
}

/// Aggregate counts used by the public evaluation result.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct InterventionTelemetryAggregate {
    pub selected_card_count: u64,
    pub injection_count: u64,
    pub action_attempt_count: u64,
    pub agent_verification_attempt_count: u64,
    pub independent_verification_pass_count: u64,
    pub outcome_count: u64,
    pub unknown_event_count: u64,
}

impl InterventionTelemetryAggregate {
    pub fn add(&mut self, result: &InterventionTelemetryResult) {
        self.selected_card_count = self
            .selected_card_count
            .saturating_add(u64::from(result.selected_card_count));
        self.injection_count = self
            .injection_count
            .saturating_add(u64::from(result.injection_count));
        self.action_attempt_count = self
            .action_attempt_count
            .saturating_add(u64::from(result.action_attempt_count));
        self.agent_verification_attempt_count = self
            .agent_verification_attempt_count
            .saturating_add(u64::from(result.agent_verification_attempt_count));
        self.independent_verification_pass_count = self
            .independent_verification_pass_count
            .saturating_add(u64::from(result.independent_verification_pass_count));
        self.outcome_count = self
            .outcome_count
            .saturating_add(u64::from(result.outcome_count));
        self.unknown_event_count = self
            .unknown_event_count
            .saturating_add(u64::from(result.unknown_event_count));
    }
}

/// Derive private event evidence from bounded trace observations.
pub fn derive_intervention_telemetry(
    context: &InterventionTelemetryContext,
    selected_card_count: usize,
    injection_count: u64,
    trace: Option<&RolloutTrace>,
    independent_verification_passed: Option<bool>,
    outcome: Option<bool>,
) -> InterventionTelemetryEvidence {
    let selected_card_count = u32::try_from(selected_card_count).unwrap_or(u32::MAX);
    let injection_count = u32::try_from(injection_count).unwrap_or(u32::MAX);
    let action_count = trace
        .map(|trace| matching_signature_count(&context.action_signatures, trace))
        .unwrap_or_default();
    let action_state = if action_count > 0 {
        AttributionState::Yes
    } else if context.reviewed_action_attempted != AttributionState::Unknown {
        context.reviewed_action_attempted
    } else {
        AttributionState::Unknown
    };
    let verification_count = trace
        .map(|trace| matching_signature_count(&context.agent_verification_signatures, trace))
        .unwrap_or_default();
    let verification_state = if verification_count > 0 {
        AttributionState::Yes
    } else {
        AttributionState::Unknown
    };
    let selected_state = if selected_card_count > 0 {
        AttributionState::Yes
    } else {
        AttributionState::No
    };
    let injection_state = if injection_count > 0 {
        AttributionState::Yes
    } else if selected_card_count == 0 {
        AttributionState::No
    } else {
        AttributionState::Unknown
    };
    let independent_state = independent_verification_passed
        .map(|passed| {
            if passed {
                AttributionState::Yes
            } else {
                AttributionState::No
            }
        })
        .unwrap_or(AttributionState::Unknown);
    let outcome_state = outcome
        .map(|_| AttributionState::Yes)
        .unwrap_or(AttributionState::Unknown);

    InterventionTelemetryEvidence {
        context: context.clone(),
        events: vec![
            event(
                InterventionEventKind::Selected,
                selected_state,
                selected_card_count,
                None,
            ),
            event(
                InterventionEventKind::Injected,
                injection_state,
                injection_count,
                None,
            ),
            event(
                InterventionEventKind::ActionAttempted,
                action_state,
                action_count.max(u32::from(action_state == AttributionState::Yes)),
                None,
            ),
            event(
                InterventionEventKind::AgentVerificationAttempted,
                verification_state,
                verification_count,
                None,
            ),
            event(
                InterventionEventKind::IndependentVerificationPassed,
                independent_state,
                u32::from(independent_verification_passed == Some(true)),
                None,
            ),
            event(
                InterventionEventKind::Outcome,
                outcome_state,
                u32::from(outcome.is_some()),
                outcome,
            ),
        ],
    }
}

fn event(
    kind: InterventionEventKind,
    state: AttributionState,
    count: u32,
    outcome_success: Option<bool>,
) -> InterventionTelemetryEvent {
    InterventionTelemetryEvent {
        kind,
        state,
        count,
        outcome_success,
    }
}

fn matching_signature_count(
    signatures: &[InterventionActionSignature],
    trace: &RolloutTrace,
) -> u32 {
    if signatures.is_empty() {
        return 0;
    }
    let mut count = 0usize;
    for operation in trace.terminal_operations.values() {
        let matched = signatures
            .iter()
            .any(|signature| signature_matches_terminal(signature, &operation.request));
        if matched {
            count = count.saturating_add(1);
        }
    }
    for tool in trace
        .tool_calls
        .values()
        .filter(|tool| tool.terminal_operation_id.is_none())
    {
        let tool_family = tool_family_for_kind(&tool.kind);
        let operation_class = if tool_family == ToolFamily::None {
            OperationClass::None
        } else {
            OperationClass::Tool
        };
        if signatures
            .iter()
            .any(|signature| signature_matches_tool(signature, operation_class, tool_family))
        {
            count = count.saturating_add(1);
        }
    }
    u32::try_from(count).unwrap_or(u32::MAX)
}

fn signature_matches_terminal(
    signature: &InterventionActionSignature,
    request: &TerminalRequest,
) -> bool {
    let (tool_family, operation_class, command_tokens) = match request {
        TerminalRequest::ExecCommand {
            command,
            display_command,
            ..
        } => (
            ToolFamily::ExecCommand,
            OperationClass::Command,
            if command.is_empty() {
                split_command_tokens(display_command)
            } else {
                command
                    .iter()
                    .map(|token| token.to_ascii_lowercase())
                    .collect()
            },
        ),
        TerminalRequest::WriteStdin { .. } => {
            (ToolFamily::WriteStdin, OperationClass::Command, Vec::new())
        }
    };
    (signature.operation_class == OperationClass::Unknown
        || signature.operation_class == operation_class)
        && signature.tool_family == tool_family
        && command_tokens_match(&signature.command_tokens, &command_tokens)
}

fn signature_matches_tool(
    signature: &InterventionActionSignature,
    operation_class: OperationClass,
    tool_family: ToolFamily,
) -> bool {
    (signature.operation_class == OperationClass::Unknown
        || signature.operation_class == operation_class)
        && signature.tool_family == tool_family
        && signature.command_tokens.is_empty()
}

fn command_tokens_match(expected: &[String], actual: &[String]) -> bool {
    expected.iter().all(|expected| {
        let expected = expected.to_ascii_lowercase();
        actual.iter().any(|actual| actual == &expected)
    })
}

fn split_command_tokens(command: &str) -> Vec<String> {
    command
        .split_ascii_whitespace()
        .map(|token| token.trim_matches(['\'', '"', '`']).to_ascii_lowercase())
        .filter(|token| !token.is_empty())
        .collect()
}

fn tool_family_for_kind(kind: &ToolCallKind) -> ToolFamily {
    match kind {
        ToolCallKind::ExecCommand => ToolFamily::ExecCommand,
        ToolCallKind::WriteStdin => ToolFamily::WriteStdin,
        ToolCallKind::ApplyPatch => ToolFamily::ApplyPatch,
        ToolCallKind::Mcp { .. } => ToolFamily::Mcp,
        ToolCallKind::Web => ToolFamily::Web,
        ToolCallKind::ImageGeneration => ToolFamily::ImageGeneration,
        ToolCallKind::SpawnAgent
        | ToolCallKind::AssignAgentTask
        | ToolCallKind::SendMessage
        | ToolCallKind::WaitAgent
        | ToolCallKind::CloseAgent => ToolFamily::Agent,
        ToolCallKind::Other { .. } => ToolFamily::Other,
    }
}

fn ensure_safe_id(value: &str, label: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        !value.is_empty()
            && value.chars().count() <= MAX_TELEMETRY_ID_CHARS
            && !value
                .chars()
                .any(|character| character.is_control() || matches!(character, '<' | '>')),
        "{label} is empty, oversized, or unsafe"
    );
    Ok(())
}

fn ensure_safe_fingerprint(value: &str, label: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        !value.is_empty()
            && value.chars().count() <= MAX_TELEMETRY_FINGERPRINT_CHARS
            && value.starts_with("sha256:")
            && value.len() == "sha256:".len() + 64
            && value["sha256:".len()..]
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit()),
        "{label} is not a bounded fingerprint"
    );
    Ok(())
}
