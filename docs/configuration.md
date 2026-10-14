# Configuration / 配置

[English README](../README.md) · [中文首页](../README.zh-CN.md) · [Default configuration / 默认配置](../duet.default.json)

This repository includes a minimal `duet.json` selecting Chinese; other settings inherit from previous layers. Edit it directly. The init command below is for a destination without an existing file.

本仓库附带精简 `duet.json`，选择中文，其他选项继承上层值，可直接编辑。下方 init 命令用于尚无配置文件的目标。

## Quick start / 快速开始

```bash
# Explicitly create a project configuration; never overwrites an existing file.
# 明确生成项目配置，不覆盖已有文件。
./target/release/duet-workbench config init duet.json

# Print effective preferences without opening a workspace or starting Pi.
# 查看最终生效配置，不打开工作区、不启动 Pi。
./target/release/duet-workbench config show /path/to/project

# Use another configuration file / 指定其他配置文件
./target/release/duet-workbench --config /path/to/preferences.json open /path/to/project

# Override UI preferences for this launch / 本次启动覆盖界面偏好
./target/release/duet-workbench --language en --theme light open /path/to/project
```

The shipped `duet.default.json` is embedded at build time, so defaults do not depend on the launch directory. Opening a workspace does not generate or overwrite a configuration file. Edit `duet.json` or an explicit file to customize the built binary; rebuild only when changing the embedded defaults.

仓库内 `duet.default.json` 在构建时嵌入，默认值不依赖启动目录。打开工作台不自动生成或覆盖配置。使用 `duet.json` 或显式指定文件即可调整已编译程序；修改内置默认文件才需要重新构建。

## Precedence / 优先级

Lowest to highest / 从低到高：

1. Embedded `duet.default.json` / 内置默认配置。
2. `$XDG_CONFIG_HOME/duet/config.json`; otherwise `$HOME/.config/duet/config.json` / 用户全局配置。
3. `<opened-project>/duet.json` / 打开的项目配置，不是任意启动目录的配置。
4. File supplied with `--config` / 显式配置文件。
5. CLI overrides: `--language`, `--theme`, `--keymap`, `--helix`, `--review-editor`; `code` also accepts `--pi`, `--model`, `--timeout-secs` / 命令行参数。

JSON objects merge recursively. Missing fields inherit previous values; explicit `"model": null` uses Pi's own configured default model. Unknown fields, invalid enum values, out-of-range limits, malformed JSON, symlink configuration files, and files larger than 64 KiB are rejected before workspace initialization. Relative keymap paths resolve against their configuration file; Pi/editor paths containing directories do likewise. Bare executable names such as `pi` or `vi` use `PATH`. `~` and environment variables inside JSON strings are not expanded.

JSON 对象递归合并，缺失字段继承上层值；`"model": null` 使用 Pi 自身默认模型。未知字段、错误枚举/数值、损坏 JSON、符号链接配置文件及超过 64 KiB 的文件在初始化工作区前拒绝。相对快捷键文件按所在配置文件目录解析；带目录的 Pi/编辑器路径同样处理，`pi`、`vi` 等裸名称从 `PATH` 查找。JSON 字符串不展开 `~` 或环境变量。

Project configuration is ordinary source data and can be committed. If creating/editing it makes main Git dirty, commit it before controlled integration. For private preferences, use the user-global file or `--config` pointing to a Git-ignored location; Duet never stashes your changes automatically.

项目配置是普通项目文件，可以提交 Git；创建/编辑后若主项目变脏，须正常提交后再受控集成。私有偏好放在用户全局文件，或用 `--config` 指向被 Git 忽略的位置；工作台不会自动 stash。

## Options / 配置项

