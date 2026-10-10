use crate::{
    checkpoint::{Repository, Snapshot},
    keymap::{Action, Keymap},
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
    Input { search: bool, text: String },
    Message { text: String },
    ConfirmRead,
    ConfirmCode { prompt: String },
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
    pub evidence: String,
    pub explanation: String,
    pub read: bool,
    pub scroll: u16,
    pub evidence_scroll: u16,
}

pub struct App {
    pub name: String,
    pub content_revision: u64,
    pub branch: String,
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
    pub queued_corrections: usize,
    pub controls: Vec<RunControl>,
    pub task: Option<String>,
    pub requested_task: Option<String>,
    pub keymap: Keymap,
    pub quit: bool,
    pub notes: Vec<String>,
}

impl App {
    pub fn empty(keymap: Keymap) -> Self {
        Self {
            name: "工作台".into(),
            content_revision: 0,
            branch: String::new(),
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
            queued_corrections: 0,
            controls: Vec::new(),
            task: None,
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
        app.items = files
            .iter()
            .take(80)
            .map(|path| ReadingItem {
                title: path.clone(),
                version: snapshot.commit.clone(),
                intent: String::new(),
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
            app.items.insert(
                0,
                ReadingItem {
                    title: "变更".into(),
                    version: snapshot.commit.clone(),
                    intent: String::new(),
                    code: repo.diff(&snapshot)?,
                    evidence: String::new(),
                    explanation: String::new(),
                    read: false,
                    scroll: 0,
                    evidence_scroll: 0,
                },
            );
        }
        if repo.is_dirty()? {
            app.status = "工作区有未提交改动 · 当前为固定快照".into();
        }
        Ok(app)
    }

    pub fn current(&self) -> Option<&ReadingItem> {
        self.items.get(self.selected)
    }
    pub fn pending(&self) -> usize {
        self.items.iter().filter(|i| !i.read).count()
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
            Action::Review => self.view = View::Review,
            Action::Project => self.view = View::Project,
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
                    self.move_item(if action == Action::Up { -1 } else { 1 });
                } else if self.panel == Panel::Reader && self.view == View::Tasks {
                    self.task_scroll = if action == Action::Up {
                        self.task_scroll.saturating_sub(1)
                    } else {
                        self.task_scroll.saturating_add(1)
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
            Action::PreviousItem => self.move_item(-1),
            Action::NextItem => self.move_item(1),
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

    fn move_item(&mut self, delta: isize) {
        self.selected = self
            .selected
            .saturating_add_signed(delta)
            .min(self.items.len().saturating_sub(1));
        self.filter.clear();
    }

    fn command(&mut self, command: &str) {
        match command.trim() {
            "quit" => self.quit = true,
            "help" => self.mode = Mode::Help,
            "theme" => self.light = !self.light,
            "review" => self.view = View::Review,
            "tasks" => self.view = View::Tasks,
            "project" => self.view = View::Project,
            "knowledge" => self.view = View::Knowledge,
            "read" => self.act(Action::MarkRead),
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
                let Some(item) = self.current() else {
                    return "暂无内容".into();
                };
                let text = match self.page {
                    Page::Intent => &item.intent,
                    Page::Code => &item.code,
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
