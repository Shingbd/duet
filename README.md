# Duet Workbench

**A local-first Rust terminal workbench for human–AI development.**

[English](README.md) · [简体中文](README.zh-CN.md)

Turn a request into structured tasks, implement them in isolated Git worktrees, review immutable diffs, and decide explicitly what enters your project.

> **The AI proposes and implements. You authorize, review, and merge.**
> Opening Duet never calls a model or resumes an old task automatically.

## Why Duet?

- **Structured work, not just chat.** Each task contains dependency-ordered nodes with objectives, scope, boundaries, implementation guidance, constraints, and acceptance criteria.
- **Keep your project separate.** AI implementation runs in a managed worktree. Uncommitted changes in your main working directory are not silently included in a task.
- **Review fixed versions.** Live previews and final deliveries are immutable Git snapshots. Review follows the selected node without stealing unrelated reading positions.
- **Separate implementation, verification, and approval.** “Implemented” is not “tests passed.” Approval is tied to a specific node checkpoint.
- **Human-led conflict resolution.** A read-only AI advisor can explain conflicting changes and their trade-offs. Only your confirmed choice modifies the integration draft.
- **Local continuity.** SQLite stores discussions, contracts, attempts, notes, and reading state. Restarting restores history, not an executing agent.
- **Readable terminal UI.** Markdown, semantic contract colors, light/dark themes, and Unicode-aware input wrapping support Chinese text and long messages.

**Early-stage project:** the current TUI is primarily Chinese. An optional English UI currently covers core navigation/session labels and help; workflow dialogs and many errors remain Chinese. Complete localization and multi-agent automation are not yet available.

## Quick start

### Requirements

- Rust **1.90+** and Cargo.
- Git; AI coding requires a repository with a committed baseline.
- A terminal capable of displaying Unicode. The automated process/PTY tests target Unix.
- For AI features: Node.js **22.19+** and a configured, authenticated **Pi CLI** available as `pi` on `PATH`. Pi 1.1.0 has been used in development.

You can browse and restore local records without starting Pi. Model calls require explicit authorization.

### Build and open

```bash
git clone https://github.com/Shingbd/duet.git
cd duet
cargo build --release
./target/release/duet-workbench open /path/to/your/project
```

To open Duet's own repository:

```bash
./target/release/duet-workbench open .
```

Prefer release builds for everyday use, especially over SSH. Quit an old process with `q` before opening the rebuilt version.

## Configuration

Defaults are shipped in [`duet.default.json`](duet.default.json). This repository includes a minimal [`duet.json`](duet.json) selecting Chinese; edit it directly. For another project without a config, create one explicitly:

```bash
./target/release/duet-workbench config init duet.json
./target/release/duet-workbench config show .
./target/release/duet-workbench --language en --theme light open .
```

Preferences include language (`zh-CN` / partial `en`), theme, initial view, keymap, editor, Pi executable/model, execution budget, and UI polling/save delays. Precedence: built-in → user-global → project's `duet.json` → `--config` → CLI overrides. Preferences never grant model/write/merge permission. See the [configuration guide](docs/configuration.md) for paths, validation, and language coverage.

## Editable review + contextual AI

Review now automatically shows full code, line numbers, lexical highlighting and colored additions/deletions—no `h` or editor subprocess needed. **Tab** focuses the native editor; **i** edits; **Ctrl+]** returns to workbench shortcuts.

**Ctrl+G** or **:ask** opens a code question. Enter sends the cursor word/line or selection directly, including unsaved edits, **without a range-confirmation dialog**. Replies appear next to the code. Discussion disables all Pi tools; suggested patches require **Ctrl+P → Enter** and reject changed buffers.

**Ctrl+S** saves the draft and attempts a new immutable revision, resetting approval/verification without touching main or the original candidate. First-version publication supports the final-delivery node of a fully implemented, unmerged task, within its original scope; earlier/live/historical-node drafts can be edited/discussed, but their publication/downstream rebuild is not implemented.

