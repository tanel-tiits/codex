# Agent improvement cards

Codex can optionally use small, local experience cards derived from reduced
rollout traces. Cards make recurring execution lessons available to later
turns without replaying prompts, model responses, terminal output, paths, or
secrets into the model context. A card may contain a bounded set of opaque
hashes for normalized task terms so task-specific lessons can be retrieved
without storing those terms as readable card text.

The feature is disabled by default. It is advisory only: a card cannot change
instruction precedence, permissions, approvals, sandbox policy, or safety
requirements.

## Enable retrieval

Add this to the effective Codex configuration:

```toml
[agent_improvement]
enabled = true
max_cards = 8
max_context_tokens = 1200
min_relevance_score = 0
capture = "off"
max_candidates = 64
# Optional scoped-intervention lane; legacy remains the default.
retrieval_mode = "legacy"
intervention_context_tokens = 480
```

The card store is:

```text
$CODEX_HOME/agent-improvement/cards.jsonl
```

Codex reads that file at turn preparation time and selects active cards with
deterministic structured matches (workspace, task family, failure class, tool
and toolchain family) plus evidence quality, confidence, recurrence, recency,
and bounded lexical/topic overlap. `min_relevance_score` can add a bounded
relevance floor after those matches; zero preserves the normal broad retrieval
behavior. A mismatched opaque workspace is filtered; unknown scope remains a
fallback. Missing stores, malformed lines, oversized cards, and read failures
are ignored with a bounded warning.

Set `retrieval_mode = "intervention"` only for an explicitly reviewed,
offline-qualified comparison. This lane reads versioned intervention
projections from reviewed cards, injects at most one historical/advisory card
per task trigger, and bounds its context to 300–600 tokens. It cannot invoke a
tool, grant permission, or override any instruction or sandbox boundary. Cards
without a useful trigger or verification target, and legacy cards without an
intervention projection, remain available only to the legacy lane.

Cards use schema version 2. Each card contains a bounded observed sequence
(`turn`, `inference`, `tool`, `terminal_operation`, verification, policy, or
recovery categories) alongside the evidence profile. Version 1 cards are
upgraded in memory with
conservative unknown values, while unknown future schema versions are skipped.
Recurring observations with the same lesson identity merge their evidence and
provenance. Retrieval treats direct runtime facts as stronger than hypotheses;
hypotheses and recommendations are rendered as advisory labels, never as
policy. Old cards receive lower recency scores over time, but are not silently
deleted.

Retrieval and capture are independent. capture = "off" is the default.
Setting capture = "candidate" enables local rollout tracing under
$CODEX_HOME/agent-improvement/traces and creates reviewable candidates in:

    $CODEX_HOME/agent-improvement/candidates.jsonl

Candidates are never injected into prompts and are not active cards until
explicitly accepted.

## Create a card from a trace

Rollout tracing is a separate opt-in diagnostic path. Set
CODEX_ROLLOUT_TRACE_ROOT before a Codex run to create a local trace bundle.
Trace bundles can contain sensitive raw evidence, so review their handling
accordingly.

Summarize a bundle without persisting anything:

```bash
codex debug agent-improvement summarize /path/to/trace-bundle
```

Write the derived JSON to a file:

```bash
codex debug agent-improvement summarize \
  /path/to/trace-bundle \
  --output /tmp/experience-card.json
```

Persist it explicitly in the local card store:

```bash
codex debug agent-improvement summarize \
  /path/to/trace-bundle \
  --store "$CODEX_HOME/agent-improvement"
```

The summarizer is deterministic and uses reduced trace state. It classifies
coarse task families and outcomes such as command failure, tool failure,
policy blocking, cancellation, and incomplete verification. It does not make
a model call.

## Automatic candidate capture and review

To enable automatic, review-gated capture:

    [agent_improvement]
    capture = "candidate"
    max_candidates = 64

This records local trace bundles and, when a turn completes, errors, is
aborted, or the root thread stops, reduces bounded evidence into a pending
candidate. Capture is performed in a blocking worker outside the model/tool
execution path; failures to replay or write a candidate do not change the turn
result. Raw trace bundles may contain sensitive diagnostic material and should
be deleted when no longer needed.

List pending candidates:

    codex debug agent-improvement candidates list --store "$CODEX_HOME/agent-improvement"

Inspect one candidate:

    codex debug agent-improvement candidates show --store "$CODEX_HOME/agent-improvement" candidate_<ID>

