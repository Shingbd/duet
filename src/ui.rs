use crate::{
    app::{App, Mode, Page, Panel, View},
    keymap::Action,
};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, Paragraph, Tabs, Wrap},
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

#[derive(Clone, Copy)]
struct Theme {
    bg: Color,
    text: Color,
    muted: Color,
    accent: Color,
    border: Color,
    good: Color,
    warning: Color,
    removed: Color,
}
impl Theme {
    fn new(light: bool) -> Self {
        if light {
            Self {
                bg: Color::Rgb(245, 246, 250),
                text: Color::Rgb(31, 41, 55),
                muted: Color::Rgb(90, 101, 119),
                accent: Color::Rgb(55, 77, 170),
                border: Color::Rgb(173, 183, 204),
                good: Color::Rgb(22, 112, 74),
                warning: Color::Rgb(153, 91, 12),
                removed: Color::Rgb(168, 45, 69),
            }
        } else {
            Self {
                bg: Color::Rgb(17, 21, 30),
                text: Color::Rgb(220, 226, 237),
                muted: Color::Rgb(143, 156, 176),
                accent: Color::Rgb(144, 168, 255),
                border: Color::Rgb(53, 65, 87),
                good: Color::Rgb(122, 205, 169),
                warning: Color::Rgb(234, 190, 116),
                removed: Color::Rgb(231, 136, 156),
            }
        }
    }
    fn block<'a>(&self, title: impl Into<Line<'a>>, focused: bool) -> Block<'a> {
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(title)
            .border_style(Style::default().fg(if focused { self.accent } else { self.border }))
    }
}

#[derive(Default)]
pub struct Renderer {
    reader: ReaderCache,
}
impl Renderer {
    pub fn layout_builds(&self) -> usize {
        self.reader.builds
    }
    pub fn draw(&mut self, frame: &mut Frame, app: &App) {
        draw_cached(frame, app, &mut self.reader);
    }
}

pub fn draw(frame: &mut Frame, app: &App) {
    Renderer::default().draw(frame, app);
}