See [editable review and contextual discussion](docs/review-editor.md) for controls, persistence and limitations. The default view is host-owned, not Helix; syntax coloring is lexical rather than full language-server semantics.

## Project file tree + seamless Helix

**3** shows a directory tree and directly opens the selected project file in real embedded Helix. **j/k**, **h/l**, Enter navigate/open; **Tab** focuses Helix and **Ctrl+]** returns to the tree. **r** refreshes, **z** toggles ignored files; Git changes are marked.

Files switch in the same PTY, preserving unsaved buffers without automatic saves. Targets are backed up. Install/configure `hx`; editor configuration/LSP tools have account permissions, not sandbox permissions. Startup/restart only restores tree preferences, never automatically launches an editor. Review retains its separate node/file navigation.

See [project tree and Helix](docs/project-tree.md) for limits and real multi-buffer PTY validation.

## Optional review/conflict Helix

Helix can run **inside the code/diff panel** using a real Unix PTY, without leaving Duet. Install `hx` with its runtime, or set `helix_path` / use `--helix /path/to/hx`.

- In review/conflict, **h → Enter** opens the selected target; **Ctrl+]** returns focus to Duet. Project tree uses **3 / Enter**, then **Tab** to focus; **h/l** are directory navigation there.
- **:wq → Enter** saves/exits. No automatic commit, approval, model call, or merge.
- Project code edits actual main files after backup. Review editing currently requires a fully approved, unmerged task and creates a separate integration revision, not a mutation of its node snapshots.
- Editing/advisory/model write workflows cannot run concurrently. First release is Unix/keyboard-only; mouse/clipboard protocol passthrough and pending-node rework are not implemented.

See [embedded Helix](docs/helix.md) for target semantics, safety, and real-editor PTY tests.

## The workflow

```text
Discuss → Confirm the plan → Implement nodes → Review checkpoints
        → Approve nodes → Review integration diff → Confirm merge
```

| Step | Action | What it authorizes |
| --- | --- | --- |
| Discuss | `1` → `i`, enter your request | Read-only AI discussion; no automatic implementation |
| Start or continue | `s` → `Enter` | Write access in the task worktree for the confirmed plan |
| Review/edit draft | `2`, Tab; `{` / `}` in workbench focus switch nodes | Full-code view and independent local edits; no automatic approval |
| Approve a node | `a` → `Enter` in review | Approve that exact checkpoint; no testing or merging |
| Continue after a review pause | `p` | Explicitly resume; approval alone does not resume execution |
| Integrate | `g` | Open the whole-task integration diff or conflict draft |
| Merge from the final diff | `Enter`, then `Enter` | Open risk confirmation, then explicitly apply the merge |

`Esc` cancels a dialog without granting its requested authorization. All nodes must be implemented and approved at their current final checkpoints before integration.

### If changes conflict

Duet creates a separate integration draft; your main project and original candidate remain untouched.

- `y` → `Enter`: ask AI to explain the current file. This may send code to a provider and consume quota; the advisor cannot write files.
- `o` / `t` / `a` → `Enter`: explicitly choose the main version, candidate version, or an available AI patch. Whole-file choices can discard the other side's changes.
- `e` → `Enter`: edit locally; `m` captures the repair and marks the file handled. The editor defaults to `vi`; `DUET_EDITOR` accepts one executable path.
- `3` views the fixed draft; `4` shows explanations. `0` / `1` / `2` compare the base, main, and candidate.
- `l` → `Enter`: separately authorize `Cargo.lock` when needed and the task already authorizes `Cargo.toml`; this does not generate files or run Cargo.
- When every file is handled, `Enter` opens a **new immutable final diff**. Review it, then use the two confirmation steps to merge.

A handled conflict is not a passed test. If the draft is already prepared, follow its highlighted next step rather than choosing a version again. Main HEAD/branch or approval changes invalidate the old draft; `r` explicitly creates a new one while preserving old repairs.

