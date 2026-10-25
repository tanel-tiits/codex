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
```

The card store is:

```text
$CODEX_HOME/agent-improvement/cards.jsonl
```

Codex reads that file at turn preparation time and selects cards with
deterministic lexical overlap and opaque topic-key overlap against the current
user text. Missing stores, malformed lines, oversized cards, and read failures
are ignored with a bounded warning.

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

## Inspect and remove cards

Preview the cards that would be relevant to a task:

```bash
codex debug agent-improvement render \
  --store "$CODEX_HOME/agent-improvement" \
  --query "fix the parser and run targeted tests"
```

The store is append-only during normal card creation and deduplicates
deterministic card IDs. To remove experience, stop Codex and delete
$CODEX_HOME/agent-improvement/cards.jsonl; the data is local and can be
recreated from reviewed traces. Delete the entire agent-improvement directory
if its empty-directory metadata is also unwanted.

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

1. observe a rollout;
2. reduce and review the trace;
3. create a bounded card only when the lesson is useful;
4. inspect the rendered context;
5. enable retrieval for a narrowly matched workflow;
6. verify future changes independently.
