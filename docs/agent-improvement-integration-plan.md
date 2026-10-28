# Agent Improvement Integration Plan

Status: Milestone 8a.3 benchmark/evaluator qualification infrastructure is in
progress; Milestones 1–7 and 8a–8a.2 are complete; the 8a.3 exit gate, the
remaining practical-value track (8a.4–8a.7), and full program-level evolution
remain gated

Latest live study: [Milestone 8a.1 pilot report](agent-improvement-pilot-8a1.md).
It found no task-success lift in the first corrected 8-task study, so Milestone
8 remains gated.

## Objective

Integrate the immediately adoptable, essential, and high-value ideas from the
Meta^n repository into this Codex fork without creating a second agent runtime.
The implementation should make Codex better at learning from observed work while
preserving existing instruction, permission, sandbox, privacy, and extension
boundaries.

The target is a reviewed, measurable improvement flywheel:

```text
Codex rollout trace
        |
        v
Reduced evidence and failure classification
        |
        v
Candidate lesson in a local review queue
        |
        v
Diagnosis, candidate strategy, or bounded experience card
        |
        v
Sandboxed baseline/holdout evaluation
        |
        v
Explicit review, promotion, rollback, and scoped retrieval
        |
        v
Better execution and verification, producing new evidence
```

This deliberately uses the existing Codex rollout-trace and extension systems.
It does not execute Meta^n-generated code, mutate `AGENTS.md` automatically, or
send local traces anywhere. The endpoint is controlled continual improvement of
the agent's local operating guidance, not model-weight training and not an
unreviewed self-modifying runtime.

## Evidence and design constraints

The Meta^n repository contributes the following reusable concepts:

- structured execution traces instead of transcript-only memory;
- explicit failure classes and next-step guidance;
- an archive of reusable experience with bounded retrieval;
- adoption and provenance signals;
- regression-aware promotion rather than unconditional prompt mutation.

The Codex fork already provides the required foundations:

- opt-in local rollout bundles and deterministic replay in
  `codex-rs/rollout-trace`;
- extension prompt and turn-input contributors in
  `codex-rs/ext/extension-api`;
- persistent Goals and existing Skills/AGENTS.md support;
- a `codex debug` CLI surface;
- local-only state and sandboxing primitives.

OpenAI's current Codex workflow guidance also recognizes `AGENTS.md` as durable
repository guidance and recommends separating goals, plans, prompts, phase
context, and observed evidence. See:

- <https://developers.openai.com/cookbook/examples/codex/iterating-development-workflows-with-codex>
- <https://developers.openai.com/cookbook/examples/codex/using_goals_in_codex>
- <https://developers.openai.com/cookbook/examples/agents_sdk/agent_improvement_loop>

## Non-goals

- No automatic model-generated code execution from Meta^n.
- No automatic edits to `AGENTS.md`, Skills, prompts, or repository code.
- No upload or remote analytics path for raw rollout traces.
- No claim that a per-task best card is a generally superior policy.
- No replacement of the existing memories subsystem.
- No new model calls in the hot path for classification or card selection.
- No change to default Codex behavior until the feature is explicitly enabled.

These are release-one constraints. Later milestones may use a model in an
offline proposer/evaluation path, but they must retain the same permission,
privacy, provenance, rollback, and human-review boundaries.

## Roadmap and milestones

Milestones 1–7, the bounded Milestone 8a bridge, and the independent
Milestone 8a.1 evaluator are implemented in this working tree. Milestone 4
intentionally lands as an offline replay/evaluation core: it consumes matched
artifacts produced by separately controlled baseline and treatment runs.
Milestone 8a adds an explicitly manual bridge that can produce those artifacts
in isolated worktrees, while Milestone 8a.1 makes the live outcome measurable
without conflating policy diagnostics with coding success. Milestone 8a.2
hardens the evaluator contract and makes retrieval exposure observable without
leaking card identity into published results. The first pilot still found no
task-success lift, so 8a.2 is treated as measurement and retrieval
infrastructure, not as evidence that the agent improved. Milestones 8a.3–8a.7
are a practical-value track: qualify a harder benchmark, turn broad advisory
cards into scoped interventions, measure whether an intervention was
attempted, and repeat the study before allowing a canary. None of these
milestones promotes a result or changes normal Codex behavior by default. The
remaining milestones are ordered by the dependency that makes each new form
of autonomy safe and measurable. A milestone is not considered complete merely
because its code exists; its exit gate and regression evidence must also be
satisfied.

| Milestone | Outcome | Priority | Depends on | Status |
| --- | --- | --- | --- | --- |
| 1. Safe local experience foundation | Deterministic, privacy-bounded cards can be created explicitly and retrieved as advisory context | Essential | Existing trace and extension systems | Complete |
| 2. Automatic candidate capture and review | Long sessions produce reviewable candidates without requiring a manual CLI step; nothing is promoted automatically | Immediately next | 1 | Complete |
| 3. Evidence-rich lessons and retrieval | Cards describe recurring failure signatures, interventions, outcomes, scope, and confidence rather than only coarse summaries | High value | 2 | Complete |
| 4. Replay and evaluation harness | Baseline versus memory-assisted artifacts can be measured on paired and held-out tasks with bounded provenance and cost accounting | Essential before autonomy | 2–3 | Offline core complete; live bridge added in 8a |
| 5. Offline proposer and candidate artifacts | A focused Skill and deterministic offline command can produce evidence-linked, review-only proposals for cards and bounded guidance/tooling changes | High value | 3–4 | Review-only core complete; live model proposer gated |
| 6. Promotion, rollback, and policy control plane | Candidate changes become versioned, scoped, reversible active knowledge only after evidence and review | Essential | 4–5 | Complete; manual promotion only |
| 7. Slow loop for long-horizon improvement | Periodic analysis finds recurring patterns, stale knowledge, and missed evidence, then proposes the next experiments | High value / research | 4–6 | Complete; deterministic/manual loop |
| 8a. Bounded live evaluation and feedback bridge | Run a private held-out brief in paired isolated worktrees, produce a redacted result, and optionally record one explicit outcome | Essential before optional autonomy | 4–7 | Complete; opt-in/manual; no automatic promotion |
| 8a.1. Independent evaluation and observability hardening | Execute bounded evaluator-side checks before cleanup and separate coding outcome from trace and policy diagnostics | Essential before Milestone 8 | 8a | Complete; opt-in/manual; no automatic promotion |
| 8a.2. Evaluator and retrieval hardening | Preflight evaluator contracts, bounded live retrieval, and separate selection/exposure evidence make the next pilot trustworthy | Essential before Milestone 8 | 8a.1 | Complete; opt-in/manual; no automatic promotion |
| 8a.3. Benchmark and evaluator qualification | A realistic, non-ceiling task pack and adversarial evaluator fixtures make behavioral claims trustworthy | Essential | 8a.2 | In progress; contract and no-model qualification harness implemented, private pack/calibration/review pending |
| 8a.4. Intervention cards and scoped retrieval | Cards contain a trigger, one bounded action, a verification target, and explicit scope; retrieval is task-local and low-noise | High value | 8a.3 | Planned |
| 8a.5. Causal exposure and action telemetry | Private evidence distinguishes card selection, injection, attempted action, verification, and outcome without claiming causation from exposure | Essential | 8a.4 | Planned |
| 8a.6. Repeated practical-value pilot | A preregistered 32–48-task study tests correctness, recovery, cost, and negative transfer across repeated trials | Essential decision gate | 8a.3–8a.5 | Planned |
| 8a.7. Scoped canary and operating loop | A passing intervention can be manually activated for a narrow scope with TTL, monitoring, and rollback | Conditional high value | 8a.6 | Planned; only if 8a.6 passes |
| 8. Optional program-level evolution and transfer | Isolated evolution of skills, prompts, or extension policy can be explored across task families without mutating the live agent | Research only | 8a.6–8a.7 | Planned; do not start before practical-value gates |

### Milestone 1 — Safe local experience foundation (complete)

The implemented first increment is intentionally narrow. It provides the safe
memory substrate on which the rest of the roadmap depends:

- a deterministic reduced-trace summarizer and bounded `ExperienceCard` schema;
- an append-only, local JSONL store with malformed-data and resource limits;
- explicit CLI capture and inspection commands;
- opt-in advisory retrieval through the extension system;
- no automatic capture, no generated policy changes, and no model call in the
  hot path.

The detailed implementation tasks and validation record for this milestone are
below. Its known limitation is important: one aggregate card per manually
selected rollout is useful for repeated local failure patterns, but it is not
yet an automatic learning loop or a causal record that a recommendation helped.

### Milestone 2 — Automatic candidate capture and human review

**Implementation status: complete.**

The first review-gated capture loop is implemented and covered by focused unit
and integration tests. Capture remains independently switchable from retrieval
and is disabled by default. A candidate is never prompt-visible until the
user explicitly accepts it.

**Intent:** remove the operational burden of remembering to run the summarizer
after a long session while keeping the candidate-to-active boundary explicit.

**Deliverables:**

- Add a separate candidate store, for example:

  ```text
  $CODEX_HOME/agent-improvement/
    cards.jsonl          # reviewed/promoted cards consumed by retrieval
    candidates.jsonl     # pending or rejected proposals, never injected
  ```

- Add an opt-in capture mode alongside the existing retrieval setting. The
  default remains off. The first enabled mode is `candidate`, not
  `auto_promote`.
- Use the existing extension lifecycle boundaries (`on_turn_stop`,
  `on_turn_error`, `on_turn_abort`, and `on_thread_stop`) plus the existing
  rollout-trace finalization path to enqueue bounded post-turn or post-thread
  summarization work. Summarization must not block model/tool execution or
  make a failed turn fail harder.