Accepting is the explicit promotion boundary. It copies the bounded card into
cards.jsonl; it does not modify repository files, Skills, AGENTS.md, or
permissions:

    codex debug agent-improvement candidates accept --store "$CODEX_HOME/agent-improvement" candidate_<ID>

Reject a candidate without activating it:

    codex debug agent-improvement candidates reject --store "$CODEX_HOME/agent-improvement" candidate_<ID>

The embedded agent-card-curator Skill provides a conversational review workflow
around these commands. It must receive explicit user approval before accepting
a candidate.

## Draft a review-only improvement proposal

Milestone 5 adds a slow-path proposal artifact. Drafting is deterministic and
currently supports an experience-card update; the versioned envelope also
defines lanes for a focused Skill, an \`AGENTS.md\` patch, a prompt/extension
configuration change, and a diagnostic tooling issue. None of these artifacts
is applied automatically or injected into a turn.

Draft from one pending candidate and name the held-out scenario that must be
used to evaluate it:

\`\`\`bash
codex debug agent-improvement proposals draft-card \\
  --store "$CODEX_HOME/agent-improvement" \\
  candidate_<ID> \\
  --evaluation-scenario <SCENARIO_ID> \\
  --output /tmp/improvement-proposal.json
\`\`\`

Perform structural validation before review:

\`\`\`bash
codex debug agent-improvement proposals validate \\
  /tmp/improvement-proposal.json
\`\`\`

When the candidate store and a redacted Milestone 4 result are available, also
verify that every evidence identifier exists:

\`\`\`bash
codex debug agent-improvement proposals validate \\
  /tmp/improvement-proposal.json \\
  --candidate-store "$CODEX_HOME/agent-improvement" \\
  --evaluation-results /path/to/evaluation-results.json
\`\`\`

The proposal records its target scope, evidence IDs, rationale, expected
benefit, costs, risks, validation plan, expiry, and rollback intent. Validation
rejects oversized or instruction-like content, raw paths and secret-like
markers, dangerous commands, scope contradictions, permission/sandbox changes,
and patches outside a relative \`AGENTS.md\` path. Validation records what must
be checked; it does not run commands. A proposal remains separate from
\`candidates.jsonl\` and \`cards.jsonl\`, so explicit card acceptance is still
the only Milestone 1–3 activation boundary. Milestone 6 adds a separate
promotion path for proposals; non-card artifacts still require normal diff
review and are never applied by this subsystem.

## Promote, inspect, and roll back a proposal

Milestone 6 adds a manual, append-only promotion control plane. Use a real
candidate store and a redacted held-out evaluation result:

\`\`\`bash
codex debug agent-improvement proposals validate \\
  /tmp/improvement-proposal.json \\
  --state-store "$CODEX_HOME/agent-improvement" \\
  --candidate-store "$CODEX_HOME/agent-improvement" \\
  --evaluation-results /path/to/evaluation-results.json \\
  --actor reviewer

codex debug agent-improvement proposals evaluate \\
  /tmp/improvement-proposal.json \\
  --store "$CODEX_HOME/agent-improvement" \\
  --candidate-store "$CODEX_HOME/agent-improvement" \\
  --evaluation-results /path/to/evaluation-results.json \\
  --actor evaluator

codex debug agent-improvement proposals accept \\
  /tmp/improvement-proposal.json \\
  --store "$CODEX_HOME/agent-improvement" \\
  --actor reviewer

codex debug agent-improvement proposals promote \\
  /tmp/improvement-proposal.json \\
  --store "$CODEX_HOME/agent-improvement" \\
  --actor reviewer
\`\`\`

Promotion requires the sequence `validated → evaluated → accepted`, a held-out
baseline/treatment pair with matching execution and policy fingerprints, no
task-success or verification regression, a non-expired proposal, and an
enabled policy scope. Only the experience-card lane can become retrieval
context. Skill, `AGENTS.md`, prompt/configuration, and diagnostic proposals
remain normal review artifacts; this subsystem cannot change approvals,
permissions, sandboxing, or instruction precedence.

Inspect the state and provenance at any time:

\`\`\`bash
codex debug agent-improvement proposals status \\
  /tmp/improvement-proposal.json \\
  --store "$CODEX_HOME/agent-improvement"
\`\`\`

The status includes the candidate fingerprint, scope, evidence IDs, evaluator
summary, human actor, expiry, and previous version. Events are stored in
`promotion-events.jsonl` and the active view is rebuilt from that log. A
one-command rollback retires the active card and restores the previous active
card when one was recorded:

\`\`\`bash
codex debug agent-improvement proposals rollback \\
  /tmp/improvement-proposal.json \\
  --store "$CODEX_HOME/agent-improvement" \\
  --actor reviewer
\`\`\`

An optional `promotion-policy.json` can disable promotion/retrieval, set a
TTL, cap active improvements, or allowlist repository/task-family scopes. If
the file is absent, bounded manual defaults are used; if an existing policy or
event log is malformed, the runtime fails closed. A global emergency policy
can be written as:

\`\`\`json
{
  "schema_version": 1,
  "enabled": false,
  "kill_switch": true,
  "default_ttl_days": 30,
  "default_max_active": 32,
  "scopes": []
}
\`\`\`

## Run the slow improvement loop

Milestone 7 adds a deterministic, offline slow loop over local cards,
candidates, and explicit experiment outcomes. It runs outside turn preparation
and never calls a model or changes active behavior:

```bash
codex debug agent-improvement evolve run \
  --store "$CODEX_HOME/agent-improvement" \
  --output /tmp/experiment-briefs.json
