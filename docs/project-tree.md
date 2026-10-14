# Project tree and seamless Helix / 项目文件树与 Helix

Project (`3`) now has a hierarchical file tree, separate from Review's task/node changed-file list. **Pressing 3 explicitly opens the current project file in embedded Helix without h or an extra editor confirmation.** Opening/restarting Duet only restores navigation preferences; it never revives or automatically starts an editor/model.

`3` 项目视图使用目录文件树；`2` 审核仍按任务/节点显示变更文件。**主动按 3 会直接在代码框中打开项目文件的 Helix，无需 h 或额外启动确认。** 打开/重启 Duet 只恢复导航偏好，不自动启动或恢复编辑器/模型。

Default `hx` resolution prefers PATH, then checks only bundled tools relative to the running Duet executable: `helix/hx` beside it or `../tools/helix-25.07.1-x86_64-linux/hx` (the local debug/release development bundle). No downloads or opened-project tool discovery occur. Explicit custom paths/names are not silently replaced. Otherwise install `hx` with its runtime or configure `helix_path` / `--helix /path/to/hx`.

默认 `hx` 先查 PATH，再仅查运行中的 Duet 程序旁的配套工具：同目录 `helix/hx`，或 `../tools/helix-25.07.1-x86_64-linux/hx`（本机 debug/release 开发包）。不下载、不从打开的项目搜工具；明确自定义路径/名称不会静默替换。 If missing, Duet shows an actionable error and retains the tree. Helix is a real Unix PTY process, not the native review editor. It can invoke shell/LSP/configured tools with account permissions: this is **not an OS sandbox**. The user's project-entry/open-file action authorizes this editor launch; it does not authorize any AI call, commit, merge or deployment.

安装包含 runtime 的 `hx`，或配置 `helix_path` / `--helix /path/to/hx`；缺失时说明错误，文件树仍保留。这里是真正 Unix PTY Helix，不是原生审核编辑器。Helix 可以账号权限调用 shell/LSP/配置工具，**不是 OS 沙箱**；用户按项目入口/打开文件即授权编辑器启动，不授权 AI、提交、合入或部署。

## Navigation / 操作

- Sidebar: `j/k` or arrows select rows; `h/l` or left/right collapse/expand; selecting a file immediately opens it without Enter; Enter toggles a directory or explicitly reopens a file.
- `Tab` focuses the editor; `Ctrl+]` returns to the tree. Editor-focused keys go to Helix, not Duet. Outside Project, a project editor does not obscure task/review content.
- `r` refreshes the tree; `z` toggles ignored files. Directories sort before files. `M/A/D/?/!` mark modified, added, deleted, untracked, ignored files.
- Refresh/restart retain selected path and expanded directories; missing paths fall back safely. Restart does not restore Helix's unsaved buffers.
- `.git` and `.duet` stay hidden, including with ignored files shown. Symlinks are not followed/opened. Deleted Git entries stay visible but cannot be opened as ordinary files.

- 侧栏 `j/k` 或方向键选择；`h/l` 或左右键折叠/展开；选中文件即联动打开，无需 Enter；Enter 切换目录或明确重开文件。
- `Tab` 进入编辑器；`Ctrl+]` 回文件树。编辑器焦点下按键交给 Helix，不触发 Duet。切到其他视图后，项目编辑器不会盖住任务/审核正文。
- `r` 刷新，`z` 切换被忽略文件；目录优先。`M/A/D/?/!` 对应修改、新增、删除、未跟踪、被忽略。
- 刷新/重启保留选中路径和展开状态；路径消失安全回退。重启不恢复 Helix 未保存缓冲区。
- `.git` 和 `.duet` 始终隐藏，包括显示忽略项时；不跟随/打开符号链接。Git 删除项仍显示，但不能作为现存文件打开。

Opening another file sends a bounded, quoted `:open` command to the **same Helix PTY**, without restarting or saving. Unsaved buffers remain managed by Helix. Each target is backed up before opening. Review and Project now share this real session and Duet-managed theme. If the user switches buffers inside Helix, Ctrl+] uses the explicit metadata/stdin bridge to validate and synchronize the managed file back to the tree; there is no continuous picker watcher. Contextual AI is enabled only for managed Review buffers, not Project files.

打开另一文件通过有界、引用的 `:open` 命令交给**同一个 Helix PTY**，不重启、不保存；未保存缓冲区由 Helix 保留管理。每个目标打开前备份。审核与项目共享真实会话及 Duet 配色。Helix 内切换文件后，Ctrl+] 通过明确的元数据／stdin 桥校验实际文件并同步树；不是连续文件选择器监视。上下文 AI 仅开放受管审核缓冲区，未开放项目文件。

Use Helix `:w`, `:wa`, `:wqa` or its ordinary buffer commands explicitly. `:wq` only follows Helix's own save/quit rules; Duet never silently saves other buffers. Quitting Duet with an editor running still requires termination confirmation and warns about losing unsaved buffers. Saved files remain uncommitted. Source-mutating AI/integration workflows are interlocked with the editor session.

使用 Helix 自身 `:w`、`:wa`、`:wqa` 或缓冲区命令明确保存；`:wq` 遵循其自身规则，Duet 不偷偷保存其他缓冲区。编辑器运行时退出工作台仍需终止确认，提示未保存缓冲区丢失；已保存文件仍未提交。编辑器与 AI/集成写入流程互斥。

## Limits / 边界

- Git tree includes tracked/deleted/untracked files and optionally ignored files, capped at 5000 entries; it is not limited to the old 80-file snapshot reader. Non-Git folders use bounded filesystem traversal (depth 32).
- Unix PTY; ordinary UTF-8 files up to 1 MiB. No mouse tree. Review command/stdin bridge is described separately in [Helix](helix.md).
- Filename spaces are supported. Literal single/backtick quoting prevents percent/shell interpolation, including filenames containing spaces, percent or shell metacharacters. The internal F12/open.json bridge now supports quote/backtick names too; unsafe relative paths/control-character filenames remain rejected. Original buffers are preserved on rejection.
- Refresh is explicit, not a background filesystem watcher. For non-Git folders, no Git ignore/status semantics are promised.

- Git 文件树包含跟踪、删除、未跟踪及可选忽略文件，最多 5000 项，不再受旧 80 文件快照阅读限制。非 Git 文件夹使用有界遍历，深度 32。
- Unix PTY，最多 1 MiB 的 UTF-8 普通文件；无鼠标树操作；审核命令／stdin 桥另见 [Helix](helix.md)。
- 支持空格文件名。宿主使用单引号／反引号字面引用，支持百分号与 shell 特殊字符而不插值。现在 F12/open.json 内部桥也支持引号／反引号名字；不安全相对路径／控制字符文件名仍拒绝，原缓冲区保留；可在 Helix 自身文件选择器手动打开。
- 刷新为明确操作，不是后台监视器；非 Git 文件夹不承诺 Git 忽略/状态语义。

Validation: `tests/file_tree.rs`, `scripts/project-tree-smoke.py` (real Helix 25.07.1, nested/spaced paths, stable PTY PID, two unsaved buffers, explicit save-all, backups, untouched HEAD), plus existing review/Helix/PTY regression suites. No live models are called.

验证包括上述单元与真实 Helix 25.07.1 PTY：嵌套/空格路径、同 PID、两个未保存缓冲区、明确全部保存、备份和 HEAD 不变，以及原审核/Helix/PTY 回归；没有真实模型调用。
