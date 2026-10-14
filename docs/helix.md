# Embedded Helix / 框内 Helix

[English README](../README.md) · [中文首页](../README.zh-CN.md) · [Review](review-editor.md) · [Project tree](project-tree.md)

**Review 2 and Project 3 use the same real Unix PTY Helix session by default.** Duet now preloads a fresh editor at startup (`helix_startup:true`), without waiting for first file entry; no h confirmation. It opens the selected eligible project file or current immutable-bound review draft, using a private welcome buffer if there is no usable file. Startup retains the view and sidebar focus; file switching reuses this PID. Sidebar navigation opens the corresponding file without resetting other buffers. Tab focuses code, Ctrl+] returns. Restart launches a new editor, never recovers old execution/unsaved editor state. Explicit native review, helix_startup:false, and CLI active coding skip preloading; explicit stop does not trigger auto-start again. Missing hx is an error with a read-only fixed comparison, not a native editor disguised as Helix.

**审核 2 / 项目 3 默认共享真正 Unix PTY Helix**。打开 Duet 默认预载新的编辑器（helix_startup:true），不再等第一次进入文件；加载选中的可编辑项目文件或当前快照绑定审核草稿，无合适文件则用私有欢迎页。保留视图／侧栏焦点，首次文件入口复用 PID；不需 h 确认；侧栏选文件联动打开，保留其他缓冲区。Tab 进入代码，Ctrl+] 返回。重启启动新的编辑器，不复活旧执行／未保存状态；明确 native、helix_startup:false、CLI 活动编码不预载；明确停止后不会又自动启动；hx 缺失时说明错误、只读显示固定代码，不把原生编辑器假称 Helix。

## Setup / 设置

Install hx with its matching runtime; set `helix_path` / `--helix /absolute/path/to/hx` if needed. Default hx tries PATH, then bundled tools relative to the running Duet executable (`helix/hx`, or `../tools/helix-25.07.1-x86_64-linux/hx`). No download or opened-repository tool search. Explicit custom paths/names are not replaced when missing. `editor` / `DUET_EDITOR` still select the separate external conflict editor.

安装匹配 runtime 的 hx；必要时设置 helix_path / --helix。默认 hx 先查 PATH，再查运行中的 Duet 程序旁配套包；不下载、不从打开的仓库搜索工具。明确的自定义路径／名称失效不静默替换。editor / DUET_EDITOR 仍用于独立外部冲突编辑。

Duet creates private per-session configuration under `.duet/helix-workspaces/<nonce>/config`, with duet-dark/duet-light palettes, auto-format disabled and bridge bindings. Host theme changes propagate to Helix. The child's XDG config/cache paths are isolated; global Helix settings are not rewritten or imported. Ordinary Helix bindings remain, except the documented bridge keys; A-p is reserved internally for confirmed patch application. Shell commands typed explicitly by the human continue through /bin/sh; bridge commands are parsed as raw argv and selection text arrives through stdin, never through shell interpolation.

每次会话在私有目录生成配套配置／缓存，主题与 Duet 同步，关闭自动格式化，不改写或导入全局 Helix 配置。除桥接键及内部保留 A-p 外，普通按键沿用 Helix。人工明确输入的 shell 命令仍可调用 /bin/sh；桥接命令以原始 argv 解析，代码经 stdin 传递，不把代码／文件名拼进 shell 求值。

## Rust latency profile / Rust 低延迟配置

`helix_rust_syntax:"fast"` is the default. Session-local empty Rust locals/injections queries retain upstream base syntax highlighting, grammar, indentation, textobjects and LSP, while simplifying local-reference semantic colors and nested macro/format/comment/embedded-language highlighting (nested macro selection can be coarser). No global config/runtime rewrite. Choose `"full"` in duet.json and restart for all upstream processing. Other languages are unchanged. Measured on a copy of src/ui.rs at 210×49: release ~71→10 ms, debug ~97→11 ms local glyph feedback; not a network latency guarantee. See [measurements and reproduction](performance.md).