| Field / 字段 | Default / 默认 | Meaning / 含义 |
| --- | --- | --- |
| `schema_version` | `1` | Configuration schema; other versions rejected / 配置结构版本 |
| `language` | `"zh-CN"` | `zh-CN` or `en`; UI preference, not translation of task content / 界面语言，不翻译任务正文 |
| `theme` | `"restore"` | `restore`, `dark`, `light`; restore keeps saved preference, dark on first launch / 恢复历史主题，首次深色 |
| `default_view` | `"restore"` | `restore`, `tasks`, `review`, `project`, `knowledge` / 初始视图，可保留历史位置 |
| `keymap_file` | `null` | Optional keymap JSON; default bindings otherwise / 快捷键文件，空值使用默认 |
| `editor` | `"vi"` | Single executable path, used only after confirmed editing; `DUET_EDITOR` overrides / 单个编辑器路径，确认后启动，环境变量优先 |
| `helix_path` | `"hx"` | Embedded Helix executable; `--helix` overrides / 框内 Helix 可执行路径，--helix 覆盖 |
| `helix_rust_syntax` | `"fast"` | Rust base grammar/highlighting/LSP retained; omit locals analysis and nested language injections for low latency. `"full"` restores upstream; restart required. / 快速模式简化局部变量语义分类及宏／注释／内嵌语言着色，完整模式可恢复，不改全局 |
| `helix_startup` | `true` | Preload fresh Helix on open, sidebar focus, no models or old process recovery; native review / active CLI coding skip preloading / 启动预载，可关闭 |
| `helix_keepalive` | `true` | Reopen a successfully exited workspace editor; explicit stop and crashes do not restart / 普通成功退出后重建，明确停止／崩溃不重启 |
| `review_editor` | `"helix"` | Real Helix default; explicit `"native"` legacy backend / 默认真实 Helix，native 明确选择旧原生后端 |
| `agent.pi_path` | `"pi"` | Pi executable for planning, coding, retry, advisory, and probe / Pi 程序路径 |
| `agent.model` | `null` | Explicit model identifier or Pi default; no automatic fallback / 模型标识或 Pi 默认，不自动切换 |
| `agent.timeout_secs` | `600` | 1–3600 seconds per active start/continuation; advisory uses the same budget / 执行预算，顾问也使用该值 |
| `performance.active_poll_ms` | `16` | 16–250 ms during active model execution / 活动时事件轮询 |
| `performance.idle_poll_ms` | `500` | 100–5000 ms when idle / 空闲轮询 |
| `performance.save_delay_ms` | `300` | 50–5000 ms idle delay before UI-state saving / 界面状态闲置保存延迟 |
| `performance.save_max_delay_ms` | `2000` | At least save_delay_ms, at most 10000 ms / 连续输入的最长保存延迟 |

Theme and view configuration apply **after** saved UI-state restoration. `restore` preserves the old behavior; choosing a fixed theme reapplies that theme on each launch, even if `t` changed it during the previous session. Configuration is read at startup, not hot-reloaded. Credential storage remains owned by Pi; do not add API keys to this file.

主题和视图在历史界面恢复**之后**应用。`restore` 保持原行为；固定主题每次启动重新应用，即使上次用 `t` 切换过。配置启动时读取，不热重载。认证仍由 Pi 管理，不要在此文件写入 API 密钥。

### Language coverage / 语言覆盖范围

`en` currently translates the main navigation, project/Pi session header, focus/footer hints, panel titles, and keyboard help. **Workflow dialogs, contract/status labels, and many error messages remain Chinese.** The UI explicitly labels English as partial. User text, source code, diffs, historical AI replies, and approvals are never translated or rewritten. Complete English localization is future work.

`en` 当前翻译主导航、项目/Pi 会话页眉、焦点/底栏提示、面板标题与快捷键帮助。**工作流弹窗、契约/状态标签和许多错误仍是中文**，界面明确标注英文覆盖不完整。不翻译或改写用户输入、源码、diff、历史 AI 回复或批准。完整英文界面待后续完善。

## Minimal project example / 精简项目示例

```json
{
  "language": "zh-CN",
  "theme": "dark",
  "agent": {
    "pi_path": "pi",
    "model": null,
    "timeout_secs": 900
  }
}
```

Configuration is **not permission**: it cannot enable automatic model calls, writes, tests, merge, retries, push, deployment, extensions, or session recovery. Opening, showing, or generating configuration starts no model. Existing explicit authorization gates remain mandatory.

**配置不是授权**：不能打开自动模型调用、写入、测试、合并、重试、推送、部署、扩展或会话恢复。打开、查看或生成配置不启动模型，现有明确授权流程仍必需。
