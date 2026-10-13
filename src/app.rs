use crate::{
    checkpoint::{ChangeKind, Repository, Snapshot},
    keymap::{Action, Keymap},
    nodes::{Execution, NodeContract, NodeRecord, Review, TaskRecord, TaskState, Verification},
    rpc::{RunState, RunStatus},
    storage::Message,
};
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum View {
    Tasks,
    Review,
    Project,
    Knowledge,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Panel {
    Queue,
    Reader,
    Evidence,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Page {
    Intent,
    Code,
    Evidence,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    Browse,
    ConfirmEmbeddedEditor {
        selection: crate::embedded::Selection,
    },
    ConfirmStopEditor,
    ReviewChat {
        text: String,
    },
    ConfirmReviewPatch {
        proposal: Box<crate::review_editor::Proposal>,
    },
    Help,
    Input {
        search: bool,
        text: String,
    },
    Message {
        text: String,
    },
    MergePreview {
        preview: crate::merge::MergePreview,
        scroll: usize,
    },
    ConfirmMerge {
        preview: crate::merge::MergePreview,
    },
    ConfirmRead,
    ConfirmStart {
        task_id: String,
        plan: String,
    },
    ConfirmDelete {
        selection: TaskSelection,
    },
    Conflict {
        draft: crate::conflict::Draft,
        index: usize,
        side: u8,
        scroll: usize,
        body: String,
    },
    ConfirmConflict {
        draft: crate::conflict::Draft,
        index: usize,
        kind: ConflictKind,
    },
    ConfirmRetry {
        task_id: String,
        failure: String,
    },
    ConfirmCode {
        prompt: String,
    },
    ConfirmNodeApproval {
        task_id: String,
        node_id: String,
        checkpoint: String,
    },
}

/// Owned, domain-only snapshot for contract readers. No terminal styles or
/// Markdown parsing; existing plain-text APIs remain available unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeContractContent {
    pub task_id: String,
    pub contract: NodeContract,
    pub task_constraints: Vec<String>,
    pub execution: Execution,
    pub verification: Verification,
    pub review: Review,
    pub start_version: Option<String>,
    pub completion_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConflictKind {
    View(u8),
    Choose(u8),
    Edit,
    Mark,
    Finish,
    Rebuild,
    Advise,
    ApplyAI,
    StopAdvisor,
    Lockfile,
}
#[derive(Debug, Clone)]
pub struct ConflictRequest {
    pub draft: crate::conflict::Draft,
    pub index: usize,
    pub kind: ConflictKind,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunControl {
    TogglePause,
    Pause,
    Resume,
    Say(String),
}

#[derive(Debug, Clone)]
pub struct ReadingItem {
    pub title: String,
    pub version: String,
    pub intent: String,
    pub code: String,
    pub diff: String,
    pub change: Option<ChangeKind>,
    pub evidence: String,
    pub explanation: String,
    pub read: bool,
    pub scroll: u16,
    pub evidence_scroll: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskSelection {
    pub task_id: String,
    pub node_id: Option<String>,
}

pub struct App {
    pub helix_review: bool,
    pub requested_review_helix: bool,
    pub requested_helix_context: bool,
    pub helix_unknown: bool,
    pub review_compare: bool,
    pub project_tree: crate::file_tree::Tree,
    pub requested_tree_refresh: bool,
    pub requested_project_open: Option<String>,
    pub config: crate::config::Config,
    pub review_buffers: Vec<crate::review_editor::Buffer>,
    pub review_revision: u64,
    pub review_chat_open: bool,
    pub review_chat_scroll: Option<usize>,
    pub review_chat_context: Option<crate::review_editor::Context>,
    pub review_messages: Vec<crate::review_editor::ChatMessage>,
    pub review_chat_text: String,
    pub review_proposal: Option<crate::review_editor::Proposal>,
    pub requested_review_chat: Option<(crate::review_editor::Context, String)>,
    pub requested_review_publish: bool,
    pub review_chat_busy: bool,
    pub editor_screen: Option<vt100::Screen>,
    pub editor_focused: bool,
    pub editor_project: bool,
    pub editor_conflict: bool,
    pub requested_editor: Option<crate::embedded::Selection>,
    pub requested_editor_stop: bool,
    pub name: String,
    pub content_revision: u64,
    pub branch: String,
    pub project_version: Option<String>,
    pub latest_checkpoint: Option<String>,
    pub items: Vec<ReadingItem>,
    pub selected: usize,
    pub view: View,
    pub panel: Panel,
    pub page: Page,
    pub mode: Mode,
    pub fullscreen: bool,
    pub explain: bool,
    pub light: bool,
    pub status: String,
    pub filter: String,
    pub run: RunState,
    pub agent_connected: bool,
    pub cancel_requested: bool,
    pub history: Vec<Message>,
    pub task_scroll: u16,
    pub task_evidence_scroll: u16,
    pub queued_corrections: usize,
    pub controls: Vec<RunControl>,
    pub task: Option<String>,
    pub tasks: Vec<TaskRecord>,
    pub tasks_revision: u64,
    pub task_selection: Option<TaskSelection>,
    pub review_version: Option<String>,
    pub requested_task: Option<String>,
    pub requested_start: Option<(String, String)>,
    pub requested_delete: Option<TaskSelection>,
    pub planning_only: bool,
    pub requested_conflict: Option<ConflictRequest>,
    pub requested_retry: Option<(String, String)>,
    pub requested_merge_preview: Option<String>,
    pub requested_merge: Option<crate::merge::MergePreview>,
    pub keymap: Keymap,
    pub quit: bool,
    pub notes: Vec<String>,
}

fn task_state_label(s: TaskState) -> &'static str {
    match s {
        TaskState::Merged => "已合并",
        TaskState::Planning => "讨论规划中",
        TaskState::Ready => "计划待启动",
        TaskState::Running => "运行中",
        TaskState::Paused => "已暂停",
        TaskState::Implemented => "实现结束",
        TaskState::Failed => "失败",
        TaskState::Cancelled => "已取消",
        TaskState::Interrupted => "已中断",
    }
}
fn execution_label(s: Execution) -> &'static str {
    match s {
        Execution::Planned => "未开始",
        Execution::Running => "执行中",
        Execution::Paused => "已暂停",
        Execution::Implemented => "实现完成",
        Execution::NoChanges => "未交付（无文件改动）",
        Execution::Failed => "失败",
        Execution::Cancelled => "已取消",
        Execution::Interrupted => "已中断",
    }
}
fn verification_label(s: Verification) -> &'static str {
    match s {
        Verification::Unverified => "未验证",
        Verification::Passed => "通过",
        Verification::Failed => "失败",
    }
}
fn review_label(s: Review) -> &'static str {
    match s {
        Review::Pending => "待审",
        Review::Approved => "已批准",
        Review::ChangesRequested => "要求返工",
    }
}