fn draw_cached(frame: &mut Frame, app: &App, cache: &mut ReaderCache) {
    let theme = Theme::new(app.light);
    let area = frame.area();
    frame.render_widget(
        Block::default().style(Style::default().bg(theme.bg).fg(theme.text)),
        area,
    );
    if area.width < 24 || area.height < 8 {
        frame.render_widget(Paragraph::new("Duet · 请扩大终端至至少 24×8"), area);
        return;
    }
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Min(2),
            Constraint::Length(2),
        ])
        .split(area);
    let badge = if app.branch.is_empty() {
        "未打开项目"
    } else {
        "项目只读浏览"
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled(
                    " DUET ",
                    Style::default()
                        .fg(theme.accent)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(safe_text(&app.name)),
                Span::styled(format!("  ·  {badge}"), Style::default().fg(theme.warning)),
            ]),
            Line::from(Span::styled(
                format!(
                    " {}  ·  {}  ·  待读 {}",
                    safe_text(&app.branch),
                    app.pi_session_label(),
                    app.pending()
                ),
                Style::default().fg(theme.muted),
            )),
        ]),
        rows[0],
    );
    let views = [
        format!("{} 任务", app.keymap.hint(Action::Tasks)),
        format!("{} 审核", app.keymap.hint(Action::Review)),
        format!("{} 项目", app.keymap.hint(Action::Project)),
        format!("{} 知识", app.keymap.hint(Action::Knowledge)),
    ];
    let index = match app.view {
        View::Tasks => 0,
        View::Review => 1,
        View::Project => 2,
        View::Knowledge => 3,
    };
    frame.render_widget(
        Tabs::new(views)
            .select(index)
            .style(Style::default().fg(theme.muted))
            .highlight_style(
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            )
            .divider("  "),
        rows[1],
    );

    let body = rows[2];
    if app.fullscreen {
        reader(frame, app, body, theme, cache);
    } else if body.width >= 116 {
        let cols = Layout::horizontal([
            Constraint::Length(28),
            Constraint::Min(36),
            Constraint::Length(32),
        ])
        .split(body);
        queue(frame, app, cols[0], theme);
        reader(frame, app, cols[1], theme, cache);
        evidence(frame, app, cols[2], theme);
    } else if body.width >= 78 {
        let cols = Layout::horizontal([Constraint::Length(25), Constraint::Min(30)]).split(body);
        queue(frame, app, cols[0], theme);
        if app.panel == Panel::Evidence {
            evidence(frame, app, cols[1], theme);
        } else {
            reader(frame, app, cols[1], theme, cache);
        }
    } else {
        match app.panel {
            Panel::Queue => queue(frame, app, body, theme),
            Panel::Reader => reader(frame, app, body, theme, cache),
            Panel::Evidence => evidence(frame, app, body, theme),
        }
    }
    // Keep the existing status entry, including failure details even when the
    // workflow's status message only reports that a node has stopped.
    let status = if app.run.status == crate::rpc::RunStatus::Failed {
        match app.run.error.as_deref() {
            Some(error) if !app.status.contains(error) => {
                format!("{} · 错误：{error}", app.status)
            }
            _ => app.status.clone(),
        }
    } else {
        app.status.clone()
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                format!(
                    " 焦点：{} · {}",
                    match app.panel {
                        Panel::Queue if app.view == View::Tasks => "任务 / 节点",
                        Panel::Queue => "文件列表",
                        Panel::Reader => "内容",
                        Panel::Evidence => "说明 / 验证",
                    },
                    safe_text(&status)
                ),
                Style::default().fg(theme.warning),
            )),
            Line::from(Span::styled(
                format!(
                    " {}{} 切面板 · {} 解释 · {} 全屏 · {} 已读 · {} 命令 · {} 帮助",
                    if app.view == View::Review {
                        format!("{} 批准节点 · ", app.keymap.hint(Action::ApproveNode))
                    } else {
                        String::new()
                    },
                    app.keymap.hint(Action::NextPanel),
                    app.keymap.hint(Action::Explain),
                    app.keymap.hint(Action::Fullscreen),
                    app.keymap.hint(Action::MarkRead),
                    app.keymap.hint(Action::Palette),
                    app.keymap.hint(Action::Help)
                ),
                Style::default().fg(theme.muted),
            )),
        ]),
        rows[3],
    );

    match &app.mode {
        Mode::Help => {
            let lines: Vec<Line> = app
                .keymap
                .help
                .iter()
                .map(|(_, keys, label)| Line::from(format!("{keys:14} {label}")))
                .collect();
            popup(frame, theme, " 快捷键 · Esc 关闭 ", lines, 64, 24);
        }
        Mode::Message { text } => {
            let title = if matches!(
                app.run.status,
                crate::rpc::RunStatus::Paused | crate::rpc::RunStatus::Pausing
            ) {
                " 记录纠正 · 继续后才发送 "
            } else {
                " 发给 AI · 仍有编码权限 "
            };
            popup(
                frame,
                theme,
                title,
                vec![
                    Line::from(safe_text(text)),
                    Line::from("Enter 提交 · Esc 返回 · 运行中消息在工具边界接收"),
                ],
                76,
                6,
            );
        }
        Mode::Input { search, text } => {
            let title = if *search {
                " 搜索 · Enter 应用 / Esc 返回 "
            } else {
                " 命令 · Enter 执行 / Esc 返回 "
            };
            let area = centered(area, 72, 5);
            frame.render_widget(Clear, area);
            frame.render_widget(
                Paragraph::new(format!("> {}", safe_text(text)))
                    .block(theme.block(title, true))
                    .style(Style::default().bg(theme.bg).fg(theme.text)),
                area,
            );
            let cursor = (2 + text.width()).min(area.width.saturating_sub(2) as usize) as u16;
            frame.set_cursor_position((area.x + cursor, area.y + 1));
        }
        Mode::ConfirmCode { prompt } => popup(
            frame,
            theme,
            " 授权 AI 编码 ",
            vec![
                Line::from(safe_text(prompt)),
                Line::from("代码可能外发 · 文件工具非安全沙箱"),
                Line::from("Enter 授权 · Esc 取消"),
            ],
            72,
            7,
        ),
        Mode::ConfirmRead => popup(
            frame,
            theme,
            " 标记阅读完成？ ",
            vec![
                Line::from("Enter 确认 · Esc 取消"),
                Line::from("仅标记已读，不批准代码。"),
            ],
            64,
            7,
        ),
        Mode::ConfirmNodeApproval {
            checkpoint,
            node_id,
            ..
        } => popup(
            frame,
            theme,
            " 明确批准节点版本 ",
            vec![
                Line::from(format!("节点：{}", safe_text(node_id))),
                Line::from("验收仍未验证。此操作不会运行测试或合并代码。"),
                Line::from(safe_text(checkpoint)),
                Line::from("Enter 批准该版本 · Esc 返回"),
            ],
            76,
            8,
        ),
        Mode::Browse => {}
    }
}