```

The report identifies recurring failure signatures, repeated verification
omissions, frequently exposed cards that were explicitly not followed,
conflicting recommendations, and task-family scopes without useful coverage.
Each brief contains bounded provenance, competing hypotheses, a card-lane
candidate change, an expected metric, and a held-out evaluation-set rule.

After a separate evaluation, record the outcome so repeated failures are
suppressed rather than proposed indefinitely:

```bash
codex debug agent-improvement evolve record \
  --store "$CODEX_HOME/agent-improvement" \
  experiment_<ID> \
  --fingerprint experiment_fingerprint_<ID> \
  --outcome improved \
  --actor reviewer
```

Supported outcomes are `improved`, `no_improvement`, `regression`,
`inconclusive`, and `retired`. The history is append-only in
`evolution-history.jsonl`.

Plan or explicitly apply bounded maintenance:

```bash
codex debug agent-improvement evolve maintain \
  --store "$CODEX_HOME/agent-improvement"

codex debug agent-improvement evolve maintain \
  --store "$CODEX_HOME/agent-improvement" \
  --apply
```

`--apply` retires stale, negative-feedback, or excess cards but does not
delete their audit records. Export or compact with an archive, and use
`forget` only for explicitly named cards:

```bash
codex debug agent-improvement evolve export \
  --store "$CODEX_HOME/agent-improvement" \
  --output /tmp/cards.jsonl

codex debug agent-improvement evolve forget \
  --store "$CODEX_HOME/agent-improvement" \
  --archive /tmp/cards-before-forget.jsonl \
  card_<ID>
```

The slow loop proposes experiments; it does not accept proposals, promote
cards, edit Skills, modify `AGENTS.md`, change configuration, or grant
permissions. Held-out improvement evidence and the Milestone 6 human review
workflow remain required.

## Run a live held-out evaluation

Milestones 8a and 8a.1 add a manual bridge from an experiment brief to paired
live Codex runs. It is useful when a deterministic brief is ready for a real
test, but it is intentionally outside normal turn preparation and never
promotes a card automatically.

First generate and inspect the brief report:

```bash
codex debug agent-improvement evolve run \
  --store "$CODEX_HOME/agent-improvement" \
  --output /private/tmp/experiment-briefs.json
