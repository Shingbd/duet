use crate::{
    app::{App, RunControl},
    checkpoint::Repository,
    checkpoint::Snapshot,
    nodes::{Execution, TaskPlan, TaskRecord, TaskState, planning_prompt},
    rpc::{PiClient, RunStatus, message_text},
    storage::{Message, WorkspaceStore},
};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

// clear_queue must finish before abort: RPC handlers are asynchronous.
enum ControlStage {
    Clear(String),
    Abort(String),
    State(String),
    Retry(Instant),
}
struct PendingControl {
    pause: bool,
    stage: ControlStage,
    deadline: Instant,
}

#[derive(Clone, Copy)]
enum Phase {
    Legacy,
    Planning,
    Node(usize),
}

pub struct CodingRun {
    client: PiClient,
    phase: Phase,
    executable: PathBuf,
    model: Option<String>,
    node_base: Option<Snapshot>,
    plan_output: String,
    waiting: bool,
    pub worktree: PathBuf,
    pub branch: String,
    pub session: String,
    pub id: String,
    pub checkpoint: Option<String>,
    repo: Repository,
    timeout: Duration,
    deadline: Instant,
    remaining: Duration,
    finished: bool,
    closed: bool,
    settled: bool,
    control: Option<PendingControl>,
    corrections: Vec<String>,
    pending_echoes: VecDeque<String>,
}

impl CodingRun {
    pub fn start(
        project: &Path,
        executable: &Path,
        model: Option<&str>,
        prompt: &str,
        timeout: Duration,
        store: &mut WorkspaceStore,
        app: &mut App,
    ) -> Result<Self> {
        Self::start_internal(
            project, executable, model, prompt, timeout, store, app, false,
        )
    }
    pub fn start_planning(
        project: &Path,
        executable: &Path,
        model: Option<&str>,
        prompt: &str,
        timeout: Duration,
        store: &mut WorkspaceStore,
        app: &mut App,
    ) -> Result<Self> {
        Self::start_internal(
            project, executable, model, prompt, timeout, store, app, true,
        )
    }
    #[allow(clippy::too_many_arguments)]
    fn start_internal(
        project: &Path,
        executable: &Path,
        model: Option<&str>,
        prompt: &str,
        timeout: Duration,
        store: &mut WorkspaceStore,
        app: &mut App,
        planning: bool,
    ) -> Result<Self> {
        ensure!(!prompt.trim().is_empty(), "任务不能为空");
        let source = Repository::open(project)?;
        source
            .baseline()
            .context("编码前请先提交项目基线；不会自动提交用户文件")?;
        let id = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_nanos()
            .to_string();
        let branch = format!("duet/task-{id}");
        let parent = store.directory().join("worktrees");
        std::fs::create_dir_all(&parent)?;
        ensure!(
            !std::fs::symlink_metadata(&parent)?.file_type().is_symlink(),
            "worktrees 不允许符号链接"
        );
        let worktree = parent.join(&id);
        source.create_worktree(&worktree, &branch)?;
        let repo = Repository::open(&worktree)?;
        let baseline = repo.baseline()?;
        let sessions = store.directory().join("sessions");
        std::fs::create_dir_all(&sessions)?;
        ensure!(
            !std::fs::symlink_metadata(&sessions)?
                .file_type()
                .is_symlink(),
            "sessions 不允许符号链接"
        );
        let mut client = if planning {
            PiClient::spawn_planner(executable, &worktree, &sessions, model)?
        } else {
            PiClient::spawn_coder(executable, &worktree, &sessions, model)?
        };
        let state = client.get_state(Duration::from_secs(20))?;
        let session = state["sessionId"]
            .as_str()
            .context("Pi 未返回会话 ID")?
            .to_owned();
        ensure!(
            state["model"].is_object(),
            "Pi 没有可用模型，请先运行 pi 配置模型与认证"
        );
        store.begin_run(&id, &session, &worktree, &branch, prompt)?;
        let mut task = TaskRecord::legacy(&id, prompt, &session);
        task.baseline = Some(baseline.commit);
        if planning {
            task.state = TaskState::Planning;
        }
        store.save_task(&task)?;
        app.tasks.push(task);
        if planning {
            app.view = crate::app::View::Tasks;
            app.task_selection = Some(crate::app::TaskSelection {
                task_id: id.clone(),
                node_id: None,
            });
            app.review_version = None;
        }
        if app.task_selection.is_none() {
            app.task_selection = Some(crate::app::TaskSelection {
                task_id: id.clone(),
                node_id: None,
            });
        }
        app.agent_connected = true;
        app.task = Some(prompt.into());
        app.task_scroll = 0;
        app.queued_corrections = 0;
        app.history.clear();
        app.run = Default::default();
        app.run.status = RunStatus::Running;
        let resolved_model = model.map(str::to_owned).or_else(|| {
            state["model"]["provider"]
                .as_str()
                .zip(state["model"]["id"].as_str())
                .map(|(p, id)| format!("{p}/{id}"))
        });
        let mut run = Self {
            client,
            phase: if planning {
                Phase::Planning
            } else {
                Phase::Legacy
            },
            executable: executable.into(),
            model: resolved_model,
            node_base: None,
            plan_output: String::new(),
            waiting: false,
            worktree,
            branch,
            session,
            id,
            repo,
            timeout,
            deadline: Instant::now() + timeout,
            remaining: timeout,
            checkpoint: None,
            finished: false,
            closed: false,
            settled: false,
            control: None,
            corrections: Vec::new(),
            pending_echoes: VecDeque::new(),
        };
        run.record("user", prompt, app, store)?;
        let message = if planning {
            planning_prompt(prompt)
        } else {
            prompt.into()
        };
        run.pending_echoes.push_back(message.clone());
        run.send_prompt(&message, app)?;
        app.status = if planning {
            "只读拆分中 · 尚未开始写入"
        } else {
            "编码中 · 当前阅读基线"
        }
        .into();
        Ok(run)
    }