fn queue(frame: &mut Frame, app: &App, area: Rect, theme: Theme) {
    if app.view == View::Tasks {
        task_tree(frame, app, area, theme);
        return;
    }
    if app.view == View::Review && !app.task_rows(true).is_empty() {
        review_tree(frame, app, area, theme);
        return;
    }
    let indices = app.visible_file_indices();
    let selected_path = app.current().map(|item| item.title.as_str());
    let selected_row = indices
        .iter()
        .position(|&i| Some(app.items[i].title.as_str()) == selected_path);
    let items: Vec<ListItem> = indices
        .iter()
        .enumerate()
        .map(|(row, &i)| {
            let item = &app.items[i];
            let kind = match item.change {
                Some(crate::checkpoint::ChangeKind::Added) => "A",
                Some(crate::checkpoint::ChangeKind::Modified) => "M",
                Some(crate::checkpoint::ChangeKind::Deleted) => "D",
                None => " ",
            };
            let color = match item.change {
                Some(crate::checkpoint::ChangeKind::Added) => theme.good,
                Some(crate::checkpoint::ChangeKind::Modified) => theme.warning,
                Some(crate::checkpoint::ChangeKind::Deleted) => theme.removed,
                None if Some(row) == selected_row => theme.accent,
                None => theme.muted,
            };
            let marker = if item.read { "✓" } else { "○" };
            let style = if Some(row) == selected_row {
                Style::default().fg(color).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(color)
            };
            ListItem::new(Line::from(format!(
                "{} {kind} {marker} {}",
                if Some(row) == selected_row {
                    "›"
                } else {
                    " "
                },
                safe_text(&item.title)
            )))
            .style(style)
        })
        .collect();
    // Window follows selection only when the user moves selection, never new delivery.
    let visible = area.height.saturating_sub(2).max(1) as usize;
    let start = selected_row.unwrap_or(0).saturating_sub(visible - 1);
    frame.render_widget(
        List::new(
            items
                .into_iter()
                .skip(start)
                .take(visible)
                .collect::<Vec<_>>(),
        )
        .block(theme.block(" 文件 / 待审 ", app.panel == Panel::Queue)),
        area,
    );
}

fn task_tree(frame: &mut Frame, app: &App, area: Rect, theme: Theme) {
    let rows = app.task_rows(false);
    let selected = rows
        .iter()
        .position(|(s, _)| Some(s) == app.task_selection.as_ref())
        .unwrap_or(0);
    let height = area.height.saturating_sub(2) as usize;
    let start = selected.saturating_sub(height.saturating_sub(1));
    let items: Vec<_> = rows
        .iter()
        .enumerate()
        .skip(start)
        .take(height)
        .map(|(i, (s, label))| {
            let color = if s.node_id.is_some() {
                theme.accent
            } else {
                theme.text
            };
            ListItem::new(Line::from(Span::styled(
                format!(
                    "{} {}",
                    if Some(s) == app.task_selection.as_ref() {
                        "›"
                    } else {
                        " "
                    },
                    safe_text(label)
                ),
                if i == selected {
                    Style::default().fg(color).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(color)
                },
            )))
        })
        .collect();
    frame.render_widget(
        List::new(items).block(theme.block(" 任务 · 节点 ", app.panel == Panel::Queue)),
        area,
    );
}