- Produce bounded, deterministic candidates at turn and root-thread lifecycle
  boundaries. Candidate records retain the source trace, source turn, trigger,
  and derived card identity; queue deduplication collapses repeated lifecycle
  notifications for one source turn. Milestone 3 now adds a bounded observed
  sequence and verification counters to the promoted card without copying raw
  trace content; full episode segmentation remains evaluation/hardening work.
- Add candidate lifecycle metadata: candidate ID, source trace ID, source turn
  or episode ID, creation time, status (`pending`, `accepted`, `rejected`, or
  `expired`), and a deterministic content fingerprint. Keep raw trace data in
  the trace bundle; do not copy it into the candidate store.
- Add explicit inspection and review commands, such as:

  ```text
  codex debug agent-improvement candidates list --store <DIR>
  codex debug agent-improvement candidates show <ID> --store <DIR>
  codex debug agent-improvement candidates accept <ID> --store <DIR>
  codex debug agent-improvement candidates reject <ID> --store <DIR>
  ```

  `accept` must create or update a reviewed card through the same schema and
  validation path as the manual flow. A pending candidate must never enter
  prompt context.
- Ship a focused `agent-card-curator` Skill as a user-invoked review workflow.
  It may list candidates, explain bounded evidence, request an explicit
  accept/reject decision, and call the CLI. The Skill is an interface to the
  review queue, not permission to promote silently. It cannot recover a trace
  that was not captured before the session started; automatic capture solves
  that lifecycle limitation.

**Configuration shape:** extend `[agent_improvement]` with an explicitly
documented capture mode and independent queue limits. Keep retrieval and
capture independently switchable. The implemented controls are
`capture = "off"` or `capture = "candidate"` and `max_candidates`; the store
also enforces fixed byte, line, and record bounds. Candidate retention and
episode-specific limits remain follow-up work after Milestone 3 supplies the
metadata needed to expire or merge candidates safely. Automatic promotion is
not exposed.

**Tests and verification:**

- fake lifecycle sequences for normal completion and root-thread shutdown;
- prove capture runs in a blocking worker and that store or replay errors do
  not affect the turn result;
- prove replay idempotence, candidate deduplication, queue limits,
  append-only status transitions, malformed-line tolerance, and no
  candidate-to-prompt leakage;
- integration-test the review CLI against a temporary store with explicit
  accept/reject decisions;
- defer richer episode grouping, expiration, concurrent-writer stress testing,
  and capture-overhead benchmarking to the follow-up hardening work attached
  to Milestones 3–4.

**Exit gate:** a user can finish a long session, find a bounded candidate
without remembering a follow-up command, and review it later. The live agent's
behavior is unchanged until the user explicitly accepts the candidate.

The exit gate is satisfied by the automatic root-trace path, lifecycle capture
callbacks, bounded candidate JSONL store, review CLI, and embedded
`agent-card-curator` Skill. Acceptance writes the validated card through the
existing active-card store and then appends the terminal candidate status.

### Milestone 3 — Evidence-rich lessons and retrieval

**Implementation status: complete.**

This increment keeps the existing local JSONL format and review boundary, but
upgrades the derived card contract to schema version 2. Existing version 1
cards and embedded candidate cards are migrated in memory with conservative
unknown values; unknown schema versions are skipped. Raw prompts, outputs,
paths, secrets, and concrete model identifiers remain out of durable card
text.

The implementation is deliberately measurable: controlled signature,
observed-sequence, and evidence fields, opaque workspace scope,
recurrence/confidence/adoption
metadata, retirement, deterministic structured ranking, recency decay, and
conflict suppression. Attribution remains descriptive (`unknown` unless
observed), and paired baseline/treatment claims remain Milestone 4 work.

**Intent:** make retrieved experience more actionable than a coarse failure
label while retaining the privacy and prompt-budget properties of Milestone 1.

**Deliverables:**

- Version the card schema to represent a bounded failure signature and an
  observed sequence, using controlled vocabularies rather than unrestricted
  transcript text. Useful fields include operation class, tool family, exit
  bucket, policy/sandbox reason, verification state, intervention labels,
  recovery outcome, recurrence count, and last-seen time.
- Represent the distinction between:
  - an observed fact (`command_failed`, `verification_missing`);
  - a hypothesis (`likely_missing_dependency`); and
  - a recommendation (`rerun the narrow verification command`).
  Hypotheses must carry lower confidence than observations and must not be
  presented as policy.
- Add scope to every card: repository/workspace identity, task family, and
  optionally toolchain/model family. Scope must prevent a lesson learned in
  one repository from silently becoming guidance in an unrelated one.
- Add evidence of adoption without claiming causality: retrieval exposure,
  explicit user feedback, subsequent verification result, and whether the
  recommendation was followed when that can be observed. If the linkage is
  unknown, record `unknown`, not success.
- Improve ranking from token overlap alone with deterministic structured
  filters and recency/confidence/recurrence signals. Keep selection stable,
  bounded, and explainable; retain a lexical fallback for unknown task types.
- Add score-based stale-card decay, contradiction handling, duplicate merging,
  and an explicit `retire` state. Stale cards are not silently deleted;
  retirement preserves their audit record.
- Add explicit local controls for feedback and descriptive outcome attribution:
  `cards feedback` records usefulness feedback, while `cards outcome` records
  whether a recommendation was followed or verified. Unknown remains the
  default when the linkage is not observable.

**Tests and verification:**

- schema migration and downgrade/unknown-field tests;
- fixture-based ranking tests for task family, failure class, tool class,
  scope, recency, confidence, recurrence, and ties;
- tests proving a low-confidence hypothesis cannot outrank a directly observed
  matching failure without evidence;
- privacy tests for secrets, paths, prompts, outputs, identifiers, and control
  characters across all new fields;
- an offline retrieval fixture benchmark with fixed cards and a query,
  tracking relevant-card precision, irrelevant-card rate, context size, and
  stable ordering;
- tests for exposure/outcome attribution that distinguish `seen`, `accepted`,
  `followed`, `verified`, and `unknown`.

**Exit gate:** the fixed offline retrieval fixture now demonstrates scoped,
evidence-first selection, conflict suppression, bounded rendering, and stable
ordering without unsafe or instruction-conflicting card content. This is a
retrieval-contract result, not a claim about task success; Milestone 4 remains
required for a paired baseline/treatment claim.

### Milestone 4 — Replay and evaluation harness

**Implementation status: offline replay/evaluation core complete; live runner
execution remains separately gated.**

**Intent:** turn “this card seems helpful” into a repeatable baseline-versus-
assisted experiment before introducing model-generated changes or automatic
promotion.

**Implementation boundary:** the first implementation evaluates two existing
trace bundles per scenario. It does not launch a model, inherit credentials,
modify a worktree, or read/write the active card store. This is intentional:
the result schema and pairing/isolation checks can run in offline CI before a
live runner is trusted with Codex's approval, sandbox, and repository
boundaries. A live runner must be added as a separately gated adapter that
produces the same bundle contract.

**Deliverables:**

- Define a local scenario manifest containing a task prompt or task reference,
  repository snapshot/worktree, expected verification command(s), allowed
  configuration, time/token budget, and an evaluator type. Keep scenario
  inputs and private traces separate from published aggregate results.
- Add an offline evaluation command/library, for example:

  ```text
  codex debug agent-improvement eval <SCENARIO_SET> \
    --output <RESULTS>
  ```

  It must validate that matched baseline and memory-assisted conditions were
  recorded from isolated worktrees or sandboxes and preserve the exact
  card/policy versions used. The current command consumes those artifacts;
  it does not launch a fresh model run.
- Reuse rollout traces to calculate outcome metrics: task verification
  success, incomplete verification, repeated failure, retry count, tool and
  terminal operations, tokens, wall time, cancellation, policy/permission
  events, and user correction/override when available.
- Support both deterministic evaluators (tests, linters, type checks, expected
  repository state) and explicitly labeled human/model-judge evaluators. A
  judge score is evidence, not a substitute for an executable verification
  signal when one exists.
- Require a held-out split by task instance and report results per task family,
  not only a pooled mean. Keep routed/per-task-best results separate from a
  single policy result.

**Tests and verification:**

- deterministic fixture scenarios with known baseline and treatment outcomes;
- isolation tests proving an evaluation cannot modify the primary checkout or
  active card store;
- result-schema tests for missing metrics, interrupted runs, partial traces,
  evaluator failure, and cost limits;
- paired-run tests proving the same scenario/configuration is used for baseline
  and treatment except for the declared memory difference;
- regression tests for false attribution, where a task improves for an
  unrelated reason or a retrieved card is present but not followed;
- a small smoke suite that runs in CI without network/model credentials, plus
  an opt-in live suite for real model comparisons.

**Exit gate:** given reproducible paired artifacts, the project can answer
whether a card or retrieval policy changes success, cost, latency,
verification, and safety outcomes on held-out tasks. The evaluation harness
becomes a required dependency for Milestones 5 and 6. A live runner is not
implicitly part of this gate; it requires its own permission and isolation
review.

### Milestone 5 — Offline proposer and candidate artifacts

**Implementation status: review-only proposal envelope, deterministic card
drafting, evidence validation, and curator Skill workflow complete; live model
proposer and artifact application remain gated.**

**Intent:** use Codex itself, a focused Skill, or an offline model call to
  propose better lessons and operating guidance without giving that proposer
  direct control of the live agent.

**Deliverables:**

