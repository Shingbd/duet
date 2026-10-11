use crate::{
    app::{App, Mode, NodeContractContent, Page, Panel, View},
    keymap::Action,
    markdown::{self, Context as MarkdownContext, Fragment, InlineStyle, Role, SemanticLine, Status},
    nodes::{Execution, Review, Verification},
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
    fn markdown_style(&self, context: MarkdownContext, inline: InlineStyle) -> Style {
        // Outer role, actual contract status, and Markdown are independent axes.
        // Role/status labels remain the caller's responsibility; document task
        // markers deliberately do not affect the status palette.
        let role_color = match context.role {
            Role::Body => self.text,
            Role::User | Role::Acceptance => self.good,
            Role::Assistant | Role::Field => self.accent,
            Role::System => self.muted,
            Role::Constraint => self.warning,
        };
        let color = match context.status {
            Status::None => role_color,
            Status::Pending => self.warning,
            Status::Success => self.good,
            Status::Failure => self.removed,
        };
        let mut style = Style::default().fg(color);
        if context.status != Status::None {
            style = style.add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
        }
        if inline.quote && context.role == Role::Body && context.status == Status::None {
            style = style.fg(self.muted);
        }
        if inline.marker && context.status == Status::None {
            style = style.fg(self.muted);
        }
        if inline.heading > 0 {
            style = style.fg(self.accent).add_modifier(Modifier::BOLD);
        }
        if inline.strong { style = style.add_modifier(Modifier::BOLD); }
        if inline.emphasis { style = style.add_modifier(Modifier::ITALIC); }
        if inline.strike { style = style.add_modifier(Modifier::CROSSED_OUT); }
        if inline.link {
            style = style.fg(self.accent).add_modifier(Modifier::UNDERLINED);
        }
        if inline.code {
            style = style.fg(self.warning).bg(self.bg);
        }
        // Real status and constraint warnings take priority over Markdown
        // heading/link/code colors. Inline modifiers remain intact.
        if context.status != Status::None {
            style = style.fg(color);
        } else if context.role == Role::Constraint {
            style = style.fg(self.warning);
        }
        style
    }
    fn block<'a>(&self, title: impl Into<Line<'a>>, focused: bool) -> Block<'a> {
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(title)
            .border_style(Style::default().fg(if focused { self.accent } else { self.border }))
    }
}