    fn record(
        &self,
        role: &str,
        content: &str,
        app: &mut App,
        store: &mut WorkspaceStore,
    ) -> Result<()> {
        if content.trim().is_empty() {
            return Ok(());
        }
        store.append_message(&self.session, role, content)?;
        let message = Message {
            session: self.session.clone(),
            role: role.into(),
            content: content.into(),
        };
        app.history.push(message.clone());
        if let Some(task) = app.tasks.iter_mut().find(|t| t.id == self.id) {
            task.history.push(message);
        }
        app.content_revision = app.content_revision.wrapping_add(1);
        Ok(())
    }

    fn send_prompt(&mut self, message: &str, app: &mut App) -> Result<()> {
        // Works both during streaming (steering) and if the preceding turn just settled.
        let request = self
            .client
            .send(json!({"type":"prompt","message":message,"streamingBehavior":"steer"}))?;
        app.run.register(&request, "prompt");
        Ok(())
    }

    pub fn interact(
        &mut self,
        control: RunControl,
        app: &mut App,
        store: &mut WorkspaceStore,
    ) -> Result<()> {
        if let RunControl::Say(_) = &control {
            ensure!(app.task_selection.as_ref().is_none_or(|s| s.task_id == self.id &&
                s.node_id.as_ref().is_none_or(|id| matches!(self.phase,Phase::Node(i) if app.tasks.iter().find(|t| t.id==self.id).is_some_and(|t| &t.nodes[i].contract.id==id)))),
                "所选任务/节点不是当前编码会话，未发送消息");
        }
        match control {
            RunControl::TogglePause if app.run.status == RunStatus::Paused => {
                self.resume(app, store)
            }
            RunControl::TogglePause | RunControl::Pause => self.pause(app),
            RunControl::Resume => self.resume(app, store),
            RunControl::Say(message) => {
                ensure!(!self.closed, "该会话已关闭；请启动新任务");
                ensure!(
                    !matches!(
                        app.run.status,
                        RunStatus::Cancelling | RunStatus::Failed | RunStatus::Aborted
                    ),
                    "任务已取消或失败，不能发送"
                );
                ensure!(!message.trim().is_empty(), "消息不能为空");
                self.record("user", &message, app, store)?;
                if matches!(app.run.status, RunStatus::Pausing | RunStatus::Paused) {
                    self.corrections.push(message);
                    app.queued_corrections = self.corrections.len();
                    app.status = "纠正已记录 · 继续后发送，尚未交给 AI".into();
                } else {
                    if self.finished
                        && let Phase::Node(index) = self.phase
                    {
                        self.deadline = Instant::now() + self.timeout;
                        self.begin_node(index, &message, app, store)?;
                        return Ok(());
                    }
                    if self.finished {
                        self.finished = false;
                        self.settled = false;
                        self.deadline = Instant::now() + self.timeout;
                        app.run.status = RunStatus::Running;
                        app.run.error = None;
                        store.finish_run(&self.id, "Running", self.checkpoint.as_deref())?;
                    }
                    self.pending_echoes.push_back(message.clone());
                    self.send_prompt(&message, app)?;
                    app.status = "消息已提交 · 运行中在工具调用边界接收".into();
                }
                Ok(())
            }
        }
    }