- Define a versioned `ImprovementCandidate` envelope containing candidate type,
  target scope, evidence IDs, rationale, expected benefit, expected costs and
  risks, proposed diff/content, validation commands, and expiry/rollback
  metadata. Candidate types should include:
  - experience-card update;
  - focused Skill change;
  - repository `AGENTS.md` patch;
  - prompt/extension configuration change; and
  - diagnostic/tooling issue that should be fixed in Codex code.
- Start with the `agent-card-curator` Skill as the user-facing proposer. It
  can summarize candidates, ask Codex to draft a bounded improvement, and
  produces a reviewable artifact. The deterministic command
  `codex debug agent-improvement proposals draft-card` now creates a card
  proposal from one pending candidate; `proposals validate` performs structural
  and optional evidence-catalog validation. A future offline model adapter may
  produce the other artifact lanes, but it remains a separate slow-path
  permission and privacy review.
- Keep the proposer on the slow path: it may read reduced, redacted evidence
  and selected trace references, but it must not run in every turn or alter
  permissions, approval policy, sandbox policy, system instructions, or
  repository files as a side effect.
- Validate model output against a strict schema, reject prompt-injection-like
  instructions and unbounded content, and require a concrete regression test
  or evaluation scenario for any behavioral candidate.
- For code or repository-guidance candidates, emit a diff in an isolated
  worktree. Do not apply it to the user's checkout automatically.

**Tests and verification:**

- fake proposer outputs for valid, malformed, oversized, contradictory, and
  adversarial candidates;
- schema validation and redaction tests for every candidate type;
- tests proving candidate generation is read-only with respect to the live
  Codex configuration and repository;
- evaluator integration tests proving a candidate references real evidence and
  executable checks;
- curator Skill tests for explicit approval, rejection, and cancellation.

The implemented core covers valid, malformed, oversized, contradictory, and
adversarial proposal fixtures across all five artifact lanes; redaction and
read-only serialization; deterministic IDs; evidence-catalog validation;
candidate-store immutability during drafting; CLI parser and end-to-end draft
and validation tests; and embedded Skill asset validation. It does not invoke
a model or apply a generated diff.

**Exit gate:** a proposer can generate reviewable, evidence-linked artifacts
  and a test plan. No proposed artifact influences live behavior until the
  Milestone 6 control plane accepts it. Card proposals now have a separate
  promotion path; the older candidate-accept command remains a direct,
  explicitly user-invoked card review path for the foundational workflow.

### Milestone 6 — Promotion, rollback, and policy control plane

**Implementation status: complete for the manual card lane; non-card lanes and
automatic promotion remain deliberately gated.**

**Intent:** make improvement reversible and scoped when evidence justifies
  moving it from a candidate into active knowledge.

The implementation adds an append-only `promotion-events.jsonl` log under the
agent-improvement root. The current state is always rebuilt by replaying that
log; there is no authoritative mutable index to become stale after a crash.
Runtime retrieval requires both an active card record and an active promotion
event for promoted cards. A failed event-store or policy read therefore skips
improvement context rather than changing behavior on incomplete evidence.

The supported manual workflow is:

```text
proposals validate --state-store <ROOT> --candidate-store <ROOT> ...
proposals evaluate --store <ROOT> --candidate-store <ROOT> ...
proposals accept --store <ROOT> <PROPOSAL>
proposals promote --store <ROOT> <PROPOSAL>
proposals status --store <ROOT> <PROPOSAL>
proposals rollback --store <ROOT> <PROPOSAL>
```

`evaluate` requires held-out baseline/treatment evidence with matching
execution and policy fingerprints, no task-success or verification regression,
and no additional policy-boundary events. `promote` requires the evaluated and
accepted state, a non-expired proposal, a passing policy scope, and a card
lane. Skill, `AGENTS.md`, prompt/configuration, and diagnostic lanes remain
review artifacts that must be applied through their normal diff/review
process; this subsystem never grants permissions or changes sandbox/approval
behavior.

The optional `promotion-policy.json` file supports global enable/disable,
kill-switch, TTL, active-count, and most-specific repository/task-family
allowlist scopes. If it is absent, the bounded manual defaults are used. If an
existing policy or event log is malformed, runtime retrieval fails closed and
promotion commands refuse to proceed.

**Deliverables:**

- Add a versioned state machine:

  ```text
  proposed -> validated -> evaluated -> accepted -> active -> retired
                 \-> rejected
  ```

  Every transition records actor, timestamp, evidence, evaluator result, scope,
  and previous version. State transitions are append-only or recoverable; the
  active index can be rebuilt from the log.
- Add explicit commands for `validate`, `evaluate`, `promote`, `rollback`,
  `retire`, and `status`. `promote` must refuse candidates without required
  evaluation evidence or an explicit documented override.
- Keep cards, Skills, `AGENTS.md` patches, and configuration changes in
  separate promotion lanes. A card may become active retrieval context; a
  repository guidance patch requires a normal diff review; a permission or
  sandbox change is never promoted by this subsystem.
- Add per-repository and per-task-family policy scopes, TTLs, kill switches,
  and a one-command rollback to the previous active version. A failed store or
  policy lookup falls back to the last known safe behavior or disabled mode.
- Preserve provenance from active context back to the candidate, trace IDs,
  evaluator runs, and human decision. Render this provenance in inspection
  output so a user can explain why a card was shown.

**Tests and verification:**

- state-machine transition and recovery tests, including crashes between log
  append and index update;
- promotion-gate tests for missing evidence, regression, expired candidate,
  scope mismatch, and stale schema;
- rollback tests proving the previous active policy is restored without
  rewriting unrelated cards;
- permission-boundary tests proving promoted content cannot grant tools,
  bypass approvals, change sandboxing, or outrank repository/system
  instructions;
- end-to-end baseline/active/rollback evaluation tests.

**Exit gate:** every active improvement is attributable, scoped, measurable,
  and reversible. Manual promotion is the only supported promotion mode at
  first. Any future automatic promotion must be a separately gated experiment,
  disabled by default.

The exit gate is satisfied for the card lane. The implementation records
candidate fingerprints, target scope, evidence IDs, held-out evaluation
summaries, actor decisions, expiry, and rollback ancestry in each lifecycle
projection. The focused suite covers log replay without an index, malformed
store fail-closed behavior, missing evidence, regression, expiry, stale
schema, policy scope mismatch, kill switches, TTL filtering, non-card lane
boundaries, and previous-card rollback. The CLI integration test exercises the
complete manual lifecycle against a temporary store.

### Milestone 7 — Slow loop for long-horizon improvement (complete)

**Implementation status: complete for the deterministic/manual local loop;
model-backed proposing and automatic promotion remain gated. Live execution is
provided separately by Milestones 8a and 8a.1.**

The extension now provides `codex debug agent-improvement evolve` operations
that run outside turn preparation and consume only bounded local cards,
candidates, and explicit experiment outcomes. The aggregation layer identifies:

- recurring failure signatures;
- repeated verification omissions;
- frequently exposed cards with explicit `followed = no` attribution;
- conflicting active recommendations in one task/scope/signature bucket; and
- observed task-family/scope combinations with no useful active coverage.

Each deterministic experiment brief contains a stable ID and fingerprint,
bounded card/candidate/trace provenance, the problem statement, competing
hypotheses, card-lane candidate changes, an expected metric, and a held-out
evaluation-set selection rule. The brief is an inert proposal; it cannot edit
cards, prompts, Skills, `AGENTS.md`, configuration, permissions, or sandbox
policy.

The append-only `evolution-history.jsonl` store records explicit improved,
no-improvement, regression, inconclusive, or retired outcomes. Repeated failed
outcomes lower priority and suppress a brief after the configured threshold,
preventing an unproductive experiment from being proposed indefinitely.

Bounded maintenance is explicit and separate from the slow-loop report:

- `maintain` plans stale decay, negative-feedback retirement, and active-card
  retention; `--apply` retires selected cards without deleting history;
- `compact` merges the canonical card view after archiving the previous JSONL;
- `export` writes a canonical bounded JSONL snapshot; and
- `forget` permanently removes named cards only after requiring an archive.

The supported commands are:

```text
evolve run --store <ROOT> [--output <FILE>]
evolve record --store <ROOT> <BRIEF_ID> --fingerprint <FP> --outcome <OUTCOME>
evolve maintain --store <ROOT> [--apply] [--compact --archive <FILE>]
evolve export --store <ROOT> --output <FILE>
evolve forget --store <ROOT> --archive <FILE> <CARD_ID>...
```

**Tests and verification:** the focused suite covers deterministic multi-signal
brief generation, held-out evaluation metadata, bounded budgets, repeated-run
stability, stagnation suppression, stale/negative/retention planning,
non-destructive retirement, archive-backed compaction, export, explicit
forgetting, CLI parsing, and an end-to-end CLI lifecycle. The slow loop makes
no model calls and does not promote its own output.

**Exit gate:** complete. Long local histories produce a small explainable set
of scoped experiments with provenance, and repeated failed outcomes stop being
re-proposed. Held-out outcome improvement is still required before describing
any resulting card or proposal as learned.

### Milestone 8a — Bounded live evaluation and feedback bridge (complete)

**Implementation status: complete as an opt-in execution bridge; no behavioral
gain is claimed until a user runs a real held-out study.**

Milestone 8a connects the deterministic Milestone 7 briefs to the existing
Milestone 4 evaluator. It closes the practical gap between “a brief describes
an experiment” and “the same experiment can be run reproducibly,” while keeping
the live path outside normal turn preparation.

**Deliverables:**

- A private, versioned live-plan contract containing held-out prompts or task
  references, the brief ID and fingerprint, a clean primary checkout, bounded
  evaluators, and per-scenario budgets. The plan is an operator input and must
  not be published with aggregate results.