fn review_tree(frame: &mut Frame, app: &App, area: Rect, theme: Theme) {
    let rows = app.task_rows(true);
    let indices = app.changed_indices();
    let mut items = Vec::new();
    let mut selected = 0;
    for (selection, label) in rows {
        let chosen = Some(&selection) == app.task_selection.as_ref();
        if chosen {
            selected = items.len();
        }
        items.push(ListItem::new(Line::from(Span::styled(
            format!("{} {}", if chosen { "›" } else { " " }, safe_text(&label)),
            Style::default().fg(if chosen { theme.accent } else { theme.muted }),
        ))));
        if chosen {
            for &index in &indices {
                let item = &app.items[index];
                let color = match item.change {
                    Some(crate::checkpoint::ChangeKind::Added) => theme.good,
                    Some(crate::checkpoint::ChangeKind::Deleted) => theme.removed,
                    _ => theme.warning,
                };
                if index == app.selected {
                    selected = items.len();
                }
                items.push(ListItem::new(Line::from(Span::styled(
                    format!(
                        "{} {} {} {}",
                        if index == app.selected { "›" } else { " " },
                        match item.change {
                            Some(crate::checkpoint::ChangeKind::Added) => "A",
                            Some(crate::checkpoint::ChangeKind::Deleted) => "D",
                            _ => "M",
                        },
                        if item.read { "✓" } else { "○" },
                        safe_text(&item.title)
                    ),
                    Style::default().fg(color),
                ))));
            }
        }
    }
    let height = area.height.saturating_sub(2) as usize;
    let start = selected.saturating_sub(height.saturating_sub(1));
    frame.render_widget(
        List::new(
            items
                .into_iter()
                .skip(start)
                .take(height)
                .collect::<Vec<_>>(),
        )
        .block(theme.block(
            format!(
                " 待审节点 · {} / {} ",
                app.keymap.hint(Action::PreviousNode),
                app.keymap.hint(Action::NextNode)
            ),
            app.panel == Panel::Queue,
        )),
        area,
    );
}

#[derive(PartialEq, Eq)]
struct ReaderKey {
    node_evidence: Option<String>,
    scope: Option<String>,
    width: u16,
    selected: usize,
    view: View,
    page: Page,
    version: String,
    filter: String,
    revision: u64,
    is_diff: bool,
}
#[derive(Default)]
struct ReaderCache {
    key: Option<ReaderKey>,
    lines: Vec<(String, i8)>,
    builds: usize,
}
impl ReaderCache {
    fn prepare(&mut self, app: &App, width: u16) {
        let key = ReaderKey {
            node_evidence: if app.view == View::Review && app.page == Page::Evidence {
                app.node_evidence()
            } else {
                None
            },
            scope: if app.view == View::Review {
                app.review_checkpoint().map(str::to_owned)
            } else {
                None
            },
            width,
            is_diff: app.view == View::Review
                && app.current().is_some_and(|item| item.change.is_some()),
            selected: app.selected,
            view: app.view,
            page: app.page,
            version: app
                .current()
                .map_or_else(String::new, |item| item.version.clone()),
            filter: app.filter.clone(),
            revision: if matches!(app.view, View::Tasks | View::Knowledge) {
                app.content_revision
            } else {
                0
            },
        };
        if self.key.as_ref() == Some(&key) {
            return;
        }
        self.lines.clear();
        self.builds += 1;
        let page = match app.page {
            Page::Intent => "意图",
            Page::Code
                if app.view == View::Review
                    && app.current().is_some_and(|item| item.change.is_some()) =>
            {
                "差异"
            }
            Page::Code => "代码",
            Page::Evidence => "证据",
        };
        if app.view == View::Tasks {
            for (line, role) in app
                .task_lines()
                .into_iter()
                .filter(|(line, _)| app.filter.is_empty() || line.contains(&app.filter))
            {
                for wrapped in wrap_cells(&safe_text(&line), width as usize) {
                    self.lines.push((wrapped, role));
                }
            }
            self.key = Some(key);
            return;
        }
        if let Some(item) = app.current() {
            for line in wrap_cells(
                &format!("{page} · {}", safe_text(&item.version)),
                width as usize,
            ) {
                self.lines.push((line, 2));
            }
            self.lines.push((String::new(), 2));
        }
        let body = app.body();
        for line in body
            .split('\n')
            .filter(|line| app.filter.is_empty() || line.contains(&app.filter))
        {
            let role = if line.starts_with('+') {
                1
            } else if line.starts_with('-') {
                -1
            } else {
                0
            };
            for wrapped in wrap_cells(&safe_text(line), width as usize) {
                self.lines.push((wrapped, role));
            }
        }
        self.key = Some(key);
    }
}