### Recover without starting over

Errors and their stages are persisted. `r` confirms a retry of a failed discussion; implementation recovery uses `s` and the saved plan. Continuation preserves candidate files and includes a fixed-version handoff in a **new** Pi session; it does not recover the original session or guarantee that the model will preserve every line.

The default active budget is 600 seconds per start/continuation, excluding pauses and review backpressure. No automatic retries or model switches occur.

## Navigation

| Keys | Purpose |
| --- | --- |
| `1` / `2` / `3` / `4` | Tasks / Review / Project / Knowledge |
| `Tab`, `j` / `k`, arrow keys | Change focus; select or scroll according to the focused panel |
| `[` / `]` | Previous / next file |
| `f`, `t`, `?` | Full screen, theme, help |
| `/`, `:` | Search, command input |
| `p`, `Ctrl+X` | Pause/resume execution, cancel execution |
| `d` → `Enter` | Delete a task, or roll back a node and its topological successors, after backup |
| `q` | Quit; does not resume work on next launch |

Keys are contextual: for example, `a` approves a node in review but selects a proposal inside a conflict dialog. Lowercase shortcuts are configurable with `--keymap`; uppercase `J/K` are reserved by default. Input wraps visually without inserting newlines; `Enter` submits and `Esc` cancels.

## Local data and safety

```text
your-project/.duet/
├── workspace.db    # Tasks, messages, contracts, notes, reading state
├── workspace.lock  # Single-instance project lock
├── sessions/       # Pi session logs
├── worktrees/      # Implementation candidates
├── integrations/   # Conflict drafts and advice
└── backups/        # Merge/discard backups
```

- Data is **local plaintext** and Git-ignored by default. Do not publish private session logs or databases.
- A worktree is **not an OS security sandbox**. Model tools have the current account's filesystem access; authorized model calls may disclose code to providers.
- Duet's AI workflow does not expose bash or load arbitrary Pi extensions, MCP servers, skills, or repository context.
- Duet does not automatically run tests, stash, push, deploy, or merge. Controlled merge disables Git hooks, fsmonitor, and signing.
- Dirty main workspaces, stale approvals, failed verification, unsafe paths, and unsupported Git drivers/filters or hidden index flags are rejected. Do not modify main Git state concurrently with integration.
- Deleted records/candidates are backed up first. Deletion does not undo an already merged commit or unknown changes outside the managed workspace.

## Current limits

The verification Runner and fixed-version test evidence are **not yet implemented**. Acceptance criteria remain unverified unless independently recorded; external maintenance tests do not change node verification status.

Also pending: arbitrary historical-node rework, original Pi session/queue recovery, independent review agents, multi-agent scheduling, background startup/snapshotting, and an OS sandbox. The fallback fixed-baseline reader loads the first 80 files, up to 1 MiB per file; project tree navigation supports up to 5000 entries, and node-change review has no 80-file limit.

## Development

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release

python3 scripts/conflict-smoke.py target/release/duet-workbench
python3 scripts/merge-smoke.py target/release/duet-workbench
```

The test suite uses fake RPC processes and real temporary Git repositories/PTYs, not live model calls. Python 3 is required for the smoke scripts. Additional scripts cover task controls, retry, nodes, conversations, navigation, terminal restoration, and performance.

## Documentation

- [Project tree and seamless Helix — bilingual](docs/project-tree.md)
- [Editable review and contextual AI — bilingual](docs/review-editor.md)
- [Embedded Helix — bilingual](docs/helix.md)
- [Configuration reference — bilingual](docs/configuration.md)
- [Detailed operating guide — Chinese](docs/user-guide.zh-CN.md)
- [Task and node contracts](docs/nodes.md)
- [Task pipeline and integration](docs/task-pipeline.md)
- [Implementation status and history](docs/phase-1.md)
- [Performance measurements and caveats](docs/performance.md)
- [Example keymap](examples/keymap.json)

The detailed design and history documents are currently in Chinese.
