# 阶段 0：Rust 技术原型

## 选型变更

用户明确要求 Rust。宿主由 TypeScript/Pi SDK 方案调整为 Rust/ratatui/crossterm + Pi CLI RPC。保留产品契约、检查点、版本绑定证据、双轨学习审核与本地优先原则。

Pi 不需要改写，也不需要额外 TypeScript 服务。后续角色通过独立 Pi 子进程与工具配置接入；工作台业务状态由 Rust 管理。

## 完成情况

- P0-01：TUI 原型，响应式布局、焦点、深浅主题、键位注册、搜索和命令输入。已通过 ratatui TestBackend 多尺寸测试；真人 IME 体验待验证。
- P0-02：Pi RPC get_state 真实连通，未发送模型 prompt。消息、工具与生命周期状态处理有本地协议测试；真实模型流待下一阶段验证。
- P0-03：EOF 关闭、超时强制清理、只读启动 flags、未知交互 fail-closed。真实任务取消、后代进程清理和写入工具权限仍未完成。
- P0-04：选择性 Git 对象快照、人类继续读旧版本、保留 HEAD/index/工作区，具备真实 Git 集成测试。
- P0-05：测试输入版本隔离尚未完成，需要验证 Runner 与独立验证目录。

不把部分完成的任务标为完整完成，不把离线模拟视为真实 AI 协作。

## 模块

- src/app.rs：阅读队列、焦点、输入状态、离线演示。
- src/ui.rs：ratatui 渲染、视觉语义、终端控制字符过滤。
- src/keymap.rs：动作与键位映射、配置、冲突检查。
- src/rpc.rs：严格 LF JSONL、命令 ID、生命周期、子进程。
- src/checkpoint.rs：Git 基线、选择性快照、只读内容、diff。
- src/main.rs：CLI、终端恢复与事件循环。
- tests/：状态、协议、Git 与布局测试。
- scripts/terminal-smoke.py：真实 PTY 交互与终端状态恢复测试。

## 关键不变量

人类读 Git 对象，不读变化中的工作区。新交付不抢焦点。代码和 AI 输出不能直接发出终端控制序列。prompt 被接受和 agent_end 均不代表运行最终完成；需要 agent_settled。已读不等于批准。

阶段快照将明确选择的工作区文件叠加到 HEAD：临时独立 index → write-tree → commit-tree → 创建私有 ref。不会使用 git add 修改用户 index，不会提交到用户当前分支，不会运行 Git clean/smudge filter 来捕获内容。调用方必须确保安全的无写入边界。

## 下一轮：阶段 1

1. 引入 SQLite schema 和迁移，持久化任务、运行、检查点、阅读进度和个人笔记。
2. 实现任务契约和确认边界。
3. 管理隔离 worktree；区分用户未提交改动与任务基线。
4. Rust 事件循环接入真实 Pi 运行；模型外发说明与明确授权。
5. 增加受控写入与取消策略，先建立权限，不直接开放无限 bash。
6. 实现固定版本验证 Runner，完成 P0-05。
7. 逐步将模拟队列替换为真实检查点，保留离线 demo 用于演示与 UI 回归。

## 运行前提

Rust 1.90、Git；真实 RPC 需 Node.js 和 Pi。测试不需要 API key。Rustup 使用了用户级安装，没有修改系统 Rust 包或 shell PATH；镜像只在下载/构建命令中指定，没有提交或写全局 registry 配置。
