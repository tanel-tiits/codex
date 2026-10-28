---
name: agent-card-curator
description: Review captured Codex agent-improvement candidates and draft evidence-linked, review-only card or operating-guidance proposals before anything can influence future turns.
---

# Agent Card Curator

Use this Skill when the user asks to review, curate, accept, reject, inspect,
or draft an improvement from automatically captured agent-improvement
candidates.

## Review workflow

1. Locate the configured agent-improvement store. Prefer the user's explicit
   path; otherwise use $CODEX_HOME/agent-improvement.
2. List pending candidates:

       codex debug agent-improvement candidates list --store <STORE>

3. Show each candidate that may be relevant:

       codex debug agent-improvement candidates show --store <STORE> <CANDIDATE_ID>

4. Explain the bounded evidence, failure class, recommendation, verification
   labels, source trigger, and any uncertainty. Do not infer facts that are not
   present in the candidate.
5. If the user asks for a proposal, identify one named pending candidate and a
   concrete held-out evaluation scenario. Draft a review-only card artifact:

       codex debug agent-improvement proposals draft-card \
         --store <STORE> <CANDIDATE_ID> \
         --evaluation-scenario <SCENARIO_ID> \
         --output <PROPOSAL.json>

   Validate it structurally, and when the local candidate store and redacted
   evaluation result are available, validate the evidence references too:

       codex debug agent-improvement proposals validate <PROPOSAL.json> \
         --candidate-store <STORE> \
         --evaluation-results <RESULTS.json>

   Summarize the proposal's target, evidence, expected benefit, costs, risks,
   validation plan, expiry, and rollback intent. A proposal is not an accepted
   card and does not affect retrieval.
   For a focused Skill, AGENTS.md, configuration, or diagnostic/tooling
   proposal, produce only the corresponding versioned envelope and bounded
   diff/content, then run the same validation command. Keep code and guidance
   changes as reviewable diffs; do not apply them to the checkout.
6. Ask the user for an explicit decision for each candidate. A request to
   “review” does not authorize acceptance or promotion.
7. For a proposed behavioral card, use the Milestone 6 control plane after
   the held-out result is available. Record validation and evaluation in the
   promotion root, then ask separately before each human decision:

       codex debug agent-improvement proposals validate <PROPOSAL.json> \
         --state-store <STORE> --candidate-store <STORE> \
         --evaluation-results <RESULTS.json> --actor <ACTOR>
       codex debug agent-improvement proposals evaluate <PROPOSAL.json> \
         --store <STORE> --candidate-store <STORE> \
         --evaluation-results <RESULTS.json> --actor <ACTOR>

   Only after explicit approval may the named proposal be accepted and
   promoted:

       codex debug agent-improvement proposals accept <PROPOSAL.json> \
         --store <STORE> --actor <ACTOR>
       codex debug agent-improvement proposals promote <PROPOSAL.json> \
         --store <STORE> --actor <ACTOR>

   Inspect provenance with `proposals status`. If the user instead explicitly
   requests the foundational candidate workflow, accept or reject only the
   named candidate:

       codex debug agent-improvement candidates accept --store <STORE> <CANDIDATE_ID>
       codex debug agent-improvement candidates reject --store <STORE> <CANDIDATE_ID>

8. After promotion or foundational acceptance, verify the active card with the
   read-only render command or codex debug prompt-input. Explain that active
   cards are advisory, untrusted context and cannot change permissions or
   instruction precedence. A promoted card can be rolled back explicitly:

       codex debug agent-improvement proposals rollback <PROPOSAL.json> \
         --store <STORE> --actor <ACTOR>

9. For periodic long-horizon review, run the Milestone 7 slow loop over the
   bounded local store. It proposes inert experiment briefs; it does not call a
   model, accept candidates, or promote cards:

       codex debug agent-improvement evolve run \
         --store <STORE> --output <BRIEFS.json>

   Inspect recurring failure signatures, verification omissions, explicit
   not-followed attribution, conflicts, coverage gaps, held-out metrics, and
   provenance. After a separate evaluation, record its outcome:

       codex debug agent-improvement evolve record \
         --store <STORE> <BRIEF_ID> \
         --fingerprint <FINGERPRINT> --outcome <OUTCOME> --actor <ACTOR>

   Use `evolve maintain` to plan stale/negative/retention cleanup. Only use
   `--apply`, archive-backed compaction, export, or explicit `forget` when the
   user has asked for that maintenance operation.

10. When a brief has a concrete held-out task set, prepare the private
    Milestone 8a evaluation plan and run the bridge only after the user asks
    for a live evaluation. Keep prompts, checkout paths, and task references
    private; run `evolve evaluate --dry-run` first, then inspect the redacted
    result before recording an outcome. Never treat a live result as permission
    to accept or promote a card.

## Boundaries

- Never accept a candidate silently or in bulk without explicit user approval.
- Never apply a proposal, edit AGENTS.md, Skills, configuration, repository
  code, permissions, approval policy, or sandbox policy as part of this
  workflow. Emit diffs for review only.
- Never promote a proposal without a held-out evaluation result and separate
  human acceptance. Non-card lanes remain normal diff-review artifacts.
- Never launch a live evaluation without an explicit user request, a private
  held-out plan, and a successful dry run. The live bridge is bounded and
  isolated, but it can consume model/tool budget and inherit the operator's
  credentials and network access.
- Never draft from raw prompts, model responses, terminal output, secrets, or
  filesystem paths. Use only bounded cards and opaque evidence identifiers.
- Never treat a structural validation pass as an evaluation result. A
  behavioral proposal needs a real regression command or paired held-out
  evaluation evidence before a later promotion workflow can consider it.
- Never copy raw prompts, model responses, terminal output, secrets, or paths
  into a candidate, proposal, or card.
- Pending, rejected, and expired candidates must not be rendered as active
  experience context.
- If the candidate store is missing or malformed, report that it is unavailable
  and do not manufacture a card.