    fn begin_control(&mut self, pause: bool, app: &mut App) -> Result<()> {
        let clear = self.client.send(json!({"type":"clear_queue"}))?;
        app.run.register(&clear, "clear_queue");
        self.control = Some(PendingControl {
            pause,
            stage: ControlStage::Clear(clear),
            deadline: Instant::now() + Duration::from_secs(15),
        });
        app.run.status = if pause {
            RunStatus::Pausing
        } else {
            RunStatus::Cancelling
        };
        app.content_revision = app.content_revision.wrapping_add(1);
        Ok(())
    }

    pub fn pause(&mut self, app: &mut App) -> Result<()> {
        ensure!(
            !self.closed && app.run.status == RunStatus::Running,
            "只能暂停正在运行的任务；暂停确认前不能继续"
        );
        self.remaining = self.deadline.saturating_duration_since(Instant::now());
        self.begin_control(true, app)?;
        app.status = "暂停中 · 正在清队列并等待 Pi 停止确认".into();
        Ok(())
    }

    pub fn resume(&mut self, app: &mut App, store: &mut WorkspaceStore) -> Result<()> {
        if self.waiting {
            ensure!(app.run.status == RunStatus::Paused, "节点边界尚未暂停");
            ensure!(!self.remaining.is_zero(), "任务时间预算已耗尽");
            self.deadline = Instant::now() + self.remaining;
            self.waiting = false;
            if !self.corrections.is_empty()
                && let Phase::Node(index) = self.phase
            {
                let correction = self.corrections.join("\n\n");
                self.corrections.clear();
                app.queued_corrections = 0;
                if let Err(error) = self.begin_node(index, &correction, app, store) {
                    return self.fail_workflow(error, app, store);
                }
            } else {
                if app
                    .tasks
                    .iter()
                    .find(|t| t.id == self.id)
                    .unwrap()
                    .pending_reviews()
                    >= 2
                {
                    self.waiting = true;
                    anyhow::bail!("仍有两个待审节点；请先明确批准节点，再继续");
                }
                if let Err(error) = self.start_next(app, store) {
                    return self.fail_workflow(error, app, store);
                }
            }
            return Ok(());
        }
        ensure!(
            !self.closed && app.run.status == RunStatus::Paused,
            "请等待已暂停确认后再继续"
        );
        ensure!(
            !self.remaining.is_zero(),
            "任务时间预算已耗尽，请取消后启动新任务"
        );
        let message = if self.corrections.is_empty() {
            let message = "继续原任务，保留已有改动。任务完成不代表测试或验收通过。".to_owned();
            self.record("user", &message, app, store)?;
            message
        } else {
            self.corrections.join("\n\n")
        };
        self.pending_echoes.push_back(message.clone());
        self.send_prompt(&message, app)?;
        self.corrections.clear();
        app.queued_corrections = 0;
        self.finished = false;
        self.settled = false;
        self.deadline = Instant::now() + self.remaining;
        app.run.status = RunStatus::Running;
        app.run.error = None;
        store.finish_run(&self.id, "Running", self.checkpoint.as_deref())?;
        app.status = "继续编码 · 已提交纠正意见".into();
        app.content_revision = app.content_revision.wrapping_add(1);
        Ok(())
    }

