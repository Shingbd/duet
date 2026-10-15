# 固定版本测试 Runner / Fixed-version verification

## 使用
1. `1` 或 `2` 选中已交付的节点，必须是当前最终固定检查点，不是任务根行、活动预览或空交付。
2. Helix 焦点先 `Ctrl+]` 回工作台，按 **v** 配置测试。Rust 项目会预填 `cargo test --offline`；有历史命令则预填上次配置，均不是授权。`Ctrl+U` 可清空，粘贴完整 `verify JSON`；也可直接用 `:verify JSON`。
3. Enter 打开独立授权框，用 j/k 浏览完整参数／条件／风险，再 Enter 执行；Esc 没有任何命令执行。
4. 固定验收／测试证据侧栏已按用户要求暂时隐藏，不删除数据，也不改变验证／审批检查。节点主契约仍有验收与总体状态；真实命令／版本／时间／退出码／条件关联／环境摘要／有界日志保存在 `.duet/verifications/<id>/record.json` 和 SQLite，运行／结果在状态栏提示。Tab 不再聚焦隐藏栏，AI就地讨论仍有独立侧栏。
5. 工作台焦点 `Ctrl+X` 或 `:verify-stop` 停止测试；Helix 焦点按键仍属于 Helix。退出也停止当前命令，重启不执行／恢复／重试旧测试；遗留 Running 标为 Interrupted。

```text
:verify {"argv":["cargo","test","--offline"],"criteria":[],"timeout_secs":300}
:verify {"argv":["python3","-m","pytest","tests/test_feature.py"],"criteria":[1,2],"timeout_secs":120}
```

- `argv`：1—64 个非空参数，单参数 ≤4 KiB，总配置 ≤16 KiB，不允许控制字符。直接传给进程，**不做 shell 分词／插值**；要 shell 必须人类显式写 `sh -c ...` 并授权。
- `criteria`：节点验收序号，从 1 开始，不能重复／越界。**空数组只记录命令结果**，exit 0 不会伪造全部验收通过。
- 选择序号是人类声明该命令能验证这些条件；宿主记录实际退出结果，不能判断任意测试是否语义充分。每条条件当前版本最近关联命令通过，且无未解除的失败通用检查，才记关联测试 Passed。部分覆盖保持 Unverified；失败／超时／源码改变／启动采集错误记 Failed。取消／重启不解除已知失败，需对应条件／通用检查重新通过。
- `timeout_secs`：1—3600，默认 300。每节点最多 128 条记录，不自动覆盖旧证据。

## 版本和安全
- 在 `.duet/verifications/<id>/worktree` 从完整不可变 Git 对象创建**新的独立 worktree**，不拿主项目脏文件、候选后续修改或 Helix 未发布草稿来冒充测试输入。不会把生成产物自动放回项目，也不自动批准、合入、调用模型或重试。
- 证据绑定 task/node、固定 snapshot、完整契约签名、argv、关联条件、开始／结束时间、退出码、环境摘要、日志和目录。新版本／契约不继承旧结果；结果可保留为历史，不回写新版本验证。
- 测试前拒绝自定义 Git clean/smudge/process filter；结束检查 HEAD、索引隐藏标记和跟踪源码变更。exit 0 但源码／HEAD／index 改变，证据无效。生成／忽略的未跟踪产物可保留。检测不是 OS 防护，不证明瞬时修改后复原没有发生。
- 异步执行，stdout/stderr 有界合流，日志 ≤128 KiB，过长行省略；UI 仅预览 4 KiB，本地 JSON/SQLite 批量保存约 200 ms，结束即时保存。凭据环境不透传：只允许 PATH/HOME/USER/LOGNAME/LANG/LC_ALL/CARGO_HOME/RUSTUP_HOME/RUSTUP_TOOLCHAIN/LD_LIBRARY_PATH/LIBRARY_PATH/SDKROOT，加 TERM=dumb、NO_COLOR、GIT_TERMINAL_PROMPT=0。记录不包含环境值，常见凭据参数拒绝，疑似凭据日志行隐藏。**不要把凭据放在 argv 或让测试打印它们；这不是完整 DLP。**
- 测试可联网、运行任意项目程序，并以账号权限访问其他目录／HOME；**worktree 不是沙箱**。不是 hermetic/reproducible 环境，也没有自动记录全部依赖／工具版本。仅运行可信项目的测试。
- Unix 进程组支持取消、超时、正常退出回收；主动脱离进程组的工具与 SIGKILL 后清理不保证。后台模型／删除／合入与活动测试互斥；固定快照测试可与 Helix 独立编辑共存，明确不测试那些草稿。
- 验证仅针对当前节点交付；**最终冲突集成结果的 Runner、GUI 条件映射编辑器、容器／沙箱、手工业务验收证明及产物清理仍未接入**。

## 已读功能移除
普通文件已读快捷键 m、确认框、列表标记和文件已读积压计数已移除；实际节点待审批背压继续保留。旧 SQLite is_read 字段／API兼容标识保留但不参与 UI 或流程，不破坏旧阅读滚动记录。冲突界面的 m 仍是“标记冲突解决”，不是已读，不删除该安全步骤。

## English summary
Press v in workbench focus on a current delivered node, edit `verify JSON`, Enter previews and Enter explicitly runs. j/k scroll the confirmation. argv is executed directly in an independent snapshot worktree; empty criteria never marks acceptance passed. Mapped conditions express human-declared coverage, not proof of test adequacy. Evidence survives restart without execution; new versions/contracts do not inherit old results. Ctrl+X or `:verify-stop` cancels. Commands may run arbitrary trusted project code with account permissions, **not a sandbox**. Node approval and merge remain separate. Final integration verification and a richer mapping editor remain future work. The file-read feature is removed; legacy storage stays inert, while conflict marking remains.
