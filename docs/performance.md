# 交互性能排查

## Rust 大文件延迟优化（当前）

用户当前实际使用 debug、终端 210×49、src/ui.rs。先前纯文本基准不能代表该 Rust 文件：新基准只复制源码到临时 Git 仓库、头部加入普通注释、50 次在注释内部输入（不破坏语法），无模型／网络／用户文件修改。以下测本机字形输出，不含 SSH／客户端渲染、切文件或初始化。

| 210×49、同一 Rust 源码 | 优化前 p50 / p95 | 优化后 p50 / p95 |
| --- | --- | --- |
| 独立上游 Helix（对照） | 68.00 / 69.74 ms | 67.84 / 69.60 ms |
| Duet release | 70.93 / 73.38 ms | **9.82 / 10.35 ms** |
| Duet debug | 96.98 / 104.70 ms | **11.30 / 12.42 ms** |

本轮优化后 max release10.57/debug13.19ms，空闲CPU采样均0%（时钟精度限制，不保证绝对零）。full 同程序对照仍70.76/73.00ms，说明主要改善不是网络变动。默认旧纯文本120×32数据仍在下方，不能混成同一基准。

### 两个针对性改动及取舍
- `helix_rust_syntax:"fast"` 默认在**私有会话配置**放置仅有注释的 Rust `locals.scm` 与 `injections.scm`，抑制昂贵的全文件局部变量查询和嵌套语法层处理；原始 Rust grammar、基础 highlights、indents、textobjects、LSP、实际选区／桥接和保存流程不变。未关闭全部语言功能、未删上游运行库或改全局设置。
- **不是完全等价的着色**：局部变量引用的语义分类、format占位符、宏体内部递归Rust及注释/内嵌SQL等专用语言高亮简化，宏内的嵌套语法选择也可能更粗。基础关键字／类型／字符串／注释仍有Rust高亮；LSP不由此禁用。重启前在项目duet.json设 `"helix_rust_syntax":"full"` 可恢复全部上游查询，其他语言不变。不存在性能保证／无声全局变更。
- Cargo `[profile.dev] opt-level=1` 优化宿主终端单元格热路径，仍保留调试符号和断言，不改release profile；debug首次构建会较慢。日常仍推荐release。
- 无忙轮询／自动调用模型／测试／合入，旧审批／固定快照／未保存文本stale保护不变；配置生效需要保存各缓冲区后退出并新开进程，构建不会接管正在运行的编辑器。

复现（将 full 与 fast、debug 与 release 在同一文件／尺寸下比较）：
```bash
python3 scripts/helix-latency-probe.py target/tools/helix-25.07.1-x86_64-linux/hx \
 target/release/duet-workbench target/debug/duet-workbench --source src/ui.rs --rows 49 --cols 210
# 上游完整语法对照
python3 scripts/helix-latency-probe.py target/tools/helix-25.07.1-x86_64-linux/hx \
 target/release/duet-workbench --source src/ui.rs --rows 49 --cols 210 --rust-syntax full
```

服务器SSH连接观测存在约54–237ms RTT及重传，但因权限无法把具体socket与用户UI会话精确关联，不作用户端到端结论；此修复不解决网络延迟。复现实验不对用户原项目运行测试命令。

## 嵌入式 Helix 事件唤醒优化（此前）

`scripts/helix-latency-probe.py` 对比独立真实 Helix 25.07.1、Duet release/debug：临时 Git 仓库、6000 行文件、相同外部 120×32 PTY、50 次插入按键至新字形输出，无模型／网络调用。嵌入代码区尺寸小于独立 Helix 全屏；测的是本机 stdout 字形反馈，不是 SSH 客户端端到端或首个文件／Git 工作区加载时间。

| 构建 | 修复前 p50 / p95 | 修复后 p50 / p95 |
| --- | --- | --- |
| 独立 Helix 对照 | 6.34 / 6.54 ms | 6.27 / 6.43 ms |
| 嵌入 release | 17.88 / 18.33 ms | 7.28 / 7.55 ms |
| 嵌入 debug | 37.93 / 40.70 ms | 17.13 / 19.82 ms |

修复后 release 的本轮最大值 8.94 ms；约减少 59% 中位延迟，宿主额外延迟约 1 ms；不是零延迟保证。1 秒空闲样本在时钟 tick 精度下 CPU 0%，多次样本偶见 1%，不代表忙轮询或严格零占用。