    fn advance_control(
        &mut self,
        record: &Value,
        app: &mut App,
        store: &mut WorkspaceStore,
    ) -> Result<()> {
        let Some(mut control) = self.control.take() else {
            return Ok(());
        };
        if app.run.status == RunStatus::Failed {
            return Ok(());
        }
        let response_id = if record["type"] == "response" {
            record["id"].as_str()
        } else {
            None
        };
        match &control.stage {
            ControlStage::Clear(id) if response_id == Some(id.as_str()) => {
                // Previously queued steering was not delivered: retain it for explicit resume.
                if control.pause {
                    for field in ["steering", "followUp"] {
                        for message in record["data"][field]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(Value::as_str)
                        {
                            self.corrections.push(message.into());
                        }
                    }
                    app.queued_corrections = self.corrections.len();
                }
                let id = self.client.send(json!({"type":"abort"}))?;
                app.run.register(&id, "abort");
                control.stage = ControlStage::Abort(id);
            }
            ControlStage::Abort(id) if response_id == Some(id.as_str()) => {
                let id = self.client.send(json!({"type":"get_state"}))?;
                app.run.register(&id, "get_state");
                control.stage = ControlStage::State(id);
            }
            ControlStage::State(id) if response_id == Some(id.as_str()) => {
                let state = &record["data"];
                if state["isStreaming"] == false
                    && state["isCompacting"] == false
                    && state["pendingMessageCount"] == 0
                {
                    app.run.streaming_text = false;
                    if control.pause {
                        app.run.status = RunStatus::Paused;
                        store.finish_run(&self.id, "Paused", self.checkpoint.as_deref())?;
                        app.status = "已暂停 · 可记录纠正，明确继续后才发送（已有改动保留）".into();
                    } else {
                        app.run.status = RunStatus::Aborted;
                        self.settled = true;
                    }
                    return Ok(());
                }
                control.stage = ControlStage::Retry(Instant::now() + Duration::from_millis(100));
            }
            _ => {}
        }
        app.run.status = if control.pause {
            RunStatus::Pausing
        } else {
            RunStatus::Cancelling
        };
        self.control = Some(control);
        Ok(())
    }

    pub fn poll(&mut self, app: &mut App, store: &mut WorkspaceStore) -> Result<bool> {
        if self.waiting && app.run.status == RunStatus::Aborted {
            self.waiting = false;
            self.finished = true;
            store.finish_run(&self.id, "Aborted", self.checkpoint.as_deref())?;
            self.sync_state(app, store)?;
            return Ok(true);
        }
        if self.finished || self.waiting {
            return Ok(false);
        }
        let mut changed = false;
        for _ in 0..128 {
            match self.client.events.try_recv() {
                Ok(Ok(record)) => {
                    changed = true;
                    if record["type"] == "extension_ui_request" {
                        self.client.dismiss_dialog(&record)?;
                    }
                    if record["type"] == "message_end" {
                        let message = &record["message"];
                        if let Some(role @ ("user" | "assistant")) = message["role"].as_str() {
                            let content = message_text(message);
                            let echoed = role == "user"
                                && self
                                    .pending_echoes
                                    .iter()
                                    .position(|text| text == &content)
                                    .map(|index| {
                                        self.pending_echoes.remove(index);
                                    })
                                    .is_some();
                            if !echoed {
                                if matches!(self.phase, Phase::Planning) && role == "assistant" {
                                    if !content.trim().is_empty() {
                                        self.plan_output = content;
                                    }
                                } else {
                                    self.record(role, &content, app, store)?;
                                }
                            }
                        }
                    }
                    let was_paused = app.run.status == RunStatus::Paused;
                    app.run.apply(&record);
                    if matches!(self.phase, Phase::Planning) {
                        app.run.streaming_text = false;
                    }
                    if record["type"] == "agent_start" {
                        self.settled = false;
                    }
                    if record["type"] == "agent_settled" {
                        self.settled = true;
                    }
                    if record["type"] == "response"
                        && record["success"] == true
                        && record["data"]["disposition"] == "started"
                    {
                        self.settled = false;
                        app.run.status = RunStatus::Running;
                    }
                    self.advance_control(&record, app, store)?;
                    if was_paused && self.control.is_none() && app.run.status != RunStatus::Failed {
                        app.run.status = RunStatus::Paused;
                    }
                }
                Ok(Err(error)) => {
                    changed = true;
                    app.run.status = RunStatus::Failed;
                    app.run.error = Some(error.to_string());
                    break;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    changed = true;
                    app.run.status = RunStatus::Failed;
                    break;
                }
            }
        }
        if let Some(control) = &mut self.control {
            if matches!(control.stage, ControlStage::Retry(when) if Instant::now() >= when) {
                let id = self.client.send(json!({"type":"get_state"}))?;
                app.run.register(&id, "get_state");
                control.stage = ControlStage::State(id);
            }
            if Instant::now() >= control.deadline {
                app.run.status = RunStatus::Failed;
                app.run.error = Some("Pi 停止确认超时；没有宣称已暂停".into());
                changed = true;
            }
        } else if Instant::now() >= self.deadline && app.run.status == RunStatus::Running {
            self.cancel(app)?;
            app.status = "时间预算耗尽 · 正在停止".into();
            changed = true;
        }
        let pending_prompt = app.run.pending.values().any(|command| command == "prompt");
        if pending_prompt && self.control.is_none() && app.run.status == RunStatus::Settled {
            // An older turn can settle before a new prompt is accepted.
            app.run.status = RunStatus::Running;
        }
        if app.run.status == RunStatus::Failed
            || (self.control.is_none()
                && app.run.status != RunStatus::Paused
                && !pending_prompt
                && (self.settled || app.run.status == RunStatus::Idle))
        {
            self.finish_turn(app, store)?;
            changed = true;
        }
        if changed {
            self.sync_state(app, store)?;
            app.content_revision = app.content_revision.wrapping_add(1);
        }
        Ok(changed)
    }