- `codex debug agent-improvement evolve evaluate`, with `--dry-run` validation
  before any child process is launched. The command accepts either one brief
  or the JSON report from `evolve run`.
- One baseline and one treatment run per selected scenario, each from the same
  detached Git commit and in separate worktrees and trace roots. The baseline
  has retrieval disabled; the treatment receives a private copy of the active
  card store and preserves the source workspace scope while running in its
  temporary worktree.
- Explicit child-run controls: ephemeral execution, `workspace-write` sandbox,
  `approval_policy=never`, capture disabled, a per-scenario wall-time cap, and
  no automatic promotion. Trace, token, tool-call, and terminal-operation
  budgets are enforced by the existing evaluator after each run. The bridge
  does not bypass the sandbox or execute generated program changes outside
  Codex's normal command path.
- Reuse of the existing evaluator and redacted result writer, plus optional
  append-only recording of the derived experiment outcome. Exposure is recorded
  separately from adoption; recommendation-followed attribution remains
  unknown unless a separate evidence workflow establishes it.

The first operator workflow is:

```text
evolve run -> create private held-out plan -> evolve evaluate --dry-run
           -> evolve evaluate -> inspect result -> human review/promotion
```

The bridge requires a clean source checkout for a real run, inherits the
operator's model authentication and network availability, and may incur model
and tool costs. It supports the deterministic trace-outcome, trace-based
verification, and post-run verification evaluators in the live path;
human/model-judge labels remain private offline inputs. A dry run and the
isolated runner contract are tested, but no live model execution is part of
the repository test suite.

**Exit gate:** complete for the bridge implementation: private plans are
validated, paired execution is isolated, results are redacted and reusable by
the existing evaluator, and promotion remains a separate human action. The
research claim that a card improves Codex remains gated on a real historical
benchmark, held-out gain, cost accounting, no safety regression, and rollback.

### Milestone 8a.1 — Independent evaluation and observability hardening

**Implementation status: complete; opt-in/manual and no automatic promotion.**

Milestone 8a.1 closes the measurement gap found in the live pilot. A child
trace can contain a policy or sandbox event while still producing a completed
implementation, so trace diagnostics must not substitute for an independent
coding outcome.

**Deliverables:**

- Add the `post_run_verification` evaluator kind. Its checks are declared as
  bounded argv token vectors in the private plan and are executed only after
  the child process exits.
- Run post-run checks from the isolated condition worktree, without a shell,
  with standard streams disconnected, a scrubbed environment containing only
  the inherited `PATH`, and a bounded total evaluator time budget.
- Retain only check IDs and boolean results in the private manifest and
  redacted aggregate result. Command tokens, output, local paths, and judge
  evidence remain private.
- Publish explicit `trace_status` and `policy_status` metrics alongside the
  existing failure classification. A policy boundary is diagnostic evidence;
  it does not automatically turn an independently passing coding evaluator
  into a failed task.
- Run post-run checks before normal worktree cleanup. `--keep-worktrees`
  remains available for investigation, while the default path removes only
  the explicitly created evaluation worktrees after result writing.

**Tests and verification:**

- evaluator tests for passing, failing, missing, and trace-cost-limited
  post-run evidence;
- redaction tests proving command tokens and private paths do not enter the
  published result;
- argv/no-shell and timeout tests for the live command runner;
- trace/policy classification tests proving completed work with a policy
  boundary remains distinguishable from an evaluator failure;
- existing isolation, cleanup, and paired-condition regression suites.

**Exit gate:** a live held-out study can evaluate the produced worktree with
independent bounded checks before cleanup, publish only privacy-bounded
evidence, and report coding success, trace completion, and policy events as
separate signals. Milestone 8 remains gated on repeated studies using this
stronger evaluator.

### Milestone 8a.2 — Evaluator and retrieval hardening

**Implementation status: complete; opt-in/manual and no automatic promotion.**

The first corrected 8-task pilot exposed three measurement defects in the
operator harness: several private checks were themselves invalid, card
exposure was inferred from aggregate store deltas, and the live treatment used
a broad retrieval budget without recording which cards were actually selected.
Milestone 8a.2 is the focused hardening increment before another live pilot.

**Implementation plan and deliverables:**

1. **Make evaluator plans self-validating.** Bump the private live-plan
   contract and add an optional preflight argv to each verification check.
   Live post-run plans must provide that preflight. Before any model process is
   launched, the runner executes each distinct preflight once in a detached
   source worktree. Commands use argv vectors, no shell, disconnected
   standard streams, a scrubbed environment, and a shared 60-second bound.
   A failing or timing-out preflight aborts the run before it can produce
   outcome data.
2. **Separate retrieval selection from outcome evidence.** Record the card IDs
   whose exposure counters increase during each treatment task and the exact
   exposure increment in the private manifest. Publish only the selected-card
   count, exposure count, and an opaque order-independent fingerprint in the
   redacted result. This prevents “card was available” or “card was exposed”
   from being mistaken for “the agent followed the recommendation.”
3. **Reduce live treatment confounding.** Add a bounded, configurable
   relevance floor. Normal retrieval keeps the backward-compatible default of
   zero; the live bridge uses two cards, a 1,200-token context budget, and a
   minimum relevance score of six. The policy fingerprint records these values
   so future results cannot be compared as if they used the old policy.
4. **Cover the contracts with regression tests and documentation.** Test
   preflight rejection and success, argv/no-shell behavior, timeout behavior,
   retrieval relevance filtering, exposure deltas, result redaction, config
   clamping, and schema-versioned plan/result serialization. Update the
   operator documentation with the new private-plan fields and the next-pilot
   procedure.

**Exit gate:** complete for implementation and regression coverage. The next
   pilot must still demonstrate that the hardened checks pass on every task,
   that treatment exposure is measurable, and that any task-success or
   verification improvement survives a larger held-out study. No card or
   program-level change is promoted by this milestone.

### Practical-value track after Milestone 8a.2

The current implementation is a trustworthy substrate, not yet a demonstrated
agent improvement. The next hypothesis is narrower and more testable:

> A small, task-local intervention card that describes one recurring failure,
> one bounded next action, and one verification target can improve recovery or
> verification quality on difficult coding tasks without adding harmful
> context or causing negative transfer.

This hypothesis is deliberately about a failure/recovery slice of Codex's
work, not generic intelligence. The track below must falsify the hypothesis if
the card is merely extra prompt text. The existing broad advisory cards remain
useful as diagnostic history, but they must not be treated as proven treatment
material in the next study.

### Milestone 8a.3 — Benchmark and evaluator qualification

**Implementation status: in progress; no behavioral claim is allowed before
this milestone passes.**

The first pilot was too close to a ceiling and its private checks contained
three defects. This milestone makes the task pack and evaluator a testable
measurement instrument before another treatment run is attempted.

**Implementation changes:**

1. Create a versioned local/private benchmark manifest with 32–48 coding tasks
   across at least four families, such as parsing/escaping, stateful behavior,
   configuration and CLI contracts, test/debugging recovery, and repository
   integration. Keep the task prompt, seed commit, expected contract, and
   evaluator definition separate. Evaluator commands and reference solutions
   must never be included in the model-visible task context.
2. Give every task a difficulty and failure-motif label, but do not use those
   labels as treatment context. Calibrate the pack with retrieval disabled so
   a fresh baseline succeeds on roughly 40–75% of tasks. A pack with a success
   rate above 85% is a ceiling test; a pack below 25% is not a useful memory
   test and must be repaired before proceeding.
3. Add evaluator qualification fixtures for every task family: a known-good
   worktree, a no-op or incomplete worktree, and targeted mutants that violate
   the contract. The private preflight must pass on the good fixture and the
   relevant checks must fail on each bad fixture. Include cases for false
   positives, missing checks, ordering/escaping mistakes, and unrelated
   changes.
4. Add a manifest validation/report command that checks task uniqueness,
   source-commit consistency, worktree disjointness, evaluator preflight
   coverage, budget bounds, and absence of model-visible expected answers.
   Record an evaluator version/fingerprint in every live result.
5. Add a small harness smoke run, using fresh worktrees and no cards, that
   exercises every evaluator before spending model budget on the full pilot.

**Tests and verification:**

- unit tests for manifest bounds, family coverage, duplicate-task rejection,
  evaluator-version matching, and prompt/evaluator separation;
- fixture tests proving every good implementation passes and every targeted
  mutant fails the intended check;
- preflight tests proving a failing, timing-out, or malformed check prevents
  model execution;
- a baseline calibration report with task-level and family-level success,
  verification completeness, and cost distributions;
- a review of the task pack by a person who did not write the evaluator.

**Exit gate:** all preflights pass on the good fixtures, all targeted mutants
are detected, no task or evaluator leaks private expected answers, and the
retrieval-disabled baseline is neither a ceiling nor a universal failure. If
the gate fails, repair the benchmark; do not tune cards against invalid
measurements.

The current implementation provides the versioned private manifest contract,
model-visible task projection, evaluator fingerprints, fixture qualification
report, isolated argv-only no-model runner, and retrieval-disabled baseline
calibration report. The repository tests use synthetic 32-task fixtures to
exercise the gate. A real private task pack, fresh retrieval-disabled model
observations in the 40–75% range, and review by someone who did not author the
evaluator are still required before this milestone can be marked complete.

### Milestone 8a.4 — Intervention cards and task-local retrieval

**Implementation status: planned; the existing broad-card path remains
opt-in and is not a proven treatment.**

This milestone changes the unit of memory from a general historical paragraph
to a bounded intervention that a model can recognize and verify. It should
reduce prompt noise before attempting to increase card sophistication.

