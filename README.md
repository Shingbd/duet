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

## Unified Helix + contextual review AI

**F2 Review and F3 Project now use the same real embedded Helix session**, with Duet's dark/light palette. No `h` launch step. Entry focus stays in the sidebar; **Tab** enters Helix and **F6** returns. Editor-focused keys belong to Helix, not workbench shortcuts.

In Review, files are independent private drafts bound to fixed task/node snapshots. **Ctrl+D** shows the read-only full additions/deletions comparison; Ctrl+D returns to Helix. **Ctrl+G** reads the actual selection (or cursor word/line), including unsaved text, through an explicit command/stdin bridge—not terminal-cell guesses. Enter directly submits the question without a range popup. AI discussion disables every Pi tool; **Ctrl+P → Enter** explicitly applies a suggestion only after rechecking the actual full buffer.

**Ctrl+S** saves the current private draft and attempts a new immutable revision. Publication remains limited to the final-delivery node of a fully implemented, unmerged task, within original scope. Old candidates, snapshots and approvals remain intact; a new revision needs new approval. Save/exit Helix before integration or starting implementation.

Project has a directory tree: **j/k** selection immediately follows the selected file, **h/l** collapse/expand directories, Enter explicitly reopens/toggles; **r** refreshes and **z** toggles ignored files. The same PTY retains unsaved buffers; selecting never saves or commits. Project targets are actual working files, backed up before host-driven opening.

Default `hx` uses PATH or the already-installed executable-relative bundle. Duet-managed temporary configuration applies its palette and bridge keys without rewriting your global Helix settings. Helix tools have account permissions, **not sandbox permissions**. Opening Duet now preloads a fresh Helix by default (`helix_startup:true`), keeping focus in the sidebar; first code entry reuses its PID. Empty projects show a private welcome buffer. Restarts restore records and open a new editor, never old executing sessions/unsaved editor state or models. `helix_startup:false` or explicit native review retains on-demand launching. Implementation/merge still require explicit editor stop. Contextual AI currently applies to managed Review buffers, not Project files or arbitrary Helix-picker buffers.

An explicitly selected legacy native review backend remains available through `review_editor: "native"` / `--review-editor native`; it is never mislabeled Helix or silently selected when hx fails. Conflict/manual final-integration editing retains `h → Enter`.

In Helix, Ctrl+] / Ctrl+[ select the next / previous managed file without leaving code or fullscreen; Ctrl+[ requires a terminal supporting keyboard disambiguation (legacy Esc stays Esc). Ctrl+PageDown/PageUp are portable alternatives. F6 returns to the workbench. Switching from insert mode ends insertion without saving; other buffers/undo remain.

File switches now execute an internal F12 binding once, not typed :open sequences; first-load IO still applies. **Space q** saves the current file and returns focus with the same PID and other unsaved buffers intact. F6 only returns focus. Native :wq still exits its process; by default Duet reopens Helix without falling back to the native reader (editor undo history is not restored). Before implementation/integration, use host `:editor-stop → Enter`, or configure `helix_keepalive:false`.

See [Helix setup/safety](docs/helix.md), [review discussion/publication](docs/review-editor.md), and [project tree](docs/project-tree.md).

Rust editing now defaults to a private fast syntax profile: base syntax/grammar/LSP remain, while locals and nested-language coloring are simplified. Set `helix_rust_syntax:"full"` in duet.json and restart to restore upstream processing. Same-file 210×49 local benchmarks improved release ~71→10 ms and debug ~97→11 ms; SSH latency is separate. See [performance](docs/performance.md).

## The workflow

```text
Discuss → Confirm the plan → Implement nodes → Review checkpoints
        → Approve nodes → Review integration diff → Confirm merge
```

| Step | Action | What it authorizes |
| --- | --- | --- |
| Discuss | `F1` → `i`, enter your request | Read-only AI discussion; no automatic implementation |
| Start or continue | `s` → `Enter` | Write access in the task worktree for the confirmed plan |
| Review/edit draft | `F2`, Tab; `{` / `}` in workbench focus switch nodes | Full-code view and independent local edits; no automatic approval |
| Test a fixed node | `v`, configure argv/criteria, Enter then Enter | Execute trusted project code in an independent snapshot worktree; no approval/merge |
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
| `F1` / `F2` / `F3` / `F4` | Tasks / Review / Project / Knowledge |
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

The first **fixed-version verification Runner** is implemented: press `v` in workbench focus on a delivered node, configure `verify JSON`, then preview and explicitly confirm. Unmapped commands record exit status without claiming acceptance; mapped criteria are human-declared test coverage, not semantic proof. Evidence survives restart without re-execution, failure blocks integration, and new snapshots/contracts do not inherit old results. Final integrated-version Runner, sandboxing and a richer criteria-mapping editor remain pending. See [verification](docs/verification.md). The file-read feature (`m`, read markers/counts) is removed; conflict `m` still marks resolution.

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
- [Fixed-version verification Runner — bilingual](docs/verification.md)
- [Task and node contracts](docs/nodes.md)
- [Task pipeline and integration](docs/task-pipeline.md)
- [Implementation status and history](docs/phase-1.md)
- [Performance measurements and caveats](docs/performance.md)
- [Example keymap](examples/keymap.json)

The detailed design and history documents are currently in Chinese.
