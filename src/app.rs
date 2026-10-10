use crate::{
    checkpoint::{ChangeKind, Repository, Snapshot},
    keymap::{Action, Keymap},
    nodes::{Execution, NodeRecord, Review, TaskRecord, TaskState, Verification},
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
    Help,
    Input {
        search: bool,
        text: String,
    },
    Message {
        text: String,
    },
    ConfirmRead,
    ConfirmCode {
        prompt: String,
    },
    ConfirmNodeApproval {
        task_id: String,
        node_id: String,
        checkpoint: String,
    },
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
    pub keymap: Keymap,
    pub quit: bool,
    pub notes: Vec<String>,
}

fn task_state_label(s: TaskState) -> &'static str {
    match s {
        TaskState::Planning => "拆分中",
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
    pub fn empty(keymap: Keymap) -> Self {
        Self {
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
            return node.published_checkpoint();
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
                        || (n.published_checkpoint().is_some()
                            && (n.review != Review::Approved
                                || self.task_selection.as_ref().is_some_and(|s| {
                                    s.task_id == task.id
                                        && s.node_id.as_deref() == Some(&n.contract.id)
                                })))
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
                    task_state_label(task.state)
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
            node.published_checkpoint().map(str::to_owned)
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
            node.checkpoint().unwrap_or("尚未生成")
        ))
    }
    pub fn node_evidence(&self) -> Option<String> {
        let node = self.selected_node()?;
        let version = self.review_checkpoint();
        let attempt = node
            .attempts
            .iter()
            .find(|a| a.checkpoint.as_deref() == version);
        Some(format!(
            "验收：{} · 人工审核：{}\n{}\n测试 Runner 未接入{}\n{}",
            verification_label(attempt.map_or(node.verification, |a| a.verification)),
            review_label(attempt.map_or(node.review, |a| a.review)),
            node.contract
                .acceptance
                .iter()
                .map(|c| format!("□ 未验证 · {c}"))
                .collect::<Vec<_>>()
                .join("\n"),
            attempt
                .filter(|a| !a.scope_violations.is_empty())
                .map(|a| format!("\n仓内越界：{}", a.scope_violations.join("、")))
                .unwrap_or_default(),
            if self.explain {
                attempt.map(|a| a.summary.as_str()).unwrap_or("")
            } else {
                ""
            }
        ))
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
        let Some(checkpoint) = self.review_checkpoint() else {
            self.status = "节点尚无检查点".into();
            return;
        };
        if node.execution != Execution::Implemented || node.checkpoint() != Some(checkpoint) {
            self.status = "只能批准当前已完成节点版本；旧版本或失败节点不能放行".into();
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
            && let Some(node) = task.nodes.iter().find(|n| n.pending_review()).or_else(|| {
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
        self.view = View::Review;
        if let Some(index) = self.changed_indices().first() {
            self.selected = *index;
        }
        self.view = View::Review;
        self.panel = Panel::Reader;
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

    pub fn handle_key(&mut self, key: KeyEvent) {
        if key.kind == KeyEventKind::Release {
            return;
        }
        match &mut self.mode {
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
                            if self.view == View::Tasks {
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
        if let Some(action) = self.keymap.resolve(key) {
            self.act(action);
        }
    }

    pub fn act(&mut self, action: Action) {
        match action {
            Action::Tasks => self.view = View::Tasks,
            Action::Review => self.open_review(),
            Action::Project => {
                let path = self.current().map(|i| i.title.clone());
                self.view = View::Project;
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
            }
            Action::Up | Action::Down => {
                if self.panel == Panel::Queue {
                    let delta = if action == Action::Up { -1 } else { 1 };
                    if self.view == View::Tasks {
                        self.move_task(delta);
                    } else {
                        self.move_item(delta);
                    }
                } else if self.view == View::Tasks {
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
                if let Some(item) = self.items.get_mut(self.selected) {
                    item.scroll = 0;
                }
            }
            Action::Explain => self.explain = !self.explain,
            Action::Fullscreen => {
                self.fullscreen = !self.fullscreen;
                if self.fullscreen {
                    self.panel = Panel::Reader;
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
            Action::Theme => self.light = !self.light,
            Action::MarkRead if self.view == View::Tasks => {
                self.status = "请在审核视图标记文件已读；节点批准使用 :approve-node".into()
            }
            Action::MarkRead => {
                if self.current().is_some() {
                    self.mode = Mode::ConfirmRead;
                }
            }
            Action::Pause => self.controls.push(RunControl::TogglePause),
            Action::Reply => {
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
            "pause" => self.controls.push(RunControl::Pause),
            "resume" => self.controls.push(RunControl::Resume),
            "cancel" => self.cancel_requested = true,
            value if value.starts_with("say ") && !value[4..].trim().is_empty() => self
                .controls
                .push(RunControl::Say(value[4..].trim().into())),
            value if value.starts_with("code ") => {
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
    pub fn task_lines(&self) -> Vec<(String, i8)> {
        if let Some(description) = self.node_description() {
            return description.lines().map(|l| (l.to_owned(), 2)).collect();
        }
        if let Some(selected) = self.selected_task()
            && self.tasks.last().is_some_and(|live| live.id != selected.id)
        {
            let mut lines = vec![
                (format!("任务：{}", selected.prompt), 2),
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
                    ("系统", 2)
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
                lines.extend(m.content.lines().map(|l| (format!("│ {l}"), role)));
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
        let mut lines: Vec<_> = format!("任务：{task}")
            .split('\n')
            .map(|line| (line.to_owned(), 2))
            .collect();
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
                _ => ("系统", 2, 2),
            };
            lines.push((format!("┌─ {label}"), header));
            lines.extend(
                message
                    .content
                    .split('\n')
                    .map(|line| (format!("│ {line}"), role)),
            );
            lines.push((String::new(), 0));
        }
        if self.run.streaming_text && !self.run.text.trim().is_empty() {
            lines.push(("┌─ AI · 正在回复".into(), 8));
            lines.extend(
                self.run
                    .text
                    .split('\n')
                    .map(|line| (format!("│ {line}"), 4)),
            );
        }
        lines
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
                        return "该节点当前没有可显示的文件变更".into();
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