/// Final theme mapping for the display-only Markdown pipeline. This does not
/// reinterpret literal code/diffs or change existing readers. Callers retain
/// explicit chat-role and real-status labels alongside the returned lines.
pub fn style_markdown(lines: &[SemanticLine], light: bool) -> Vec<Line<'static>> {
    let theme = Theme::new(light);
    lines.iter().map(|line| {
        Line::from(line.fragments.iter().map(|fragment| {
            Span::styled(
                safe_text(&fragment.text).replace('\n', " "),
                theme.markdown_style(line.context, fragment.style),
            )
        }).collect::<Vec<_>>())
    }).collect()
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
                        format!(
                            "{} 批准 · 自动实时预览 · {} 合并 · {} 删除 · ",
                            app.keymap.hint(Action::ApproveNode),
                            app.keymap.hint(Action::MergeTask),
                            app.keymap.hint(Action::DeleteSelected)
                        )
                    } else if app.view == View::Tasks {
                        format!(
                            "{} 讨论 · {} 启动 · {} 删除 · {} 合并 · ",
                            app.keymap.hint(Action::Reply),
                            app.keymap.hint(Action::StartTask),
                            app.keymap.hint(Action::DeleteSelected),
                            app.keymap.hint(Action::MergeTask)
                        )
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
            let title = if app.planning_only {
                " 只读讨论计划 · 代码可能外发 · Enter 发送 "
            } else if matches!(
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
            " 开始只读讨论与规划 ",
            vec![
                Line::from(safe_text(prompt)),
                Line::from("代码可能外发 · 文件工具非安全沙箱"),
                Line::from("Enter 只读讨论 · 编码需另按 s 确认 · Esc 取消"),
            ],
            72,
            7,
        ),
        Mode::MergePreview { preview, scroll } => {
            let height = frame.area().height.saturating_sub(4);
            let mut lines = vec![
                Line::from(format!(
                    "整任务 {} · {} 个变更文件 · {} 个节点未验证",
                    preview.task_id,
                    preview.paths.len(),
                    preview.unverified
                )),
                Line::from(format!(
                    "主项目 {} → 候选 {}",
                    &preview.source_head[..12],
                    &preview.delivery[..12]
                )),
                Line::from("j/k / PageUp/Down 查看完整集成 diff · Enter 下一步确认 · Esc 取消"),
            ];
            for line in preview
                .diff
                .lines()
                .skip(*scroll)
                .take(height.saturating_sub(7) as usize)
            {
                let color = if line.starts_with('+') {
                    theme.good
                } else if line.starts_with('-') {
                    theme.removed
                } else {
                    theme.text
                };
                lines.push(Line::from(Span::styled(
                    safe_text(line),
                    Style::default().fg(color),
                )));
            }
            popup(
                frame,
                theme,
                " 最终集成 diff · 主项目尚未变化 ",
                lines,
                128,
                height,
            );
        }
        Mode::ConfirmMerge { preview } => popup(
            frame,
            theme,
            " 明确合并整个任务到主项目 ",
            vec![
                Line::from(format!(
                    "任务：{} · 文件数：{}",
                    preview.task_id,
                    preview.paths.len()
                )),
                Line::from(format!(
                    "未验证节点 {} 个；人工批准不代表测试通过。",
                    preview.unverified
                )),
                Line::from("Enter 明确接受当前验证状态并合并 · Esc 取消"),
                Line::from("不会运行测试/仓库 hooks/推送/部署；冲突或主项目改变会拒绝。"),
            ],
            100,
            9,
        ),
        Mode::ConfirmStart { task_id, .. } => {
            let task = app.tasks.iter().find(|t| &t.id == task_id);
            let mut lines = vec![
                Line::from("确认当前计划，授权模型修改独立候选工作区；不自动测试或合并。"),
                Line::from("代码可能外发；文件工具并非 OS 安全沙箱。"),
            ];
            if let Some(t) = task {
                for n in &t.nodes {
                    lines.push(Line::from(format!(
                        "{} · {} · {}",
                        n.contract.id,
                        n.contract.title,
                        n.contract.scope.join("、")
                    )));
                }
            }
            lines.push(Line::from("Enter 启动实施 · Esc 返回讨论"));
            popup(frame, theme, " 启动已有任务计划 ", lines, 90, 18);
        }
        Mode::ConfirmDelete { selection } => popup(
            frame,
            theme,
            " 删除并撤回候选修改 ",
            vec![
                Line::from(if selection.node_id.is_some() {
                    "删除所选节点及其全部后续节点（顺序版本有依赖）。"
                } else {
                    "删除整个任务、全部节点及其候选工作区。"
                }),
                Line::from(if app.selected_task().is_some_and(|t| t.merged.is_some()) {
                    "已合并：此操作仅清理记录/候选，不撤销主项目合并。"
                } else {
                    "删除相关讨论与预览；先备份。主项目和其他任务不受影响。"
                }),
                Line::from("运行中的所选任务将停止，候选中的未追踪/忽略文件一并撤回。"),
                Line::from("Enter 确认删除 · Esc 取消"),
            ],
            90,
            9,
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
                " 节点审核 · {} / {} ",
                app.keymap.hint(Action::PreviousNode),
                app.keymap.hint(Action::NextNode)
            ),
            app.panel == Panel::Queue,
        )),
        area,
    );
}

/// Display blocks are built from fields, never by splitting node_description.
/// Each block retains its raw searchable text as well as styled rendered text.
struct ContractBlock {
    source: String,
    lines: Vec<SemanticLine>,
}

fn contract_literal(text: &str, width: usize, role: Role, status: Status, strong: bool) -> Vec<SemanticLine> {
    markdown::wrap(
        &[Fragment { text: text.to_owned(), style: InlineStyle { strong, ..Default::default() } }],
        width,
        MarkdownContext { role, status },
    )
}

fn contract_field(label: &str, values: &[String], width: usize, role: Role, literal: bool) -> ContractBlock {
    let mut lines = contract_literal(label, width, Role::Field, Status::None, true);
    if values.is_empty() {
        lines.extend(contract_literal("（无）", width, Role::System, Status::None, false));
    }
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            lines.extend(markdown::wrap(&[], width, MarkdownContext::default()));
        }
        if literal {
            lines.extend(contract_literal(value, width, role, Status::None, false));
        } else {
            // Parse each complete field/list entry, not each physical source line.
            lines.extend(markdown::render(value, width, MarkdownContext { role, status: Status::None }));
        }
    }
    ContractBlock { source: format!("{label}\n{}", values.join("\n")), lines }
}