    fn finish_turn(&mut self, app: &mut App, store: &mut WorkspaceStore) -> Result<()> {
        match self.phase {
            Phase::Planning => return self.finish_planning(app, store),
            Phase::Node(index) => return self.finish_node(index, app, store),
            Phase::Legacy => {}
        }
        // Natural agent_settled guarantees no queued/automatic work; the host serializes
        // capture and new prompts. Keep the idle session for explicit human follow-ups.
        // Failure/cancellation closes Pi before capture instead.
        if app.run.status != RunStatus::Settled {
            self.client.shutdown()?;
            self.closed = true;
            app.agent_connected = false;
        }
        self.finished = true;
        let paths = self.repo.changed_paths()?;
        self.checkpoint = None;
        if !paths.is_empty() {
            let snapshot = self.repo.capture(&paths)?;
            app.append_checkpoint(&self.repo, &snapshot)?;
            self.checkpoint = Some(snapshot.commit);
            app.status = "本轮改动待审 · 测试未运行 · i 可继续追问".into();
        } else {
            app.status = "本轮结束 · 无代码改动 · i 可追问".into();
        }
        if app.run.status == RunStatus::Failed {
            app.status = format!(
                "运行失败 · {}",
                app.run.error.as_deref().unwrap_or("Pi 连接中断")
            );
        }
        if app.run.status == RunStatus::Aborted {
            app.status = "已取消 · 已有改动保留，未验收".into();
        }
        store.finish_run(
            &self.id,
            &format!("{:?}", app.run.status),
            self.checkpoint.as_deref(),
        )?;
        Ok(())
    }

    fn sync_state(&self, app: &mut App, store: &mut WorkspaceStore) -> Result<()> {
        if let Some(task) = app.tasks.iter_mut().find(|t| t.id == self.id) {
            let previous_state = task.state;
            let mut dirty = task.checkpoint != self.checkpoint;
            task.state =
                if matches!(self.phase, Phase::Planning) && app.run.status == RunStatus::Running {
                    TaskState::Planning
                } else {
                    TaskState::from_run(&format!("{:?}", app.run.status))
                };
            dirty |= previous_state != task.state;
            task.checkpoint = self.checkpoint.clone();
            if let Phase::Node(index) = self.phase {
                let node = &mut task.nodes[index];
                if matches!(node.execution, Execution::Running | Execution::Paused) {
                    let previous_execution = node.execution;
                    node.execution = match app.run.status {
                        RunStatus::Paused => Execution::Paused,
                        RunStatus::Failed => Execution::Failed,
                        RunStatus::Aborted => Execution::Cancelled,
                        _ => Execution::Running,
                    };
                    dirty |= previous_execution != node.execution;
                    if let Some(attempt) = node.attempts.last_mut() {
                        attempt.execution = node.execution;
                    }
                }
            }
            if dirty {
                store.save_task(task)?;
            }
        }
        Ok(())
    }

