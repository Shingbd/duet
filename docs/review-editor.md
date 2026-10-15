# Editable review and contextual AI / 可编辑审核与就地 AI 讨论

## Default: real Helix / 默认：真正 Helix

**2** explicitly opens real Helix in an independent draft; **3** shares that session for project files. Sidebar selection follows the file automatically. **Tab** enters Helix, **F6** captures the current unsaved buffer and returns to workbench focus. Ordinary editor keys never dispatch workbench actions, and failed initialization stays read-only. Duet manages private dark/light Helix themes without rewriting global configuration.

**2** 主动打开独立审核草稿中的真正 Helix，**3** 共用会话；侧栏选文件自动联动。**Tab** 进入，**F6** 捕获未保存缓冲区并回工作台；编辑器按键不触发外部动作，初始化失败保持只读。使用私有深／浅配色，不改全局配置。

- **Ctrl+G** captures path, cursor, flipped selection anchor, selected stdin and full unsaved stdin through Helix commands. Single-character normal selection means cursor word/current line; explicit Select mode or a longer selection means that exact range. Multi-selection/unbound paths/inconsistent position text are rejected. Only the excerpt and internal metadata reach Pi after question Enter; no range popup. Whole text remains local for stale protection.
- **Ctrl+P → Enter** captures again before application, verifies full-buffer identity/version/revision/range, then the helper independently compares actual pipe input before replacing. Esc cancels; application stays unsaved in Helix and is undoable. No native-buffer write masquerades as a Helix edit.
- **Ctrl+S** explicitly writes the current private file, then captures and attempts fixed publication with the original final-node-only/scope boundary below. Other drafts remain local. New review/verification do not inherit old approval.
- **Ctrl+D** reads current text and opens the full read-only baseline/delivery comparison, including red deletions; j/k scroll and Ctrl+D returns to Helix. Helix itself also has the Git gutter. No ghost deletion row is editable source text.
- Closing/saving Helix does not publish/approve/merge. Read-only code chat may coexist with the workspace editor; implementation/advisor/source integration still requires exit. Saved drafts remain in private worktrees; captured drafts/history persist in SQLite. Restart never starts Helix/model or restores pending suggestions.

上述流程读取实际 Helix 状态而非屏幕。Ctrl+G 取得原始 argv 定位与 stdin 文本；普通单字符光标取词／行，Select 模式或较长选区按原范围；多选／未绑定路径／不一致文本拒绝。全文只在本地校验过期，Enter 才将摘录发送 Pi。Ctrl+P 先预览，Enter 后再次捕获和校验，helper 还会核对实际 pipe 输入才替换，Esc 不改；建议仍是 Helix 未保存、可撤销的编辑。Ctrl+S 明确保存私有文件，再按下方最终节点边界发布；Ctrl+D 为含红色删除的只读完整对照，不把删除行当源代码编辑。原候选／主文件／index／HEAD／旧批准不改，退出也不自动发布／批准／合入。

Full setup, focus, shell safety and limits: [embedded Helix](helix.md). Project contextual AI is not yet enabled; internal-picker arbitrary files are not implicitly authorized for code disclosure. Global Helix configuration is not imported. Other unobserved review buffers conservatively block approval until captured or the editor exits. Private worktrees are retained; no automatic cleanup. The native-specific description below is a legacy option, not a fallback on hx failure.

## Legacy native backend / 旧原生后端

The remaining controls below describe explicitly configured `review_editor: "native"` / `--review-editor native`, **not the default Helix backend**. Native review (`2`) displays a **native editor-style view**: full file contents, old/new line numbers, lexical code coloring, additions in green and deletions in red. No `h`, Helix install, editor subprocess, or model request is needed to display it.

审核（`2`）默认显示**原生编辑器式视图**：完整文件、旧/新行号、基础词法高亮、绿色新增和红色删除，无需按 `h`、安装 Helix、启动编辑器子进程或调用模型。

This is a host-owned review buffer, not a Helix screen bridge. Owning the actual buffer lets Duet obtain exact cursor/selection context—including unsaved edits—rather than guessing from terminal cells. The optional embedded Helix entry remains separate; its PTY does not provide this contextual-chat bridge.

默认视图由 Duet 持有真实缓冲区，并不是 Helix 的屏幕桥接；因此能准确获取光标/选区和未保存代码，不从终端像素猜测。原有可选内嵌 Helix 仍保留，但其 PTY 不提供此上下文聊天桥接。

## Focus and editing / 焦点与编辑

Entry focus stays in the sidebar so approval/task shortcuts cannot unexpectedly become typing. **Tab** focuses code. With code focus, normal characters belong to the editor—not Duet's approval, task-start or quit shortcuts.