fn contract_status(label: &str, value: &str, status: Status, width: usize) -> ContractBlock {
    let text = format!("{label}：{value}");
    ContractBlock {
        source: text.clone(),
        lines: contract_literal(&text, width, Role::Body, status, true),
    }
}

/// Human-started checks (not automatically run):
/// `cargo test --lib ui::contract_tests`. In Tasks and Review/Intent, inspect
/// the same node in both themes; search a body phrase, path and version; clear
/// search; resize; scroll through every acceptance entry. Confirm literal
/// `[x]` source markers never change the separate verification/review labels.
fn contract_layout(content: &NodeContractContent, width: usize, filter: &str) -> Vec<SemanticLine> {
    let node = &content.contract;
    let mut blocks = Vec::new();
    let mut title = markdown::render(&node.title, width, MarkdownContext { role: Role::Field, status: Status::None });
    for fragment in title.iter_mut().flat_map(|l| &mut l.fragments) {
        fragment.style.strong = true;
    }
    let mut title_lines = contract_literal("节点契约", width, Role::Field, Status::None, true);
    title_lines.extend(title);
    blocks.push(ContractBlock { source: format!("节点契约\n{}", node.title), lines: title_lines });
    blocks.push(contract_field("节点 ID", std::slice::from_ref(&node.id), width, Role::System, true));
    blocks.push(contract_field("任务 ID", std::slice::from_ref(&content.task_id), width, Role::System, true));

    let (execution, execution_status) = match content.execution {
        Execution::Planned => ("未开始", Status::Pending),
        Execution::Running => ("执行中", Status::Pending),
        Execution::Paused => ("已暂停", Status::Pending),
        Execution::Implemented => ("实现完成（不代表验收通过）", Status::None),
        Execution::NoChanges => ("未交付（无文件改动）", Status::Pending),
        Execution::Failed => ("失败", Status::Failure),
        Execution::Cancelled => ("已取消", Status::Pending),
        Execution::Interrupted => ("已中断", Status::Pending),
    };
    blocks.push(contract_status("执行状态", execution, execution_status, width));
    let (verification, verification_status) = match content.verification {
        Verification::Unverified => ("未验证", Status::Pending),
        Verification::Passed => ("通过", Status::Success),
        Verification::Failed => ("失败", Status::Failure),
    };
    blocks.push(contract_status("验收状态", verification, verification_status, width));
    let (review, review_status) = match content.review {
        Review::Pending => ("待审", Status::Pending),
        Review::Approved => ("已批准", Status::Success),
        Review::ChangesRequested => ("要求返工", Status::Failure),
    };
    blocks.push(contract_status("人工审核", review, review_status, width));
    blocks.push(contract_field("目标", std::slice::from_ref(&node.objective), width, Role::Body, false));
    blocks.push(contract_field("文件范围", &node.scope, width, Role::Body, true));
    blocks.push(contract_field("边界条件", &node.boundaries, width, Role::Constraint, false));
    blocks.push(contract_field("实现方法", std::slice::from_ref(&node.implementation), width, Role::Body, false));
    blocks.push(contract_field("任务约束", &content.task_constraints, width, Role::Constraint, false));
    blocks.push(contract_field("节点约束", &node.constraints, width, Role::Constraint, false));
    blocks.push(contract_field("依赖", &node.dependencies, width, Role::Body, true));

    blocks.push(contract_status("验收条件", "逐项未验证；文档复选框不代表真实验收或批准", Status::Pending, width));
    if node.acceptance.is_empty() {
        blocks.push(contract_status("验收条件", "未提供条件，不能视为通过", Status::Pending, width));
    }
    for (index, acceptance) in node.acceptance.iter().enumerate() {
        // This warning is a distinct literal block: Markdown cannot restyle it.
        let label = format!("□ 未验证 · 验收条件 {}", index + 1);
        let mut lines = contract_literal(&label, width, Role::Constraint, Status::Pending, true);
        lines.extend(markdown::render(acceptance, width, MarkdownContext::default()));
        blocks.push(ContractBlock { source: format!("{label}\n{acceptance}"), lines });
    }
    let start = content.start_version.clone().unwrap_or_else(|| "尚未执行".into());
    let completion = content.completion_version.clone().unwrap_or_else(|| "尚未生成".into());
    blocks.push(contract_field("开始版本", &[start], width, Role::System, true));
    blocks.push(contract_field("完成版本", &[completion], width, Role::System, true));

    let mut lines = Vec::new();
    for block in blocks {
        // Filter whole already-parsed blocks, retaining labels and cross-line
        // Markdown structure. Match raw and rendered content before cell wrapping
        // can split a phrase. Concatenation also permits long literal metadata.
        let rendered: String = block.lines.iter().map(SemanticLine::text).collect();
        if !filter.is_empty() && !block.source.contains(filter) && !rendered.contains(filter) {
            continue;
        }
        if !lines.is_empty() {
            lines.extend(markdown::wrap(&[], width, MarkdownContext::default()));
        }
        lines.extend(block.lines);
    }
    lines
}