    fn fail_workflow(
        &mut self,
        error: impl std::fmt::Display,
        app: &mut App,
        store: &mut WorkspaceStore,
    ) -> Result<()> {
        self.client.shutdown()?;
        self.closed = true;
        self.finished = true;
        app.agent_connected = false;
        app.run.status = RunStatus::Failed;
        app.run.error = Some(error.to_string());
        app.status = format!("节点流程停止：{error}");
        store.finish_run(&self.id, "Failed", self.checkpoint.as_deref())?;
        self.sync_state(app, store)
    }
    fn finish_planning(&mut self, app: &mut App, store: &mut WorkspaceStore) -> Result<()> {
        if app.run.status != RunStatus::Settled {
            self.client.shutdown()?;
            self.closed = true;
            self.finished = true;
            app.agent_connected = false;
            app.status = "拆分已停止 · 未进入节点实现".into();
            store.finish_run(&self.id, &format!("{:?}", app.run.status), None)?;
            return Ok(());
        }
        let result = (|| -> Result<()> {
            let plan = TaskPlan::parse(&self.plan_output)?;
            self.client.shutdown()?;
            self.closed = true;
            app.agent_connected = false;
            let task = app.tasks.iter_mut().find(|t| t.id == self.id).unwrap();
            task.install_plan(plan)?;
            store.save_task(task)?;
            self.client = PiClient::spawn_coder(
                &self.executable,
                &self.worktree,
                &store.directory().join("sessions"),
                self.model.as_deref(),
            )?;
            let state = self.client.get_state(Duration::from_secs(20))?;
            ensure!(state["model"].is_object(), "节点编码模型不可用");
            self.session = state["sessionId"]
                .as_str()
                .context("节点编码会话 ID 缺失")?
                .into();
            store.set_session(&self.id, &self.session)?;
            let task = app.tasks.iter_mut().find(|t| t.id == self.id).unwrap();
            if !task.sessions.contains(&self.session) {
                task.sessions.push(self.session.clone());
            }
            store.save_task(task)?;
            self.closed = false;
            app.agent_connected = true;
            self.start_next(app, store)
        })();
        if let Err(error) = result {
            self.fail_workflow(error, app, store)?;
        }
        Ok(())
    }
    fn begin_node(
        &mut self,
        index: usize,
        extra: &str,
        app: &mut App,
        store: &mut WorkspaceStore,
    ) -> Result<()> {
        ensure!(
            Instant::now() < self.deadline,
            "任务时间预算已耗尽，后续节点未启动"
        );
        let task = app.tasks.iter_mut().find(|t| t.id == self.id).unwrap();
        let base = self.repo.resolve_snapshot(
            task.checkpoint
                .as_deref()
                .or(task.baseline.as_deref())
                .context("节点执行基线缺失")?,
        )?;
        let task_context=task.plan.as_ref().map(|p|json!({"objective":p.objective,"constraints":p.constraints,"acceptance":p.acceptance}));
        let node = &mut task.nodes[index];
        let message = format!(
            "仅执行当前节点，不要开始其他节点。不要运行测试、合并或部署。原始需求：{}\n任务总体契约：{}\n当前节点契约：{}\n明确补充意见：{}\n只可在节点 scope 内实现，越界需要停止并说明。完成后总结实现及未验证验收条件，不得宣称用户已批准。",
            serde_json::to_string(&task.prompt)?,
            serde_json::to_string(&task_context)?,
            serde_json::to_string(&node.contract)?,
            serde_json::to_string(extra)?
        );
        node.begin(&base.commit);
        task.active_node_id = Some(node.contract.id.clone());
        task.state = TaskState::Running;
        store.save_task(task)?;
        self.node_base = Some(base);
        self.phase = Phase::Node(index);
        self.finished = false;
        self.waiting = false;
        self.settled = false;
        app.run = Default::default();
        app.run.status = RunStatus::Running;
        self.pending_echoes.push_back(message.clone());
        self.send_prompt(&message, app)?;
        store.finish_run(&self.id, "Running", self.checkpoint.as_deref())?;
        app.status = format!(
            "节点 {} 执行中 · 验收未验证",
            app.tasks.iter().find(|t| t.id == self.id).unwrap().nodes[index]
                .contract
                .title
        );
        Ok(())
    }
    fn start_next(&mut self, app: &mut App, store: &mut WorkspaceStore) -> Result<()> {
        let task = app.tasks.iter().find(|t| t.id == self.id).unwrap();
        let Some(index) = task.next_ready() else {
            ensure!(
                task.nodes
                    .iter()
                    .all(|n| n.execution == Execution::Implemented),
                "依赖未完成，无法继续节点调度"
            );
            self.finished = true;
            app.run.status = RunStatus::Settled;
            app.status = "所有节点实现结束 · 待审，验收未验证".into();
            store.finish_run(&self.id, "Settled", self.checkpoint.as_deref())?;
            return self.sync_state(app, store);
        };
        if task.pending_reviews() >= 2 {
            self.remaining = self.deadline.saturating_duration_since(Instant::now());
            self.waiting = true;
            app.run.status = RunStatus::Paused;
            app.status = "节点边界已暂停 · 已有两个待审节点；明确批准后 p 继续".into();
            store.finish_run(&self.id, "Paused", self.checkpoint.as_deref())?;
            return self.sync_state(app, store);
        }
        self.begin_node(index, "", app, store)
    }
    fn finish_node(
        &mut self,
        index: usize,
        app: &mut App,
        store: &mut WorkspaceStore,
    ) -> Result<()> {
        let status = app.run.status.clone();
        if status != RunStatus::Settled {
            self.client.shutdown()?;
            self.closed = true;
            app.agent_connected = false;
        }
        // Rework remains a full delivery of THIS node, not merely its latest patch.
        // Actual attempt start is separately recorded; only the active/latest node
        // can be reworked so downstream node changes cannot leak into this diff.
        let review_base = app.tasks.iter().find(|t| t.id == self.id).unwrap().nodes[index]
            .attempts
            .first()
            .context("节点开始版本缺失")?
            .base
            .clone();
        let captured = self
            .repo
            .resolve_snapshot(&review_base)
            .and_then(|base| self.repo.capture_worktree_from(&base));
        let result = (|| -> Result<()> {
            let snapshot = captured?;
            let changes = self.repo.snapshot_changes(&snapshot)?;
            let task = app.tasks.iter_mut().find(|t| t.id == self.id).unwrap();
            let node = &mut task.nodes[index];
            let violations: Vec<_> = changes
                .iter()
                .filter(|c| !node.contract.contains_path(&c.path))
                .map(|c| c.path.clone())
                .collect();
            node.execution = match status {
                RunStatus::Settled if violations.is_empty() => Execution::Implemented,
                RunStatus::Aborted => Execution::Cancelled,
                _ => Execution::Failed,
            };
            let attempt = node.attempts.last_mut().context("节点执行记录缺失")?;
            attempt.execution = node.execution;
            attempt.checkpoint = Some(snapshot.commit.clone());
            attempt.summary = app.run.text.clone();
            attempt.scope_violations = violations.clone();
            task.checkpoint = Some(snapshot.commit.clone());
            self.checkpoint = Some(snapshot.commit.clone());
            store.save_task(task)?;
            app.append_node_checkpoint(&self.repo, &snapshot, &self.id, index)?;
            if !violations.is_empty() {
                anyhow::bail!(
                    "节点发生仓内范围越界：{}；后续节点未启动",
                    violations.join("、")
                );
            }
            if status != RunStatus::Settled {
                self.finished = true;
                app.status = "节点已停止 · 已有改动留在固定检查点，未验收".into();
                store.finish_run(&self.id, &format!("{status:?}"), self.checkpoint.as_deref())?;
                return Ok(());
            }
            self.start_next(app, store)
        })();
        if let Err(error) = result {
            let node = &mut app
                .tasks
                .iter_mut()
                .find(|t| t.id == self.id)
                .unwrap()
                .nodes[index];
            if node.execution != Execution::Implemented {
                node.execution = Execution::Failed;
                if let Some(attempt) = node.attempts.last_mut() {
                    attempt.execution = Execution::Failed;
                }
            }
            self.fail_workflow(error, app, store)?;
        }
        Ok(())
    }

    pub fn is_waiting(&self) -> bool {
        self.waiting
    }
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    pub fn cancel(&mut self, app: &mut App) -> Result<()> {
        if self.waiting {
            self.client.shutdown()?;
            self.closed = true;
            app.agent_connected = false;
            app.run.status = RunStatus::Aborted;
            return Ok(());
        }
        if !self.finished && !self.closed {
            self.begin_control(false, app)?;
        }
        Ok(())
    }

    pub fn stop(&mut self, app: &mut App, store: &mut WorkspaceStore) -> Result<()> {
        if !self.finished {
            // Exit never resumes queued work; close stdin disposes the runtime.
            self.client.shutdown()?;
            app.run.status = RunStatus::Aborted;
            store.finish_run(&self.id, "Interrupted", self.checkpoint.as_deref())?;
            self.finished = true;
        } else {
            self.client.shutdown()?;
        }
        self.closed = true;
        app.agent_connected = false;
        if let Some(task) = app.tasks.iter_mut().find(|t| t.id == self.id) {
            if app.run.status == RunStatus::Aborted && task.state != TaskState::Cancelled {
                task.interrupt();
            }
            store.save_task(task)?;
        }
        Ok(())
    }
}