进入时焦点留在侧栏，防止批准/任务快捷键突然变成输入；**Tab** 进入代码区。代码焦点下普通字符交给编辑器，不触发 Duet 的批准、启动、退出。

| Keys / 按键 | Action / 操作 |
| --- | --- |
| `h/j/k/l`, arrows / 方向键 | Move cursor / 移动光标 |
| `i`, `a`, `o`, Esc | Insert, append, new line, return to NORMAL / 插入、追加、新行、回普通模式 |
| `v` + movement, Shift+arrows, Ctrl+A | Select code / 选区、选择全部 |
| `x`, `d`, Delete, Backspace in INSERT | Delete selection/character / 删除选区或字符 |
| Ctrl+Z | Undo local edits (bounded, not restored across restart) / 撤销本地修改（有界，重启不恢复撤销栈） |
| `g`, `G`, Home/End, PageUp/PageDown | File/line/page navigation / 文件、行、页导航 |
| `[` / `]` | Previous/next changed current-code row / 上一处/下一处当前代码修改 |
| `/text` → Enter | Search / 搜索 |
| `:42` → Enter | Jump to line / 跳行 |
| Ctrl+D | Compare draft to node baseline or fixed delivery / 切换草稿相对节点基线／固定交付的差异 |
| Ctrl+G, `:ask` → Enter | Open contextual question input / 打开就地提问输入 |
| Ctrl+P, `:apply` → Enter | Preview an AI suggestion; Enter applies, Esc cancels / 查看建议补丁，再 Enter 应用或 Esc 取消 |
| Ctrl+S, `:w` → Enter | Persist draft and try publishing a new fixed revision / 保存草稿并尝试发布新固定修订 |
| F6 | Return to sidebar/workbench / 回侧栏、工作台 |
| Ctrl+X | Stop active execution/discussion / 停止活动执行或讨论 |

Clipboard paste edits the selected/current buffer; it never silently overwrites the approved snapshot. The gutter's `*` identifies unpublished human modifications. Deleted lines are read-only comparison rows, not text in the current editable buffer. Emptying code produces an empty file, not filesystem deletion.

粘贴写入当前缓冲区/选区，不覆盖批准快照；行侧 `*` 标识尚未发布的人工修改。红色删除行属于只读对照，不是当前缓冲区的可编辑文字。清空代码得到空文件，不等于删除文件。

## Direct contextual discussion / 直接发送上下文

1. Place the cursor or select code; use **Ctrl+G** or **:ask**.
2. Type a question and press **Enter**. This directly authorizes the model call—**no range-confirmation dialog**.
3. With a selection, send exactly that selection. Without a selection, send the Unicode word at the cursor, or the current line if there is no word. The reference is frozen when the question input opens and includes unsaved buffer changes.
4. Path, line, byte offsets, snapshot and draft revision travel as internal location metadata. The rest of the file/buffer is not attached. Up to eight same-file/snapshot discussion messages are included, capped at 128 KiB.
5. Replies appear alongside the code. **Esc** in chat focus returns to code; `i` in chat focus continues that reference, while Ctrl+G captures a fresh cursor/selection. Home/End and j/k in chat focus browse replies.

1. 定位光标或选择代码，按 **Ctrl+G** 或 **:ask**。
2. 输入问题并 **Enter**，即明确发送模型请求，**没有范围确认弹窗**。
3. 有选区只发选区；无选区发光标词，不能取词时发当前行。打开提问输入时冻结引用，包括当时未保存的编辑。
4. 文件路径、行、字节位置、快照和草稿版本作为内部定位信息附带；不附带整个文件/缓冲区。同文件、同快照最多携带八条历史讨论，总量最多 128 KiB。
5. 回复出现在代码旁；聊天焦点下 **Esc** 回代码，`i` 继续当前引用，Ctrl+G 重新取得光标/选区。Home/End 与 j/k 可浏览回复。

This runtime uses Pi RPC with **all tools disabled**, and repository context, extensions, MCP, skills and templates disabled. It cannot edit files, run tests, approve or merge. It has the configured model/budget, explicit cancellation, no automatic retry or restart recovery. No model is started by opening review or opening/cancelling the question input.

此讨论通过 Pi RPC 执行，**禁用全部工具**，也禁用仓库上下文、扩展、MCP、skills 和模板。不能写文件、运行测试、批准或合入。使用配置的模型/预算，允许明确停止，不自动重试或恢复运行；打开审核或打开/取消提问不调用模型。

AI may return a local suggestion. Only **Ctrl+P → Enter** applies it to the human draft. Application is tied to the exact referenced buffer/version/revision and byte range. If code changes while discussing, application fails rather than overwriting it. Repeated words outside the selection are not patch targets. No automatic publish, approval or merge follows.

