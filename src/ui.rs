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
                        format!(
                            "{} 批准 · 自动实时预览 · {} 合并 · {} 删除 · ",
                            app.keymap.hint(Action::ApproveNode),
                            app.keymap.hint(Action::MergeTask),
                            app.keymap.hint(Action::DeleteSelected)
                        )
                    } else if app.view == View::Tasks {
                        format!(
                            "{} 讨论 · {} 启动 · {} 删除 · {} 合并 · {} 重试 · ",
                            app.keymap.hint(Action::Reply),
                            app.keymap.hint(Action::StartTask),
                            app.keymap.hint(Action::DeleteSelected),
                            app.keymap.hint(Action::MergeTask),
                            app.keymap.hint(Action::RetryTask)
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
        Mode::Conflict {
            draft,
            index,
            side,
            scroll,
            body,
        } => {
            let height = frame.area().height.saturating_sub(4);
            let done = draft.all_resolved();
            let good = Style::default().fg(theme.good).add_modifier(Modifier::BOLD);
            let accent = Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD);
            let muted = Style::default().fg(theme.muted);
            let mut lines = vec![
                Line::from(Span::styled(
                    if done {
                        format!(
                            "✓ {} 个文件已标记处理 · 等你审阅，尚未合入",
                            draft.conflicts.len()
                        )
                    } else {
                        format!(
                            "⚠ 还有 {} 个文件待处理",
                            draft
                                .conflicts
                                .iter()
                                .filter(|p| !draft.resolved.contains(p))
                                .count()
                        )
                    },
                    if done {
                        good
                    } else {
                        Style::default().fg(theme.warning)
                    },
                )),
                Line::from(Span::styled(draft.next_step(), accent)),
                Line::from(Span::styled(
                    if done {
                        "无需再选 o / t / a；Enter 只打开最终 diff，合入还有单独确认。"
                    } else {
                        "AI 只解释，不替你决定；o / t / a 经 Enter 确认才改草稿。"
                    },
                    muted,
                )),
                Line::from(""),
            ];
            if let Some(path) = draft.conflicts.get(*index) {
                lines.push(Line::from(Span::styled(
                    format!(
                        "文件 {}/{}：{}  {}",
                        index + 1,
                        draft.conflicts.len(),
                        safe_text(path),
                        if draft.resolved.contains(path) {
                            "✓ 已处理"
                        } else {
                            "⚠ 待决定"
                        }
                    ),
                    accent,
                )));
            }
            lines.push(Line::from(Span::styled(
                match side {
                    0 => "旧基线代码",
                    1 => "现在主项目的代码",
                    2 => "AI 交付的代码",
                    3 => "已准备的固定草稿代码",
                    _ => "说明与下一步",
                },
                muted,
            )));
            lines.push(Line::from(""));
            lines.extend(
                body.lines()
                    .skip(*scroll)
                    .take(height.saturating_sub(14) as usize)
                    .map(|l| Line::from(safe_text(l))),
            );
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "[ / ] 切文件 · j/k 滚动 · 3 看代码 / 4 看说明 · Esc 返回",
                muted,
            )));
            lines.push(Line::from(Span::styled(
                if done {
                    "其他操作：y 重新分析 · e 编辑 · m 标记处理 · r 另建草稿 · x 停止 AI"
                } else {
                    "选方案：o 主项目 / t AI候选 / a AI建议 · e 编辑 · m 标记处理 · x 停止AI"
                },
                muted,
            )));
            popup(
                frame,
                theme,
                if done {
                    " 修复已准备 · 等待你审阅 "
                } else {
                    " 选择如何处理 · 主项目未变 "
                },
                lines,
                140,
                height,
            );
        }
        Mode::ConfirmConflict { draft, index, kind } => {
            let operation = match kind {
                crate::app::ConflictKind::Choose(1) => {
                    "← 用主项目版本覆盖此冲突文件（候选相关改动可能丢失）"
                }
                crate::app::ConflictKind::Choose(2) => {
                    "→ 用候选版本覆盖此冲突文件（主项目新修复可能丢失）"
                }
                crate::app::ConflictKind::ApplyAI => "◇ 应用 AI 建议到独立草稿；不是批准或合并",
                crate::app::ConflictKind::Lockfile => {
                    "允许额外集成范围 Cargo.lock · 不改原节点批准/契约、不生成文件/测试 · 仍需审核最终 diff"
                }
                crate::app::ConflictKind::Edit => {
                    "启动本地编辑器，仅编辑独立草稿；编辑器具有当前账号权限，可运行其配置/插件"
                }
                crate::app::ConflictKind::Advise => {
                    "▶ 调用 AI 只读解释冲突，代码可能外发并消耗额度；不写文件"
                }
                _ => "另建当前主项目的集成草稿；旧草稿和人工修改保留",
            };
            popup(
                frame,
                theme,
                " 人类决定 ",
                vec![
                    Line::from(safe_text(
                        draft.conflicts.get(*index).map_or("", String::as_str),
                    )),
                    Line::from(operation),
                    Line::from("Enter 确认 · Esc 取消"),
                ],
                110,
                7,
            );
        }
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
            let mut lines = Vec::new();
            if let Some(t) = task {
                let resume = t.resume_index().ok();
                // Always keep the actual start visible, including long plans.
                if let Some(index) = resume {
                    lines.push(Line::from(Span::styled(
                        format!("▶ {}", safe_text(&t.nodes[index].contract.title)),
                        Style::default()
                            .fg(theme.accent)
                            .add_modifier(Modifier::BOLD),
                    )));
                    lines.push(Line::from(""));
                }
                let order = t
                    .plan
                    .as_ref()
                    .and_then(|p| p.order().ok())
                    .unwrap_or_else(|| (0..t.nodes.len()).collect());
                for index in order.into_iter().take(8) {
                    let n = &t.nodes[index];
                    let (symbol, color) = if n.execution == crate::nodes::Execution::Implemented {
                        ("✓", theme.good)
                    } else if Some(index) == resume {
                        ("▶", theme.accent)
                    } else {
                        ("○", theme.muted)
                    };
                    lines.push(Line::from(Span::styled(
                        format!("{symbol} {}", safe_text(&n.contract.title)),
                        Style::default().fg(color),
                    )));
                }
                if t.nodes.len() > 8 {
                    lines.push(Line::from(format!("… +{}", t.nodes.len() - 8)));
                }
            }
            lines.push(Line::from(""));
            lines.push(Line::from("授权模型写入候选；不自动测试/合并。"));
            lines.push(Line::from("代码可能外发；worktree 不是安全沙箱。"));
            lines.push(Line::from("Enter ▶     Esc ×"));
            popup(frame, theme, " 启动 / 继续已有任务计划 ", lines, 100, 18);
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
        Mode::ConfirmRetry { task_id, .. } => popup(
            frame,
            theme,
            " 重试失败讨论 ",
            vec![
                Line::from(format!("原任务：{task_id}")),
                Line::from("重新调用模型重试只读讨论，可能消耗额度；不新建任务/不写代码。"),
                Line::from("保留需求、讨论与候选；不会自动切换模型或循环重试。"),
                Line::from("Enter 明确重试 · Esc 取消"),
            ],
            100,
            8,
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