**Implementation changes:**

1. Add a versioned intervention representation, while keeping old cards
   readable as legacy advisory history. An intervention contains bounded
   `trigger`, `scope`, `action`, `verification`, and `evidence` fields. The
   trigger identifies a failure/task motif; the action contains one or two
   imperative steps; verification identifies the expected check or outcome;
   evidence records provenance and prior observed outcomes. No field may
   contain raw prompts, responses, terminal output, secrets, or unrestricted
   paths.
2. Make scope explicit and ranked: exact workspace/project scope first, then
   task family and failure signature, then a deliberately small generic
   fallback. Unknown-scope generic cards must not outrank a scoped card. Cards
   without a useful trigger or verification target remain diagnostic and are
   excluded from the intervention pilot.
3. Add a task-local retrieval path that can use the current task family and a
   reduced current failure signal, in addition to the user query. Retrieval
   must remain deterministic and model-free. At the start of a task and after
   a matching failure, inject at most one intervention once per trigger; do
   not repeat the same card on every turn.
4. Render a compact intervention block targeted at 300–600 tokens, with the
   card clearly marked as historical, advisory, and untrusted. The pilot
   treatment uses one card unless a preregistered experiment explicitly tests
   another budget. Cards can suggest a command or check but cannot invoke a
   tool, grant permission, or override repository instructions.
5. Keep a legacy/broad-retrieval mode for comparison and migration, but make
   the policy fingerprint distinguish legacy retrieval, scoped intervention
   retrieval, card count, context budget, and relevance floor.

**Tests and verification:**

- schema migration and bounded-field tests for intervention cards;
- exact-scope, family-scope, failure-signature, generic-fallback, and
  conflict-resolution retrieval fixtures;
- negative-transfer fixtures proving an unrelated task does not receive a
  scoped intervention;
- injection-frequency tests proving one card is not repeated across turns or
  after an unrelated failure;
- context-budget and redaction tests for the 300–600-token rendering target;
- disabled-mode and legacy-card compatibility tests;
- an offline precision/coverage report comparing broad retrieval with the
  scoped policy before any live run.

**Exit gate:** on the qualified benchmark, scoped retrieval materially reduces
irrelevant card exposure and context cost while preserving relevant-card
recall. The intervention lane is deterministic, bounded, and incapable of
changing permissions or instruction precedence. No live treatment is run if
the scoped policy still presents generic cards on unrelated tasks.

### Milestone 8a.5 — Causal exposure and action telemetry

**Implementation status: planned; attribution remains unknown until this
milestone is complete.**

Exposure alone cannot explain an outcome. This milestone records the smallest
private event chain needed to distinguish a card being available from an agent
attempting the recommended action and passing the independent check.

**Implementation changes:**

1. Add an opaque per-task intervention context containing the experiment ID,
   task ID, card fingerprint, trigger ID, and injection count. Keep raw card
   IDs and card text in the private manifest only; aggregate/public results
   contain counts or opaque fingerprints.
2. Record separate events for `selected`, `injected`, `action_attempted`,
   `agent_verification_attempted`, `independent_verification_passed`, and
   `outcome`. An action is marked attempted only when a structured action
   signature matches a bounded tool/terminal event or an explicit review
   annotation. If evidence is absent, record `unknown`, never `not followed`.
3. Link independent post-run checks to the same task condition and preserve
   the distinction between agent verification and evaluator verification.
   Derive recovery-after-failure and verification-completeness metrics without
   claiming that a card caused an outcome merely because it was selected.
4. Add redacted result fields for selected-card count, injection count,
   action-attempt count, verification-attempt count, and outcome count. Keep
   per-card attribution, raw event order, and diagnostic evidence private.

**Tests and verification:**

- synthetic event-sequence tests for exposure-only, action-attempted,
  verification-passed, failed-recovery, and unknown-attribution cases;
- duplicate-injection and baseline-no-exposure tests;
- deterministic action-signature matching tests with false-match rejection;
- redaction tests proving prompts, command tokens, paths, and raw card IDs do
  not enter published results;
- cost-limited and interrupted-run tests proving partial telemetry remains
  explicit rather than being interpreted as failure or success.

**Exit gate:** the harness can demonstrate, using synthetic and held-out
fixtures, the difference between “shown,” “attempted,” “verified,” and
“successful.” It must still report unknown when the trace cannot support a
stronger claim. This is an observability gate, not a behavioral improvement
claim.

### Milestone 8a.6 — Repeated practical-value pilot

**Implementation status: planned; this is the primary decision gate for
whether the project adds value over baseline Codex.**

Run the experiment only after 8a.3–8a.5 pass. Do not repeat the original
8-task pilot with the same easy task distribution and broad cards.

**Experiment design:**

- Use 32–48 held-out tasks from 4–6 families, with the calibrated baseline
  range from 8a.3. Include recurring failure motifs for which the intervention
  cards have a plausible trigger/action/verification chain, plus unrelated
  tasks that test negative transfer.
- Use a reviewed pool of 6–12 scoped interventions. Each treatment task may
  receive at most one 300–600-token intervention unless the manifest says
  otherwise. Baseline receives no active cards but uses the same model,
  repository commit, budgets, permissions, prompt, and evaluator.
- Run fresh isolated worktrees and fresh child processes for every condition.
  Randomize or counterbalance baseline/treatment order, task order, and trial
  assignment. Run at least two independent trials per task for a promotion
  decision; one trial is a harness smoke test only.
- Freeze the task pack, card pool, policy fingerprint, primary endpoint,
  exclusion rules, and analysis method before launching model runs. The
  private plan must record model/configuration identity, source commit,
  evaluator fingerprint, card-policy fingerprint, and trial ID.

**Primary and secondary measures:**

- primary: independently verified task correctness, chosen before the run;
- secondary: verification completeness, recovery after an induced or observed
  failure, failed verification attempts, unnecessary retries, tool/terminal
  operations, input/context tokens, output/reasoning tokens, wall time,
  cancellation, policy events, and negative transfer on unrelated tasks;
- attribution: selected, injected, attempted, and verified counts from 8a.5;
- report paired task-level deltas and family-level deltas. Raw attempt counts
  are diagnostic only; more attempts are not an improvement by themselves.

**Pre-registered decision rule:**

1. Validity must pass first: every evaluator preflight succeeds, no leakage or
   worktree overlap is found, and the baseline remains in the calibrated
   difficulty range.
2. A treatment claim requires a practically meaningful primary improvement,
   proposed in advance as a default target of at least 10 percentage points
   absolute, replicated in the second trial or supported by a predeclared
   non-inferiority analysis when the baseline is high.
3. No family may regress by more than 5 percentage points, no safety or
   permission boundary may regress, and median context/cost overhead must stay
   within 15% unless the primary improvement is large enough to justify it in
   the preregistered plan.
4. If the primary result is flat, cards are not promoted. Inspect the private
   action/verification telemetry to decide whether the problem is retrieval,
   card wording, lack of actionability, or no underlying opportunity. Do not
   infer improvement from exposure or from a post-hoc favorable subset.

**Exit gate:** a replicated, independently verified improvement over the
retrieval-disabled baseline with no unacceptable family, safety, privacy, or
cost regression. If the gate fails twice with different scoped card pools,
stop expanding generic card retrieval and retain the subsystem as diagnostic
capture/evaluation infrastructure. Do not begin Milestone 8 on a flat result.

### Milestone 8a.7 — Scoped canary and operating loop

**Implementation status: planned and conditional on a passing 8a.6.**

This is the first step that can make a proven intervention useful in ordinary
Codex work. It is a narrow canary, not automatic self-modification.

**Implementation changes:**

1. Extend the existing manual promotion path with an experiment/canary
   identity, explicit workspace/task-family allowlist, maximum active-card
   count of one for the first canary, short TTL, and kill switch.
2. Make canary status show the card fingerprint, scope, evidence, expiry,
   observed exposure/action/verification counts, and rollback predecessor. Do
   not show or persist raw prompts or command output in the durable status.
3. Add a documented review cadence: inspect at least 20 affected tasks or a
   predeclared time window, whichever comes first; retire or roll back on a
   family regression, safety/privacy issue, unexpected context growth, or
   repeated unverified recommendations.
4. Feed reviewed canary outcomes back into the existing evolution history and
   candidate queue. The loop may propose another experiment, but it must not
   promote a successor automatically.

**Tests and verification:**

- scope allowlist, TTL, kill-switch, and rollback tests;
- proof that a canary cannot leak into an unrelated workspace or task family;
- status/redaction tests for private evidence and public summaries;
- simulated positive, neutral, negative, and expired canary lifecycles;
- a documented ordinary-work workflow showing enable, inspect, rollback, and
  retire operations.

**Exit gate:** a passing intervention can be enabled for a narrow scope,
observed, and rolled back without changing default Codex behavior. If the
canary is not stable in ordinary work, retire the card and keep the benchmark,
telemetry, and evaluation improvements rather than widening the scope.

### Milestone 8 — Optional program-level evolution and transfer (research)

**Intent:** explore the strongest Meta^n idea—evolving the agent harness or
operating program itself—only after the safer memory and evaluation loops have
earned enough evidence.

**Possible scope:**

- generate candidate versions of a Skill, prompt template, extension routing
  rule, or repository guidance patch in isolated worktrees;
- evaluate candidate versions against a broad regression suite, held-out task
  families, safety fixtures, and cost budgets;
- study transfer between repositories and task families, including negative
  transfer, forgetting, stale guidance, and evaluator overfitting;
- support multi-step evolution budgets with checkpoints and automatic abort on
  regression, while retaining human review before activation.

