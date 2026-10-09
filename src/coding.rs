use crate::{
    app::{App, ReadingItem},
    checkpoint::Repository,
    rpc::{PiClient, RunStatus},
    storage::WorkspaceStore,
};
use anyhow::{Context, Result, ensure};
use serde_json::json;
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub struct CodingRun {
    pub client: PiClient,
    pub worktree: PathBuf,
    pub branch: String,
    pub session: String,
    pub id: String,
    pub checkpoint: Option<String>,
    repo: Repository,
    deadline: Instant,
    finished: bool,
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
        let sessions = store.directory().join("sessions");
        std::fs::create_dir_all(&sessions)?;
        ensure!(
            !std::fs::symlink_metadata(&sessions)?
                .file_type()
                .is_symlink(),
            "sessions 不允许符号链接"
        );
        let mut client = PiClient::spawn_coder(executable, &worktree, &sessions, model)?;
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
        app.agent_connected = true;
        app.task = Some(prompt.into());
        app.history.clear();
        app.run.text.clear();
        app.run.error = None;
        app.run.status = RunStatus::Running;
        let request = client.send(json!({"type":"prompt","message":prompt}))?;
        app.run.register(&request, "prompt");
        app.status = "编码中 · 当前阅读基线".into();
        Ok(Self {
            client,
            worktree,
            branch,
            session,
            id,
            repo,
            deadline: Instant::now() + timeout,
            finished: false,
            checkpoint: None,
        })
    }

    pub fn poll(&mut self, app: &mut App, store: &mut WorkspaceStore) -> Result<()> {
        if self.finished {
            return Ok(());
        }
        for _ in 0..128 {
            match self.client.events.try_recv() {
                Ok(Ok(record)) => {
                    if record["type"] == "extension_ui_request" {
                        self.client.dismiss_dialog(&record)?;
                    }
                    if record["type"] == "message_end" {
                        let message = &record["message"];
                        if let Some(role @ ("user" | "assistant")) = message["role"].as_str() {
                            let content = if let Some(text) = message["content"].as_str() {
                                text.to_owned()
                            } else {
                                message["content"]
                                    .as_array()
                                    .into_iter()
                                    .flatten()
                                    .filter(|b| b["type"] == "text")
                                    .filter_map(|b| b["text"].as_str())
                                    .collect::<Vec<_>>()
                                    .join("\n")
                            };
                            store.append_message(&self.session, role, &content)?;
                            app.history.push(format!("{role}\n{content}"));
                        }
                    }
                    app.run.apply(&record);
                    if record["type"] == "agent_settled" {
                        break;
                    }
                }
                Ok(Err(error)) => {
                    app.run.status = RunStatus::Failed;
                    app.run.error = Some(error.to_string());
                    break;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    app.run.status = RunStatus::Failed;
                    break;
                }
            }
        }
        if Instant::now() >= self.deadline
            && matches!(app.run.status, RunStatus::Running | RunStatus::Cancelling)
        {
            self.cancel(app)?;
            self.client.shutdown()?;
            app.run.status = RunStatus::Aborted;
            app.status = "超时 · 工作区已保留".into();
        }
        if matches!(
            app.run.status,
            RunStatus::Settled | RunStatus::Failed | RunStatus::Aborted | RunStatus::Idle
        ) {
            // Close the agent before capturing. No agent writer can race the snapshot.
            self.client.shutdown()?;
            self.finished = true;
            let paths = self.repo.changed_paths()?;
            if !paths.is_empty() {
                let snapshot = self.repo.capture(&paths)?;
                let item = ReadingItem {
                    title: "变更".into(),
                    version: snapshot.commit.clone(),
                    intent: app.task.clone().unwrap_or_default(),
                    code: self.repo.diff(&snapshot)?,
                    evidence: "测试未运行".into(),
                    explanation: app.run.text.clone(),
                    read: false,
                    scroll: 0,
                    evidence_scroll: 0,
                };
                // Explicit CLI tasks are not a continuous producer. Always retain delivery;
                // scheduler backpressure for multi-stage tasks is handled separately.
                app.items.push(item);
                self.checkpoint = Some(snapshot.commit);
                app.status = "改动待审 · 测试未运行".into();
            } else {
                app.status = "任务结束 · 无代码改动".into();
            }
            if app.run.status == RunStatus::Failed {
                app.status = format!(
                    "运行失败 · {}",
                    app.run.error.as_deref().unwrap_or("Pi 连接中断")
                );
            }
            store.finish_run(
                &self.id,
                &format!("{:?}", app.run.status),
                self.checkpoint.as_deref(),
            )?;
        }
        Ok(())
    }

    pub fn is_finished(&self) -> bool {
        self.finished
    }

    pub fn cancel(&mut self, app: &mut App) -> Result<()> {
        if !self.finished {
            let clear = self.client.send(json!({"type":"clear_queue"}))?;
            app.run.register(&clear, "clear_queue");
            let abort = self.client.send(json!({"type":"abort"}))?;
            app.run.register(&abort, "abort");
            app.run.status = RunStatus::Cancelling;
        }
        Ok(())
    }

    pub fn stop(&mut self, app: &mut App, store: &mut WorkspaceStore) -> Result<()> {
        if !self.finished {
            self.cancel(app)?;
            self.client.shutdown()?;
            app.run.status = RunStatus::Aborted;
            store.finish_run(&self.id, "Interrupted", None)?;
            self.finished = true;
        }
        Ok(())
    }
}
