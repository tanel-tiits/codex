# Milestone 8a.1 live pilot report

Date: 2026-10-28  
Status: completed; no promotion or live-behavior change

## Study

The pilot used a private, dependency-free Python project with eight held-out
coding tasks covering parser escaping, serialization, query behavior, merging,
configuration, CLI contracts, document parsing, and aggregation. Every task
started from the same clean benchmark snapshot and ran in paired isolated
worktrees:

- baseline: retrieval disabled;
- treatment: retrieval enabled with three reviewed experience cards;
- both: identical model, approval, sandbox, resource limits, and independent
  post-run checks.

Prompts, hidden checks, raw traces, worktrees, and the private plan remain
outside this repository. No outcome was recorded or promoted automatically.

## Result

The first generated manifest reported 5/8 for both conditions. Review found
three defects in the private evaluator rather than in the candidates: one
ordering expectation contradicted the documented ascending order, one boolean
case contradicted the task's “reject unknown values” requirement, and the CLI
check accidentally shadowed the imported `main` function. After correcting
those private checks, all retained worktrees passed all eight checks: 8/8 for
baseline and 8/8 for treatment. This was an evaluator recheck; the model runs
were not rerun.

| Signal | Baseline | Treatment | Interpretation |
| --- | ---: | ---: | --- |
| Corrected task checks | 8/8 | 8/8 | No correctness lift observed |
| Verification attempts | 23 | 31 | Treatment attempted more verification |
| Failed verification attempts | 0 | 1 | More attempts were not uniformly better |
| Tool calls | 87 | 84 | Three fewer treatment calls |
| Terminal operations | 75 | 72 | Three fewer treatment operations |
| Retries | 1 | 1 | No change |
| Input tokens | 1,384,680 | 1,441,143 | Treatment used 56,463 more input tokens |
| Output tokens | 47,862 | 47,534 | Essentially unchanged |
| Reasoning output tokens | 29,963 | 28,569 | Treatment used fewer reasoning tokens |
| Wall time | 388,688 ms | 391,193 ms | Essentially unchanged |

The treatment retrieved all three cards for each task (24 exposures in total).
Exposure is not adoption: the pilot did not establish that a recommendation
was followed or caused a result, so attribution remains inconclusive.

## Assessment

This is a useful infrastructure result, not evidence that cards improve coding
success. The independent evaluator worked, the treatment was isolated, and the
cards were actually visible to the model, but the benchmark was too easy to
show a binary success difference: both conditions solved every corrected task.
The increased verification activity is a promising process signal, but the
single failed verification and small sample do not justify promotion.

The broad three-card retrieval is also a measurable cost and relevance issue.
Unknown-scope cards were eligible on every task, so the treatment received
generic advice even when a narrower card would have been preferable.

## Next actions

1. Keep Milestone 8 gated. Do not promote these cards or claim a behavioral
   improvement from this study.
2. Improve the next pilot's retrieval treatment: use scoped cards and a
   minimum relevance threshold, cap the context to the most relevant one or
   two cards, and report selected-card IDs/exposure separately from outcomes.
3. Add evaluator-plan preflight coverage for private check contracts so a
   malformed or self-shadowing check cannot produce a false regression result.
4. Repeat with a larger, more difficult set of realistic coding tasks,
   multiple task families, and repeated baseline/treatment trials. Measure
   correctness, verification quality, cost, policy events, and negative
   transfer before considering promotion.
5. Only after repeated held-out gains and a rollback demonstration should the
   optional program-level evolution work in Milestone 8 be considered.
