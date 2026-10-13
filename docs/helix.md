# Embedded Helix / 框内 Helix

[English README](../README.md) · [中文首页](../README.zh-CN.md)

Project entry **3** now uses [a project file tree and seamless Helix](project-tree.md): no h confirmation there; Tab focuses the editor, Ctrl+] returns to the tree, h/l navigate directories. This page's h-confirmation controls apply to optional review/conflict editing.

项目入口 **3** 已接入[文件树与无缝 Helix](project-tree.md)，无需 h 确认；Tab 进入编辑器，Ctrl+] 回树，h/l 导航目录。本页 h 确认流程适用于审核/冲突可选编辑。

The default review panel is now [a native editable review buffer with contextual AI](review-editor.md), not a Helix process. This page documents the optional Helix entry. Its PTY cannot supply exact cursor/selection/unsaved-buffer context to Duet.

默认审核现在使用[原生可编辑缓冲区与上下文 AI](review-editor.md)，不是 Helix 进程。本页说明可选 Helix 入口，其 PTY 不向 Duet 提供精确光标、选区或未保存缓冲区。

Helix runs in a real Unix PTY. Duet parses the child terminal output into styled cells and renders them inside the code/diff panel, with its own navigation and evidence panels still visible. This is not a suspend-and-launch external editor.

Helix 在真实 Unix PTY 中运行，Duet 将输出解析为带样式的单元格，绘制在代码/diff 框内，保留工作台侧栏与证据区，不是暂停工作台后另开编辑器。

## Setup / 配置

Install Helix with its matching runtime files, and make `hx` available on `PATH`, or configure:

安装 Helix 及匹配的 runtime，让 `hx` 可从 `PATH` 调用，或者配置：

```json
{
  "helix_path": "/absolute/path/to/helix/hx"
}
```

Temporary CLI override / 本次启动指定：

```bash
./target/release/duet-workbench --helix /absolute/path/to/hx open /path/to/project
```

`helix_path` selects the embedded editor. `editor` and `DUET_EDITOR` continue to select the separate external editor used by `e` in conflict dialogs. Duet never automatically downloads or installs an editor when opening a project.

`helix_path` 用于内嵌编辑；`editor` / `DUET_EDITOR` 仍控制冲突界面 `e` 调用的外部编辑器。打开项目不会自动下载或安装编辑器。

## Keys / 操作

1. In Project code, approved Review diff, or an integration draft, press **h**, then **Enter** to authorize the selected target. **Esc** cancels without starting a process.
2. Normal keys—including `i`, `q`, `:`, Esc, arrows, control keys and paste—go to Helix while it has focus; they do not trigger Duet shortcuts.
3. **Ctrl+]** switches focus back to Duet without closing Helix; **h** returns focus to Helix. Keep the editor running while browsing other sidebar selections.
4. **:wq → Enter** saves and exits Helix; **:q** exits without saving when Helix allows it. Exit does not approve, test, commit, or merge anything.
5. Trying to quit Duet with an editor still running opens a termination confirmation. Enter stops the editor process group; unsaved buffers are lost, saved files are preserved. Esc cancels so you can return with h and save normally.

1. 在项目代码、已批准任务的审核 diff 或集成草稿中按 **h → Enter** 明确授权目标；**Esc** 取消且不启动程序。
2. 编辑器获得焦点时，`i`、`q`、`:`、Esc、方向/控制键及粘贴都交给 Helix，不触发 Duet 快捷键。
3. **Ctrl+]** 回到 Duet，编辑器仍运行；**h** 回到 Helix。可以浏览侧栏，但禁止并发写入流程。
4. **:wq → Enter** 保存退出；**:q** 按 Helix 自身规则退出。退出不批准、不测试、不提交、不合入。
5. Helix 运行中退出 Duet 会进入终止确认：Enter 停止编辑器进程组，未保存缓冲区丢失，已保存文件保留；Esc 取消，再 h 回编辑器正常保存。

The default h binding is `embedded_editor` in keymap overrides. Ctrl+] is reserved for the child-terminal focus switch. Terminal resizing updates both PTY size and the virtual screen. Normal shutdown and panic unwinding drop and reap the editor child. SIGKILL cannot run host cleanup; the OS closes the PTY, but separately daemonized tools are not guaranteed to exit. PTY is not a container.

默认 h 的可配置动作名为 `embedded_editor`；Ctrl+] 专用于焦点切换。终端缩放同步更新 PTY 与虚拟屏幕。正常退出/panic 展开会回收子进程；SIGKILL 不会运行宿主清理，只能依赖内核关闭 PTY，不能承诺另行 daemonize 的工具退出；PTY 不是容器。