AI 可返回局部建议，但只有 **Ctrl+P → Enter** 才写入人工草稿。应用绑定引用缓冲区、版本、修订和字节位置；讨论期间改过代码则拒绝应用，不覆盖后来的修改。引用之外的同名词不会被替换。应用后仍不自动发布、批准、合入。

## Persistence and publication / 持久化与发布

Drafts and recent visible discussions are saved in private local SQLite state. Restart restores drafts and history, **not** model processes, queued prompts, pending proposals, insertion mode or undo stacks. Fixed Git objects remain the source of the original delivery. Unsaved drafts block approval/merge of the displayed old version.

草稿和最近显示的讨论保存在私有本地 SQLite；重启恢复内容和历史，**不恢复**模型进程、排队问题、建议待应用、插入模式或撤销栈。原交付仍来自固定 Git 对象；草稿未发布时阻止把屏幕背后的旧版本批准/合入。

**Current publication boundary:** publishing is supported for the current final-delivery node of a fully implemented, unmerged task, only within that node's original scope. Editing/discussing earlier/live/historical nodes is local draft work; publishing those nodes and rebuilding their downstream deliveries is not implemented. Ctrl+S saves such a draft but reports the publication restriction; it does not silently fold it into another node or change contracts.

**当前发布边界：**支持全部实现结束、尚未合入任务的当前最终交付节点，并限该节点原契约范围。更早／运行中／历史节点可以保留本地草稿和讨论，但其发布以及重建下游交付尚未实现。Ctrl+S 会保存这类草稿并说明限制，不偷偷归入其他节点或改契约。

Publishing uses a new owned `.duet/review-revisions/<nonce>/worktree`, captures a fixed revision, and records a new manual attempt. The main files/index/HEAD, original implementation candidate, old snapshots and old attempt approvals are retained. New review and verification are reset; an empty node diff is NoChanges and cannot be approved. After publication, return to ordinary **a → Enter** approval and final **g** integration/risk confirmations. No tests, commits to main, pushes or merge run automatically. Revision worktrees are retained; automatic cleanup is not implemented.

发布使用独立 `.duet/review-revisions/<nonce>/worktree`，捕获新固定版本并记录人工尝试；主文件/index/HEAD、原实现候选、旧快照及旧尝试批准保留，新审核和验证重置。节点差异变空则记 NoChanges，不可批准。发布后仍需 **a → Enter** 批准，再 **g** 最终集成与风险确认。不自动测试、提交主项目、推送或合入。修订工作区保留，尚无自动清理。

## First-version limits / 首版限制

- Lexical coloring for strings/comments/numbers/common language keywords; not full language-server/tree-sitter/Helix semantics.
- Keyboard-oriented editing; no mouse selection, completion, LSP, multi-cursor editing or embedded system clipboard protocol.
- 1 MiB per UTF-8 file, up to 64 buffers and 24 MiB buffer text total, 256 KiB per AI excerpt. Oversized references are rejected rather than silently truncated.
- Deleted comparison rows are not selectable/editable current text. Whole-file creation/deletion and arbitrary earlier-node rework are not implemented by this entry.
- Draft persistence is debounced, not a transactional journal for every keystroke. Catastrophic termination can lose the most recent unsaved batch.
- Local data and authorized code discussions are plaintext and may be sent to the configured provider. No OS sandbox or credential filtering is implied.

- 基础词法高亮，不是完整语法树/LSP/Helix 语义高亮。
- 以键盘为主，无鼠标选区、补全、LSP、多光标或内嵌系统剪贴板协议。
- 每个 UTF-8 文件最多 1 MiB，最多 64 个缓冲区、总正文 24 MiB，每次 AI 引用最多 256 KiB。过大的引用直接拒绝，不静默截断。
- 删除对照行不是可选择／可编辑的当前文本；此入口未支持整文件创建/删除及任意前序节点返工。
- 持久化批量防抖，不是逐键事务日志；强制终止可能丢失最近尚未保存批次。
- 本地数据与明确发送的代码讨论是明文，可能传给配置的模型提供方；不宣称 OS 沙箱或凭据过滤。

## Validation / 验证

`tests/review_editor.rs`, native publication/rendering cases in `tests/merge.rs`, and `scripts/review-editor-smoke.py` cover exact unsaved Unicode context, no range confirmation, isolated focus, additions/deletions, explicit patch cancellation/application, stale/repeated-word safety, new fixed approval, preserved original worktree/main/index, and restart. Tests use fake RPC and real Git/PTYs, never live model calls.

上述测试覆盖未保存 Unicode 上下文、无范围确认、焦点隔离、增删显示、补丁取消/明确应用、过期/同名词安全、新固定批准、原候选/主项目/index 保留与重启；使用假 RPC、真实 Git/PTY，没有真实模型调用。