```

Create a private plan for the selected brief. Keep this file outside the
repository and do not publish it: it contains held-out prompts and filesystem
paths. Its essential shape is:

```json
{
  "schema_version": 2,
  "plan_id": "parser-held-out-v1",
  "brief_id": "experiment_<ID>",
  "brief_fingerprint": "sha256:<brief fingerprint>",
  "primary_checkout": "/private/path/to/clean/codex-checkout",
  "scenarios": [
    {
      "scenario_id": "parser-001",
      "task_family": "coding",
      "split": "held_out",
      "task": {
        "task_id": "parser-fix-001",
        "prompt": "<private held-out task prompt>",
        "fingerprint": "sha256:<task fingerprint>"
      },
      "evaluator": {
        "kind": "post_run_verification",
        "checks": [
          {
            "check_id": "hidden-contract",
            "command_tokens": ["python3", "private_hidden_tests.py"],
            "expected_exit_code": 0,
            "preflight_command_tokens": ["python3", "private_hidden_tests.py", "--preflight"],
            "preflight_expected_exit_code": 0
          }
        ],
        "require_all_checks": true
      },
      "budget": {
        "max_trace_bytes": 67108864,
        "max_tokens": 2000000,
        "max_wall_time_ms": 1800000,
        "max_tool_calls": 512,
        "max_terminal_operations": 512,
        "max_post_run_time_ms": 120000
      }
    }
  ]
}
```

The plan must have at least the number of scenarios requested by the brief,
and every scenario must be held out, use the same task family, and contain a
prompt. The source checkout must be clean for a real run. Use a dry run first:

For a live `post_run_verification` plan, every check must include a
`preflight_command_tokens` argv. Codex runs each distinct preflight once in a
detached source worktree before launching any model. The preflight must test
the evaluator fixture itself (for example, a private test command's
`--preflight` mode); a failing or timing-out preflight aborts the entire run.
Preflight and post-run commands are executed without a shell, with output
discarded, a scrubbed environment, and bounded time.

```bash
codex debug agent-improvement evolve evaluate experiment_<ID> \
  --briefs /private/tmp/experiment-briefs.json \
  --plan /private/tmp/parser-held-out-plan.json \
  --store "$CODEX_HOME/agent-improvement" \
  --run-root /private/tmp/codex-live-evaluations \
  --output /private/tmp/parser-evaluation-results.json \
  --model gpt-5.6-luna \
  --reasoning-effort high \
  --dry-run
```

Embedded benchmark child runs default to the explicit `gpt-5.6-luna` model and
`high` reasoning effort. Keep those settings visible in operator commands when
reproducing a run.

The live evaluator defaults to `--retrieval-mode legacy`; an intervention
comparison must opt in explicitly with `--retrieval-mode intervention` and
`--intervention-context-tokens 480`. That option is for a later gated pilot,
not a replacement for the offline retrieval report or the 8a.5 telemetry gate.

The live runner rejects Astra for this path and verifies the model and effort
recorded in each private trace before accepting the run.

After reviewing the printed execution contract, remove `--dry-run` to run
baseline and treatment from the same commit in separate detached worktrees.
The child runs use the `workspace-write` sandbox, `approval_policy=never`,
ephemeral sessions, isolated trace/state roots, and a per-scenario wall-time
cap. The treatment receives a copy of the active card store; the source
workspace scope is preserved so retrieval remains comparable. Trace size,
token, tool-call, and terminal-operation budgets are enforced by the existing
evaluator after each run. Successful runs clean up their worktrees; add
`--keep-worktrees` when investigating a failed study.

The output is a redacted aggregate result suitable for the offline evaluator.
It does not contain prompts, paths, command tokens, or raw model output. With
`post_run_verification`, checks execute after the child exits, in the isolated
condition worktree, as argv tokens without a shell, with a scrubbed
environment, and before normal worktree cleanup. Only check IDs and boolean
results enter the redacted output. Results also separate `trace_status` and
`policy_status` from the evaluator's coding outcome. To record the derived
outcome in the brief history, only after inspecting the result, add
`--record-outcome --actor <ACTOR>`. This records measured task or verification
deltas; it does not infer that a recommendation was followed.
Model authentication, network access, runtime, and cost are inherited from the
operator's Codex environment, so start with a small held-out set.

## Qualify a benchmark before a live run

Milestone 8a.3 adds a private, versioned benchmark contract and a no-model
qualification gate. Keep the manifest outside published results. Each task
has a model-visible \`prompt\`, while its seed commit, expected contract,
reference solution, evaluator commands, evaluator preflights, budgets, and
fixture worktrees are separate private fields. Pass only the task's
\`model_visible_context\` projection to a model runner.

The practical benchmark contains 32–48 tasks across at least four task
families. Every family must have a good fixture, a no-op or incomplete
fixture, and at least one targeted mutant. Targeted mutants name the checks
that must fail; a mutant that fails an unrelated check does not qualify.

Inspect a manifest without exposing private prompts or commands:

\`\`\`bash
codex debug agent-improvement benchmark validate \\
  /private/path/to/benchmark-v1.json \\
  --output /private/tmp/benchmark-validation.json
\`\`\`

Run the evaluator fixtures in fresh temporary copies. This command performs
argv-only, bounded preflights and checks, never launches a model, never reads
active cards, and removes its generated copies after writing the report:

\`\`\`bash
codex debug agent-improvement benchmark qualify \\
  /private/path/to/benchmark-v1.json \\
  --run-root /private/tmp/codex-benchmark-qualification \\
  --output /private/tmp/benchmark-qualification.json
\`\`\`

The report must have \`qualified = true\`, all preflights and good fixtures
passing, and every targeted mutant detected before a real live evaluation is
allowed. A non-dry-run \`evolve evaluate\` therefore requires
\`--qualification-report\` pointing to this passing report. Dry runs remain
available for checking the live-plan contract without launching Codex.

Baseline calibration is a separate private input collected with retrieval
disabled. It must contain exactly one task-level observation per benchmark
task and the matching source commit and evaluator fingerprint:

\`\`\`json
{
  "schema_version": 1,
  "benchmark_id": "benchmark-v1",
  "source_commit": "private-commit",
  "evaluator_version": 1,
  "evaluator_fingerprint": "sha256:...",
  "retrieval_enabled": false,
  "observations": [
    {
      "task_id": "task-001",
      "success": false,
      "verification_complete": true,
      "input_tokens": 12000,
      "output_tokens": 900,
      "wall_time_ms": 80000
    }
  ]
}
\`\`\`

Generate the calibration report:

\`\`\`bash
codex debug agent-improvement benchmark calibrate \\
  /private/path/to/benchmark-v1.json \\
  --baseline /private/path/to/baseline-observations.json \\
  --output /private/tmp/benchmark-baseline.json
\`\`\`

The qualified difficulty range is 40–75% baseline success. Above 85% is a
ceiling test; below 25% is too difficult. This report calibrates benchmark
difficulty only and does not promote cards or establish a treatment effect.

Before any treatment run, compare broad and scoped retrieval with a private
case set. The case set contains the private query, reduced retrieval context,
and expected relevant card IDs; the aggregate report contains counts and
precision/coverage only:

```bash
codex debug agent-improvement benchmark retrieval-report \
  /private/path/to/retrieval-cases.json \
  --cards "$CODEX_HOME/agent-improvement" \
  --output /private/tmp/retrieval-report.json