#[derive(PartialEq, Eq)]
struct ReaderKey {
    contract: Option<NodeContractContent>,
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
    contract_lines: Option<Vec<SemanticLine>>,
    builds: usize,
}
impl ReaderCache {
    fn prepare(&mut self, app: &App, width: u16) {
        let contract = if app.showing_node_contract() { app.node_contract_content() } else { None };
        // The complete domain snapshot already tracks contract edits and actual
        // statuses. Unrelated chat/note revisions must not rebuild this layout.
        let revision = if contract.is_none() && matches!(app.view, View::Tasks | View::Knowledge) {
            app.content_revision
        } else {
            0
        };
        let key = ReaderKey {
            contract,
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
            revision,
        };
        if self.key.as_ref() == Some(&key) {
            return;
        }
        self.lines.clear();
        self.contract_lines = None;
        self.builds += 1;
        if let Some(contract) = &key.contract {
            self.contract_lines = Some(contract_layout(contract, width as usize, &app.filter));
            self.key = Some(key);
            return;
        }
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
    let scroll = if app.view == View::Tasks || cache.contract_lines.is_some() {
        app.task_scroll
    } else {
        app.current().map_or(0, |item| item.scroll)
    } as usize;
    let content: Vec<Line> = if let Some(lines) = &cache.contract_lines {
        lines.iter().skip(scroll).take(area.height.saturating_sub(2) as usize).map(|line| {
            Line::from(line.fragments.iter().map(|fragment| {
                Span::styled(fragment.text.as_str(), theme.markdown_style(line.context, fragment.style))
            }).collect::<Vec<_>>())
        }).collect()
    } else {
        cache.lines
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
        .collect()
    };
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

/// Manual regression procedure (run only when explicitly requested):
/// 1. `cargo test --lib ui::contract_tests`.
/// 2. Open one node in Tasks and Review/Intent, in light and dark themes.
///    Inspect title/labels, normal body, warning boundaries/constraints and
///    muted literal IDs/paths/versions; scroll to the last acceptance condition.
/// 3. Search a Markdown body phrase and a literal path/version, clear search,
///    switch nodes, revise the contract and resize. Check updated content and
///    inline emphasis on wrapped lines; scrolling alone must not rebuild it.
/// 4. Put `- [x] document only` in an acceptance field. Its separate `□ 未验证`
///    label and the actual verification/review statuses must remain independent.
#[cfg(test)]
mod contract_tests {
    use super::*;
    use crate::{
        app::TaskSelection,
        keymap::Keymap,
        nodes::{NodeContract, NodeRecord, TaskRecord},
    };

    fn content() -> NodeContractContent {
        NodeContractContent {
            task_id: "task_literal".into(),
            contract: NodeContract {
                id: "node_literal".into(),
                title: "**契约标题**".into(),
                objective: "正文 **强调中文 emoji 👩‍💻 e\u{301} continuation**".into(),
                scope: vec!["src/[literal]_file.rs".into()],
                boundaries: vec!["不可 [链接](https://example.test) **越界**".into()],
                implementation: "## 方法\n\n> 说明\n\n- 第一步\n- 第二步".into(),
                constraints: vec!["节点 **约束**".into()],
                dependencies: vec!["dependency_literal".into()],
                acceptance: vec!["- [x] document only".into(), "第二条验收".into()],
            },
            task_constraints: vec!["任务 **约束**".into()],
            execution: Execution::Implemented,
            verification: Verification::Unverified,
            review: Review::Pending,
            start_version: Some("base_[literal]".into()),
            completion_version: Some("completion_*literal*".into()),
        }
    }

    fn text(lines: &[SemanticLine]) -> String {
        lines.iter().map(SemanticLine::text).collect::<Vec<_>>().join("\n")
    }

    fn app() -> App {
        let content = content();
        let mut task = TaskRecord::legacy(&content.task_id, "需求", "session");
        task.nodes.push(NodeRecord::new(content.contract));
        let mut app = App::empty(Keymap::default());
        app.tasks.push(task);
        app.view = View::Tasks;
        app.select_task_entry(TaskSelection {
            task_id: content.task_id,
            node_id: Some("node_literal".into()),
        });
        app
    }

    #[test]
    fn fields_and_literal_metadata_are_retained() {
        let lines = contract_layout(&content(), 100, "");
        let output = text(&lines);
        for expected in ["契约标题", "节点 ID", "任务 ID", "执行状态", "验收状态", "人工审核",
            "目标", "文件范围", "边界条件", "实现方法", "任务约束", "节点约束", "依赖",
            "第二条验收", "开始版本", "完成版本", "src/[literal]_file.rs",
            "dependency_literal", "base_[literal]", "completion_*literal*"] {
            assert!(output.contains(expected), "missing {expected}");
        }
        assert!(!output.contains("**契约标题**"));
        assert!(lines.iter().flat_map(|l| &l.fragments).any(|f| f.style.heading > 0));
        for light in [false, true] {
            let theme = Theme::new(light);
            let normal = theme.markdown_style(MarkdownContext::default(), InlineStyle::default());
            for (role, color) in [(Role::Field, theme.accent), (Role::Constraint, theme.warning), (Role::System, theme.muted)] {
                let style = theme.markdown_style(MarkdownContext { role, status: Status::None }, InlineStyle::default());
                assert_eq!(style.fg, Some(color));
                assert_ne!(style.fg, normal.fg);
            }
        }
    }

    #[test]
    fn wrapped_body_preserves_inline_and_warning_semantics() {
        let field = contract_field("目标", &[content().contract.objective], 12, Role::Body, false);
        assert!(field.lines.iter().all(|l| l.width() <= 12));
        let strong: Vec<_> = field.lines.iter().skip(1)
            .filter(|l| l.fragments.iter().any(|f| f.style.strong)).collect();
        assert!(strong.len() > 1);
        let joined: String = field.lines.iter().skip(1).map(SemanticLine::text).collect();
        assert!(joined.contains("👩‍💻 e\u{301}"));
        assert!(!joined.contains("**"));
        let warning = contract_field("边界", &["# 标题\n\n[链接](url) `code`".into()], 12, Role::Constraint, false);
        for light in [false, true] {
            assert!(style_markdown(&warning.lines[1..], light).iter().flat_map(|l| &l.spans)
                .all(|s| s.style.fg == Some(Theme::new(light).warning)));
        }
    }

    #[test]
    fn document_checkboxes_never_become_real_success() {
        let mut content = content();
        let lines = contract_layout(&content, 100, "document only");
        assert!(text(&lines).contains("□ 未验证 · 验收条件 1"));
        assert!(lines.iter().all(|l| l.context.status != Status::Success));
        content.verification = Verification::Passed;
        content.review = Review::Approved;
        let lines = contract_layout(&content, 100, "");
        assert!(lines.iter().any(|l| l.text() == "验收状态：通过" && l.context.status == Status::Success));
        assert!(lines.iter().any(|l| l.text() == "人工审核：已批准" && l.context.status == Status::Success));
        // No per-condition evidence exists: even an aggregate Passed status
        // cannot silently check individual conditions or document markers.
        assert!(text(&lines).contains("□ 未验证 · 验收条件 2"));
    }

    #[test]
    fn search_matches_source_and_rendered_text_before_wrapping() {
        for filter in ["正文 **强调", "正文 强调", "src/[literal]_file.rs", "completion_*literal*"] {
            let lines = contract_layout(&content(), 8, filter);
            assert!(!lines.is_empty(), "missing search result for {filter}");
            assert!(lines.iter().all(|l| l.width() <= 8));
        }
        assert!(contract_layout(&content(), 8, "不存在的内容").is_empty());
        assert!(contract_layout(&content(), 0, "").is_empty());
    }

    #[test]
    fn cache_tracks_contract_selection_page_width_and_search_not_scroll() {
        let mut app = app();
        let mut cache = ReaderCache::default();
        cache.prepare(&app, 40);
        assert_eq!(cache.builds, 1);
        app.task_scroll = 4;
        app.content_revision += 1; // unrelated chat/note update
        app.light = !app.light;
        cache.prepare(&app, 40);
        assert_eq!(cache.builds, 1);
        app.tasks[0].nodes[0].contract.objective = "修订目标".into();
        cache.prepare(&app, 40);
        assert_eq!(cache.builds, 2);
        assert!(text(cache.contract_lines.as_ref().unwrap()).contains("修订目标"));
        app.tasks[0].nodes[0].verification = Verification::Failed;
        cache.prepare(&app, 40);
        assert_eq!(cache.builds, 3);
        assert!(text(cache.contract_lines.as_ref().unwrap()).contains("验收状态：失败"));
        app.filter = "修订目标".into();
        cache.prepare(&app, 40);
        assert_eq!(cache.builds, 4);
        cache.prepare(&app, 12);
        assert_eq!(cache.builds, 5);
        app.filter.clear();
        app.view = View::Review;
        app.page = Page::Intent;
        cache.prepare(&app, 40);
        assert_eq!(cache.builds, 6);
        let review_lines = cache.contract_lines.clone().unwrap();
        app.view = View::Tasks;
        cache.prepare(&app, 40);
        assert_eq!(review_lines, *cache.contract_lines.as_ref().unwrap());
        let mut second = content().contract;
        second.id = "second_node".into();
        second.title = "第二个节点".into();
        app.tasks[0].nodes.push(NodeRecord::new(second));
        app.select_task_entry(TaskSelection { task_id: "task_literal".into(), node_id: Some("second_node".into()) });
        cache.prepare(&app, 40);
        assert!(text(cache.contract_lines.as_ref().unwrap()).contains("第二个节点"));
        app.view = View::Review;
        app.page = Page::Code;
        cache.prepare(&app, 40);
        assert!(cache.contract_lines.is_none());
    }

    #[test]
    fn legacy_plain_text_interfaces_remain_available() {
        let app = app();
        let description = app.node_description().unwrap();
        assert!(description.contains("src/[literal]_file.rs"));
        assert!(description.contains("□ - [x] document only"));
        assert_eq!(app.task_lines(), description.lines().map(|l| (l.to_owned(), 2)).collect::<Vec<_>>());
    }
}

#[cfg(test)]
mod markdown_tests {
    use super::*;
    use crate::markdown::{Fragment, render};

    #[test]
    fn theme_mapping_preserves_inline_modifiers_and_role_colors() {
        for light in [false, true] {
            let theme = Theme::new(light);
            let context = MarkdownContext { role: Role::Assistant, status: Status::None };
            let semantic = render("**bold** *italic* ~~strike~~ [link](url) `code`", 80, context);
            let styled = style_markdown(&semantic, light);
            let spans: Vec<_> = styled.iter().flat_map(|l| &l.spans).collect();
            assert!(spans.iter().any(|s| s.style.add_modifier.contains(Modifier::BOLD)));
            assert!(spans.iter().any(|s| s.style.add_modifier.contains(Modifier::ITALIC)));
            assert!(spans.iter().any(|s| s.style.add_modifier.contains(Modifier::CROSSED_OUT)));
            assert!(spans.iter().any(|s| s.style.add_modifier.contains(Modifier::UNDERLINED)));
            assert!(spans.iter().any(|s| s.content == "bold" && s.style.fg == Some(theme.accent)));
            assert_eq!(semantic.iter().map(SemanticLine::text).collect::<Vec<_>>(),
                styled.iter().map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect::<String>()).collect::<Vec<_>>());
        }
    }

    #[test]
    fn source_checkbox_is_not_application_success() {
        for light in [false, true] {
            let semantic = render("- [x] document only", 80, MarkdownContext::default());
            assert!(semantic.iter().all(|l| l.context.status == Status::None));
            let theme = Theme::new(light);
            let style = theme.markdown_style(MarkdownContext { role: Role::Body, status: Status::Success }, InlineStyle::default());
            assert!(style.add_modifier.contains(Modifier::BOLD | Modifier::UNDERLINED));
            let mapped = style_markdown(&semantic, light);
            assert!(!mapped.iter().flat_map(|l| &l.spans).any(|s| s.style.fg == Some(theme.good)));
        }
    }

    #[test]
    fn final_theme_boundary_sanitizes_fragments() {
        let lines = [SemanticLine {
            fragments: vec![Fragment { text: "\x1b[2J\u{009b}".into(), style: InlineStyle::default() }],
            context: MarkdownContext::default(),
        }];
        let mapped = style_markdown(&lines, false);
        assert!(mapped[0].spans[0].content.chars().all(|c| !c.is_control()));
    }
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
