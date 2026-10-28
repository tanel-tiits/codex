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
capture = "off"
max_candidates = 64
```

The card store is:

```text
$CODEX_HOME/agent-improvement/cards.jsonl
```

Codex reads that file at turn preparation time and selects active cards with
deterministic structured matches (workspace, task family, failure class, tool
and toolchain family) plus evidence quality, confidence, recurrence, recency,
and bounded lexical/topic overlap. A mismatched opaque workspace is filtered;
unknown scope remains a fallback. Missing stores, malformed lines, oversized
cards, and read failures are ignored with a bounded warning.

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
