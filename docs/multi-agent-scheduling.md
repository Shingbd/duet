# Multi-Agent Scheduling

Status: design proposal. Informed by Microsoft Agensh (MIT, October 2026). Not implemented yet.

## Background

Duet's roadmap includes a multi-agent scheduler (alongside the integrated Runner, English UI localization, and OS sandbox). Agensh, from Microsoft Research, coordinates up to 1,024 coding agents with no central orchestrator: workers self-claim sub-tasks, share findings, and merge into a common Git repo. On ProgramBench's five hardest tasks, 128 agents lifted the mean test-pass rate from 19.31% to 28.78%.

Duet is a local, single-user Rust TUI workbench, not a benchmark fleet. This document takes only the ideas that fit Duet's invariants: structured tasks, isolated worktrees, immutable reviews, and human approval before merge.

## What we borrow

1. Claim-based task assignment. Agents pull tasks from a shared pool instead of waiting for a central scheduler to assign work. A claim is recorded as a CLAIM note; overlapping claims are resolved by direct negotiation between the claiming agents, not by a coordinator. In Duet, a claim maps to exclusive ownership of an isolated worktree for that sub-task.
2. Typed shared board. Shared context is not free-form chat. Notes are typed: OBSERVED, FACT, FAIL, CLAIM, PATCH_SUMMARY. The board is append-only, which fits Duet's immutable-review model and is cheaper to maintain than unstructured agent chatter.
3. Verify-then-submit. Every sub-task carries acceptance criteria. The worker verifies its work against those criteria (using Duet's fixed-version verification Runner) before the work enters review.

## What we deliberately do NOT borrow

- Full decentralization. Agensh removes the orchestrator because the orchestrator becomes the bottleneck at hundreds of agents. Duet runs a handful of agents on one machine, where a lightweight coordinator for the task pool and worktree allocation is simpler and more controllable. Keep the coordinator; make claiming agent-driven inside it.
- Agent self-merge. Agensh workers merge into the shared repo directly. In Duet, agent work merges only to a review branch; it enters the main line only after human approval. Immutable review plus human approval before merge is a core invariant and stays.
- Heavy infrastructure. No Gitea / Mattermost / Docker fleet. The task pool and the typed board live in Duet's local state, next to its worktrees and local git repos.

## Proposed worker loop

1. Read the goal and the board.
2. Claim a sub-task: write a CLAIM note. On conflict, negotiate with the other claimant; the loser releases.
3. Work in an isolated worktree. Post OBSERVED / FACT / FAIL notes early, not only at the end.
4. Verify against the sub-task's acceptance criteria with the verification Runner.
5. Submit a PATCH_SUMMARY to review. A human approves, then it merges. Repeat.

## Open questions

- Claim-conflict protocol details: timeout, priority, and what happens when negotiation stalls.
- Board storage format (JSONL vs SQLite) and retention policy.
- How Runner verification results feed back into FAIL notes.

## References

- Microsoft Agensh (MIT): github.com/microsoft/agensh, research paper on arXiv, MarkTechPost overview 2026-10-24.
