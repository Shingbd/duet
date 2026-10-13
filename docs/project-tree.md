# Project tree and seamless Helix / 项目文件树与 Helix

Project (`3`) now has a hierarchical file tree, separate from Review's task/node changed-file list. **Pressing 3 explicitly opens the current project file in embedded Helix without h or an extra editor confirmation.** Opening/restarting Duet only restores navigation preferences; it never revives or automatically starts an editor/model.

`3` 项目视图使用目录文件树；`2` 审核仍按任务/节点显示变更文件。**主动按 3 会直接在代码框中打开项目文件的 Helix，无需 h 或额外启动确认。** 打开/重启 Duet 只恢复导航偏好，不自动启动或恢复编辑器/模型。

Install `hx` with its runtime or configure `helix_path` / `--helix /path/to/hx`. If missing, Duet shows an actionable error and retains the tree. Helix is a real Unix PTY process, not the native review editor. It can invoke shell/LSP/configured tools with account permissions: this is **not an OS sandbox**. The user's project-entry/open-file action authorizes this editor launch; it does not authorize any AI call, commit, merge or deployment.

安装包含 runtime 的 `hx`，或配置 `helix_path` / `--helix /path/to/hx`；缺失时说明错误，文件树仍保留。这里是真正 Unix PTY Helix，不是原生审核编辑器。Helix 可以账号权限调用 shell/LSP/配置工具，**不是 OS 沙箱**；用户按项目入口/打开文件即授权编辑器启动，不授权 AI、提交、合入或部署。

## Navigation / 操作

- Sidebar: `j/k` or arrows select rows; `h/l` or left/right collapse/expand; Enter toggles a directory or opens a file.
- `Tab` focuses the editor; `Ctrl+]` returns to the tree. Editor-focused keys go to Helix, not Duet. Outside Project, a project editor does not obscure task/review content.
- `r` refreshes the tree; `z` toggles ignored files. Directories sort before files. `M/A/D/?/!` mark modified, added, deleted, untracked, ignored files.
- Refresh/restart retain selected path and expanded directories; missing paths fall back safely. Restart does not restore Helix's unsaved buffers.
- `.git` and `.duet` stay hidden, including with ignored files shown. Symlinks are not followed/opened. Deleted Git entries stay visible but cannot be opened as ordinary files.

- 侧栏 `j/k` 或方向键选择；`h/l` 或左右键折叠/展开；Enter 切换目录或打开文件。
- `Tab` 进入编辑器；`Ctrl+]` 回文件树。编辑器焦点下按键交给 Helix，不触发 Duet。切到其他视图后，项目编辑器不会盖住任务/审核正文。
- `r` 刷新，`z` 切换被忽略文件；目录优先。`M/A/D/?/!` 对应修改、新增、删除、未跟踪、被忽略。
- 刷新/重启保留选中路径和展开状态；路径消失安全回退。重启不恢复 Helix 未保存缓冲区。
- `.git` 和 `.duet` 始终隐藏，包括显示忽略项时；不跟随/打开符号链接。Git 删除项仍显示，但不能作为现存文件打开。

Opening another file sends a bounded, quoted `:open` command to the **same Helix PTY**, without restarting or saving. Unsaved buffers remain managed by Helix. Each target is backed up before opening. The tree selection is not an authoritative report of the editor's current buffer if the user switches buffers inside Helix; the Helix status line is authoritative. No cursor/selection AI bridge is added to Helix by this feature.

打开另一文件通过有界、引用的 `:open` 命令交给**同一个 Helix PTY**，不重启、不保存；未保存缓冲区由 Helix 保留管理。每个目标打开前备份。用户在 Helix 内自行切换缓冲区时，树选中项不代表实际编辑器缓冲区，须以 Helix 状态行判断。此功能未给 Helix 增加光标/选区 AI 桥接。

Use Helix `:w`, `:wa`, `:wqa` or its ordinary buffer commands explicitly. `:wq` only follows Helix's own save/quit rules; Duet never silently saves other buffers. Quitting Duet with an editor running still requires termination confirmation and warns about losing unsaved buffers. Saved files remain uncommitted. Source-mutating AI/integration workflows are interlocked with the editor session.

使用 Helix 自身 `:w`、`:wa`、`:wqa` 或缓冲区命令明确保存；`:wq` 遵循其自身规则，Duet 不偷偷保存其他缓冲区。编辑器运行时退出工作台仍需终止确认，提示未保存缓冲区丢失；已保存文件仍未提交。编辑器与 AI/集成写入流程互斥。

## Limits / 边界

- Git tree includes tracked/deleted/untracked files and optionally ignored files, capped at 5000 entries; it is not limited to the old 80-file snapshot reader. Non-Git folders use bounded filesystem traversal (depth 32).
- Unix PTY; ordinary UTF-8 files up to 1 MiB. No mouse tree or editor bridge.
- Filename spaces are supported. Percent, quotes or backslashes are rejected by host-driven file switching to prevent Helix command interpolation; use Helix's own picker manually for these names. Original buffers are preserved on rejection.
- Refresh is explicit, not a background filesystem watcher. For non-Git folders, no Git ignore/status semantics are promised.

- Git 文件树包含跟踪、删除、未跟踪及可选忽略文件，最多 5000 项，不再受旧 80 文件快照阅读限制。非 Git 文件夹使用有界遍历，深度 32。
- Unix PTY，最多 1 MiB 的 UTF-8 普通文件；无鼠标树操作或编辑器桥接。
- 支持空格文件名。宿主切换含百分号、引号、反斜杠的名称时拒绝命令插值，原缓冲区保留；可在 Helix 自身文件选择器手动打开。
- 刷新为明确操作，不是后台监视器；非 Git 文件夹不承诺 Git 忽略/状态语义。

Validation: `tests/file_tree.rs`, `scripts/project-tree-smoke.py` (real Helix 25.07.1, nested/spaced paths, stable PTY PID, two unsaved buffers, explicit save-all, backups, untouched HEAD), plus existing review/Helix/PTY regression suites. No live models are called.

验证包括上述单元与真实 Helix 25.07.1 PTY：嵌套/空格路径、同 PID、两个未保存缓冲区、明确全部保存、备份和 HEAD 不变，以及原审核/Helix/PTY 回归；没有真实模型调用。
