# Duet Workbench

Rust 本地 AI 协作工作台：任务、代码审核、项目与知识分区。默认无示例数据、无模拟 Agent。

## 打开项目

```bash
./target/release/duet-workbench open /path/to/project
```

首次打开自动创建 `.duet/`，再次打开恢复笔记、阅读位置、主题及最近任务对话。支持普通文件夹；Git 项目默认阅读固定 HEAD，不覆盖未提交改动。

```text
.duet/
  .gitignore        本地数据排除规则
  workspace.db      对话、任务记录、笔记、阅读进度
  workspace.lock    单实例项目锁
  sessions/         Pi 原始会话
  worktrees/        独立编码工作区
```

数据本地明文保存。默认被 Git 忽略，不修改项目根 .gitignore。不要提交或公开包含敏感信息的会话。当前一次只允许一个工作台打开同一数据目录；崩溃中的任务恢复为中断，不自动重跑。

## AI 编码

```bash
./target/debug/duet-workbench code --repo /path/to/project --prompt "具体改动要求" --allow-write
```

也可以在已打开的工作台中输入 `:code 具体改动要求`，确认授权后开始编码。

需要 Git 已提交基线及已配置模型/认证的 Pi。默认使用 Pi 当前模型，可指定 `--model provider/model`。

- 创建独立 worktree 和任务分支，原工作区不变。
- Pi 运行期间可同步阅读基线和记笔记。
- 对话按“你 / AI”分块并用不同颜色显示；工具/思考专用的空正文不会作为聊天消息展示，旧空消息重启时也会隐藏。
- 对话持久保存；本轮结束后生成固定检查点供审核，当前会话仍可明确追问；退出后不自动续跑。
- 重启恢复最近任务的审核快照与全部变更文件，审核侧栏只显示最近任务的变更文件，以颜色和 `M/A/D` 区分修改、新增、删除。审核读取固定 Git 版本，不读取正在变化的 worktree。
- 不自动测试、合并、部署或递归启动任务。
- 不加载 Pi 扩展、MCP、skills 或仓库上下文；仅开放文件读取、编辑和写入等工具，不开放 bash。
- 默认超时 600 秒；浏览状态 p 暂停/继续，i 纠正/追问，Ctrl+X 取消，退出中断任务。
- 暂停按 clear_queue → abort → get_state 确认停止，保留已产生的改动，不回滚。暂停时输入的纠正仅记录，明确继续后才发送；暂停等待时间不计入剩余执行时间。
- 运行中发送的消息在工具调用边界接收，不代表立即中断。需立即制止时先暂停，并等待“已暂停”。
- 工作目录和 worktree 不是安全沙箱；授权后 Agent 仍有当前账号的文件访问权限，模型调用可能向提供商发送代码。
- 未提交的源项目改动不自动纳入任务基线。

非交互执行可加 `--headless`，仍需明确写入授权。默认交互界面不展示任何虚构教学内容。任务完成后仍须人类审核。

## 纠正与追问

按 `1` 进入任务视图：
1. 发现方向不对，按 `p`，等状态变为“已暂停”。
2. 按 `i` 输入纠正意见，Enter 提交；此时只记录，AI 不会自行继续。
3. 再按 `p` 明确继续，沿用原会话及工作区执行纠正。
4. 运行中或本轮结束后也可按 `i` 追问，输入内容会交给同一编码会话。

对应命令：`:pause`、`:resume`、`:say 消息`、`:cancel`。
发送对话仍可能触发文件修改，**尚无独立的只读讨论模式**；输入框会显示“仍有编码权限”。
退出/重启不恢复运行中的 Pi；暂停期间未发送的纠正需重新提交。已有文件改动和历史记录保留。