impl App {
    pub fn tr<'a>(&self, zh: &'a str, en: &'a str) -> &'a str {
        self.config.language.text(zh, en)
    }
    /// Describes only this workbench's current RPC session, not external Pi
    /// availability or the selected historical task. This never starts a session.
    pub fn pi_session_label(&self) -> &'static str {
        if self.config.language == crate::config::Language::English {
            return match (self.agent_connected, &self.run.status) {
                (false, RunStatus::Idle) => "Pi session not started",
                (false, RunStatus::Settled) => "Pi ended · ready for review",
                (false, RunStatus::Aborted) => "Pi ended · cancelled",
                (false, RunStatus::Failed) => "Pi failed · disconnected",
                (false, RunStatus::Running) => "Pi disconnected · running",
                (false, RunStatus::Pausing) => "Pi disconnected · confirming pause",
                (false, RunStatus::Paused) => "Pi disconnected · paused",
                (false, RunStatus::Cancelling) => "Pi disconnected · cancelling",
                (true, RunStatus::Idle) => "Pi connected · idle",
                (true, RunStatus::Running) => "Pi connected · running",
                (true, RunStatus::Pausing) => "Pi connected · confirming pause",
                (true, RunStatus::Paused) => "Pi connected · paused",
                (true, RunStatus::Cancelling) => "Pi connected · cancelling",
                (true, RunStatus::Settled) => "Pi connected · idle · ready for review",
                (true, RunStatus::Aborted) => "Pi connected · idle · cancelled",
                (true, RunStatus::Failed) => "Pi failed · connected",
            };
        }
        match (self.agent_connected, &self.run.status) {
            (false, RunStatus::Idle) => "Pi 会话未启动",
            (false, RunStatus::Settled) => "Pi 会话已结束 · 本轮结束待审核",
            (false, RunStatus::Aborted) => "Pi 会话已结束 · 已取消",
            (false, RunStatus::Failed) => "Pi 会话失败 · 未连接",
            (false, RunStatus::Running) => "Pi 会话未连接 · 运行中",
            (false, RunStatus::Pausing) => "Pi 会话未连接 · 暂停中（等待确认）",
            (false, RunStatus::Paused) => "Pi 会话未连接 · 已暂停",
            (false, RunStatus::Cancelling) => "Pi 会话未连接 · 取消中",
            (true, RunStatus::Idle) => "Pi 会话已连接 · 空闲",
            (true, RunStatus::Running) => "Pi 会话已连接 · 运行中",
            (true, RunStatus::Pausing) => "Pi 会话已连接 · 暂停中（等待确认）",
            (true, RunStatus::Paused) => "Pi 会话已连接 · 已暂停",
            (true, RunStatus::Cancelling) => "Pi 会话已连接 · 取消中",
            (true, RunStatus::Settled) => "Pi 会话已连接 · 空闲 · 本轮结束待审核",
            (true, RunStatus::Aborted) => "Pi 会话已连接 · 空闲 · 已取消",
            (true, RunStatus::Failed) => "Pi 会话失败 · 已连接",
        }
    }

    pub fn empty(keymap: Keymap) -> Self {
        Self {
            helix_review: false,
            requested_review_helix: false,
            requested_helix_context: false,
            helix_unknown: false,
            review_compare: false,
            project_tree: crate::file_tree::Tree::default(),
            requested_tree_refresh: false,
            requested_project_open: None,
            config: crate::config::Config::default(),
            review_buffers: Vec::new(),
            review_revision: 0,
            review_chat_open: false,
            review_chat_scroll: None,
            review_chat_context: None,
            review_messages: Vec::new(),
            review_chat_text: String::new(),
            review_proposal: None,
            requested_review_chat: None,
            requested_review_publish: false,
            review_chat_busy: false,
            editor_screen: None,
            editor_focused: false,
            editor_project: false,
            editor_conflict: false,
            requested_editor: None,
            requested_editor_stop: false,
            name: "工作台".into(),
            content_revision: 0,
            branch: String::new(),
            project_version: None,
            latest_checkpoint: None,
            items: Vec::new(),
            selected: 0,
            view: View::Review,
            panel: Panel::Reader,
            page: Page::Code,
            mode: Mode::Browse,
            fullscreen: false,
            explain: true,
            light: false,
            status: String::new(),
            filter: String::new(),
            run: RunState::default(),
            agent_connected: false,
            cancel_requested: false,
            history: Vec::new(),
            task_scroll: 0,
            task_evidence_scroll: 0,
            queued_corrections: 0,
            controls: Vec::new(),
            task: None,
            tasks: Vec::new(),
            tasks_revision: 0,
            task_selection: None,
            review_version: None,
            requested_task: None,
            requested_start: None,
            requested_delete: None,
            planning_only: true,
            requested_conflict: None,
            requested_retry: None,
            requested_merge_preview: None,
            requested_merge: None,
            keymap,
            quit: false,
            notes: Vec::new(),
        }
    }

    pub fn browse(repo: &Repository, snapshot: Snapshot, keymap: Keymap) -> Result<Self> {
        let files = repo.files(&snapshot)?;
        let mut app = Self::empty(keymap);
        app.name = repo
            .root()
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into();
        app.branch = snapshot.commit[..12].into();
        app.project_version = Some(snapshot.commit.clone());
        app.items = files
            .iter()
            .take(80)
            .map(|path| ReadingItem {
                title: path.clone(),
                version: snapshot.commit.clone(),
                intent: String::new(),
                diff: String::new(),
                change: None,
                code: repo
                    .read(&snapshot, path)
                    .unwrap_or_else(|error| format!("无法打开：{error}")),
                evidence: String::new(),
                explanation: String::new(),
                read: false,
                scroll: 0,
                evidence_scroll: 0,
            })
            .collect();
        if snapshot.parent.is_some() {
            app.append_checkpoint(repo, &snapshot)?;
        }
        if repo.is_dirty()? {
            app.status = "工作区有未提交改动 · 当前为固定快照".into();
        }
        Ok(app)
    }

    /// Retain immutable versions internally; the sidebar projects one row per path.
    /// Incoming versions never replace the user's selected historical reading item.
    pub fn append_checkpoint(&mut self, repo: &Repository, snapshot: &Snapshot) -> Result<()> {
        let changes = repo.snapshot_changes(snapshot)?;
        let summary = self
            .history
            .iter()
            .rev()
            .find(|m| m.role == "assistant")
            .map(|m| m.content.clone())
            .unwrap_or_default();
        let mut delivery = Vec::new();
        for change in changes {
            let code = if change.deleted {
                "该文件在此检查点已删除。".into()
            } else {
                repo.read(snapshot, &change.path)
                    .unwrap_or_else(|error| format!("无法打开固定版本文件：{error}"))
            };
            delivery.push(ReadingItem {
                title: change.path.clone(),
                version: snapshot.commit.clone(),
                intent: self.task.clone().unwrap_or_default(),
                code,
                diff: repo.diff_file(snapshot, &change.path)?,
                change: Some(change.kind),
                evidence: "测试未运行".into(),
                explanation: summary.clone(),
                read: false,
                scroll: 0,
                evidence_scroll: 0,
            });
        }
        for item in delivery {
            if let Some(index) = self
                .items
                .iter()
                .position(|old| old.version == item.version && old.title == item.title)
            {
                if self.items[index].change.is_none() {
                    self.items[index] = item;
                }
            } else {
                self.items.push(item);
            }
        }
        self.latest_checkpoint = Some(snapshot.commit.clone());
        self.content_revision = self.content_revision.wrapping_add(1);
        Ok(())
    }

    pub fn append_node_checkpoint(
        &mut self,
        repo: &Repository,
        snapshot: &Snapshot,
        task_id: &str,
        index: usize,
    ) -> Result<()> {
        self.append_checkpoint(repo, snapshot)?;
        let task = self.tasks.iter().find(|t| t.id == task_id).unwrap();
        let node = &task.nodes[index];
        let attempt = node
            .attempts
            .iter()
            .find(|a| a.checkpoint.as_deref() == Some(&snapshot.commit));
        let intent = format!(
            "任务：{}\n节点：{}\n目标：{}\n范围：{}\n实现方法：{}\n约束：{}\n依赖：{}\n验收条件（未验证）：\n{}",
            task.prompt,
            node.contract.title,
            node.contract.objective,
            node.contract.scope.join("、"),
            node.contract.implementation,
            node.contract.constraints.join("；"),
            node.contract.dependencies.join("、"),
            node.contract.acceptance.join("\n")
        );
        for item in self
            .items
            .iter_mut()
            .filter(|i| i.version == snapshot.commit)
        {
            item.intent = intent.clone();
            item.explanation = attempt.map(|a| a.summary.clone()).unwrap_or_default();
            item.evidence = format!(
                "验收：{:?} · 人工审核：{:?}\n测试 Runner 未接入{}",
                node.verification,
                node.review,
                attempt
                    .filter(|a| !a.scope_violations.is_empty())
                    .map(|a| format!("\n仓内越界变更：{}", a.scope_violations.join("、")))
                    .unwrap_or_default()
            );
        }
        Ok(())
    }

    /// Live follow swaps only immutable snapshots of the explicitly selected node.
    pub fn publish_following_snapshot(
        &mut self,
        repo: &Repository,
        snapshot: &Snapshot,
        task_id: &str,
        index: usize,
    ) -> Result<()> {
        let follows = !self.current_buffer().is_some_and(|b| b.changed())
            && self.view == View::Review
            && self.task_selection.as_ref().is_some_and(|s| {
                s.task_id == task_id
                    && s.node_id.as_deref()
                        == Some(
                            self.tasks.iter().find(|t| t.id == task_id).unwrap().nodes[index]
                                .contract
                                .id
                                .as_str(),
                        )
            });
        let position = self
            .current()
            .map(|i| (i.title.clone(), i.scroll, i.evidence_scroll));
        self.append_node_checkpoint(repo, snapshot, task_id, index)?;
        if follows {
            self.review_version = Some(snapshot.commit.clone());
            let indices = self.changed_indices();
            if let Some(i) = position
                .as_ref()
                .and_then(|(p, _, _)| indices.iter().find(|&&i| self.items[i].title == *p))
                .or_else(|| indices.first())
            {
                self.selected = *i;
                if let Some((_, scroll, evidence)) = position {
                    self.items[*i].scroll = scroll;
                    self.items[*i].evidence_scroll = evidence;
                }
            }
        }
        Ok(())
    }
    pub fn refresh_project(&mut self, repo: &Repository) -> Result<()> {
        let snapshot = repo.baseline()?;
        let files = repo.files(&snapshot)?;
        let version = snapshot.commit.clone();
        let current_path = self.current().map(|i| i.title.clone());
        for path in files.iter().take(80) {
            if !self
                .items
                .iter()
                .any(|i| i.title == *path && i.version == version)
            {
                self.items.push(ReadingItem {
                    title: path.clone(),
                    version: version.clone(),
                    intent: String::new(),
                    diff: String::new(),
                    change: None,
                    code: repo
                        .read(&snapshot, path)
                        .unwrap_or_else(|e| format!("无法打开固定版本：{e}")),
                    evidence: String::new(),
                    explanation: String::new(),
                    read: false,
                    scroll: 0,
                    evidence_scroll: 0,
                });
            }
        }
        self.project_version = Some(version.clone());
        self.branch = version[..12].into();
        if self.view == View::Project
            && let Some(i) = self
                .items
                .iter()
                .position(|i| Some(&i.title) == current_path.as_ref() && i.version == version)
        {
            self.selected = i;
        }
        self.content_revision = self.content_revision.wrapping_add(1);
        Ok(())
    }
    pub fn selected_task(&self) -> Option<&TaskRecord> {
        self.task_selection
            .as_ref()
            .and_then(|s| self.tasks.iter().find(|t| t.id == s.task_id))
            .or_else(|| self.tasks.last())
    }
    pub fn selected_node(&self) -> Option<&NodeRecord> {
        let id = self.task_selection.as_ref()?.node_id.as_ref()?;
        self.selected_task()?
            .nodes
            .iter()
            .find(|n| &n.contract.id == id)
    }
    pub fn review_checkpoint(&self) -> Option<&str> {
        if let Some(version) = &self.review_version {
            return Some(version);
        }
        if let Some(node) = self.selected_node() {
            return node.review_snapshot();
        }
        self.selected_task()
            .and_then(|t| t.checkpoint.as_deref())
            .or(self.latest_checkpoint.as_deref())
    }
    pub fn task_rows(&self, review_only: bool) -> Vec<(TaskSelection, String)> {
        let mut rows = Vec::new();
        for task in self.tasks.iter().rev() {
            let nodes: Vec<_> = task
                .nodes
                .iter()
                .filter(|n| {
                    !review_only
                        || n.published_checkpoint().is_some()
                        || n.preview_checkpoint().is_some()
                        || matches!(n.execution, Execution::Running | Execution::Paused)
                })
                .collect();
            if review_only && nodes.is_empty() && (task.plan.is_some() || task.checkpoint.is_none())
            {
                continue;
            }
            rows.push((
                TaskSelection {
                    task_id: task.id.clone(),
                    node_id: None,
                },
                format!(
                    "{} · {}",
                    task.prompt.lines().next().unwrap_or("任务"),
                    if task.discussion_pending {
                        "讨论待补充"
                    } else {
                        task_state_label(task.state)
                    }
                ),
            ));
            for node in nodes {
                let marker = if node.review == Review::Approved {
                    "✓"
                } else {
                    match node.execution {
                        Execution::Running => "◐",
                        Execution::Implemented => "◇",
                        Execution::Failed => "×",
                        _ => "○",
                    }
                };
                rows.push((
                    TaskSelection {
                        task_id: task.id.clone(),
                        node_id: Some(node.contract.id.clone()),
                    },
                    format!(
                        "  {marker} {} · {}",
                        node.contract.title,
                        execution_label(node.execution)
                    ),
                ));
            }
        }
        rows
    }
    pub fn select_task_entry(&mut self, selection: TaskSelection) {
        self.task_selection = Some(selection);
        self.review_version = if let Some(node) = self.selected_node() {
            node.review_snapshot().map(str::to_owned)
        } else {
            self.selected_task().and_then(|t| t.checkpoint.clone())
        };
        if let Some(index) = self.changed_indices().first() {
            self.selected = *index;
        }
        self.task_scroll = 0;
        self.task_evidence_scroll = 0;
        self.filter.clear();
        self.content_revision = self.content_revision.wrapping_add(1);
    }
    fn move_task(&mut self, delta: isize) {
        let rows = self.task_rows(false);
        if rows.is_empty() {
            return;
        }
        let index = rows
            .iter()
            .position(|(s, _)| Some(s) == self.task_selection.as_ref())
            .map_or(0, |i| i.saturating_add_signed(delta).min(rows.len() - 1));
        self.select_task_entry(rows[index].0.clone());
    }
    fn move_node(&mut self, delta: isize) {
        let rows: Vec<_> = self
            .task_rows(self.view == View::Review)
            .into_iter()
            .filter(|(s, _)| {
                s.node_id.is_some()
                    || self
                        .tasks
                        .iter()
                        .find(|t| t.id == s.task_id)
                        .is_some_and(|t| t.nodes.is_empty())
            })
            .collect();
        if rows.is_empty() {
            return;
        }
        let index = rows
            .iter()
            .position(|(s, _)| Some(s) == self.task_selection.as_ref())
            .map_or(0, |i| i.saturating_add_signed(delta).min(rows.len() - 1));
        self.select_task_entry(rows[index].0.clone());
    }
    pub fn node_contract_content(&self) -> Option<NodeContractContent> {
        let node = self.selected_node()?;
        let task = self.selected_task()?;
        Some(NodeContractContent {
            task_id: task.id.clone(),
            contract: node.contract.clone(),
            task_constraints: task
                .plan
                .as_ref()
                .map(|p| p.constraints.clone())
                .unwrap_or_default(),
            execution: node.execution,
            verification: node.verification,
            review: node.review,
            start_version: node.attempts.last().map(|a| a.base.clone()),
            completion_version: node.review_snapshot().map(str::to_owned),
        })
    }

    /// Contract scrolling also works when a review node has no file item.
    pub fn showing_node_contract(&self) -> bool {
        self.selected_node().is_some()
            && (self.view == View::Tasks
                || (self.view == View::Review && self.page == Page::Intent))
    }

    pub fn node_description(&self) -> Option<String> {
        let node = self.selected_node()?;
        let task = self.selected_task()?;
        Some(format!(
            "节点：{} [{}]\n状态：{} · 验收：{} · 审核：{}\n目标：{}\n文件范围：{}\n边界条件：{}\n实现方法：{}\n任务约束：{}\n节点约束：{}\n依赖：{}\n验收条件（未验证）：\n{}\n开始版本：{}\n完成版本：{}",
            node.contract.title,
            node.contract.id,
            execution_label(node.execution),
            verification_label(node.verification),
            review_label(node.review),
            node.contract.objective,
            node.contract.scope.join("、"),
            node.contract.boundaries.join("；"),
            node.contract.implementation,
            task.plan
                .as_ref()
                .map(|p| p.constraints.join("；"))
                .unwrap_or_default(),
            node.contract.constraints.join("；"),
            node.contract.dependencies.join("、"),
            node.contract
                .acceptance
                .iter()
                .map(|c| format!("□ {c}"))
                .collect::<Vec<_>>()
                .join("\n"),
            node.attempts
                .last()
                .map(|a| a.base.as_str())
                .unwrap_or("尚未执行"),
            node.review_snapshot().unwrap_or("尚未生成")
        ))
    }
    pub fn node_evidence(&self) -> Option<String> {
        Some(
            self.node_evidence_blocks()?
                .into_iter()
                .map(|(text, _)| text)
                .collect::<Vec<_>>()
                .join("\n"),
        )
    }

    /// Trusted status records are separate from authored evidence/acceptance.
    /// `true` identifies literal application metadata, not markup instructions.
    pub fn node_evidence_blocks(&self) -> Option<Vec<(String, bool)>> {
        let node = self.selected_node()?;
        let version = self.review_checkpoint();
        let attempt = node.attempts.iter().find(|a| {
            a.checkpoint.as_deref() == version
                || version.is_some_and(|v| a.previews.iter().any(|p| p == v))
        });
        let mut blocks = vec![
            (
                format!(
                    "验收：{} · 人工审核：{}",
                    verification_label(attempt.map_or(node.verification, |a| a.verification)),
                    review_label(attempt.map_or(node.review, |a| a.review))
                ),
                true,
            ),
            (
                "测试 Runner 未接入；文档复选框不代表真实验证或人工批准。".into(),
                true,
            ),
        ];
        if let Some(a) = attempt.filter(|a| !a.scope_violations.is_empty()) {
            blocks.push((format!("仓内越界：{}", a.scope_violations.join("、")), true));
        }
        for (index, acceptance) in node.contract.acceptance.iter().enumerate() {
            blocks.push((format!("□ 未验证 · 验收条件 {}", index + 1), true));
            blocks.push((acceptance.clone(), false));
        }
        if self.explain
            && let Some(a) = attempt
        {
            blocks.push((a.summary.clone(), false));
        }
        Some(blocks)
    }
    fn request_node_approval(&mut self) {
        let Some(selection) = self.task_selection.clone() else {
            self.status = "请先选择待审节点".into();
            return;
        };
        let Some(node_id) = selection.node_id else {
            self.status = "请先选择待审节点".into();
            return;
        };
        let Some(node) = self.selected_node() else {
            return;
        };
        if node.execution == Execution::NoChanges {
            self.status =
                "节点未交付文件，不能把只读方案批准为实现；请纠正或重新提交修改任务".into();
            return;
        }
        let Some(checkpoint) = self.review_checkpoint() else {
            self.status = "节点尚无检查点".into();
            return;
        };
        if self
            .review_buffers
            .iter()
            .any(|b| b.task_id == selection.task_id && b.version == checkpoint && b.changed())
        {
            self.status="当前交付有人工草稿修改：先在代码区 Ctrl+s 发布新固定修订，再批准；不会批准可变缓冲区".into();
            return;
        }
        if node.execution != Execution::Implemented || node.checkpoint() != Some(checkpoint) {
            self.status = "只能批准当前已完成节点版本；旧版本或失败节点不能放行".into();
            return;
        }
        if node.review == Review::Approved
            && node.reviewed_checkpoint.as_deref() == Some(checkpoint)
        {
            self.status = "该节点版本已批准，仍可查看文件；未自动测试或合并".into();
            return;
        }
        self.mode = Mode::ConfirmNodeApproval {
            task_id: selection.task_id,
            node_id,
            checkpoint: checkpoint.into(),
        };
    }

    pub fn current(&self) -> Option<&ReadingItem> {
        self.items.get(self.selected)
    }
    pub fn pending(&self) -> usize {
        if self.tasks.iter().any(|t| !t.nodes.is_empty()) {
            return self.tasks.iter().map(|t| t.pending_reviews()).sum();
        }
        let changed = self.changed_indices();
        if changed.is_empty() {
            self.file_indices()
                .iter()
                .filter(|&&i| !self.items[i].read)
                .count()
        } else {
            changed.iter().filter(|&&i| !self.items[i].read).count()
        }
    }
    pub fn file_indices(&self) -> Vec<usize> {
        let mut paths = std::collections::BTreeMap::new();
        let version = if self.view == View::Review {
            self.review_checkpoint()
        } else {
            self.latest_checkpoint.as_deref()
        };
        for (index, item) in self.items.iter().enumerate() {
            if item.change.is_none() || version == Some(item.version.as_str()) {
                paths.insert(item.title.as_str(), index);
            }
        }
        if let Some(item) = self.current() {
            paths.entry(item.title.as_str()).or_insert(self.selected);
        }
        paths.into_values().collect()
    }
    pub fn changed_indices(&self) -> Vec<usize> {
        self.file_indices()
            .into_iter()
            .filter(|&i| {
                self.items[i].change.is_some()
                    && self.review_checkpoint() == Some(self.items[i].version.as_str())
            })
            .collect()
    }
    /// The sidebar and every file navigation action share this exact projection.
    pub fn visible_file_indices(&self) -> Vec<usize> {
        if self.view == View::Review {
            self.changed_indices()
        } else if self.view == View::Project {
            let mut paths = std::collections::BTreeMap::new();
            for (i, item) in self.items.iter().enumerate() {
                if self.project_version.as_deref() == Some(&item.version)
                    || (self.project_version.is_none() && item.change.is_none())
                {
                    paths.insert(&item.title, i);
                }
            }
            paths.into_values().collect()
        } else {
            self.file_indices()
        }
    }
    fn open_review(&mut self) {
        if self
            .task_selection
            .as_ref()
            .is_none_or(|s| s.node_id.is_none())
            && let Some(task) = self.selected_task()
            && let Some(node) = task
                .nodes
                .iter()
                .find(|n| matches!(n.execution, Execution::Running | Execution::Paused))
                .or_else(|| task.nodes.iter().find(|n| n.pending_review()))
                .or_else(|| {
                    task.nodes
                        .iter()
                        .find(|n| n.published_checkpoint().is_some())
                })
        {
            self.select_task_entry(TaskSelection {
                task_id: task.id.clone(),
                node_id: Some(node.contract.id.clone()),
            });
        }
        if self.selected_node().is_some() {
            self.review_version = self
                .selected_node()
                .and_then(|n| n.review_snapshot())
                .map(str::to_owned);
        }
        self.view = View::Review;
        if let Some(index) = self.changed_indices().first() {
            self.selected = *index;
        }
        self.view = View::Review;
        self.panel = Panel::Queue;
        self.page = Page::Code;
        self.filter.clear();
    }
    pub fn can_continue(&self) -> bool {
        self.pending() < 2
    }

    /// Add real deliveries without changing the user's selection or reading position.
    pub fn enqueue(&mut self, item: ReadingItem) -> bool {
        if !self.can_continue() {
            return false;
        }
        self.items.push(item);
        self.content_revision = self.content_revision.wrapping_add(1);
        true
    }

    pub fn has_review_draft(&self, task: &TaskRecord) -> bool {
        (self.helix_review && self.helix_unknown)
            || self.review_buffers.iter().any(|b| {
                b.task_id == task.id
                    && b.changed()
                    && (task.checkpoint.as_deref() == Some(b.version.as_str())
                        || task
                            .nodes
                            .iter()
                            .any(|n| n.review_snapshot() == Some(b.version.as_str())))
            })
    }
    pub fn current_buffer(&self) -> Option<&crate::review_editor::Buffer> {
        let item = self.current()?;
        let task = self.selected_task()?;
        let node = self
            .task_selection
            .as_ref()
            .and_then(|s| s.node_id.as_deref());
        self.review_buffers
            .iter()
            .find(|b| b.matches(&task.id, node, &item.title, &item.version))
    }
    pub fn current_buffer_mut(&mut self) -> Option<&mut crate::review_editor::Buffer> {
        let item = self.current()?;
        let task = self.selected_task()?.id.clone();
        let path = item.title.clone();
        let version = item.version.clone();
        let node = self.task_selection.as_ref().and_then(|s| s.node_id.clone());
        self.review_buffers
            .iter_mut()
            .find(|b| b.matches(&task, node.as_deref(), &path, &version))
    }
    pub fn open_review_chat(&mut self) {
        if self.helix_review {
            if self.editor_screen.is_some() {
                self.requested_helix_context = true;
            } else {
                self.status =
                    "请先按 2 / Tab 打开真正的 Helix，再 Ctrl+g 引用实际选区；未发送代码".into();
            }
            return;
        }
        if let Some(buffer) = self.current_buffer() {
            self.review_chat_context = Some(buffer.context());
            self.review_chat_open = true;
            self.mode = Mode::ReviewChat {
                text: String::new(),
            };
            self.panel = Panel::Evidence;
        }
    }
    pub fn handle_key(&mut self, key: KeyEvent) {
        if key.kind == KeyEventKind::Release {
            return;
        }
        if self.view == View::Review
            && key.modifiers.contains(KeyModifiers::CONTROL)
            && key.code == KeyCode::Char('x')
        {
            self.cancel_requested = true;
            return;
        }
        if self.view == View::Review
            && self.review_chat_open
            && self.panel == Panel::Evidence
            && matches!(self.mode, Mode::Browse)
        {
            match key.code {
                KeyCode::Char('i') => {
                    self.mode = Mode::ReviewChat {
                        text: String::new(),
                    };
                    return;
                }
                KeyCode::Esc => {
                    self.panel = Panel::Reader;
                    self.editor_focused = self.editor_screen.is_some() && !self.review_compare;
                    return;
                }
                KeyCode::Char('j') | KeyCode::Down => {
                    self.review_chat_scroll =
                        Some(self.review_chat_scroll.unwrap_or(0).saturating_add(1));
                    return;
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    self.review_chat_scroll =
                        Some(self.review_chat_scroll.unwrap_or(0).saturating_sub(1));
                    return;
                }
                KeyCode::Home => {
                    self.review_chat_scroll = Some(0);
                    return;
                }
                KeyCode::End => {
                    self.review_chat_scroll = None;
                    return;
                }
                _ => {}
            }
        }
        if self.view == View::Review
            && self.editor_screen.is_none()
            && matches!(self.mode, Mode::Browse)
        {
            if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('g') {
                self.open_review_chat();
                return;
            }
            if self.helix_review && self.panel == Panel::Reader && self.selected_task().is_some() {
                if matches!(key.code, KeyCode::Char('1' | '2' | '3' | '4')) && !self.review_compare
                {
                    if let Some(action) = self.keymap.resolve(key) {
                        self.act(action);
                    }
                    return;
                }
                if self.review_compare
                    && matches!(
                        key.code,
                        KeyCode::Char('j' | 'k')
                            | KeyCode::Up
                            | KeyCode::Down
                            | KeyCode::PageUp
                            | KeyCode::PageDown
                    )
                {
                    if let Some(b) = self.current_buffer_mut() {
                        let down = matches!(
                            key.code,
                            KeyCode::Char('j') | KeyCode::Down | KeyCode::PageDown
                        );
                        let n = if matches!(key.code, KeyCode::PageUp | KeyCode::PageDown) {
                            15
                        } else {
                            1
                        };
                        b.top = if down {
                            b.top.saturating_add(n).min(b.rows.len().saturating_sub(1))
                        } else {
                            b.top.saturating_sub(n)
                        };
                    }
                    return;
                }
                if key.code == KeyCode::Tab {
                    self.act(Action::NextPanel);
                } else if key.code == KeyCode::Esc
                    || (key.modifiers.contains(KeyModifiers::CONTROL)
                        && matches!(key.code, KeyCode::Char('d' | ']' | '5')))
                {
                    self.review_compare = false;
                    if key.code == KeyCode::Char('d') || key.code == KeyCode::Esc {
                        self.editor_focused = self.editor_screen.is_some();
                        self.panel = if self.editor_focused {
                            Panel::Reader
                        } else {
                            Panel::Queue
                        };
                    } else {
                        self.panel = Panel::Queue;
                    }
                }
                return;
            }
            if self.panel == Panel::Reader && self.current_buffer().is_some() {
                let action = self.current_buffer_mut().unwrap().handle(key);
                self.review_revision = self.review_revision.wrapping_add(1);
                match action {
                    Ok(crate::review_editor::Action::Ask) => self.open_review_chat(),
                    Ok(crate::review_editor::Action::Publish) => {
                        self.requested_review_publish = true
                    }
                    Ok(crate::review_editor::Action::Apply) => {
                        if let Some(proposal) = &self.review_proposal {
                            self.mode = Mode::ConfirmReviewPatch {
                                proposal: Box::new(proposal.clone()),
                            };
                        } else {
                            self.status = "尚无可应用补丁；Ctrl+g 提问，AI 默认只讨论".into();
                        }
                    }
                    Ok(crate::review_editor::Action::Workbench) => self.panel = Panel::Queue,
                    Err(error) => self.status = error.to_string(),
                    _ => {}
                }
                return;
            }
        }
        match &mut self.mode {
            Mode::ReviewChat { text } => {
                match key.code {
                    KeyCode::Esc => {
                        self.mode = Mode::Browse;
                        self.panel = Panel::Reader;
                        self.editor_focused = self.editor_screen.is_some() && !self.review_compare;
                    }
                    KeyCode::Enter if !text.trim().is_empty() => {
                        if let Some(context) = self.review_chat_context.clone() {
                            self.requested_review_chat = Some((context, std::mem::take(text)));
                            self.mode = Mode::Browse;
                        }
                    }
                    KeyCode::Backspace => {
                        text.pop();
                    }
                    KeyCode::Char(c)
                        if !key
                            .modifiers
                            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                    {
                        text.push(c)
                    }
                    _ => {}
                }
                return;
            }
            Mode::ConfirmReviewPatch { proposal } => {
                if key.code == KeyCode::Enter {
                    let proposal = proposal.clone();
                    let result = self
                        .current_buffer_mut()
                        .ok_or_else(|| anyhow::anyhow!("缓冲区选择变化"))
                        .and_then(|b| b.apply(&proposal));
                    self.status = match result {
                        Ok(()) => "建议已写入人工草稿；未批准/发布/合入".into(),
                        Err(e) => e.to_string(),
                    };
                    self.review_revision = self.review_revision.wrapping_add(1);
                    self.mode = Mode::Browse;
                    self.panel = Panel::Reader;
                } else if key.code == KeyCode::Esc {
                    self.mode = Mode::Browse;
                    self.panel = Panel::Reader;
                    self.editor_focused = self.editor_screen.is_some() && self.helix_review;
                }
                return;
            }
            Mode::Message { text } => {
                match key.code {
                    KeyCode::Esc => self.mode = Mode::Browse,
                    KeyCode::Enter if !text.trim().is_empty() => {
                        self.controls.push(RunControl::Say(std::mem::take(text)));
                        self.mode = Mode::Browse;
                    }
                    KeyCode::Backspace => {
                        text.pop();
                    }
                    KeyCode::Char(c)
                        if !key
                            .modifiers
                            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                    {
                        text.push(c)
                    }
                    _ => {}
                }
                return;
            }
            Mode::Input { search, text } => {
                match key.code {
                    KeyCode::Esc => self.mode = Mode::Browse,
                    KeyCode::Enter => {
                        let is_search = *search;
                        let value = std::mem::take(text);
                        self.mode = Mode::Browse;
                        if is_search {
                            self.filter = value;
                            if self.view == View::Tasks || self.showing_node_contract() {
                                self.task_scroll = 0;
                            }
                            if let Some(item) = self.items.get_mut(self.selected) {
                                item.scroll = 0;
                            }
                        } else {
                            self.command(&value);
                        }
                    }
                    KeyCode::Backspace => {
                        text.pop();
                    }
                    KeyCode::Char(c)
                        if !key
                            .modifiers
                            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                    {
                        text.push(c)
                    }
                    _ => {}
                }
                return;
            }
            Mode::Help => {
                if key.code == KeyCode::Esc || self.keymap.resolve(key) == Some(Action::Help) {
                    self.mode = Mode::Browse;
                }
                return;
            }
            Mode::ConfirmStart { task_id, plan } => {
                if key.code == KeyCode::Enter {
                    self.requested_start = Some((task_id.clone(), plan.clone()));
                    self.mode = Mode::Browse;
                } else if key.code == KeyCode::Esc {
                    self.mode = Mode::Browse;
                }
                return;
            }
            Mode::ConfirmDelete { selection } => {
                if key.code == KeyCode::Enter {
                    self.requested_delete = Some(selection.clone());
                    self.mode = Mode::Browse;
                } else if key.code == KeyCode::Esc {
                    self.mode = Mode::Browse;
                }
                return;
            }
            Mode::MergePreview { preview, scroll } => {
                match key.code {
                    KeyCode::Esc => self.mode = Mode::Browse,
                    KeyCode::Enter => {
                        self.mode = Mode::ConfirmMerge {
                            preview: preview.clone(),
                        }
                    }
                    KeyCode::Char('j') | KeyCode::Down => {
                        *scroll = scroll
                            .saturating_add(1)
                            .min(preview.diff.lines().count().saturating_sub(1))
                    }
                    KeyCode::Char('k') | KeyCode::Up => *scroll = scroll.saturating_sub(1),
                    KeyCode::PageDown => {
                        *scroll = scroll
                            .saturating_add(12)
                            .min(preview.diff.lines().count().saturating_sub(1))
                    }
                    KeyCode::PageUp => *scroll = scroll.saturating_sub(12),
                    _ => {}
                }
                return;
            }
            Mode::ConfirmMerge { preview } => {
                if key.code == KeyCode::Enter {
                    self.requested_merge = Some(preview.clone());
                    self.mode = Mode::Browse;
                } else if key.code == KeyCode::Esc {
                    self.mode = Mode::Browse;
                }
                return;
            }
            Mode::Conflict {
                draft,
                index,
                side,
                scroll,
                body,
            } => {
                let kind = match key.code {
                    KeyCode::Esc => {
                        self.mode = Mode::Browse;
                        return;
                    }
                    KeyCode::Char('j') | KeyCode::Down => {
                        *scroll = scroll
                            .saturating_add(1)
                            .min(body.lines().count().saturating_sub(1));
                        return;
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        *scroll = scroll.saturating_sub(1);
                        return;
                    }
                    KeyCode::PageDown => {
                        *scroll = scroll
                            .saturating_add(12)
                            .min(body.lines().count().saturating_sub(1));
                        return;
                    }
                    KeyCode::PageUp => {
                        *scroll = scroll.saturating_sub(12);
                        return;
                    }
                    KeyCode::Char(']') => {
                        *index = (*index + 1).min(draft.conflicts.len().saturating_sub(1));
                        ConflictKind::View(*side)
                    }
                    KeyCode::Char('[') => {
                        *index = index.saturating_sub(1);
                        ConflictKind::View(*side)
                    }
                    KeyCode::Char(c @ '0'..='4') => {
                        *side = c as u8 - b'0';
                        ConflictKind::View(*side)
                    }
                    KeyCode::Char('h') => {
                        self.mode = Mode::ConfirmEmbeddedEditor {
                            selection: crate::embedded::Selection::Conflict {
                                draft: Box::new(draft.clone()),
                                index: *index,
                            },
                        };
                        return;
                    }
                    KeyCode::Char('l') => ConflictKind::Lockfile,
                    KeyCode::Char('y') => ConflictKind::Advise,
                    KeyCode::Char('o') => ConflictKind::Choose(1),
                    KeyCode::Char('t') => ConflictKind::Choose(2),
                    KeyCode::Char('a') => ConflictKind::ApplyAI,
                    KeyCode::Char('e') => ConflictKind::Edit,
                    KeyCode::Char('m') => ConflictKind::Mark,
                    KeyCode::Char('r') => ConflictKind::Rebuild,
                    KeyCode::Char('x') => ConflictKind::StopAdvisor,
                    KeyCode::Enter => ConflictKind::Finish,
                    _ => return,
                };
                if matches!(
                    kind,
                    ConflictKind::Choose(_)
                        | ConflictKind::Advise
                        | ConflictKind::ApplyAI
                        | ConflictKind::Lockfile
                        | ConflictKind::Edit
                        | ConflictKind::Rebuild
                ) {
                    self.mode = Mode::ConfirmConflict {
                        draft: draft.clone(),
                        index: *index,
                        kind,
                    };
                } else {
                    self.requested_conflict = Some(ConflictRequest {
                        draft: draft.clone(),
                        index: *index,
                        kind,
                    });
                }
                return;
            }
            Mode::ConfirmEmbeddedEditor { selection } => {
                if key.code == KeyCode::Enter {
                    self.requested_editor = Some(selection.clone());
                    self.mode = Mode::Browse;
                } else if key.code == KeyCode::Esc {
                    self.mode = Mode::Browse;
                }
                return;
            }
            Mode::ConfirmStopEditor => {
                if key.code == KeyCode::Enter {
                    self.requested_editor_stop = true;
                    self.mode = Mode::Browse;
                } else if key.code == KeyCode::Esc {
                    self.mode = Mode::Browse;
                }
                return;
            }
            Mode::ConfirmConflict { draft, index, kind } => {
                if key.code == KeyCode::Enter {
                    self.requested_conflict = Some(ConflictRequest {
                        draft: draft.clone(),
                        index: *index,
                        kind: kind.clone(),
                    });
                    self.mode = Mode::Conflict {
                        draft: draft.clone(),
                        index: *index,
                        side: 4,
                        scroll: 0,
                        body: String::new(),
                    };
                } else if key.code == KeyCode::Esc {
                    self.requested_conflict = Some(ConflictRequest {
                        draft: draft.clone(),
                        index: *index,
                        kind: ConflictKind::View(4),
                    });
                    self.mode = Mode::Conflict {
                        draft: draft.clone(),
                        index: *index,
                        side: 4,
                        scroll: 0,
                        body: String::new(),
                    };
                }
                return;
            }
            Mode::ConfirmRetry { task_id, failure } => {
                if key.code == KeyCode::Enter {
                    self.requested_retry = Some((task_id.clone(), failure.clone()));
                    self.mode = Mode::Browse;
                } else if key.code == KeyCode::Esc {
                    self.mode = Mode::Browse;
                }
                return;
            }
            Mode::ConfirmCode { prompt } => {
                if key.code == KeyCode::Enter {
                    self.requested_task = Some(prompt.clone());
                    self.mode = Mode::Browse;
                } else if key.code == KeyCode::Esc {
                    self.mode = Mode::Browse;
                }
                return;
            }
            Mode::ConfirmNodeApproval {
                task_id,
                node_id,
                checkpoint,
            } => {
                if key.code == KeyCode::Enter {
                    let result = self
                        .tasks
                        .iter_mut()
                        .find(|t| &t.id == task_id)
                        .and_then(|t| t.nodes.iter_mut().find(|n| &n.contract.id == node_id))
                        .ok_or_else(|| anyhow::anyhow!("节点已不存在"))
                        .and_then(|n| n.approve(checkpoint));
                    self.status = match result {
                        Ok(()) => "已明确批准该节点版本 · 验收仍未验证，未合并".into(),
                        Err(e) => format!("批准未完成：{e}"),
                    };
                    self.mode = Mode::Browse;
                    self.content_revision = self.content_revision.wrapping_add(1);
                    self.tasks_revision = self.tasks_revision.wrapping_add(1);
                } else if key.code == KeyCode::Esc {
                    self.mode = Mode::Browse;
                }
                return;
            }
            Mode::ConfirmRead => {
                if key.code == KeyCode::Enter {
                    if let Some(item) = self.items.get_mut(self.selected) {
                        item.read = true;
                        self.status = "已读 · 未批准".into();
                    }
                    self.mode = Mode::Browse;
                } else if key.code == KeyCode::Esc {
                    self.mode = Mode::Browse;
                }
                return;
            }
            Mode::Browse => {}
        }
        if key.code == KeyCode::Esc {
            if !self.filter.is_empty() {
                self.filter.clear();
            } else {
                self.fullscreen = false;
            }
            return;
        }
        if self.view == View::Project
            && self.panel == Panel::Queue
            && !self.project_tree.entries.is_empty()
        {
            let previous = self.project_tree.selected.clone();
            match key.code {
                KeyCode::Char('j') | KeyCode::Down => self.project_tree.move_by(1),
                KeyCode::Char('k') | KeyCode::Up => self.project_tree.move_by(-1),
                KeyCode::Char('h') | KeyCode::Left => self.project_tree.collapse(),
                KeyCode::Char('l') | KeyCode::Right => self.project_tree.expand(),
                KeyCode::Enter => {
                    if self.project_tree.entry().is_some_and(|e| e.directory) {
                        if self
                            .project_tree
                            .expanded
                            .contains(&self.project_tree.selected)
                        {
                            self.project_tree.collapse();
                        } else {
                            self.project_tree.expand();
                        }
                    } else {
                        self.requested_project_open = Some(self.project_tree.selected.clone());
                    }
                }
                KeyCode::Char('r') => self.requested_tree_refresh = true,
                KeyCode::Char('z') => {
                    self.project_tree.show_ignored = !self.project_tree.show_ignored;
                    self.requested_tree_refresh = true;
                }
                _ => {
                    if let Some(action) = self.keymap.resolve(key) {
                        self.act(action);
                    }
                    return;
                }
            }
            if self.project_tree.selected != previous
                && self
                    .project_tree
                    .entry()
                    .is_some_and(|e| !e.directory && e.status != 'D')
            {
                self.requested_project_open = Some(self.project_tree.selected.clone());
            }
            return;
        }
        if let Some(action) = self.keymap.resolve(key) {
            self.act(action);
        }
    }

    pub fn act(&mut self, action: Action) {
        match action {
            Action::Tasks => self.view = View::Tasks,
            Action::Review => {
                self.open_review();
                self.requested_review_helix = true;
                self.review_compare = false;
            }
            Action::Project => {
                let path = self.current().map(|i| i.title.clone());
                self.view = View::Project;
                self.panel = Panel::Queue;
                self.editor_focused = false;
                self.requested_tree_refresh = true;
                self.requested_project_open = Some(path.clone().unwrap_or_default());
                let indices = self.visible_file_indices();
                if let Some(&index) = indices
                    .iter()
                    .find(|&&i| Some(&self.items[i].title) == path.as_ref())
                    .or_else(|| indices.first())
                {
                    self.selected = index;
                }
            }
            Action::Knowledge => self.view = View::Knowledge,
            Action::NextPanel | Action::PreviousPanel => {
                self.fullscreen = false;
                self.panel = match (self.panel, action) {
                    (Panel::Queue, Action::NextPanel)
                    | (Panel::Evidence, Action::PreviousPanel) => Panel::Reader,
                    (Panel::Reader, Action::NextPanel) | (Panel::Queue, Action::PreviousPanel) => {
                        Panel::Evidence
                    }
                    _ => Panel::Queue,
                };
                if self.view == View::Review && self.panel == Panel::Reader {
                    self.requested_review_helix = true;
                }
                if self.editor_screen.is_some() {
                    self.editor_focused = self.panel == Panel::Reader
                        && !self.review_compare
                        && (self.editor_conflict
                            || ((!self.editor_project && self.view == View::Review)
                                || (self.editor_project && self.view == View::Project)));
                }
            }
            Action::Up | Action::Down => {
                if self.panel == Panel::Queue {
                    let delta = if action == Action::Up { -1 } else { 1 };
                    if self.view == View::Tasks {
                        self.move_task(delta);
                    } else {
                        self.move_item(delta);
                    }
                } else if self.view == View::Tasks
                    || (self.showing_node_contract() && self.panel == Panel::Reader)
                {
                    let scroll = if self.panel == Panel::Reader {
                        &mut self.task_scroll
                    } else {
                        &mut self.task_evidence_scroll
                    };
                    *scroll = if action == Action::Up {
                        scroll.saturating_sub(1)
                    } else {
                        scroll.saturating_add(1)
                    };
                } else if let Some(item) = self.items.get_mut(self.selected) {
                    let scroll = if self.panel == Panel::Reader {
                        &mut item.scroll
                    } else {
                        &mut item.evidence_scroll
                    };
                    *scroll = if action == Action::Up {
                        scroll.saturating_sub(1)
                    } else {
                        scroll.saturating_add(1)
                    };
                }
            }
            Action::PreviousItem if self.view == View::Tasks => self.move_task(-1),
            Action::NextItem if self.view == View::Tasks => self.move_task(1),
            Action::PreviousItem => self.move_review_item(-1),
            Action::NextItem => self.move_review_item(1),
            Action::PreviousNode => self.move_node(-1),
            Action::NextNode => self.move_node(1),
            Action::Open => {
                self.panel = Panel::Reader;
                if self.view == View::Tasks {
                    self.task_scroll = 0;
                    return;
                }
                self.page = match self.page {
                    Page::Intent => Page::Code,
                    Page::Code => Page::Evidence,
                    Page::Evidence => Page::Intent,
                };
                self.task_scroll = 0;
                if let Some(item) = self.items.get_mut(self.selected) {
                    item.scroll = 0;
                }
            }
            Action::Explain => self.explain = !self.explain,
            Action::Fullscreen => {
                self.fullscreen = !self.fullscreen;
                if self.fullscreen {
                    self.panel = Panel::Reader;
                    if self.helix_review && self.view == View::Review {
                        self.requested_review_helix = true;
                    }
                    self.editor_focused = self.editor_screen.is_some()
                        && !self.review_compare
                        && (self.editor_conflict
                            || (self.editor_project && self.view == View::Project)
                            || (!self.editor_project && self.view == View::Review));
                }
            }
            Action::Help => self.mode = Mode::Help,
            Action::Palette => {
                self.mode = Mode::Input {
                    search: false,
                    text: String::new(),
                }
            }
            Action::Search => {
                self.mode = Mode::Input {
                    search: true,
                    text: self.filter.clone(),
                }
            }
            Action::EmbeddedEditor => {
                if self.editor_screen.is_some() {
                    if self.editor_project {
                        self.view = View::Project;
                    }
                    self.editor_focused = true;
                    return;
                }
                let selection = self.current().and_then(|file| match self.view {
                    View::Project => Some(crate::embedded::Selection::Project {
                        path: file.title.clone(),
                        version: file.version.clone(),
                    }),
                    View::Review => {
                        self.selected_task()
                            .map(|task| crate::embedded::Selection::Review {
                                task_id: task.id.clone(),
                                path: file.title.clone(),
                                version: file.version.clone(),
                            })
                    }
                    _ => None,
                });
                if let Some(selection) = selection {
                    self.mode = Mode::ConfirmEmbeddedEditor { selection };
                } else {
                    self.status = "请在项目代码、审核 diff 或集成草稿中按 h".into();
                }
            }
            Action::Theme => self.light = !self.light,
            Action::ApproveNode if self.view == View::Review => self.request_node_approval(),
            Action::ApproveNode => self.status = "请按 2 进入审核，再批准当前节点版本".into(),
            Action::MarkRead if self.view == View::Tasks => {
                self.status = "请在审核视图标记文件已读；节点批准使用 :approve-node".into()
            }
            Action::MarkRead => {
                if self.current().is_some() {
                    self.mode = Mode::ConfirmRead;
                }
            }
            Action::StartTask if self.view == View::Tasks => {
                if let Some(task) = self.selected_task() {
                    if self.has_review_draft(task) {
                        self.status =
                            "任务有人工审核草稿；先发布新固定版本，未授权覆盖草稿继续".into();
                        return;
                    }
                    match task.startable() {
                        Ok(()) => {
                            self.mode = Mode::ConfirmStart {
                                task_id: task.id.clone(),
                                plan: serde_json::to_string(&task.plan).unwrap(),
                            }
                        }
                        Err(e) => self.status = format!("不能启动：{e}"),
                    }
                } else {
                    self.status = "请先 i 输入需求，讨论出任务计划".into();
                }
            }
            Action::StartTask => self.status = "请按 1 选择任务，再 s 启动".into(),
            Action::DeleteSelected if matches!(self.view, View::Tasks | View::Review) => {
                if let Some(selection) = self.task_selection.clone() {
                    self.mode = Mode::ConfirmDelete { selection };
                }
            }
            Action::DeleteSelected => self.status = "请在任务/审核界面选择节点或任务".into(),
            Action::RetryTask if matches!(self.view, View::Tasks | View::Review) => {
                if let Some(task) = self.selected_task() {
                    if task.state != TaskState::Failed || task.failure.is_none() {
                        self.status = "所选任务没有可重试的失败；已有计划可用 s 继续".into();
                    } else if task.failure.as_ref().unwrap().stage
                        == crate::nodes::FailureStage::Discussion
                    {
                        self.mode = Mode::ConfirmRetry {
                            task_id: task.id.clone(),
                            failure: serde_json::to_string(&task.failure).unwrap(),
                        };
                    } else {
                        self.act(Action::StartTask);
                    }
                }
            }
            Action::RetryTask => self.status = "请在任务/审核界面选中失败任务再重试".into(),
            Action::MergeTask if matches!(self.view, View::Tasks | View::Review) => {
                if let Some(task) = self.selected_task() {
                    if self.has_review_draft(task) {
                        self.status =
                            "人工审核草稿尚未发布；代码区 Ctrl+s 发布再批准，未合入旧版本".into();
                        return;
                    }
                    match task.merge_ready() {
                        Ok(()) => self.requested_merge_preview = Some(task.id.clone()),
                        Err(e) => self.status = format!("不能合并：{e}"),
                    }
                }
            }
            Action::MergeTask => self.status = "请在任务/审核界面选择任务再 g 合并".into(),
            Action::Pause => self.controls.push(RunControl::TogglePause),
            Action::Reply => {
                if !self.agent_connected {
                    self.planning_only = true;
                }
                self.view = View::Tasks;
                self.panel = Panel::Reader;
                self.mode = Mode::Message {
                    text: String::new(),
                };
            }
            Action::Cancel => self.cancel_requested = true,
            Action::Quit => self.quit = true,
        }
    }

    fn move_in(&mut self, indices: Vec<usize>, delta: isize) {
        if indices.is_empty() {
            return;
        }
        let path = self.current().map(|item| item.title.as_str());
        let position = indices
            .iter()
            .position(|&i| Some(self.items[i].title.as_str()) == path)
            .map_or(0, |i| i.saturating_add_signed(delta).min(indices.len() - 1));
        self.selected = indices[position];
        self.filter.clear();
    }
    fn move_item(&mut self, delta: isize) {
        self.move_in(self.visible_file_indices(), delta);
    }
    fn move_review_item(&mut self, delta: isize) {
        self.move_item(delta);
    }

    fn command(&mut self, command: &str) {
        match command.trim() {
            "quit" => self.quit = true,
            "help" => self.mode = Mode::Help,
            "theme" => self.light = !self.light,
            "review" => self.open_review(),
            "changes" => self.open_review(),
            "tasks" => self.act(Action::Tasks),
            "project" => self.act(Action::Project),
            "knowledge" => self.view = View::Knowledge,
            "read" => self.act(Action::MarkRead),
            "approve-node" => self.request_node_approval(),
            "start-task" => self.act(Action::StartTask),
            "delete-selected" => self.act(Action::DeleteSelected),
            "merge-task" => self.act(Action::MergeTask),
            "retry-task" => self.act(Action::RetryTask),
            "ask" => self.open_review_chat(),
            "pause" => self.controls.push(RunControl::Pause),
            "resume" => self.controls.push(RunControl::Resume),
            "cancel" => self.cancel_requested = true,
            value if value.starts_with("say ") && !value[4..].trim().is_empty() => self
                .controls
                .push(RunControl::Say(value[4..].trim().into())),
            value if value.starts_with("code ") || value.starts_with("plan ") => {
                if matches!(
                    self.run.status,
                    RunStatus::Running
                        | RunStatus::Pausing
                        | RunStatus::Paused
                        | RunStatus::Cancelling
                ) {
                    self.status = "已有任务运行中".into();
                } else if !value[5..].trim().is_empty() {
                    self.mode = Mode::ConfirmCode {
                        prompt: value[5..].trim().into(),
                    };
                }
            }
            value if value.starts_with("note ") => {
                let note = if let Some(item) = self.current() {
                    format!("{} @ {}：{}", item.title, item.version, &value[5..])
                } else {
                    value[5..].to_owned()
                };
                self.notes.push(note);
                self.content_revision = self.content_revision.wrapping_add(1);
                self.status = "笔记已记录 · 仅本次会话".into();
            }
            _ => self.status = "未知命令 · :help".into(),
        }
    }

    /// Semantic role tags, not guesses based on message text.
    /// Complete natural-language records; retain failure evidence before Markdown layout.
    pub fn task_text_blocks(&self) -> Vec<(String, i8)> {
        let mut blocks = Vec::new();
        if let Some(f) = self.selected_task().and_then(|t| t.failure.as_ref()) {
            blocks.push((format!("失败原因：{}", f.reason), -1));
            blocks.push((
                "r 重试原任务（需确认） · i 调整讨论；不会自动重试".into(),
                2,
            ));
        }
        blocks.extend(self.task_text_blocks_inner());
        blocks
    }
    fn task_text_blocks_inner(&self) -> Vec<(String, i8)> {
        if let Some(description) = self.node_description() {
            return description.lines().map(|l| (l.to_owned(), 2)).collect();
        }
        if let Some(selected) = self.selected_task()
            && self.tasks.last().is_some_and(|live| live.id != selected.id)
        {
            let mut lines = vec![
                ("任务需求".into(), 2),
                (selected.prompt.clone(), 0),
                (format!("状态：{}", task_state_label(selected.state)), 2),
                (String::new(), 0),
            ];
            for m in selected
                .history
                .iter()
                .filter(|m| !m.content.trim().is_empty())
            {
                let (label, role) = if m.role == "user" {
                    ("你", 3)
                } else if m.role == "assistant" {
                    ("AI", 4)
                } else {
                    ("系统", 5)
                };
                lines.push((
                    format!("┌─ {label}"),
                    if role == 3 {
                        7
                    } else if role == 4 {
                        8
                    } else {
                        2
                    },
                ));
                lines.push((m.content.clone(), role));
                lines.push((String::new(), 0));
            }
            return lines;
        }
        let Some(task) = &self.task else {
            return vec![("暂无任务".into(), 2)];
        };
        let state = match self.run.status {
            RunStatus::Idle => "未开始",
            RunStatus::Running => "运行中",
            RunStatus::Pausing => "暂停中（等待确认）",
            RunStatus::Paused => "已暂停",
            RunStatus::Cancelling => "取消中",
            RunStatus::Settled => "本轮结束 · 待审核",
            RunStatus::Aborted => "已中断",
            RunStatus::Failed => "失败",
        };
        let mut lines = vec![("任务需求".into(), 2), (task.clone(), 0)];
        lines.extend(vec![
            (
                format!(
                    "状态：{state} · 工具调用 {} · 待发送纠正 {}",
                    self.run.tool_count, self.queued_corrections
                ),
                2,
            ),
            (
                format!(
                    "{} 暂停/继续 · {} 纠正/追问 · {} 取消",
                    self.keymap.hint(Action::Pause),
                    self.keymap.hint(Action::Reply),
                    self.keymap.hint(Action::Cancel)
                ),
                2,
            ),
            (String::new(), 0),
        ]);
        for message in self.history.iter().filter(|m| !m.content.trim().is_empty()) {
            let (label, role, header) = match message.role.as_str() {
                "user" => ("你", 3, 7),
                "assistant" => ("AI", 4, 8),
                _ => ("系统", 5, 2),
            };
            lines.push((format!("┌─ {label}"), header));
            lines.push((message.content.clone(), role));
            lines.push((String::new(), 0));
        }
        if self.run.streaming_text && !self.run.text.trim().is_empty() {
            lines.push(("┌─ AI · 正在回复".into(), 8));
            lines.push((self.run.text.clone(), 4));
        }
        lines
    }

    /// Legacy plain-text consumers retain their line-oriented representation.
    pub fn task_lines(&self) -> Vec<(String, i8)> {
        self.task_text_blocks()
            .into_iter()
            .flat_map(|(text, role)| {
                let legacy_role = if role == 5 || (role == 0 && text.starts_with("任务：")) {
                    2
                } else {
                    role
                };
                text.split('\n')
                    .map(|line| {
                        (
                            if matches!(role, 3..=5) {
                                format!("│ {line}")
                            } else {
                                line.to_owned()
                            },
                            legacy_role,
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    pub fn body(&self) -> String {
        match self.view {
            View::Tasks => self
                .task_lines()
                .into_iter()
                .map(|(text, _)| text)
                .collect::<Vec<_>>()
                .join("\n"),
            View::Project => self
                .current()
                .map(|item| item.code.clone())
                .unwrap_or_else(|| "未打开项目".into()),
            View::Knowledge => {
                if self.notes.is_empty() {
                    "暂无笔记".into()
                } else {
                    self.notes.join("\n\n")
                }
            }
            View::Review => {
                if self.selected_node().is_some() {
                    if self.page == Page::Intent {
                        return self.node_description().unwrap();
                    }
                    if self.page == Page::Evidence {
                        return self.node_evidence().unwrap();
                    }
                    if !self.changed_indices().contains(&self.selected) {
                        return if self
                            .selected_node()
                            .is_some_and(|n| n.execution == Execution::NoChanges)
                        {
                            "节点未交付代码：没有文件变更，不能视为实现完成。".into()
                        } else {
                            "该节点当前没有可显示的文件变更".into()
                        };
                    }
                }
                let Some(item) = self.current() else {
                    return "暂无内容".into();
                };
                let text = match self.page {
                    Page::Intent => &item.intent,
                    Page::Code => {
                        if item.change.is_some() {
                            &item.diff
                        } else {
                            &item.code
                        }
                    }
                    Page::Evidence => &item.evidence,
                };
                if text.is_empty() {
                    "暂无内容".into()
                } else {
                    text.clone()
                }
            }
        }
    }
}