**Hard exclusions:** no direct mutation of the live Codex binary, no arbitrary
generated code execution outside the existing sandbox/approval model, no
automatic change to system/developer instructions, and no acceptance based on
one benchmark or one judge score.

**Exit gate:** this milestone is optional and should not be started merely
because the machinery is available. First complete 8a.3–8a.6 and, if a
candidate is activated, 8a.7. Establish a replicated practical gain for a
scoped intervention before attempting to evolve Skills, prompts, or extension
policy. Program-level evolution additionally requires a historical benchmark,
reproducible holdout gains, bounded compute/cost, no safety regression, a
rollback demonstration, and explicit human approval of the evaluation and
candidate scope. Otherwise the project should stop at the practical-value
track and retain only the diagnostic infrastructure.

### Cross-milestone requirements

Every milestone must preserve the following invariants:

- **Default safety:** disabled means no capture, retrieval, model calls, or
  behavior change from this subsystem.
- **Separation of planes:** observation, candidate generation, evaluation,
  active retrieval, and promotion are separate states and stores.
- **Evidence before attribution:** exposure is not adoption; adoption is not
  success; success is not generalization.
- **Bounded local state:** enforce limits on bytes, records, fields, model
  calls, tokens, wall time, and concurrent work.
- **No authority escalation:** historical experience can suggest, never grant
  permissions or override instruction hierarchy.
- **Reproducibility:** record versions, configuration, task scope, evaluator,
  and source trace IDs for every measured result.
- **Reversibility:** active changes have a stable identity, predecessor, TTL or
  retirement path, and a tested rollback.
- **Privacy by construction:** raw prompts, model responses, terminal output,
  secrets, and arbitrary paths remain out of durable improvement artifacts
  unless a separate, explicit diagnostic workflow authorizes them.
- **Cost awareness:** measure context tokens, model calls, tool calls, runtime,
  and storage. An improvement that wins task success by disproportionate cost
  is not automatically an improvement.
- **Benchmark validity:** do not tune cards or claim gains until good fixtures,
  targeted mutants, evaluator preflights, and baseline difficulty have all
  passed an independent qualification review.
- **Intervention budget:** the first practical treatment is one scoped card and
  300–600 context tokens. Larger context or multiple cards require a separate
  preregistered comparison.
- **Unknown attribution:** when the trace cannot establish that an action was
  attempted or verified, record `unknown`; never convert missing evidence into
  a negative behavioral label.
- **Preregistered analysis:** freeze the task pack, card pool, endpoint,
  exclusions, and decision rule before treatment runs. A favorable subset or
  metric selected after the run is exploratory only.

### Recommended execution order

The practical implementation order is:

1. Use Milestone 1 manually for real repeated Codex work and collect a small,
   reviewed set of examples.
2. Implement Milestone 2 so candidate capture becomes automatic but review
   remains human-controlled.
3. Implement Milestone 3 only where the examples show that coarse cards are
   insufficient; avoid adding fields without a retrieval or evaluation use.
4. Build Milestone 4 before accepting any claim that memory improves the
   agent.
5. Add Milestone 5 as a focused Skill first, then a model-backed offline
   proposer if the Skill workflow is valuable and safe.
6. Implement Milestone 6 before enabling any non-manual promotion.
7. Add Milestone 7 for periodic local improvement and keep the loop
   deterministic/manual.
8. Finish 8a.2 and commit its hardening changes before collecting the next
   behavioral result. Treat its current tests as harness evidence, not as a
   claim that cards help the model.
9. Implement 8a.3 and qualify the benchmark with good/bad fixtures and a
   retrieval-disabled baseline. Do not launch a treatment run if the baseline
   is at the ceiling or the evaluator is not mutation-sensitive.
10. Implement 8a.4 as the smallest behavior-facing change: one scoped,
    trigger/action/verification intervention with a 300–600-token budget.
    Compare its offline retrieval precision and context cost with the current
    broad-card policy.
11. Implement 8a.5 so selection, injection, action attempt, verification, and
    outcome are separate private signals. Keep unknown attribution explicit.
12. Run the 8a.3 smoke procedure, then the preregistered 8a.6 repeated pilot
    with 32–48 tasks and at least two independent trials per task. Inspect
    task/family deltas and cost before recording an outcome.
13. If and only if 8a.6 passes, implement and use 8a.7 for one scoped manual
    canary with a short TTL and rollback. If it fails, keep the diagnostic
    infrastructure and revise the card/retrieval hypothesis rather than
    widening activation.
14. Consider Milestone 8 only if the practical-value track establishes
    replicated benefit, acceptable cost, no safety regression, and a working
    rollback path.

The first useful product loop after the completed foundation is therefore:

```text
candidate -> curator review -> scoped intervention -> paired evaluation
    ^                    |              |                  |
    |                    v              v                  v
    +---------- measured telemetry <- canary <- replicated gain
```

## Practical-value verification sequence

The following sequence keeps harness validation, retrieval validation, and
behavioral validation separate. Each stage produces a small reviewable artifact
and has a stop condition; passing one stage does not imply that the next claim
is true.

| Stage | What runs | Required artifact | Proceed only when |
| --- | --- | --- | --- |
| Q0. Evaluator qualification | Good, no-op, incomplete, and targeted-mutant fixtures; no model | Versioned qualification report and evaluator fingerprint | Good fixtures pass, targeted mutants fail, preflights are green, and no private answer leaks |
| Q1. Retrieval qualification | Offline broad-vs-scoped selection on fixed queries and failure signals | Precision/coverage/context report and policy fingerprint | Scoped policy lowers irrelevant exposure without losing relevant matches |
| Q2. Telemetry qualification | Synthetic event chains for selected/injected/attempted/verified/unknown | Redacted telemetry fixture report | Every state transition and unknown case is represented correctly |
| Q3. Harness smoke | 8–12 fresh tasks, baseline and treatment, no promotion decision | Smoke result with task-level evaluator and cost diagnostics | Isolation, cleanup, preflight, redaction, and result writing are all green |
| Q4. Behavioral pilot | 32–48 held-out tasks, two independent trials, paired baseline/treatment | Preregistered aggregate result plus private attribution report | Validity passes and the 8a.6 decision rule is satisfied |
| Q5. Canary | One passing intervention in one allowlisted scope for at least 20 affected tasks or the predeclared window | Canary review and rollback record | Benefit remains stable in ordinary work; otherwise retire and investigate |

The Q3 smoke result is not part of the primary Q4 effect estimate. Q4 must
report both an intention-to-treat result for all assigned tasks and an
exploratory attribution view for tasks where an action attempt was observed.
The latter can explain a flat result, but cannot be used to manufacture a
positive result by excluding tasks where the card was ignored.

## Architecture

Add one extension crate, `codex-agent-improvement-extension`, with five small
responsibilities:

1. **Schema**: serializable `ExperienceCard`, evidence counters, task-family
   labels, and failure classes.
2. **Summarizer**: deterministic conversion from a reduced `RolloutTrace` into
   one privacy-bounded card.
3. **Store**: append-only JSONL persistence under
   `$CODEX_HOME/agent-improvement/cards.jsonl`, with schema migration, lesson
   merging, bounded structured retrieval, lifecycle updates, and deterministic
   ordering.
4. **Candidate review**: bounded candidate JSONL persistence, lifecycle capture,
   deterministic deduplication, and explicit terminal review transitions.
5. **Prompt integration**: an opt-in extension that selects relevant cards from
   the current user input and injects them as clearly marked advisory user
   context. Cards are not developer policy and cannot override user, system,
   safety, or repository instructions.

Add a CLI command:

```text
codex debug agent-improvement summarize <TRACE_BUNDLE> [--store <DIR>] [--output <FILE>]
codex debug agent-improvement render --store <DIR> --query <TEXT>
codex debug agent-improvement cards feedback --store <DIR> <CARD_ID> --value <VALUE>
codex debug agent-improvement cards outcome --store <DIR> <CARD_ID> --followed <VALUE> --verified <VALUE>
codex debug agent-improvement cards retire --store <DIR> <CARD_ID>
```

The first command replays an existing bundle and emits one card. `--store` is
explicit so users do not accidentally persist sensitive material. The second
command is a read-only inspection tool for verifying retrieval and prompt-sized
rendering.

## Data contract

`ExperienceCard` must contain only bounded, derived information:

- schema version and deterministic card ID;
- source trace ID and source timestamp;
- coarse task family;
- failure class;
- counts for turns, inferences, tools, terminal operations, failed operations,
  and token usage;
- bounded fixed command/tool activity labels, not full stdout/stderr;
- bounded opaque hashes of normalized task terms for useful retrieval without
  storing those terms as card text;
- a short evidence summary;
- a deterministic recommended next step;
- a verification summary;
- no raw prompt, response, path, secret, or terminal output by default.

Failure classification must be deterministic and explainable. Initial classes:

- `completed`;
- `command_failure`;
- `tool_failure`;
- `model_failure`;
- `policy_blocked`;
- `cancelled`;
- `incomplete_verification`;
- `unknown`.

Initial task families are coarse and intentionally conservative:

- `coding`;
- `testing`;
- `review`;
- `planning`;
- `investigation`;
- `documentation`;
- `general`.

## Configuration and safety

Add an `[agent_improvement]` configuration block with:

```toml
[agent_improvement]
enabled = false
max_cards = 8
max_context_tokens = 1200
min_relevance_score = 0
capture = "off"
max_candidates = 64
```

The effective default is disabled. When enabled:

- only `$CODEX_HOME/agent-improvement/cards.jsonl` is read;
- candidate capture, when explicitly set to `candidate`, writes only to the
  local trace and review-queue paths under `$CODEX_HOME/agent-improvement`;
- pending candidates are never read by prompt retrieval;
- missing or malformed cards are skipped with a bounded warning;
- oversized stores, lines, and card fields are rejected or skipped within
  fixed local resource limits;
- card content is truncated before prompt insertion;
- the prompt explicitly labels cards as historical, advisory, and untrusted;
- no card can change tool permissions, approval policy, sandbox policy, or
  instruction precedence;
- failures to read the store never fail a Codex turn.

The summarizer reads reduced trace state and does not need to read raw payloads.
This keeps the default card path safer than replaying raw prompts or terminal
output into future context.

## Milestone 1 implementation tasks (complete)

Each task has a red/green/verification checkpoint and must be completed before
starting the next task.

### Task 0 — Record and validate this plan

Files:

- `docs/agent-improvement-integration-plan.md`

Acceptance:

- plan is committed to the working tree before code changes;
- scope, non-goals, safety, data contract, test plan, and promotion gates are
  explicit;
- the plan is updated as implementation progresses.

Verification:

```text
git diff --check
```

Status: complete. The file exists and `git diff --check` passes.

### Task 1 — Add configuration types with a disabled default

Files:

- `codex-rs/config/src/config_toml.rs`
- `codex-rs/config/src/types.rs`
- `codex-rs/config/src/types_tests.rs`
- `codex-rs/config/src/lib.rs` if exports are required
- `codex-rs/core/src/config/mod.rs`
- relevant schema/config tests

Red:

- config parsing test for `[agent_improvement]`;
- default and clamping tests;
- assert effective `Config` carries the resolved settings.

Green:

- add `AgentImprovementToml` and `AgentImprovementConfig`;
- default `enabled = false`;
- clamp card count, context budget, and relevance floor to bounded values;
- thread the effective value through `Config`.

Verification:

```text
cargo test -p codex-config agent_improvement
cargo test -p codex-core config
```

### Task 2 — Implement the pure experience-card schema and store

Files:

- `codex-rs/ext/agent-improvement/Cargo.toml`
- `codex-rs/ext/agent-improvement/BUILD.bazel`
- `codex-rs/ext/agent-improvement/src/lib.rs`
- `codex-rs/ext/agent-improvement/src/schema.rs`
- `codex-rs/ext/agent-improvement/src/store.rs`
- `codex-rs/ext/agent-improvement/src/store_tests.rs`
- workspace manifests and Bazel registration

Red:

- JSON round-trip test, including opaque retrieval keys;
- deterministic card-ID test;
- append/read test using a temporary directory;
- malformed-line tolerance test;
- lexical and topic relevance, tie-breaking, card count, and byte/token-bound
  tests.

Green:

- implement bounded serializable cards with fixed activity labels and opaque
  topic hashes;
- implement append-only JSONL storage;
- implement deterministic, relevance-bounded selection;
- never include raw prompt or terminal output in the card schema.

Verification:

```text
cargo test -p codex-agent-improvement-extension
```

### Task 3 — Deterministically summarize reduced rollout traces

Files:

- `codex-rs/ext/agent-improvement/src/summarize.rs`
- `codex-rs/ext/agent-improvement/src/summarize_tests.rs`

Red:

- synthetic reduced-trace tests for completed work;
- non-zero terminal exit classification;
- failed tool classification;
- cancelled/aborted classification;
- policy-blocked signal classification;
- task-family classification;
- verification recommendation behavior.

Green:

- summarize `codex_rollout_trace::RolloutTrace` without model calls;
- derive evidence from reduced runtime objects;
- use bounded labels and counters;
- make recommendations deterministic and explainable.

Verification:

```text
cargo test -p codex-agent-improvement-extension summarize
```

### Task 4 — Add the debug CLI commands

Files:

- `codex-rs/cli/Cargo.toml`
- `codex-rs/cli/src/main.rs`
- CLI parser/command tests

Red:

- parser tests for both subcommands and required arguments;
- command test proving a reduced trace produces stable JSON;
- explicit `--store` test proving persistence is opt-in.

Green:

- add `codex debug agent-improvement summarize`;
- add `codex debug agent-improvement render`;
- preserve non-zero errors for missing/invalid trace bundles;
- keep stdout JSON stable and machine-readable.

Verification:

```text
cargo test -p codex-cli
```

### Task 5 — Inject relevant cards through the extension system

Files:

- `codex-rs/ext/agent-improvement/src/extension.rs`
- extension tests
- `codex-rs/app-server/src/extensions.rs`
- `codex-rs/app-server/Cargo.toml`
- CLI debug prompt registry setup
- `codex-rs/core/src/prompt_debug.rs`
- `codex-rs/core/src/session/turn.rs`

Red:

- disabled configuration contributes no fragments;
- enabled configuration with no store contributes no fragments;
- query selects relevant cards only;
- output is bounded and marked advisory/untrusted;
- store read errors do not fail turn preparation.

Green:

- implement a `TurnInputContributor` using current user input;
- render the selected cards as user-role contextual content;
- install the extension in the app-server registry;
- install it in debug prompt construction so prompt inspection is honest.

Verification:

```text
cargo test -p codex-agent-improvement-extension
cargo test -p codex-app-server extensions
cargo test -p codex-cli prompt
```

### Task 6 — Add user-facing documentation and operational gates

Files:

- `docs/agent-improvement.md`
- `docs/config.md` or the repository's appropriate configuration reference
- `codex-rs/ext/agent-improvement/README.md` if useful

Document:

- how to enable the feature;
- how to create a card from a local rollout trace;
- where data is stored;
- why raw prompts and outputs are excluded;
- how to inspect and delete cards;
- how to review a card before allowing it to influence future work;
- that card retrieval is not an eval or a guarantee of correctness.

Verification:

```text
git diff --check
```

### Task 7 — Full validation and handoff

Run the narrow tests for every changed crate, then the repository's appropriate
Rust and Bazel checks when toolchains are available. Record unavailable checks
instead of claiming success.

The final review must explicitly compare:

- behavior disabled vs enabled;
- no-card vs relevant-card context;
- completed vs failed vs cancelled traces;
- single-card vs many-card retrieval;
- malformed and oversized local data;
- privacy of generated card JSON;
- prompt precedence and safety boundaries.

## Validation notes

The focused validation completed with these results:

- `cargo test -p codex-config agent_improvement`: 2 passed;
- `cargo test -p codex-core --lib load_config_resolves_agent_improvement_settings`: 1 passed;
- `cargo test -p codex-agent-improvement-extension`: 73 passed;
- `cargo test -p codex-cli --test agent_improvement`: 10 passed;
- `cargo test -p codex-cli debug_agent_improvement`: 4 parser tests passed;
- `cargo test -p codex-app-server extensions`: 6 unit tests and 2 filtered
  integration tests passed;
- `cargo test -p codex-rollout-trace thread`: root/child trace context tests passed;
- `cargo test -p codex-skills`: embedded skill fingerprint tests passed;
- `cargo check -p codex-thread-manager-sample`: passed;
- `cargo fmt --all -- --check` and `git diff --check`: passed.

Milestone 2 additionally verifies that capture can be enabled while retrieval
is disabled, that a turn-stop candidate is available before the thread closes,
that pending candidates do not create `cards.jsonl`, and that explicit CLI
acceptance is the only path from pending state to prompt-visible active card.

Milestone 3 additionally verifies schema v1 migration and unknown-version
skipping, forward-compatible unknown fields, opaque workspace scoping, direct
fact versus low-confidence hypothesis ranking, recurring-lesson merging,
recency/recurrence ordering, conflict suppression, retirement, exposure,
feedback, followed/verified attribution, and a fixed offline retrieval fixture
with precision, irrelevance, ordering, and context-budget assertions. The
extension records exposure when a card is selected, but it does not infer that
the recommendation was followed or caused a later result.

Milestone 4 additionally verifies that paired manifests reject non-memory
configuration differences and overlapping worktrees, that evaluation output
cannot overlap the primary checkout or active card store, that replay failures,
missing judge evidence, and cost limits remain explicit, and that aggregate
results omit prompts, command tokens, and local paths. The evaluator consumes
pre-recorded baseline/treatment bundles and does not launch live model runs.

Milestone 6 additionally verifies append-only promotion transitions and
replayable state, evaluation and safety gates, scoped policy allowlists,
kill-switch and TTL filtering, card-only activation, provenance-bearing
status output, malformed-store fail-closed behavior, and one-command rollback.
The focused extension suite has 63 tests and the CLI agent-improvement suite
has 8 integration tests; both include the complete manual promotion lifecycle.

Milestone 7 additionally verifies deterministic multi-signal clustering,
held-out experiment-set metadata, bounded history and stagnation suppression,
stale/negative/retention planning, archive-backed compaction, canonical export,
explicit forgetting, and non-destructive retirement. The focused extension
suite now has 70 tests and the CLI agent-improvement suite has 9 integration
tests; the slow loop makes no model calls and cannot promote its own output.

Milestone 8a additionally verifies the private held-out plan contract,
brief/fingerprint matching, task-family and scenario-budget bounds, stable card
and result fingerprints, CLI dry-run validation, clean-checkout and isolated
worktree requirements in the live runner, and reuse of the redacted paired
evaluator. Milestone 8a.1 adds bounded post-run evaluator commands, redaction,
trace/policy separation, cost-limited result handling, and live-plan budget
caps. The focused extension suite now has 77 tests and the CLI
agent-improvement suite has 10 integration tests. The repository suite does not
launch a live model or claim an improvement result; that remains an explicit
operator study.

