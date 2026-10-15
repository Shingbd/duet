# Terminal bracket keys / 终端括号快捷键

In real embedded Helix (not the explicit legacy native review backend):

| Action / 操作 | Primary / 首选 | Fallback / 替代 |
| --- | --- | --- |
| Previous / next file 上一／下一文件 | Ctrl+[ / Ctrl+] | Ctrl+PageUp / Ctrl+PageDown |
| Previous / next review node 上一／下一审核节点 | Ctrl+Shift+[ / Ctrl+Shift+] | Ctrl+Shift+PageUp / Ctrl+Shift+PageDown |
| Sidebar / exit fullscreen 侧栏／退出全屏 | F6 | Space q explicitly saves current file and returns |

File and node navigation wrap at both ends and retain editor focus/fullscreen/the same Helix process, unsaved buffers and undo. Switching from insert mode finishes insertion without saving. Node navigation stays in the current task, selects the target node's first changed file, and skips deliveries without a loaded immutable snapshot containing nondeleted editable files. It does not run planned nodes, change tasks, publish, approve, test or merge. Single-node/file lists display an explicit status instead of silently doing nothing. Project and conflict editors do not perform review-node navigation.

文件和节点首尾循环，保留焦点／全屏／同一进程／其他未保存缓冲区与撤销；插入模式切换后为正常模式，不自动保存。节点仅在当前任务已有可编辑文件的固定交付快照间切换，选目标节点第一个变更文件，不执行未实施节点，不跨任务，不发布／批准／测试／合入。

## Why Ctrl+[ can fail / 为什么左括号没反应

Traditional terminals encode **Ctrl+[ as the identical byte `0x1b` as Esc**. Some also encode Ctrl+Shift+] identically to Ctrl+] and Ctrl+Shift+[ identically to Esc. The host cannot reconstruct missing modifiers. `TERM=xterm-256color` identifies neither the actual client nor extended-key support. SSH alone cannot fix this.

Duet requests keyboard escape disambiguation (Kitty/CSI-u). It accepts shifted bracket and shifted brace events, and ignores release events. But a client/multiplexer must actually deliver the distinct event; this is not guaranteed by an application setting. **Raw Esc remains Helix Esc**, never previous-file/node. Ordinary unmodified `[` / `]` remain native Helix keys.

传统终端已丢失按键差异，应用端不能“猜回”Ctrl／Shift。不能为了左括号偷走Esc，否则会破坏正常退出插入模式。若左括号无效／Shift仍在切文件，请使用表中的替代键，或在本地终端按下表明确映射；不修改服务器全局键盘配置。

| Local key / 本地键 | Bytes to send / 发送序列 |
| --- | --- |
| Ctrl+[ | `ESC [ 91 ; 5 u` |
| Ctrl+] | `ESC [ 93 ; 5 u` |
| Ctrl+Shift+[ | `ESC [ 91 ; 6 u` |
| Ctrl+Shift+] | `ESC [ 93 ; 6 u` |

For VS Code integrated terminal, optionally add these entries to the **client's** `keybindings.json` (not `duet.json`; not done automatically). Other terminal clients have their own explicit send-text/key-map mechanism. A browser/client may intercept Ctrl+PageUp or other shortcuts too; it must be configured to forward the chosen keys.

```json
[
  {"key":"ctrl+[","command":"workbench.action.terminal.sendSequence","args":{"text":"\u001b[91;5u"},"when":"terminalFocus && !terminalFindInputFocused"},
  {"key":"ctrl+]","command":"workbench.action.terminal.sendSequence","args":{"text":"\u001b[93;5u"},"when":"terminalFocus && !terminalFindInputFocused"},
  {"key":"ctrl+shift+[","command":"workbench.action.terminal.sendSequence","args":{"text":"\u001b[91;6u"},"when":"terminalFocus && !terminalFindInputFocused"},
  {"key":"ctrl+shift+]","command":"workbench.action.terminal.sendSequence","args":{"text":"\u001b[93;6u"},"when":"terminalFocus && !terminalFindInputFocused"}
]
```

Rebuilds do not hot-update a running host/editor. Save **all** buffers (`:wa`) and explicitly stop/exit the old session before restarting. No old unsaved state/undo is resurrected on startup.