默认 `helix_rust_syntax:"fast"` 仅在私有配置精简Rust局部变量查询及嵌套语法注入；保留基础语法高亮／原始grammar／缩进／textobjects／LSP，但局部引用语义色彩、宏／format占位符／注释与内嵌语言的专门高亮会简化，宏内嵌套选择可能更粗。全局配置／配套runtime不改，其他语言不变；duet.json设 `"helix_rust_syntax":"full"`，保存各缓冲区后退出重开可恢复完整处理，不会热替换旧编辑器。调试构建现也启用opt-level=1，保留符号／断言，正式使用仍建议release。

## Focus / 焦点

- Tab from sidebar → real Helix; Ctrl+] → workbench, without saving/closing.
- Normal keys, selections, undo, searching, space menus and paste belong to Helix with editor focus. The footer shows editor controls, not approval/start shortcuts.
- Review: Ctrl+G asks using real unsaved context; Ctrl+P previews then Enter applies, Esc cancels; Ctrl+S saves the private file and attempts fixed publication; Ctrl+D opens/closes read-only full diff.
- `:w`, `:wa`, `:wqa` follow Helix rules. No automatic commit/approval/merge.
- Quitting Duet with Helix running requires termination confirmation; stopping can lose uncaptured unsaved editor text. Saved private drafts and project files remain.
- Context capture briefly pauses key dispatch. Errors/timeouts reject the operation instead of using guessed/stale text.

- 侧栏 Tab → 真正 Helix；Ctrl+] → 工作台，不保存、不关闭。
- 编辑焦点下移动、选区、撤销、搜索、space 菜单、粘贴由 Helix 处理；底栏显示编辑器操作，不显示批准／启动键。
- 审核 Ctrl+G 提问，Ctrl+P 预览再 Enter 应用／Esc 取消，Ctrl+S 保存私有文件并尝试固定发布，Ctrl+D 打开／返回只读完整 diff。
- :w / :wa / :wqa 遵循 Helix；不自动提交／批准／合入。
- 退出工作台需终止确认；停止可能丢失未捕获的未保存文本，已保存草稿／项目文件保留。
- 读取上下文时短暂暂停按键处理；错误／超时拒绝操作，不猜或使用过期代码。

## Persistent code pane / 常驻代码区

File switching uses an internal F12 binding and an owned open.json request, not typed Esc/:open/Enter input. The helper emits only a manifest-bound filename as a literal command expansion; code/filenames are not evaluated by a shell. Cached project files are backed up on first host opening, not on every revisit. First-time Git worktree/file loading can still take time.

切文件通过内部 F12 与受管请求一次执行，不逐字输入命令；helper 只返回 manifest 绑定的字面文件名，不求值代码／文件名。项目文件首次宿主打开时备份，重复访问不重复拷贝；首次工作区／文件 IO 仍有开销。

**Space q** saves the current file and returns focus to the sidebar without quitting Helix or publishing review. Other unsaved buffers and undo history remain; Ctrl+] returns without saving. These are the recommended save-return/focus-return controls. Native `:wq` / `:q` retain upstream semantics: the process exits; default `helix_keepalive:true` then reopens the last managed file with saved drafts and a real Helix pane. The prior screen stays during startup, rather than a native-reader flash. This is process reconstruction, **not** a pretend interception of :wq; closed-process undo/unsaved state is not recovered. Abnormal exit and explicit stop do not auto-retry; restart of Duet follows helix_startup to preload a new editor, not the old one. Conflict/final-integration editors keep their original exit/capture lifecycle.

**Space q** 保存当前文件并回侧栏，不退出／不发布审核；其他未保存缓冲区、撤销栈保留。Ctrl+] 不保存只返回，推荐使用这两个操作。原生 :wq / :q 仍退出进程；默认 keepalive 随后根据最后受管文件及已保存草稿重建真实 Helix，加载期间保留旧画面而不闪回原生阅读器。这是**进程重建，不是假称拦截 wq**，不恢复旧进程撤销／未保存状态。异常退出／明确停止不重试，重启按 helix_startup 预载新编辑器，不复活旧编辑器；冲突／最终集成编辑沿用原退出捕获规则。

Before implementation/deletion/merge, leave focus and use host `:editor-stop → Enter` for explicit termination, or choose `helix_keepalive:false` if ordinary :wq should release the editor interlock. Stopping warns about unsaved loss. A-p and F12 are reserved internal bindings; Space q overrides the managed space-menu q entry.