Milestone 8a.2 additionally verifies evaluator preflight rejection and
success, argv/no-shell and timeout behavior, relevance-floor filtering,
private exposure deltas, order-independent selection fingerprints, result
redaction, config clamping, and schema-versioned plan/result compatibility. The
focused extension suite has 79 tests, the focused config suite has 383 tests,
and the CLI agent-improvement suite has 10 integration tests. Formatting and
diff checks pass. Strict extension Clippy still reports four pre-existing
structural lints (`large_enum_variant` in `capture.rs` and three
`too_many_arguments` cases in `schema.rs`); they are outside the hardening
scope. The full core suite remains environment-limited by unrelated listener,
proxy, and certificate-write restrictions, so no live behavioral result is
claimed here.

Milestone 8a.3 adds a private benchmark manifest with explicit model-visible
projection, source-commit and worktree isolation checks, evaluator-version and
fingerprint matching, good/no-op/incomplete/targeted-mutant qualification
evidence, bounded argv-only fixture execution, report redaction, and baseline
calibration thresholds. The focused extension benchmark suite covers
model-context leakage, duplicate and family bounds, false-positive mutants,
missing checks, timeout/spawn failure evidence, evaluator identity, and
qualified/ceiling/too-hard calibration. The CLI smoke test qualifies 32 tasks
and 128 isolated fixture copies without model execution and confirms cleanup.
The private benchmark pack, real baseline observations, and independent review
remain outstanding, so the 8a.3 exit gate is not yet claimed.

An earlier broader `cargo test -p codex-core config` run compiled the target
and then aborted in the unrelated existing
`tools::handlers::multi_agents::tests::multi_agent_v2_spawn_agent_ignores_configured_max_depth`
test with a stack overflow. It is not claimed as a green repository-wide
check. Bazel was also not run because neither `bazel` nor `bazelisk` is
available in this environment; the Bazel target was added and should be run in
CI or a checkout with that toolchain.

## Cross-milestone promotion gates

This first integration is deliberately deterministic. Any future generated
strategy, Skill, AGENTS.md change, or prompt change must pass all of these gates:

1. observed trace evidence identifies the problem;
2. the proposed change is narrowly scoped;
3. a deterministic regression test exists;
4. the change improves a held-out task set or reduces measured failure/cost;
5. no existing task family regresses beyond an agreed threshold;
6. the change stays within permission, privacy, and instruction boundaries;
7. a human reviews the final diff before promotion.

Keep routed/per-task-best results separate from single-policy results. A card
that helps one task must not be treated as a general Codex improvement without
cross-task evidence.

## Implementation log

| Task | Status | Evidence |
| --- | --- | --- |
| 0. Plan | complete | This file; `git diff --check` |
| 1. Configuration | complete | Config and effective-value tests; focused core resolution test |
| 2. Schema/store | complete | Foundation schema/store tests; the full extension suite now also covers candidate persistence |
| 3. Trace summarizer | complete | Deterministic reduced-trace tests cover completed, command/tool/model/policy/cancelled/incomplete outcomes |
| 4. Debug CLI | complete | Parser tests plus end-to-end summarize/render and explicit-persistence test |
| 5. Prompt integration | complete | App-server registry test, extension tests, and end-to-end debug prompt-input injection test |
| 6. Documentation | complete | docs/agent-improvement.md; formatting and diff checks |
| 7. Full validation | complete | Focused config/core/extension/CLI/app-server tests, formatting, and diff checks; one unrelated pre-existing core stack-overflow test remains documented in the handoff |
| 2.1 Capture configuration and trace bridge | complete | Capture mode/max-candidate config tests; automatic root trace creation; trace context lifecycle tests |
| 2.2 Candidate schema, bounded store, and capture | complete | Candidate serialization, deterministic identity, append-only status, queue-limit, malformed-line, and no-active-card tests |
| 2.3 Lifecycle wiring | complete | Turn-stop and root-thread-stop capture tests; replay failures are isolated from lifecycle callbacks |
| 2.4 Review CLI and curator Skill | complete | Candidate list/show/accept/reject parser and end-to-end acceptance tests; embedded `agent-card-curator` asset |
| 2.5 Documentation and validation | complete | Updated user guide and roadmap; focused Milestone 2 test suite and formatting checks |
| 3.1 Evidence-rich schema and migration | complete | Schema v2 controlled facts, hypotheses, interventions, outcomes, signatures, scope, confidence, recurrence, adoption, v1 migration, and unknown-version handling |
| 3.2 Structured store and retrieval | complete | Lesson merging, append-only lifecycle updates, workspace filtering, conflict suppression, confidence/evidence/recency/recurrence ranking, and retirement |
| 3.3 Adoption attribution and controls | complete | Automatic exposure recording plus feedback, followed/verified outcome recording and CLI controls |
| 3.4 Milestone 3 tests and documentation | complete | 44 extension tests, 5 CLI integration tests, migration/privacy/ranking/fixture coverage, formatting, and updated usage docs |
| 4.1 Evaluation manifest and pairing contract | complete | Versioned private scenario manifest, held-out split requirement, exact config/policy/card provenance, worktree disjointness, and protected output paths |
| 4.2 Offline replay metrics and evaluators | complete | Reduced-trace replay metrics for verification, failure/retry, tool/terminal work, tokens, latency, cancellation, policy events, corrections, deterministic checks, and labeled judge inputs |
| 4.3 Result schema, CLI, tests, and usage docs | complete | Redacted aggregate results, partial/cost-limited states, false-attribution tests, 50 extension tests, 6 CLI integration tests, and paired-evaluation documentation |
| 5.1 Review-only proposal envelope | complete | Versioned card, Skill, `AGENTS.md`, configuration, and diagnostic lanes with bounded content, target scope, expiry, rollback, and strict safety validation |
| 5.2 Evidence-linked deterministic card proposer | complete | Stable card proposal IDs, candidate/card/trace/scenario references, concrete held-out validation requirement, and evidence-catalog checks |
| 5.3 Curator Skill and CLI workflow | complete | `agent-card-curator` drafting/validation workflow; read-only `proposals draft-card` and `proposals validate` commands; no active-store mutation |
| 5.4 Milestone 5 validation | complete | 56 extension tests, 7 CLI integration tests, 5 CLI parser tests, Skill tests, formatting, and diff checks; live model proposer remains gated |
| 6.1 Promotion event log and state projection | complete | Versioned append-only JSONL transitions, replayable state, fingerprints, evidence, actors, scope, expiry, and rollback ancestry |
| 6.2 Evaluation and policy gates | complete | Held-out paired evidence, no-regression checks, execution/policy parity, scoped allowlists, TTLs, active limits, and kill switches |
| 6.3 Runtime card activation and rollback | complete | Card-only promotion, fail-closed retrieval filtering, previous-card restoration, retirement, and non-card lane boundary |
| 6.4 Promotion CLI and inspection | complete | Validate/evaluate/accept/promote/rollback/retire/reject/status commands with JSON status/provenance output |
| 6.5 Milestone 6 validation | complete | 63 extension tests, 8 CLI integration tests, parser coverage, formatting, and policy/store failure tests; automatic promotion remains disabled |
| 7.1 Deterministic slow-loop aggregation | complete | Bounded recurring-failure, verification-omission, not-followed, conflict, and coverage-gap clustering into stable held-out experiment briefs |
| 7.2 Experiment history and stagnation control | complete | Append-only `evolution-history.jsonl`, explicit outcomes, priority decay, and suppression after repeated failed attempts |
| 7.3 Bounded card maintenance | complete | Stale/negative/retention planning, explicit retirement, archive-backed compaction, canonical export, and explicit forgetting |
| 7.4 Evolve CLI and validation | complete | Run/record/maintain/export/forget commands, parser coverage, 70 extension tests, 9 CLI integration tests, and deterministic end-to-end coverage |
| 8a-A Private live-plan contract and fingerprints | complete | Held-out prompt/budget validation, brief matching, stable card/result fingerprints, and private path boundaries |
| 8a-B Paired isolated runner | complete | Same-commit baseline/treatment worktrees, isolated stores/traces, source-scope preservation, bounded child execution, and no automatic promotion |
| 8a-C Evaluator feedback bridge | complete | Redacted result reuse, optional evolution-history outcome recording, dry-run CLI coverage, 73 extension tests, and 10 CLI integration tests |
| 8a.1 Independent evaluation and observability hardening | complete | Post-run argv checks before cleanup, redacted check evidence, trace/policy separation, bounded evaluator time, live-plan cap, 77 extension tests, and 10 CLI integration tests |
| 8a.2 Evaluator and retrieval hardening | complete | Preflight contracts, bounded relevance/exposure evidence, redacted selection fingerprints, config/schema tests, 79 extension tests, 383 config tests, and 10 CLI integration tests |
| 8a.3 Benchmark and evaluator qualification | in progress | Versioned manifest/projection, no-model fixture qualification and calibration commands/tests; private non-ceiling pack and independent review still required |
| 8a.4 Intervention cards and task-local retrieval | planned | Versioned trigger/action/verification cards, scoped one-card retrieval, 300–600-token rendering, and offline precision/negative-transfer report |
| 8a.5 Causal exposure and action telemetry | planned | Private selected/injected/attempted/verified event chain, redacted aggregates, and unknown-attribution tests |
| 8a.6 Repeated practical-value pilot | planned | Preregistered 32–48-task, two-trial paired study with task/family deltas, cost accounting, and explicit go/no-go decision |
| 8a.7 Scoped canary and operating loop | planned | Conditional one-card canary with allowlist, TTL, telemetry review, rollback, and retirement evidence |