```

This command makes no model call and does not print the private case set,
card text, prompts, paths, or card IDs. Do not proceed to a live intervention
treatment if the report shows a nonzero
`scoped_generic_fallback_irrelevant_count` or loss of relevant-card coverage.

## Review card adoption

When a selected card is shown to the model, Codex records a bounded exposure
counter. Exposure does not mean the recommendation was followed or caused a
successful result. Record explicit feedback when a card is useful or harmful:

```bash
codex debug agent-improvement cards feedback \
  --store "$CODEX_HOME/agent-improvement" \
  card_<ID> --value positive
```

If a later result can be linked to the card, record the linkage explicitly;
otherwise use `unknown` rather than inventing causality:

```bash
codex debug agent-improvement cards outcome \
  --store "$CODEX_HOME/agent-improvement" \
  card_<ID> --followed yes --verified yes
```

The values for both fields are `yes`, `no`, `mixed`, or `unknown`. The stored
fields distinguish exposure, feedback, followed, and verified attribution.

Retire a card that is stale, contradictory, or no longer useful. Retirement is
append-only and preserves its local audit record:

```bash
codex debug agent-improvement cards retire \
  --store "$CODEX_HOME/agent-improvement" \
  card_<ID>
```

## Run a paired evaluation

Milestone 4 adds an offline, deterministic evaluator. It consumes two rollout
bundles that were already recorded for the same task: a baseline with memory
disabled and a treatment with the declared card set enabled. The evaluator
never launches a model, writes either worktree, or reads the active card
store. This keeps CI and local measurement independent from credentials,
approval settings, and live sandbox state.

Record the two runs in separate worktrees with tracing enabled, then create a
private scenario manifest. The manifest keeps prompts or task references,
worktree paths, command-token checks, and judge labels local. A minimal
scenario has this shape:

~~~json
{
  "schema_version": 2,
  "manifest_id": "parser-held-out-v1",
  "scenarios": [
    {
      "scenario_id": "parser-001",
      "task_family": "coding",
      "split": "held_out",
      "task": {
        "task_id": "parser-fix",
        "reference": "private/tasks/parser-fix",
        "fingerprint": "sha256-of-task"
      },
      "repository": {
        "snapshot_fingerprint": "sha256-of-repository-snapshot",
        "primary_checkout": "/private/path/to/primary-checkout"
      },
      "baseline": {
        "trace_bundle": "/private/path/to/baseline-trace",
        "worktree": "/private/path/to/baseline-worktree",
        "worktree_fingerprint": "sha256-before-run",
        "execution_config_fingerprint": "config-v1",
        "policy_fingerprint": "policy-v1",
        "memory": {
          "enabled": false,
          "card_set_fingerprint": "none",
          "retrieval_policy_fingerprint": "retrieval-v1",
          "card_schema_version": 2
        }
      },
      "treatment": {
        "trace_bundle": "/private/path/to/treatment-trace",
        "worktree": "/private/path/to/treatment-worktree",
        "worktree_fingerprint": "sha256-before-run",
        "execution_config_fingerprint": "config-v1",
        "policy_fingerprint": "policy-v1",
        "memory": {
          "enabled": true,
          "card_set_fingerprint": "sha256-of-card-set",
          "retrieval_policy_fingerprint": "retrieval-v1",
          "card_schema_version": 2,
          "card_store_path": "/private/path/to/active-card-store"
        }
      },
      "evaluator": {
        "kind": "post_run_verification",
        "checks": [
          {
            "check_id": "tests",
            "command_tokens": ["cargo", "test"],
            "expected_exit_code": 0
          }
        ],
        "require_all_checks": true
      },
      "budget": {
        "max_trace_bytes": 67108864,
        "max_tokens": 2000000,
        "max_wall_time_ms": 1800000,
        "max_tool_calls": 512,
        "max_terminal_operations": 512,
        "max_post_run_time_ms": 120000
      }
    }
  ]
}
~~~

The manifest must contain at least one held_out scenario. Baseline and
treatment fingerprints must match for repository snapshot, initial worktree,
execution configuration, policy, retrieval policy, and card schema; only the
declared memory condition may differ. Worktrees must be disjoint from each
other and from the primary checkout.

Run the evaluator:

~~~bash
codex debug agent-improvement eval \
  /private/path/to/scenario-set.json \
  --output /private/path/to/evaluation-results.json
~~~

Results contain scenario IDs, opaque provenance fingerprints, evaluator
outcomes, check results, trace/policy status, failure classes, retries, tool
and terminal counts, tokens, wall time, cancellation, policy events, held-out
aggregates, and per-task-family aggregates. For live treatment conditions they
also contain a selected-card count, exposure count, and opaque selection
fingerprint; they never contain raw card IDs. They do not contain prompts,
command tokens, local paths, or judge evidence references. Missing traces,
evaluator failures, and cost-limit interruptions remain explicit partial
results rather than being converted to zeroes. Treat
exposed_and_outcome_improved as descriptive evidence only; it is not a causal
claim.

## Inspect and remove cards and candidates

Preview the cards that would be relevant to a task:

```bash
codex debug agent-improvement render \
  --store "$CODEX_HOME/agent-improvement" \
  --query "fix the parser and run targeted tests"