## 查看 AI 生成的文件
AI 结果在独立 worktree，尚未合入主项目，所以不一定出现在主项目 HEAD 的文件列表。
本轮完成或重启后，按 `2` 进入审核；侧栏只显示本次检查点的变更文件，文件路径不重复，以 `M` 黄色、`A` 绿色、`D` 红色标记。选中文件显示该文件的 Git diff，`[` / `]` 切换变更文件。`:changes` 只是兼容入口，不再是必要步骤。
新增、修改、删除的文件都有入口；待审文件不受基线前 80 个文件的预览限制。删除文件也显示对应 diff，无法读取的文件显示错误，不回退到可变工作区。
重启恢复对应版本的选中项、已读状态与滚动位置。这里只加载最近任务结果，多任务导航仍待实现；查看 diff 不等于批准、运行测试或合并。

## 下一阶段：按子任务审核
目标流程是“需求 → AI 结构化拆分 → 逐子任务执行 → 每个子任务独立检查点与待审卡片”。切换子任务只查看其变更文件与验收条件。
**当前尚未实现自动拆分、逐子任务调度或多任务审核列表**；当前审核仍绑定最近单个任务的检查点。开发计划与契约边界见 `docs/task-pipeline.md`。

## 其他命令

`init 文件夹`：初始化本地数据，不需要终端。
`browse --repo 文件夹 --snapshot Git引用`：查看固定快照。
`snapshot --repo 文件夹 --paths 文件路径`：捕获明确选择的文件。
`pi-probe --repo 文件夹`：仅验证 RPC，不调用模型。
不带参数默认打开当前目录。旧 `demo` 命令仅为兼容别名，不再提供模拟交付。

## 操作

1—4 切视图；Tab 切面板；方向键或小写 j/k 浏览（大写 J/K 默认未绑定，保留扩展）；[ / ] 切文件；Enter 切内容页。
底栏显示当前焦点：文件列表中 j/k 切文件，内容中 j/k 滚动；全屏自动聚焦内容，Tab 返回多面板。输入命令/搜索时 j/k 正常输入，Esc 返回浏览。
e 切说明；f 全屏；t 切主题；/ 搜索；: 输入命令；? 帮助。
m 标记已读，不代表批准。命令 `note 内容` 保存笔记。快捷键可通过 `--keymap` 配置。

## 开发与验证

Rust 1.90，ratatui/crossterm，SQLite；AI 执行依赖 Node.js 22.19+ 与 Pi CLI，当前验证版本 Pi 1.1.0。

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build
python3 scripts/terminal-smoke.py
python3 scripts/navigation-smoke.py target/release/duet-workbench
python3 scripts/conversation-smoke.py target/release/duet-workbench
```

Unix 假 RPC 和 PTY 测试需要 Python 3。自动测试不调用真实模型。
crates.io 较慢时可为构建命令添加：
`--config 'source.crates-io.replace-with="ustc"' --config 'source.ustc.registry="sparse+https://mirrors.ustc.edu.cn/crates.io-index/"'`。

## 性能与 SSH
日常运行建议 cargo build --release，再使用 target/release/duet-workbench；debug 编译主要用于开发。
界面按需重绘，固定文件的折行排版缓存；连续输入合并处理，SQLite 仅写变更。
笔记与阅读状态在停顿约 300 ms 后保存，连续操作最长约 2 秒批量保存；正常退出立即保存。
突发崩溃可能丢失最后这段尚未保存的界面状态；真实任务与完整对话消息仍直接持久化。

本地 PTY 性能测试（临时 Git 项目，不调用模型或网络）：
```bash
python3 scripts/performance-probe.py target/debug/duet-workbench target/release/duet-workbench
```
6000 行文件 / 120×32 终端的优化前后测量见 docs/performance.md。
这只测本机，不包含 SSH、终端客户端或网络 RTT；高延迟 SSH 仍会给每次反馈增加网络耗时。

## 当前边界

已具备受控手动任务的第一条编码链路，不是完整自主多 Agent 平台。
待实现：结构化任务契约、多阶段调度、验证 Runner、独立审查、最终审批/合并、完整对话会话导航、按需 AI 教学。
浏览暂载入前 80 个文件，每文件 1 MiB；没有完整 IDE 编辑、语法高亮或系统级沙箱。
详见 `docs/phase-1.md`。