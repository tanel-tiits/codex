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
7. On explicit confirmation, accept or reject only the named candidate:

       codex debug agent-improvement candidates accept --store <STORE> <CANDIDATE_ID>
       codex debug agent-improvement candidates reject --store <STORE> <CANDIDATE_ID>

8. After acceptance, verify the active card with the read-only render command
   or codex debug prompt-input. Explain that active cards are advisory,
   untrusted context and cannot change permissions or instruction precedence.

## Boundaries

- Never accept a candidate silently or in bulk without explicit user approval.
- Never apply a proposal, edit AGENTS.md, Skills, configuration, repository
  code, permissions, approval policy, or sandbox policy as part of this
  workflow. Emit diffs for review only.
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
