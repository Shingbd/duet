//! Authoritative review buffers. Editable drafts are never presented as approved Git objects.
use anyhow::{Result, ensure};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde::{Deserialize, Serialize};
use similar::{ChangeTag, TextDiff};
use unicode_segmentation::UnicodeSegmentation;

pub const MAX_BYTES: usize = 1_048_576;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatMessage {
    pub task_id: String,
    pub path: String,
    pub version: String,
    pub role: String,
    pub content: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Buffer {
    pub task_id: String,
    pub node_id: Option<String>,
    pub path: String,
    pub version: String,
    pub base: String,
    pub original: String,
    pub text: String,
    pub cursor: usize,
    pub anchor: Option<usize>,
    pub insert: bool,
    pub top: usize,
    pub left: usize,
    pub revision: u64,
    pub human_diff: bool,
    #[serde(skip)]
    pub command: Option<(bool, String)>,
    #[serde(skip)]
    undo: Vec<(String, usize)>,
    #[serde(skip)]
    pub rows: Vec<Row>,
}
#[derive(Clone, Debug)]
pub struct Row {
    pub old: Option<usize>,
    pub new: Option<usize>,
    pub text: String,
    pub tag: ChangeTag,
    pub human: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Context {
    pub task_id: String,
    pub node_id: Option<String>,
    pub path: String,
    pub version: String,
    pub revision: u64,
    pub from: usize,
    pub to: usize,
    pub line: usize,
    pub excerpt: String,
    /// Local only: never sent as extra code, used to bind a suggested patch to the exact buffer.
    pub buffer: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Proposal {
    pub context: Context,
    pub old: String,
    pub new: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    None,
    Ask,
    Publish,
    Apply,
    Workbench,
}
impl Buffer {
    pub fn new(
        task_id: String,
        node_id: Option<String>,
        path: String,
        version: String,
        base: String,
        original: String,
    ) -> Self {
        let mut b = Self {
            task_id,
            node_id,
            path,
            version,
            base,
            text: original.clone(),
            original,
            cursor: 0,
            anchor: None,
            insert: false,
            top: 0,
            left: 0,
            revision: 0,
            human_diff: false,
            command: None,
            undo: Vec::new(),
            rows: Vec::new(),
        };
        b.rebuild();
        b
    }
    pub fn validate(&mut self) -> Result<()> {
        crate::checkpoint::validate_path(&self.path)?;
        ensure!(
            [&self.text, &self.original, &self.base]
                .iter()
                .all(|s| s.len() <= MAX_BYTES),
            "审核文件超过 1 MiB"
        );
        ensure!(
            self.version.len() == 40 && self.version.bytes().all(|b| b.is_ascii_hexdigit()),
            "无效审核版本"
        );
        self.cursor = self.cursor.min(self.text.len());
        while !self.text.is_char_boundary(self.cursor) {
            self.cursor -= 1;
        }
        self.anchor = self
            .anchor
            .filter(|&a| a <= self.text.len() && self.text.is_char_boundary(a));
        self.insert = false;
        self.command = None;
        self.rebuild();
        Ok(())
    }
    pub fn changed(&self) -> bool {
        self.text != self.original
    }
    pub fn matches(&self, task: &str, node: Option<&str>, path: &str, version: &str) -> bool {
        self.task_id == task
            && self.node_id.as_deref() == node
            && self.path == path
            && self.version == version
    }
    pub fn line(&self) -> usize {
        self.text[..self.cursor]
            .bytes()
            .filter(|&c| c == b'\n')
            .count()
    }
    fn line_start(&self) -> usize {
        self.text[..self.cursor].rfind('\n').map_or(0, |p| p + 1)
    }
    fn line_end(&self) -> usize {
        self.text[self.cursor..]
            .find('\n')
            .map_or(self.text.len(), |p| self.cursor + p)
    }
    pub fn column(&self) -> usize {
        self.text[self.line_start()..self.cursor]
            .graphemes(true)
            .count()
    }
    fn next(&self, p: usize) -> usize {
        self.text[p..]
            .graphemes(true)
            .next()
            .map_or(p, |c| p + c.len())
    }
    fn prev(&self, p: usize) -> usize {
        self.text[..p]
            .grapheme_indices(true)
            .next_back()
            .map_or(0, |(p, _)| p)
    }
    pub fn selected(&self) -> Option<(usize, usize)> {
        let a = self.anchor?;
        if a == self.cursor {
            None
        } else {
            Some((a.min(self.cursor), a.max(self.cursor)))
        }
    }
    pub fn context(&self) -> Context {
        let (from, to) = self.selected().unwrap_or_else(|| {
            let start = self.line_start();
            let end = self.line_end();
            let word = |c: char| c.is_alphanumeric() || c == '_';
            let mut from = self.cursor;
            let mut to = self.cursor;
            while from > start {
                let p = self.prev(from);
                if !self.text[p..from].chars().all(word) {
                    break;
                }
                from = p;
            }
            while to < end {
                let p = self.next(to);
                if !self.text[to..p].chars().all(word) {
                    break;
                }
                to = p;
            }
            if from == to { (start, end) } else { (from, to) }
        });
        Context {
            task_id: self.task_id.clone(),
            node_id: self.node_id.clone(),
            path: self.path.clone(),
            version: self.version.clone(),
            revision: self.revision,
            from,
            to,
            line: self.line() + 1,
            excerpt: self.text[from..to].into(),
            buffer: self.text.clone(),
        }
    }
    pub fn replace(&mut self, from: usize, to: usize, text: &str) -> Result<()> {
        ensure!(
            from <= to
                && to <= self.text.len()
                && self.text.is_char_boundary(from)
                && self.text.is_char_boundary(to),
            "选区失效"
        );
        ensure!(
            self.text.len() - (to - from) + text.len() <= MAX_BYTES,
            "编辑超过 1 MiB；原缓冲区未改变"
        );
        if self.undo.iter().map(|(s, _)| s.len()).sum::<usize>() + self.text.len() > 8 * MAX_BYTES {
            self.undo.clear();
        }
        if self.undo.len() >= 32 {
            self.undo.remove(0);
        }
        self.undo.push((self.text.clone(), self.cursor));
        self.text.replace_range(from..to, text);
        self.cursor = from + text.len();
        self.anchor = None;
        self.revision = self.revision.wrapping_add(1);
        self.rebuild();
        Ok(())
    }
    pub fn paste(&mut self, text: &str) -> Result<()> {
        let (a, b) = self.selected().unwrap_or((self.cursor, self.cursor));
        self.replace(a, b, text)
    }
    pub fn apply(&mut self, proposal: &Proposal) -> Result<()> {
        ensure!(
            self.matches(
                &proposal.context.task_id,
                proposal.context.node_id.as_deref(),
                &proposal.context.path,
                &proposal.context.version
            ) && self.text == proposal.context.buffer
                && self.revision == proposal.context.revision,
            "讨论后缓冲区已变化；补丁未应用，请重新讨论"
        );
        ensure!(
            !proposal.old.is_empty() && proposal.context.excerpt.contains(&proposal.old),
            "补丁必须位于讨论的词/行/选区内"
        );
        let context = &proposal.context;
        ensure!(
            context.from <= context.to
                && self.text.get(context.from..context.to) == Some(context.excerpt.as_str()),
            "引用位置失效；未应用补丁"
        );
        let mut found = context.excerpt.match_indices(&proposal.old);
        let (offset, _) = found
            .next()
            .ok_or_else(|| anyhow::anyhow!("引用中没有补丁原文"))?;
        ensure!(found.next().is_none(), "引用内部补丁原文不唯一");
        let at = context.from + offset;
        self.replace(at, at + proposal.old.len(), &proposal.new)
    }
    pub fn rebuild(&mut self) {
        let mut human = std::collections::HashSet::new();
        let mut n = 0;
        for c in TextDiff::configure()
            .timeout(std::time::Duration::from_millis(30))
            .diff_lines(&self.original, &self.text)
            .iter_all_changes()
        {
            if c.tag() != ChangeTag::Delete {
                if c.tag() == ChangeTag::Insert {
                    human.insert(n);
                }
                n += 1;
            }
        }
        let before = if self.human_diff {
            &self.original
        } else {
            &self.base
        };
        let mut old = 0;
        let mut new = 0;
        self.rows.clear();
        for c in TextDiff::configure()
            .timeout(std::time::Duration::from_millis(30))
            .diff_lines(before, &self.text)
            .iter_all_changes()
        {
            let tag = c.tag();
            self.rows.push(Row {
                old: (tag != ChangeTag::Insert).then_some(old),
                new: (tag != ChangeTag::Delete).then_some(new),
                text: c
                    .value()
                    .trim_end_matches('\n')
                    .trim_end_matches('\r')
                    .into(),
                tag,
                human: tag == ChangeTag::Delete && self.human_diff
                    || tag != ChangeTag::Delete && human.contains(&new),
            });
            if tag != ChangeTag::Insert {
                old += 1;
            }
            if tag != ChangeTag::Delete {
                new += 1;
            }
        }
        if self.text.is_empty() || self.text.ends_with('\n') {
            self.rows.push(Row {
                old: None,
                new: Some(new),
                text: String::new(),
                tag: ChangeTag::Equal,
                human: false,
            });
        }
    }
    pub fn cursor_row(&self) -> usize {
        self.rows
            .iter()
            .position(|r| r.new == Some(self.line()))
            .unwrap_or(0)
    }
    pub fn keep_visible(&mut self, height: usize, width: usize) {
        let row = self.cursor_row();
        if row < self.top {
            self.top = row;
        }
        if row >= self.top + height.max(1) {
            self.top = row + 1 - height.max(1);
        }
        let col = self.text[self.line_start()..self.cursor]
            .graphemes(true)
            .map(unicode_width::UnicodeWidthStr::width)
            .sum::<usize>();
        if col < self.left {
            self.left = col;
        }
        if col >= self.left + width.max(1) {
            self.left = col + 1 - width.max(1);
        }
    }
    fn vertical(&mut self, delta: isize) {
        let col = self.column();
        let starts = std::iter::once(0)
            .chain(self.text.match_indices('\n').map(|(p, _)| p + 1))
            .collect::<Vec<_>>();
        let line = self
            .line()
            .saturating_add_signed(delta)
            .min(starts.len() - 1);
        let start = starts[line];
        let end = self.text[start..]
            .find('\n')
            .map_or(self.text.len(), |p| start + p);
        self.cursor = self.text[start..end]
            .grapheme_indices(true)
            .nth(col)
            .map_or(end, |(p, _)| start + p);
    }
    pub fn handle(&mut self, key: KeyEvent) -> Result<Action> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl {
            match key.code {
                KeyCode::Char(']' | '5') => return Ok(Action::Workbench), // Legacy crossterm decodes ASCII 0x1d as Ctrl+5.
                KeyCode::Char('g') => return Ok(Action::Ask),
                KeyCode::Char('s') => return Ok(Action::Publish),
                KeyCode::Char('p') => return Ok(Action::Apply),
                KeyCode::Char('d') => {
                    self.human_diff = !self.human_diff;
                    self.rebuild();
                    return Ok(Action::None);
                }
                KeyCode::Char('a') => {
                    self.anchor = Some(0);
                    self.cursor = self.text.len();
                    return Ok(Action::None);
                }
                KeyCode::Char('z') => {
                    if let Some((text, cursor)) = self.undo.pop() {
                        self.text = text;
                        self.cursor = cursor;
                        self.anchor = None;
                        self.revision = self.revision.wrapping_add(1);
                        self.rebuild();
                    }
                    return Ok(Action::None);
                }
                _ => return Ok(Action::None),
            }
        }
        if let Some((_search, text)) = &mut self.command {
            match key.code {
                KeyCode::Esc => self.command = None,
                KeyCode::Backspace => {
                    text.pop();
                }
                KeyCode::Char(c) => text.push(c),
                KeyCode::Enter => {
                    let (search, text) = self.command.take().unwrap();
                    if search {
                        if let Some(pos) = self.text[self.next(self.cursor)..].find(&text) {
                            self.cursor = self.next(self.cursor) + pos;
                        } else if let Some(pos) = self.text.find(&text) {
                            self.cursor = pos;
                        }
                    } else {
                        match text.trim() {
                            "ask" => return Ok(Action::Ask),
                            "w" | "write" => return Ok(Action::Publish),
                            "apply" => return Ok(Action::Apply),
                            "clear" => {
                                self.replace(0, self.text.len(), "")?;
                            }
                            line => {
                                if let Ok(line) = line.parse::<usize>() {
                                    self.cursor = self
                                        .text
                                        .match_indices('\n')
                                        .map(|(p, _)| p + 1)
                                        .nth(line.saturating_sub(2))
                                        .filter(|_| line > 1)
                                        .unwrap_or(0);
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
            return Ok(Action::None);
        }
        let moving = matches!(
            key.code,
            KeyCode::Left
                | KeyCode::Right
                | KeyCode::Up
                | KeyCode::Down
                | KeyCode::Home
                | KeyCode::End
                | KeyCode::PageUp
                | KeyCode::PageDown
        );
        if moving && key.modifiers.contains(KeyModifiers::SHIFT) && self.anchor.is_none() {
            self.anchor = Some(self.cursor);
        }
        match key.code {
            KeyCode::Esc => {
                self.insert = false;
                self.anchor = None;
            }
            KeyCode::Tab | KeyCode::BackTab if !self.insert => return Ok(Action::Workbench),
            KeyCode::Left => self.cursor = self.prev(self.cursor),
            KeyCode::Right => self.cursor = self.next(self.cursor),
            KeyCode::Up => self.vertical(-1),
            KeyCode::Down => self.vertical(1),
            KeyCode::PageUp => self.vertical(-15),
            KeyCode::PageDown => self.vertical(15),
            KeyCode::Home => self.cursor = self.line_start(),
            KeyCode::End => self.cursor = self.line_end(),
            KeyCode::Backspace if self.insert => {
                let (a, b) = self
                    .selected()
                    .unwrap_or((self.prev(self.cursor), self.cursor));
                self.replace(a, b, "")?;
            }
            KeyCode::Delete => {
                let (a, b) = self
                    .selected()
                    .unwrap_or((self.cursor, self.next(self.cursor)));
                self.replace(a, b, "")?;
            }
            KeyCode::Enter if self.insert => {
                self.paste("\n")?;
            }
            KeyCode::Tab if self.insert => {
                self.paste("    ")?;
            }
            KeyCode::Char(c) if self.insert && !key.modifiers.contains(KeyModifiers::ALT) => {
                self.paste(&c.to_string())?;
            }
            KeyCode::Char('i') => self.insert = true,
            KeyCode::Char('a') => {
                self.cursor = self.next(self.cursor);
                self.insert = true;
            }
            KeyCode::Char('o') => {
                self.cursor = self.line_end();
                self.paste("\n")?;
                self.insert = true;
            }
            KeyCode::Char('h') => self.cursor = self.prev(self.cursor),
            KeyCode::Char('l') => self.cursor = self.next(self.cursor),
            KeyCode::Char('j') => self.vertical(1),
            KeyCode::Char('k') => self.vertical(-1),
            KeyCode::Char('g') => self.cursor = 0,
            KeyCode::Char('G') => self.cursor = self.text.len(),
            KeyCode::Char('v') => {
                self.anchor = if self.anchor.is_some() {
                    None
                } else {
                    Some(self.cursor)
                };
            }
            KeyCode::Char('x' | 'd') => {
                let (a, b) = self
                    .selected()
                    .unwrap_or((self.cursor, self.next(self.cursor)));
                self.replace(a, b, "")?;
            }
            KeyCode::Char(':' | '/') => {
                self.command = Some((key.code == KeyCode::Char('/'), String::new()))
            }
            KeyCode::Char(']' | '[') => {
                let current = self.cursor_row();
                let next = if key.code == KeyCode::Char(']') {
                    self.rows
                        .iter()
                        .skip(current + 1)
                        .find(|r| r.tag != ChangeTag::Equal && r.new.is_some())
                } else {
                    self.rows
                        .iter()
                        .take(current)
                        .rev()
                        .find(|r| r.tag != ChangeTag::Equal && r.new.is_some())
                };
                if let Some(line) = next.and_then(|r| r.new) {
                    self.cursor = std::iter::once(0)
                        .chain(self.text.match_indices('\n').map(|(p, _)| p + 1))
                        .nth(line)
                        .unwrap_or(self.text.len());
                }
            }
            _ => {}
        }
        Ok(Action::None)
    }
}
/// Small lexical highlighter: comments, quoted strings, numbers and common language keywords.
/// Not a parser, and unsupported syntax is intentionally plain text.
pub fn tokens(line: &str) -> Vec<(String, u8)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < line.len() {
        let c = line[i..].chars().next().unwrap();
        let start = i;
        let role = if line[i..].starts_with("//") || line[i..].starts_with('#') {
            i = line.len();
            3
        } else if matches!(c, '"' | '\'' | '`') {
            i += c.len_utf8();
            let mut escaped = false;
            while i < line.len() {
                let next = line[i..].chars().next().unwrap();
                i += next.len_utf8();
                if next == c && !escaped {
                    break;
                }
                escaped = next == '\\' && !escaped;
            }
            2
        } else if c.is_alphanumeric() || c == '_' {
            i += c.len_utf8();
            while i < line.len() {
                let ch = line[i..].chars().next().unwrap();
                if !ch.is_alphanumeric() && ch != '_' {
                    break;
                }
                i += ch.len_utf8();
            }
            let word = &line[start..i];
            if c.is_ascii_digit() {
                4
            } else if matches!(
                word,
                "fn" | "pub"
                    | "use"
                    | "let"
                    | "mut"
                    | "struct"
                    | "enum"
                    | "impl"
                    | "trait"
                    | "match"
                    | "if"
                    | "else"
                    | "for"
                    | "while"
                    | "return"
                    | "async"
                    | "await"
                    | "const"
                    | "class"
                    | "function"
                    | "def"
                    | "import"
                    | "from"
                    | "try"
                    | "except"
                    | "self"
                    | "true"
                    | "false"
                    | "null"
                    | "None"
            ) {
                1
            } else {
                0
            }
        } else {
            i += c.len_utf8();
            0
        };
        out.push((line[start..i].into(), role));
    }
    out
}
