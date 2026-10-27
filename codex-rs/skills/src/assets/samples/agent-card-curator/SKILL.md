---
name: agent-card-curator
description: Review automatically captured Codex agent-improvement candidates and explicitly accept or reject bounded experience cards before they influence future turns.
---

# Agent Card Curator

Use this Skill when the user asks to review, curate, accept, reject, or inspect
automatically captured agent-improvement candidates.

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
5. Ask the user for an explicit decision for each candidate. A request to
   “review” does not authorize acceptance.
6. On explicit confirmation, accept or reject only the named candidate:

       codex debug agent-improvement candidates accept --store <STORE> <CANDIDATE_ID>
       codex debug agent-improvement candidates reject --store <STORE> <CANDIDATE_ID>

7. After acceptance, verify the active card with the read-only render command
   or codex debug prompt-input. Explain that active cards are advisory,
   untrusted context and cannot change permissions or instruction precedence.

## Boundaries

- Never accept a candidate silently or in bulk without explicit user approval.
- Never edit AGENTS.md, Skills, configuration, repository code, permissions,
  approval policy, or sandbox policy as part of this workflow.
- Never copy raw prompts, model responses, terminal output, secrets, or paths
  into a candidate or card.
- Pending, rejected, and expired candidates must not be rendered as active
  experience context.
- If the candidate store is missing or malformed, report that it is unavailable
  and do not manufacture a card.