根因及热路径修复：
- 旧循环只通过 crossterm 等宿主键盘，Helix PTY 输出必须等待完整 active_poll_ms（默认 16 ms）。现在 Unix poll 同时等 stdin 与 PTY；输出立即唤醒，仍保留活动超时处理桥接、模型进度及保存期限。已缓存的 crossterm 事件优先取，非编辑器路径保留原事件等待；不降低间隔做忙轮询。
- 旧循环每个转发按键先重画旧 Helix 画面，再画真实新帧，debug 尤其昂贵。普通编辑器按键／粘贴只转发，PTY 实际改变才绘制；焦点、上下文、错误、宿主输入仍正常触发绘制。
- 普通 Helix 键不再标记 SQLite 状态待保存；实际捕获到的审核草稿、聊天／发布及宿主导航继续按原规则持久化。不是丢弃权威草稿保存，也不是缓存终端单元冒充选区。
- 宿主已排队保存仍遵守最长延迟；对话、授权、固定版本、桥接 stale 检查和合入互斥不改。

增加 child_output_wakes_host_before_the_idle_timeout 单元测试；181 项测试、十五组 PTY、严格 fmt/clippy、debug/release 构建通过。日常使用 `./target/release/duet-workbench open .`，旧进程需要退出重开；源码调试版本仍有明显额外绘制开销。下方为早期非 Helix 阅读器基准，勿混用两种测量。

复现：
```bash
python3 scripts/helix-latency-probe.py \
  target/tools/helix-25.07.1-x86_64-linux/hx \
  target/release/duet-workbench target/debug/duet-workbench
```

## 问题与证据
用户报告远程终端操作卡顿。发现旧进程使用 debug 构建，进程累计 CPU 占用约 16%。
源代码问题：
- 每 100 ms 无条件构造界面；每帧复制并净化整个文件，重新折行。
- 文本净化对每个字符分配一个 String。
- 每个输入事件同步提交 SQLite，并更新所有阅读条目；无变化的输入内容也触发保存。
服务器观察到多个 SSH TCP 连接 RTT 约 110—235 ms，有重传。但未建立用户当前终端与某一连接的一一对应，不能认定该 RTT 就是用户当前连接。没有从用户客户端做端到端测量。

## 实施修复
- Renderer 持久缓存折行结果，只有文件/宽度/筛选/内容页改变才重新排版；滚动只绘制可见行。
- 折行按 grapheme 和终端单元宽度处理，保留中文、组合字符与 emoji。
- safe_text 单次缓冲，不为每个字符建立 String。
- UI 只在输入、缩放或 Agent 状态变化后绘制；未运行 Agent 时空闲等待事件。
- 每轮合并最多 64 个排队输入事件。event::poll 会被输入即时唤醒，不需等待 500 ms 超时。
- 先反馈界面，导航状态停顿 300 ms 后保存，连续操作最长 2 秒一次；正常退出同步保存。
- SQLite 状态未变不建立事务；只写变更条目，成功提交后才更新缓存。
- Pi 对话消息继续直接持久化；没有改变固定快照、授权或审核语义。
- 正式交互建议 release 构建。

## 可复现实测
scripts/performance-probe.py 使用临时 Git 仓库的一份 6000 行源码；终端 120×32；本地 PTY，12 次输入响应；不调用模型，不通过 SSH。
测量包含按键写入至输出新增字符，不代表大文件首次打开或 AI 启动耗时。CPU 是 3 秒空闲样本的一个逻辑核占比，0% 意味着采样精度下没有可见 CPU 时间。

| 构建 | 输入响应中位数 | 本轮最大值 | 空闲 CPU |
| --- | --- | --- | --- |
| 优化前 debug | 118.06 ms | 119.93 ms | 54.66% |
| 优化后 debug | 9.71 ms | 9.83 ms | 0.00% |
| 优化后 release | 0.94 ms | 1.11 ms | 0.00% |

测试是专用大文件夹具，不等于用户当前文件或网络环境；优化后仍可能受 SSH 往返、丢包或客户端渲染影响。
现有 33 项测试及 3 项性能回归测试、严格 clippy、格式检查和 PTY 交互测试通过；release 构建通过。

## 边界与后续
- 崩溃可能丢失最后 300 ms 至 2 秒未提交的界面/阅读状态；正常退出不会因此丢失。
- 首次项目载入仍同步读取最多 80 个 Git 文件；首次显示某个大文件也会生成折行缓存。
- Pi 启动/握手、Git worktree 建立与最终检查点捕获仍在主循环同步进行；这些阶段可能短时阻塞。后续应改成后台任务，不宣称本次解决所有阶段的卡顿。
- 缓存文本折行从 Paragraph 的词级折行改成 terminal-cell/grapheme 折行，代码浏览不拆字符，但英文文本可能在单词中间折行。
- 没有干扰用户当前运行的进程；更新二进制不会替换已运行程序，需退出后重启。