```

The store is append-only during normal card creation. It deduplicates exact
card updates, merges recurring lesson identities, and loads the latest valid
record for each card ID. Candidate status transitions are also append-only;
loading uses the latest valid record for each candidate ID. To remove
experience, stop Codex and delete the relevant local files:

    $CODEX_HOME/agent-improvement/cards.jsonl
    $CODEX_HOME/agent-improvement/candidates.jsonl
    $CODEX_HOME/agent-improvement/traces/

Cards can be recreated from reviewed traces; candidates can be recreated by
running future sessions with capture enabled. Delete the entire
agent-improvement directory if its empty-directory metadata is also unwanted.

## Review before use

Cards contain derived counters, coarse labels, bounded activity categories,
opaque topic keys, bounded signals, a recommended next step, and verification
labels. They intentionally retain a source trace ID and timestamp for local
provenance, but not raw transcript or command content.

Review a card with render before enabling retrieval. Treat retrieved cards as
historical evidence, not policy or proof that a recommendation is correct.
Keep generated strategy, Skill, AGENTS.md, or source-code changes outside
this store and subject them to normal tests and human review.

Card retrieval is not an evaluation, a guarantee of correctness, or a
replacement for running relevant verification. The strongest workflow is:

1. enable candidate capture for a narrowly scoped local workflow;
2. observe a rollout and review the pending candidate;
3. accept only a lesson that is useful and bounded;
4. inspect the rendered context;
5. enable retrieval for the matching workflow;
6. verify future changes independently.