实现／删除／合入前，回工作台 `:editor-stop → Enter` 明确停止，或配置 keepalive:false 使 wq 直接释放编辑器互斥。停止提示未保存丢失；A-p/F12 为内部保留键，Space q 为保存返回。

## Targets and immutable review / 目标与不可变审核

Project edits current working files after backup; no commit. Review uses independent worktrees under `.duet/helix-workspaces/<nonce>/reviews/<index>`, based on the node baseline with only the bound file populated from fixed delivery/local draft. Git gutter compares to that baseline; Ctrl+D includes deleted comparison rows. Original delivery, main files/index/HEAD, original candidates and approvals are untouched by editing. Existing pending/historical nodes can be edited locally; publication keeps its [final-node-only boundary](review-editor.md#persistence-and-publication--持久化与发布).

项目编辑备份后的实际工作文件，不提交。审核使用独立 worktree，在节点基线之上填入该固定文件／本地草稿；Git 行侧标记基线差异，Ctrl+D 包含删除行。编辑不改原交付、主文件／index／HEAD、候选及批准。待审／历史节点可本地编辑，发布仍限最终节点。

h-confirmed final-integration/conflict editing remains separate: it requires the original approved-task/draft validation and captures a new fixed integration revision on exit. It is not the default Review backend.

h 确认的最终集成／冲突编辑保留原有批准／草稿校验，退出捕获新固定集成修订；不是默认审核后端。

## Safety and limits / 安全与边界

Unix PTY, keyboard routing, UTF-8 regular files ≤1 MiB, ≤64 review buffers/24 MiB total text, ≤128 session file bindings, ≤256 KiB AI excerpt. Bridge supports a single selection; multi-selection is rejected, not silently narrowed. Actual path is checked against host bindings, cursor and flipped-anchor positions against captured full text. Internal picker switches synchronize on explicit capture/return; arbitrary unbound buffers cannot be sent to AI. Unobserved other review buffers conservatively block approval until captured or the editor exits. Save/exit before implementation, deletion/rollback, advisor, or merge; read-only contextual chat is the deliberate exception.

Unix、键盘、普通 UTF-8 文件 ≤1 MiB；最多64审核缓冲区／24 MiB正文、128会话文件绑定、AI引用 ≤256 KiB。桥接支持单选区，多选拒绝而非偷偷缩成主选区。校验真实路径绑定、光标／反转锚点与实际全文。Helix 内选文件在明确捕获／返回时同步；任意未绑定缓冲区不能发给 AI。未观察到的其他审核缓冲区保守阻止批准，直到捕获或退出。实现、删除／撤回、顾问、合入前保存退出；只读就地聊天是明确的例外。

PTY/worktree is **not an OS sandbox**: human Helix commands/LSP may access other files with account permissions. No model is launched merely by opening an editor. Terminal OSC/DCS is not forwarded, virtual screen ≤200×400 cells. Mouse, full Kitty keyboard/system clipboard protocols and arbitrary terminal graphics remain unsupported. Normal exit/Drop reaps process groups; SIGKILL cannot run host cleanup. Private worktrees/configuration are retained, not automatically cleaned up.

PTY/worktree **不是 OS 沙箱**。Helix 命令／LSP 有账号权限；打开编辑器不调用模型。OSC/DCS 不透传，虚拟屏幕 ≤200×400；鼠标、完整 Kitty／系统剪贴板协议、任意图形未实现。正常退出／Drop 回收进程组，SIGKILL 无法运行宿主清理。私有工作区／配置保留，尚无自动清理。

## Validation / 验证

`tests/helix_workspace.rs` checks exact Unicode/repeated-word ranges, injection resistance, stale/unbound/multi-selection rejection, independent source/index/HEAD, uncertainty and real Helix palette cells. `scripts/helix-unified-smoke.py` uses real Helix 25.07.1 and fake RPC for unsaved selection discussion, explicit patch cancellation/application, fixed revision, same PID across 2/3, retained candidate, independent approval/merge and restart. `scripts/project-tree-smoke.py` verifies automatic selection switching, dark/light theme and two unsaved buffers. Legacy native review regression explicitly uses --review-editor native. No live models are called.