/// Cell-aware wrapping happens once per content/width change, not on each scroll.
/// Grapheme boundaries preserve CJK, emoji and combining characters.
pub fn wrap_cells(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut columns = 0;
    for glyph in text.graphemes(true) {
        let size = glyph.width();
        if columns + size > width && !line.is_empty() {
            lines.push(std::mem::take(&mut line));
            columns = 0;
        }
        if size > width {
            line.push('�');
            columns += 1;
        } else {
            line.push_str(glyph);
            columns += size;
        }
    }
    lines.push(line);
    lines
}

fn reader(frame: &mut Frame, app: &App, area: Rect, theme: Theme, cache: &mut ReaderCache) {
    cache.prepare(app, area.width.saturating_sub(2));
    let scroll = if app.view == View::Tasks {
        app.task_scroll
    } else {
        app.current().map_or(0, |item| item.scroll)
    } as usize;
    let content: Vec<Line> = cache
        .lines
        .iter()
        .skip(scroll)
        .take(area.height.saturating_sub(2) as usize)
        .map(|(text, role)| {
            let color = match role {
                1 => theme.good,
                -1 => theme.removed,
                2 => theme.muted,
                3 | 7 => theme.accent,
                4 | 8 => theme.good,
                _ => theme.text,
            };
            let style = if matches!(role, 7 | 8) {
                Style::default().fg(color).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(color)
            };
            Line::from(Span::styled(text.as_str(), style))
        })
        .collect();
    frame.render_widget(
        Paragraph::new(content).block(theme.block(
            format!(
                    " {} ",
                    match app.view {
                        View::Tasks =>
                            if app.selected_node().is_some() {
                                "节点契约".to_owned()
                            } else {
                                "任务对话".to_owned()
                            },
                        View::Knowledge => "笔记".to_owned(),
                        View::Project => "项目".to_owned(),
                        View::Review => app
                            .current()
                            .map(|item| safe_text(&item.title))
                            .unwrap_or_else(|| "代码 / 变更".into()),
                    }
                ),
            app.panel == Panel::Reader,
        )),
        area,
    );
}

fn evidence(frame: &mut Frame, app: &App, area: Rect, theme: Theme) {
    let text = app
        .node_evidence()
        .or_else(|| {
            if app.view == View::Tasks {
                return Some(
                    app.selected_task()
                        .and_then(|t| t.plan.as_ref())
                        .map(|p| {
                            format!(
                                "总体目标：{}\n约束：{}\n验收（未验证）：\n{}",
                                p.objective,
                                p.constraints.join("；"),
                                p.acceptance.join("\n")
                            )
                        })
                        .unwrap_or_else(|| "暂无节点契约".into()),
                );
            }
            None
        })
        .unwrap_or_else(|| {
            app.current()
                .map(|item| {
                    let mut parts = Vec::new();
                    if app.explain && !item.explanation.is_empty() {
                        parts.push(item.explanation.as_str());
                    }
                    if !item.evidence.is_empty() {
                        parts.push(item.evidence.as_str());
                    }
                    parts.join("\n\n")
                })
                .unwrap_or_default()
        });
    let text = if text.is_empty() {
        "暂无记录".to_owned()
    } else {
        text
    };
    frame.render_widget(
        Paragraph::new(safe_text(&text))
            .block(theme.block(" 说明 / 验证 ", app.panel == Panel::Evidence))
            .style(Style::default().fg(theme.muted))
            .wrap(Wrap { trim: false })
            .scroll((
                if app.view == View::Tasks {
                    app.task_evidence_scroll
                } else {
                    app.current().map_or(0, |item| item.evidence_scroll)
                },
                0,
            )),
        area,
    );
}

/// Never emit terminal control sequences from repository or model content.
pub fn safe_text(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    for ch in text.chars() {
        if ch == '\t' {
            output.push_str("    ");
        } else if ch == '\n' || !ch.is_control() {
            output.push(ch);
        } else {
            output.push('�');
        }
    }
    output
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    )
}

fn popup(frame: &mut Frame, theme: Theme, title: &str, lines: Vec<Line>, width: u16, height: u16) {
    let area = centered(frame.area(), width, height);
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines)
            .style(Style::default().bg(theme.bg).fg(theme.text))
            .block(theme.block(title, true))
            .wrap(Wrap { trim: false }),
        area,
    );
}