## What gets edited? / 实际编辑哪里？

| Entry / 入口 | Actual target / 实际目标 | After exit / 退出后 |
| --- | --- | --- |
| Project (`3`) | Current main working file, including existing uncommitted content / 主项目实际文件，包括已有未提交内容 | Saved changes remain uncommitted. The project reader still shows fixed HEAD; commit externally and reopen to refresh / 保存后仍未提交，项目阅读仍显示固定 HEAD，人工提交后重开刷新 |
| Review (`2`) | A separate final integration worktree for an implemented, fully approved, unmerged task; not the historical node snapshot on screen / 已全部实现并批准、尚未合入任务的独立最终集成工作区，不是屏幕上的历史节点快照 | Capture a new immutable integration revision, retaining original candidates and node approvals; review the new final diff and explicitly confirm merge / 捕获新固定修复版本，原候选与节点批准保留，新修复仍需最终 diff 与确认合入 |
| Integration draft | The selected managed draft file / 所选受管草稿文件 | Capture/mark resolved only if validation succeeds and markers are removed; otherwise preserve work and report failure / 校验与标记清除成功才捕获/标记处理，否则保留改动并说明失败 |

Every launch backs up the target file in private `.duet/backups/helix-*`. An editor crash or capture failure does not silently roll back saved work or claim approval. Main HEAD/task/approval changes and out-of-scope draft writes are still rejected by integration validation.

每次启动先将目标文件备份到私有 `.duet/backups/helix-*`。崩溃或捕获失败不会偷偷回滚已保存内容，也不会伪称批准；主 HEAD/任务/审批变化及草稿越界修改仍被校验拒绝。

AI execution/advisory and source-mutating operations are blocked while an editor session exists, including while focus is in Duet. Save/exit Helix before starting models, deleting/rolling back tasks, rebuilding drafts, or merging. Browse/search/note-taking remain available.

只要编辑器会话存在（包括焦点回到 Duet），会阻止 AI 执行/顾问与修改源文件的流程；先保存退出，再调用模型、撤回/删除任务、重建草稿或合入。浏览、搜索和笔记仍可使用。

## Limits and safety / 限制与安全

- Unix-only PTY implementation. Keyboard editing is supported; embedded mouse routing, full Kitty keyboard protocol, host/system clipboard OSC passthrough, and arbitrary terminal graphics are not implemented.
- Virtual terminal is capped at 200 rows × 400 columns; polling and input queues are bounded. Child OSC/DCS text is never passed through to the host terminal.
- Target files must be regular UTF-8 files up to 1 MiB, without symlink components. Deleted files, unapproved/pending task delivery edits, merged/historical-node rework, and contract rewriting are not supported by this entry.
- Helix has the account's permissions and may open other files, invoke shell commands, LSP servers, or configured tools. PTY/worktree is **not an OS sandbox**. Launch is explicitly authorized; no model is called to start the editor.

- 目前仅 Unix，支持键盘编辑；尚未实现内嵌鼠标、完整 Kitty 键盘协议、宿主/系统剪贴板 OSC 透传或任意终端图形。
- 虚拟终端最多 200 行 × 400 列，轮询和输入队列有界；子进程 OSC/DCS 不透传到宿主终端。
- 目标须为最多 1 MiB 的 UTF-8 普通文件，不允许符号链接路径。删除文件、未批准交付编辑、已合入/历史节点返工与改写契约不在此入口范围。
- Helix 具有当前账号权限，可打开其他文件、执行 shell、LSP 或配置的工具。PTY/worktree **不是 OS 沙箱**；启动需明确授权，不调用模型。

## Validation / 验证

```bash
cargo test --test embedded
python3 scripts/helix-smoke.py target/release/duet-workbench /path/to/hx
python3 scripts/helix-review-smoke.py target/release/duet-workbench /path/to/hx
```

Tests cover real PTY input/output, terminal cells/styles/Unicode, resize, focus switching, backups and child cleanup. Helix 25.07.1 was tested with real nested PTYs; the review fixture uses fake RPC only for task delivery, then real Helix and explicit merge. No live model calls occur.

测试覆盖真实 PTY、样式/Unicode 单元格、缩放、焦点、备份与子进程清理；真实 Helix 25.07.1 已验证。审核夹具仅以假 RPC 生成交付，随后使用真实 Helix 和明确合入，全程无真实模型调用。
