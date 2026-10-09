# Duet Workbench

Rust 本地 AI 协作工作台：任务、代码审核、项目与知识分区。默认无示例数据、无模拟 Agent。

## 打开项目

```bash
./target/debug/duet-workbench open /path/to/project
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

需要 Git 已提交基线及已配置模型/认证的 Pi。默认使用 Pi 当前模型，可指定 `--model provider/model`。

- 创建独立 worktree 和任务分支，原工作区不变。
- Pi 运行期间可同步阅读基线和记笔记。
- 对话持久保存；完成后生成固定检查点供审核。
- 不自动测试、合并、部署或递归启动任务。
- 不加载 Pi 扩展、MCP、skills 或仓库上下文；仅开放文件读取、编辑和写入等工具，不开放 bash。
- 默认超时 600 秒；浏览状态 Ctrl+X 取消，退出中断正在运行的任务。
- 工作目录和 worktree 不是安全沙箱；授权后 Agent 仍有当前账号的文件访问权限，模型调用可能向提供商发送代码。
- 未提交的源项目改动不自动纳入任务基线。

非交互执行可加 `--headless`，仍需明确写入授权。默认交互界面不展示任何虚构教学内容。任务完成后仍须人类审核。

## 其他命令

`init 文件夹`：初始化本地数据，不需要终端。
`browse --repo 文件夹 --snapshot Git引用`：查看固定快照。
`snapshot --repo 文件夹 --paths 文件路径`：捕获明确选择的文件。
`pi-probe --repo 文件夹`：仅验证 RPC，不调用模型。
不带参数默认打开当前目录。旧 `demo` 命令仅为兼容别名，不再提供模拟交付。

## 操作

1—4 切视图；Tab 切面板；方向键或 j/k 浏览；[ / ] 切文件；Enter 切内容页。
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
```

Unix 假 RPC 和 PTY 测试需要 Python 3。自动测试不调用真实模型。
crates.io 较慢时可为构建命令添加：
`--config 'source.crates-io.replace-with="ustc"' --config 'source.ustc.registry="sparse+https://mirrors.ustc.edu.cn/crates.io-index/"'`。

## 当前边界

已具备受控手动任务的第一条编码链路，不是完整自主多 Agent 平台。
待实现：结构化任务契约、多阶段调度、验证 Runner、独立审查、最终审批/合并、完整对话会话导航、按需 AI 教学。
浏览暂载入前 80 个文件，每文件 1 MiB；没有完整 IDE 编辑、语法高亮或系统级沙箱。
详见 `docs/phase-1.md`。