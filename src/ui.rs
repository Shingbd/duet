use crate::{
    app::{App, Mode, NodeContractContent, Page, Panel, View},
    keymap::Action,
    markdown::{
        self, Context as MarkdownContext, Fragment, InlineStyle, Role, SemanticLine, Status,
    },
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
            Role::Assistant | Role::Acceptance => self.good,
            Role::User | Role::Field => self.accent,
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
        if inline.strong {
            style = style.add_modifier(Modifier::BOLD);
        }
        if inline.emphasis {
            style = style.add_modifier(Modifier::ITALIC);
        }
        if inline.strike {
            style = style.add_modifier(Modifier::CROSSED_OUT);
        }
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
        } else if matches!(context.role, Role::User | Role::Assistant | Role::System) {
            // Chat Markdown must not erase the message's role color baseline.
            style = style.fg(role_color);
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
    lines
        .iter()
        .map(|line| {
            Line::from(
                line.fragments
                    .iter()
                    .map(|fragment| {
                        Span::styled(
                            safe_text(&fragment.text).replace('\n', " "),
                            theme.markdown_style(line.context, fragment.style),
                        )
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .collect()
}

#[derive(Default)]
pub struct Renderer {
    reader: ReaderCache,
    evidence: EvidenceCache,
}
impl Renderer {
    pub fn layout_builds(&self) -> usize {
        self.reader.builds
    }
    pub fn draw(&mut self, frame: &mut Frame, app: &App) {
        draw_cached(frame, app, &mut self.reader, &mut self.evidence);
    }
}

pub fn draw(frame: &mut Frame, app: &App) {
    Renderer::default().draw(frame, app);
}

fn draw_cached(
    frame: &mut Frame,
    app: &App,
    cache: &mut ReaderCache,
    evidence_cache: &mut EvidenceCache,
) {
    let theme = Theme::new(app.light);
    let area = frame.area();
    frame.render_widget(
        Block::default().style(Style::default().bg(theme.bg).fg(theme.text)),
        area,
    );
    if area.width < 24 || area.height < 8 {
        frame.render_widget(
            Paragraph::new(app.tr(
                "Duet · 请扩大终端至至少 24×8",
                "Duet · resize terminal to at least 24×8",
            )),
            area,
        );
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
    let badge = if app.view == View::Review && app.current_buffer().is_some() {
        app.tr("审核草稿 · 主项目不写", "Review draft · main untouched")
    } else if app.view == View::Project && app.editor_screen.is_some() && app.editor_project {
        app.tr("Helix · 编辑实际工作文件", "Helix · editing working files")
    } else if app.branch.is_empty() {
        app.tr("未打开项目", "No project")
    } else {
        app.tr("项目只读浏览", "Read-only project")
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
                    " {}  ·  {}  ·  {} {}",
                    safe_text(&app.branch),
                    app.pi_session_label(),
                    app.tr("待读", "Unread"),
                    app.pending()
                ),
                Style::default().fg(theme.muted),
            )),
        ]),
        rows[0],
    );
    let views = [
        format!(
            "{} {}",
            app.keymap.hint(Action::Tasks),
            app.tr("任务", "Tasks")
        ),
        format!(
            "{} {}",
            app.keymap.hint(Action::Review),
            app.tr("审核", "Review")
        ),
        format!(
            "{} {}",
            app.keymap.hint(Action::Project),
            app.tr("项目", "Project")
        ),
        format!(
            "{} {}",
            app.keymap.hint(Action::Knowledge),
            app.tr("知识", "Knowledge")
        ),
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
        evidence(frame, app, cols[2], theme, evidence_cache);
    } else if body.width >= 78 {
        let cols = Layout::horizontal([Constraint::Length(25), Constraint::Min(30)]).split(body);
        queue(frame, app, cols[0], theme);
        if app.panel == Panel::Evidence {
            evidence(frame, app, cols[1], theme, evidence_cache);
        } else {
            reader(frame, app, cols[1], theme, cache);
        }
    } else {
        match app.panel {
            Panel::Queue => queue(frame, app, body, theme),
            Panel::Reader => reader(frame, app, body, theme, cache),
            Panel::Evidence => evidence(frame, app, body, theme, evidence_cache),
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
    let focus = format!(
        " {}：{} · ",
        app.tr("焦点", "Focus"),
        match app.panel {
            Panel::Queue if app.view == View::Tasks => app.tr("任务 / 节点", "Tasks / nodes"),
            Panel::Queue => app.tr("文件列表", "Files"),
            Panel::Reader => app.tr("内容", "Content"),
            Panel::Evidence => app.tr("说明 / 验证", "Evidence / verification"),
        }
    );
    let mut status_spans = vec![Span::styled(
        focus.clone(),
        Style::default().fg(theme.warning),
    )];
    status_spans.extend(
        compact(
            &status,
            (area.width as usize).saturating_sub(focus.width()),
            theme,
            Role::Constraint,
        )
        .spans,
    );
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(status_spans),
            Line::from(Span::styled(
                if app.editor_focused {
                    if app.fullscreen {app.tr(" Helix 全屏 · Ctrl+] 退出全屏/回侧栏 · :w 保存"," Helix fullscreen · Ctrl+] exit to sidebar · :w save").into()}else if app.editor_conflict{" Helix · 原生按键 · Ctrl+] 工作台 · :wq 保存修复".into()}else if app.view==View::Review{" Helix · Ctrl+] 工作台 · Ctrl+g 讨论 · Ctrl+s 保存/发布 · Ctrl+p 建议 · Ctrl+d 对比".into()}else{" Helix · 原生按键 · Ctrl+] 文件树 · :w 保存 · 不自动提交".into()}
                }else if app.verification_active{app.tr(" 测试运行中 · Ctrl+X / :verify-stop 停止 · Shift+Tab 查看验收/日志 · j/k 滚动"," Tests running · Ctrl+X / :verify-stop cancel · Shift+Tab evidence · j/k scroll").into()}else if app.helix_review && app.view==View::Review && app.panel==Panel::Reader && app.selected_task().is_some() {
                    if app.review_compare{" 只读对照 · j/k 滚动 · Ctrl+d 返回 Helix".into()}else{" 只读快照 · 2 / Tab 初始化 Helix · Ctrl+] 回工作台；编辑按键不触发外部动作".into()}
                }else if app.config.language==crate::config::Language::English {
                    format!("{} discuss · {} start · {} merge · {} focus · {} help · English UI: partial",app.keymap.hint(Action::Reply),app.keymap.hint(Action::StartTask),app.keymap.hint(Action::MergeTask),app.keymap.hint(Action::NextPanel),app.keymap.hint(Action::Help))
                }else{
format!(
                    " {}{} 切面板 · {} 解释 · {} 全屏 · {} 测试 · {} 命令 · {} 帮助",
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
                    app.keymap.hint(Action::Verify),
                    app.keymap.hint(Action::Palette),
                    app.keymap.hint(Action::Help)
                )
                },
                Style::default().fg(theme.muted),
            )),
        ]),
        rows[3],
    );

    match &app.mode {
        Mode::ConfirmEmbeddedEditor { selection } => {
            let target = match selection {
                crate::embedded::Selection::Startup => "私有启动页，不编辑项目文件",
                crate::embedded::Selection::ReviewBuffer => "独立审核草稿",
                crate::embedded::Selection::ProjectFile { .. }
                | crate::embedded::Selection::Project { .. } => {
                    "将编辑主项目实际工作文件（可能含未提交内容），不是只读快照；先备份，不自动提交。"
                }
                crate::embedded::Selection::Review { .. } => {
                    "仅支持已全部批准的任务：编辑最终集成草稿，不覆盖所示历史节点/原候选；旧批准不批准此新修复。"
                }
                crate::embedded::Selection::Conflict { .. } => {
                    "仅编辑独立集成草稿，不修改主项目或原候选；退出后捕获新快照，仍需最终审阅。"
                }
            };
            popup(
                frame,
                theme,
                " 内嵌 Helix · 明确授权 ",
                vec![
                    Line::from(safe_text(selection.path())),
                    Line::from(target),
                    Line::from(
                        "Helix 具有当前账号权限，可打开其他文件/调用其配置工具；PTY/worktree 不是 OS 沙箱。",
                    ),
                    Line::from(
                        "Ctrl+] 切回 Duet · Tab 回 Helix · :wq 保存退出 · Enter 启动 / Esc 取消",
                    ),
                ],
                110,
                10,
            );
        }
        Mode::ReviewChat { .. } => {}
        Mode::ConfirmReviewPatch { proposal } => popup(
            frame,
            theme,
            " AI 建议 · 只应用到人工草稿 ",
            vec![
                Line::from("Enter 明确应用 / Esc 取消；不会发布、批准、测试或合入"),
                Line::from(format!("- {}", safe_text(&proposal.old))),
                Line::from(format!("+ {}", safe_text(&proposal.new))),
            ],
            110,
            10,
        ),
        Mode::ConfirmStopEditor => popup(
            frame,
            theme,
            " 停止内嵌编辑器？ ",
            vec![
                Line::from(
                    "编辑器仍在运行。Enter 终止进程组；未保存缓冲区会丢失，已保存文件不会回滚。",
                ),
                Line::from("Esc 取消，再 h 回编辑器用 :wq 正常保存退出。"),
            ],
            100,
            7,
        ),
        Mode::Help => {
            let lines: Vec<Line> = [
                Line::from(app.tr("列表符号：○ 未开始 · ◐ 执行 · ‖ 暂停 · ◇ 实现 · ∅ 无交付 · × 失败","Symbols: ○ planned · ◐ running · ‖ paused · ◇ implemented · ∅ no delivery · × failed")),
                Line::from(app.tr("节点尾部依次为实现／审核／验证：◇✓○ = 已实现、已批准、未验证；↺ 返工","Node suffix: execution/review/verification; ◇✓○ = implemented/approved/unverified; ↺ rework")),
            ].into_iter().chain(app.keymap.help
                .iter()
                .map(|(action, keys, label)| {
                    Line::from(format!(
                        "{keys:14} {}",
                        app.tr(label, action.english_label())
                    ))
                }))
                .collect();
            popup(
                frame,
                theme,
                app.tr(" 快捷键 · Esc 关闭 ", " Keys · Esc close "),
                lines,
                72,
                24,
            );
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
            let notice = if app.planning_only {
                "只读讨论 · 代码可能外发"
            } else if matches!(
                app.run.status,
                crate::rpc::RunStatus::Paused | crate::rpc::RunStatus::Pausing
            ) {
                "已暂停 · 继续后才发送"
            } else {
                "仍有编码权限"
            };
            input_popup(
                frame,
                theme,
                title,
                text,
                "",
                76,
                &[notice, "Enter 提交 · Esc 返回", "运行中消息在工具边界接收"],
            );
        }
        Mode::Input { search, text } => {
            let title = if *search {
                " 搜索 · Enter 应用 / Esc 返回 "
            } else {
                " 命令 · Enter 执行 / Esc 返回 "
            };
            let hint = if *search {
                "Enter 应用 · Esc 返回"
            } else {
                "Enter 执行 · Esc 返回"
            };
            input_popup(frame, theme, title, text, "> ", 72, &[hint]);
        }
        Mode::ConfirmCode { prompt } => {
            let mut lines = vec![
                Line::from("代码可能外发 · 文件工具非安全沙箱"),
                Line::from("Enter 只读讨论 · 编码需另按 s 确认 · Esc 取消"),
            ];
            lines.extend(style_markdown(
                &markdown::render(
                    prompt,
                    frame.area().width.min(72).saturating_sub(2) as usize,
                    MarkdownContext::default(),
                ),
                app.light,
            ));
            let height = (lines.len() + 2).min(u16::MAX as usize) as u16;
            popup(frame, theme, " 开始只读讨论与规划 ", lines, 72, height);
        }
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
                    "其他操作：h 内嵌 Helix · y 分析 · e 外部编辑 · m 标记处理 · r 另建草稿 · x 停止 AI"
                } else {
                    "选方案：o 主项目 / t AI候选 / a AI建议 · h Helix / e 外部编辑 · m 标记处理 · x 停止AI"
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
                    let mut spans = vec![Span::styled(
                        format!("{symbol} "),
                        Style::default().fg(color),
                    )];
                    spans.extend(
                        compact(
                            &n.contract.title,
                            frame.area().width.min(100).saturating_sub(4) as usize,
                            theme,
                            Role::Body,
                        )
                        .spans,
                    );
                    lines.push(Line::from(spans));
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
        Mode::ConfirmVerification { request, scroll } => {
            let mut lines = vec![
                Line::from(format!(
                    "节点：{} · 固定版本 {}",
                    request.node, request.snapshot
                )),
                Line::from(format!(
                    "命令 argv：{}",
                    serde_json::to_string(&request.spec.argv).unwrap_or_default()
                )),
                Line::from(format!(
                    "关联验收：{:?} · 超时 {} 秒",
                    request.spec.criteria, request.spec.timeout_secs
                )),
                Line::from("只测试该固定交付，在独立工作区；不含 Helix 未发布草稿/后续候选修改。"),
                Line::from(
                    "测试可执行任意项目代码／联网／访问账号文件，worktree 不是沙箱；不要打印凭据。",
                ),
                Line::from("空关联只记命令结果；选择验收序号是人类声明覆盖，不是自动语义证明。"),
                Line::from("不批准/合入/调用模型；Enter 明确执行 / Esc 不执行"),
            ];
            if let Some(node) = app.selected_node() {
                for (i, c) in node.contract.acceptance.iter().enumerate() {
                    lines.push(Line::from(format!("{} {}", i + 1, safe_text(c))));
                }
            }
            let area = centered(frame.area(), 110, 24);
            frame.render_widget(Clear, area);
            frame.render_widget(
                Paragraph::new(lines)
                    .style(Style::default().bg(theme.bg).fg(theme.text))
                    .block(theme.block(
                        " 授权固定版本测试 · j/k 浏览 · Enter 执行 / Esc 取消 ",
                        true,
                    ))
                    .wrap(Wrap { trim: false })
                    .scroll((*scroll, 0)),
                area,
            );
        }
        Mode::ConfirmRead => {} // Unreachable legacy UI mode; no read action.
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
    if app.view == View::Project && !app.project_tree.entries.is_empty() {
        let rows = app.project_tree.visible();
        let selected = rows
            .iter()
            .position(|e| e.path == app.project_tree.selected)
            .unwrap_or(0);
        let start = selected.saturating_sub(area.height.saturating_sub(3) as usize);
        let lines = rows
            .iter()
            .skip(start)
            .take(area.height.saturating_sub(2) as usize)
            .map(|e| {
                let depth = e.path.matches('/').count();
                let name = e.path.rsplit('/').next().unwrap_or(&e.path);
                let icon = if e.directory {
                    if app.project_tree.expanded.contains(&e.path) {
                        "▾"
                    } else {
                        "▸"
                    }
                } else {
                    " "
                };
                let color = match e.status {
                    'A' | '?' => theme.good,
                    'D' => theme.removed,
                    'M' => theme.warning,
                    _ => theme.text,
                };
                let mut style = Style::default().fg(color);
                if e.path == app.project_tree.selected {
                    style = style.bg(theme.border).add_modifier(Modifier::BOLD);
                }
                ListItem::new(Line::from(Span::styled(
                    format!(
                        "{}{} {} {}",
                        "  ".repeat(depth.min(16)),
                        icon,
                        e.status,
                        safe_text(name)
                    ),
                    style,
                )))
            })
            .collect::<Vec<_>>();
        frame.render_widget(
            List::new(lines).block(theme.block(
                if app.project_tree.truncated {
                    " 文件树 · 截断 5000 项 "
                } else {
                    " 文件树 · r 刷新 · z 忽略项 "
                },
                app.panel == Panel::Queue,
            )),
            area,
        );
        return;
    }
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

            let style = if Some(row) == selected_row {
                Style::default().fg(color).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(color)
            };
            ListItem::new(Line::from(format!(
                "{} {kind} {}",
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
        .block(theme.block(
            app.tr(" 文件 / 待审 ", " Files / review "),
            app.panel == Panel::Queue,
        )),
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
            ListItem::new(queue_label(
                label,
                Some(s) == app.task_selection.as_ref(),
                area.width.saturating_sub(2) as usize,
                theme,
            ))
            .style(if i == selected {
                Style::default().fg(color).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(color)
            })
        })
        .collect();
    frame.render_widget(
        List::new(items).block(theme.block(
            app.tr(" 任务 · 节点 ", " Tasks · nodes "),
            app.panel == Panel::Queue,
        )),
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
        items.push(ListItem::new(queue_label(
            &label,
            chosen,
            area.width.saturating_sub(2) as usize,
            theme,
        )));
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
                        "{} {} {}",
                        if index == app.selected { "›" } else { " " },
                        match item.change {
                            Some(crate::checkpoint::ChangeKind::Added) => "A",
                            Some(crate::checkpoint::ChangeKind::Deleted) => "D",
                            _ => "M",
                        },
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

fn contract_literal(
    text: &str,
    width: usize,
    role: Role,
    status: Status,
    strong: bool,
) -> Vec<SemanticLine> {
    markdown::wrap(
        &[Fragment {
            text: text.to_owned(),
            style: InlineStyle {
                strong,
                ..Default::default()
            },
        }],
        width,
        MarkdownContext { role, status },
    )
}

fn contract_field(
    label: &str,
    values: &[String],
    width: usize,
    role: Role,
    literal: bool,
) -> ContractBlock {
    let mut lines = contract_literal(label, width, Role::Field, Status::None, true);
    if values.is_empty() {
        lines.extend(contract_literal(
            "（无）",
            width,
            Role::System,
            Status::None,
            false,
        ));
    }
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            lines.extend(markdown::wrap(&[], width, MarkdownContext::default()));
        }
        if literal {
            lines.extend(contract_literal(value, width, role, Status::None, false));
        } else {
            // Parse each complete field/list entry, not each physical source line.
            lines.extend(markdown::render(
                value,
                width,
                MarkdownContext {
                    role,
                    status: Status::None,
                },
            ));
        }
    }
    ContractBlock {
        source: format!("{label}\n{}", values.join("\n")),
        lines,
    }
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
    let mut title = markdown::render(
        &node.title,
        width,
        MarkdownContext {
            role: Role::Field,
            status: Status::None,
        },
    );
    for fragment in title.iter_mut().flat_map(|l| &mut l.fragments) {
        fragment.style.strong = true;
    }
    let mut title_lines = contract_literal("节点契约", width, Role::Field, Status::None, true);
    title_lines.extend(title);
    blocks.push(ContractBlock {
        source: format!("节点契约\n{}", node.title),
        lines: title_lines,
    });
    blocks.push(contract_field(
        "节点 ID",
        std::slice::from_ref(&node.id),
        width,
        Role::System,
        true,
    ));
    blocks.push(contract_field(
        "任务 ID",
        std::slice::from_ref(&content.task_id),
        width,
        Role::System,
        true,
    ));

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
    blocks.push(contract_status(
        "执行状态",
        execution,
        execution_status,
        width,
    ));
    let (verification, verification_status) = match content.verification {
        Verification::Unverified => ("未验证", Status::Pending),
        Verification::Passed => ("通过", Status::Success),
        Verification::Failed => ("失败", Status::Failure),
    };
    blocks.push(contract_status(
        "验收状态",
        verification,
        verification_status,
        width,
    ));
    let (review, review_status) = match content.review {
        Review::Pending => ("待审", Status::Pending),
        Review::Approved => ("已批准", Status::Success),
        Review::ChangesRequested => ("要求返工", Status::Failure),
    };
    blocks.push(contract_status("人工审核", review, review_status, width));
    blocks.push(contract_field(
        "目标",
        std::slice::from_ref(&node.objective),
        width,
        Role::Body,
        false,
    ));
    blocks.push(contract_field(
        "文件范围",
        &node.scope,
        width,
        Role::Body,
        true,
    ));
    blocks.push(contract_field(
        "边界条件",
        &node.boundaries,
        width,
        Role::Constraint,
        false,
    ));
    blocks.push(contract_field(
        "实现方法",
        std::slice::from_ref(&node.implementation),
        width,
        Role::Body,
        false,
    ));
    blocks.push(contract_field(
        "任务约束",
        &content.task_constraints,
        width,
        Role::Constraint,
        false,
    ));
    blocks.push(contract_field(
        "节点约束",
        &node.constraints,
        width,
        Role::Constraint,
        false,
    ));
    blocks.push(contract_field(
        "依赖",
        &node.dependencies,
        width,
        Role::Body,
        true,
    ));

    blocks.push(contract_status(
        "验收条件",
        "逐项未验证；文档复选框不代表真实验收或批准",
        Status::Pending,
        width,
    ));
    if node.acceptance.is_empty() {
        blocks.push(contract_status(
            "验收条件",
            "未提供条件，不能视为通过",
            Status::Pending,
            width,
        ));
    }
    for (index, acceptance) in node.acceptance.iter().enumerate() {
        // This warning is a distinct literal block: Markdown cannot restyle it.
        let label = format!("□ 未验证 · 验收条件 {}", index + 1);
        let mut lines = contract_literal(&label, width, Role::Constraint, Status::Pending, true);
        lines.extend(markdown::render(
            acceptance,
            width,
            MarkdownContext::default(),
        ));
        blocks.push(ContractBlock {
            source: format!("{label}\n{acceptance}"),
            lines,
        });
    }
    let start = content
        .start_version
        .clone()
        .unwrap_or_else(|| "尚未执行".into());
    let completion = content
        .completion_version
        .clone()
        .unwrap_or_else(|| "尚未生成".into());
    blocks.push(contract_field(
        "开始版本",
        &[start],
        width,
        Role::System,
        true,
    ));
    blocks.push(contract_field(
        "完成版本",
        &[completion],
        width,
        Role::System,
        true,
    ));

    let mut lines = Vec::new();
    for block in blocks {
        // Filter whole already-parsed blocks, retaining labels and cross-line
        // Markdown structure. Match raw and rendered content before cell wrapping
        // can split a phrase. Concatenation also permits long literal metadata.
        if !block_matches(&block, filter) {
            continue;
        }
        if !lines.is_empty() {
            lines.extend(markdown::wrap(&[], width, MarkdownContext::default()));
        }
        lines.extend(block.lines);
    }
    lines
}

/// Markdown selection is explicit; source and fixed review diffs stay literal.
fn is_markdown_path(path: &str) -> bool {
    std::path::Path::new(path)
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| {
            matches!(
                s.to_ascii_lowercase().as_str(),
                "md" | "markdown" | "mdown" | "mkd"
            )
        })
}

fn natural_block(source: &str, width: usize, role: Role) -> ContractBlock {
    ContractBlock {
        source: source.to_owned(),
        lines: markdown::render(
            source,
            width,
            MarkdownContext {
                role,
                status: Status::None,
            },
        ),
    }
}

// Bound the unwrapped layout by source size: rules allocate `width` cells.
// Never request usize::MAX merely to search or flatten a short document.
fn unwrapped_width(source: &str) -> usize {
    source.len().saturating_mul(4).saturating_add(32)
}

fn block_matches(block: &ContractBlock, filter: &str) -> bool {
    if filter.is_empty() || block.source.contains(filter) {
        return true;
    }
    let context = block
        .lines
        .first()
        .map_or(MarkdownContext::default(), |l| l.context);
    // Match before wrapping, including visible phrases spanning inline markers.
    markdown::render(&block.source, unwrapped_width(&block.source), context)
        .iter()
        .any(|line| line.text().contains(filter))
        || block
            .lines
            .iter()
            .map(SemanticLine::text)
            .collect::<String>()
            .contains(filter)
}

fn record_layout(records: &[(String, i8)], width: usize, filter: &str) -> Vec<SemanticLine> {
    let mut lines = Vec::new();
    for (index, (source, kind)) in records.iter().enumerate() {
        let role = match kind {
            3 | 7 => Role::User,
            4 | 8 => Role::Assistant,
            2 | 5 => Role::System,
            6 => Role::Constraint,
            _ => Role::Body,
        };
        let chat_body = matches!(kind, 3..=5);
        let block_width = if chat_body {
            width.saturating_sub(2)
        } else {
            width
        };
        let mut block = if matches!(kind, 2 | 6 | 7 | 8) {
            ContractBlock {
                source: source.clone(),
                lines: contract_literal(
                    source,
                    block_width,
                    role,
                    Status::None,
                    matches!(kind, 6..=8),
                ),
            }
        } else {
            natural_block(source, block_width, role)
        };
        let is_header = matches!(kind, 2 | 7 | 8)
            && source.starts_with("┌─ ")
            && records
                .get(index + 1)
                .is_some_and(|(_, body_kind)| matches!(body_kind, 3..=5));
        let header_match = is_header
            && records.get(index + 1).is_some_and(|(body, _)| {
                block_matches(&natural_block(body, width.saturating_sub(2), role), filter)
            });
        // Keep real workflow status/shortcuts and evidence risks under search.
        // Only trusted headers participate in message grouping, never authored prose.
        let trusted_metadata = *kind == 6 || (*kind == 2 && !is_header);
        if !trusted_metadata && !header_match && !block_matches(&block, filter) {
            continue;
        }
        if chat_body && width >= 2 {
            for line in &mut block.lines {
                line.fragments.insert(
                    0,
                    Fragment {
                        text: "│ ".into(),
                        style: InlineStyle::default(),
                    },
                );
            }
        }
        lines.extend(block.lines);
    }
    lines
}

fn queue_label(label: &str, selected: bool, width: usize, theme: Theme) -> Line<'static> {
    let (title, status) = label.rsplit_once(" · ").unwrap_or((label, ""));
    let suffix = if status.is_empty() {
        String::new()
    } else {
        format!(" · {}", safe_text(status))
    };
    let mut spans = vec![Span::raw(if selected { "› " } else { "  " })];
    spans.extend(
        compact(
            title,
            width.saturating_sub(2 + suffix.width()),
            theme,
            Role::Body,
        )
        .spans,
    );
    spans.push(Span::styled(suffix, Style::default().fg(theme.warning)));
    Line::from(spans)
}

/// One-row natural language uses inline fragments only, with explicit ellipsis.
fn compact(text: &str, width: usize, theme: Theme, role: Role) -> Line<'static> {
    // A literal prefix keeps headings, list markers, rules and fences from
    // becoming block syntax in a one-row label. Inline markup and link targets
    // still use the shared renderer. Remove only our own prefix afterwards.
    let source = format!("x {}", safe_text(text).replace(['\n', '\r'], " "));
    let mut fragments = Vec::new();
    for line in markdown::render(
        &source,
        unwrapped_width(&source),
        MarkdownContext::default(),
    ) {
        if !fragments.is_empty() {
            fragments.push(Fragment {
                text: " ".into(),
                style: InlineStyle::default(),
            });
        }
        fragments.extend(line.fragments);
    }
    let mut prefix_remaining = 2;
    for fragment in &mut fragments {
        let remove = prefix_remaining.min(fragment.text.len());
        fragment.text.drain(..remove);
        prefix_remaining -= remove;
        if prefix_remaining == 0 {
            break;
        }
    }
    let total: usize = fragments.iter().map(|f| f.text.width()).sum();
    let clipped = total > width;
    let context = MarkdownContext {
        role,
        status: Status::None,
    };
    let lines = markdown::wrap(
        &fragments,
        width.saturating_sub(usize::from(clipped)),
        context,
    );
    let mut spans: Vec<_> = lines
        .first()
        .into_iter()
        .flat_map(|line| &line.fragments)
        .map(|f| Span::styled(f.text.clone(), theme.markdown_style(context, f.style)))
        .collect();
    if clipped && width > 0 {
        spans.push(Span::raw("…"));
    }
    Line::from(spans)
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
    title: String,
    filter: String,
    revision: u64,
    is_diff: bool,
    records: Vec<(String, i8)>,
    selection: Option<String>,
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
        let contract = if app.showing_node_contract() {
            app.node_contract_content()
        } else {
            None
        };
        // The complete domain snapshot already tracks contract edits and actual
        // statuses. Unrelated chat/note revisions must not rebuild this layout.
        let revision = if contract.is_none() && matches!(app.view, View::Tasks | View::Knowledge) {
            app.content_revision
        } else {
            0
        };
        let natural = app.view == View::Tasks
            || app.view == View::Knowledge
            || (app.view == View::Review && app.page != Page::Code)
            || (app.view == View::Project
                && app
                    .current()
                    .is_some_and(|item| is_markdown_path(&item.title)));
        let records = if contract.is_some() {
            Vec::new()
        } else if app.view == View::Tasks {
            app.task_text_blocks()
        } else if app.view == View::Review
            && app.page == Page::Evidence
            && app.selected_node().is_some()
        {
            app.node_evidence_blocks()
                .unwrap_or_default()
                .into_iter()
                .map(|(text, literal)| (text, if literal { 6 } else { 0 }))
                .collect()
        } else if app.view == View::Knowledge && !app.notes.is_empty() {
            app.notes.iter().cloned().map(|note| (note, 0)).collect()
        } else {
            vec![(app.body(), 0)]
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
            title: app
                .current()
                .map_or_else(String::new, |item| item.title.clone()),
            filter: app.filter.clone(),
            revision,
            records,
            selection: app
                .task_selection
                .as_ref()
                .map(|s| format!("{}:{}", s.task_id, s.node_id.as_deref().unwrap_or(""))),
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
        if natural {
            let mut lines = Vec::new();
            if matches!(app.view, View::Review | View::Project)
                && let Some(item) = app.current()
            {
                let label = match app.page {
                    Page::Intent => "意图",
                    Page::Evidence => "证据",
                    Page::Code => "文档",
                };
                lines.extend(contract_literal(
                    &format!("{label} · {}", item.version),
                    width as usize,
                    Role::System,
                    Status::None,
                    false,
                ));
                lines.extend(markdown::wrap(
                    &[],
                    width as usize,
                    MarkdownContext::default(),
                ));
            }
            lines.extend(record_layout(&key.records, width as usize, &app.filter));
            self.contract_lines = Some(lines);
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
            let role = if key.is_diff && line.starts_with('+') {
                1
            } else if key.is_diff && line.starts_with('-') {
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

pub fn editor_size_for(app: &App, area: Rect) -> (u16, u16) {
    let (rows, cols) = editor_size(area, app.fullscreen);
    if app.view == View::Review && app.review_chat_open && (app.fullscreen || area.width < 116) {
        (
            ((rows + 2) as u32 * 65 / 100)
                .saturating_sub(2)
                .clamp(1, 200) as u16,
            cols,
        )
    } else {
        (rows, cols)
    }
}
pub fn editor_size(area: Rect, fullscreen: bool) -> (u16, u16) {
    let body_height = area.height.saturating_sub(6);
    let width = if fullscreen || area.width < 78 {
        area.width
    } else if area.width < 116 {
        area.width.saturating_sub(25)
    } else {
        area.width.saturating_sub(60)
    };
    (
        body_height.saturating_sub(2).clamp(1, 200),
        width.saturating_sub(2).clamp(1, 400),
    )
}
fn editor_color(color: vt100::Color) -> Color {
    match color {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(i) => Color::Indexed(i),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}
fn render_editor(
    frame: &mut Frame,
    screen: &vt100::Screen,
    area: Rect,
    focused: bool,
    theme: Theme,
    persistent: bool,
) {
    let block = theme.block(
        if focused && persistent {
            " HELIX · Ctrl+] 侧栏 · Space q 保存返回 "
        } else if focused {
            " HELIX · Ctrl+] → Duet · :wq 保存退出 "
        } else {
            " HELIX · Tab 回编辑器 "
        },
        focused,
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let (rows, cols) = screen.size();
    for row in 0..inner.height.min(rows) {
        for col in 0..inner.width.min(cols) {
            let Some(cell) = screen.cell(row, col) else {
                continue;
            };
            let mut style = Style::default()
                .fg(editor_color(cell.fgcolor()))
                .bg(editor_color(cell.bgcolor()));
            if cell.bold() {
                style = style.add_modifier(Modifier::BOLD);
            }
            if cell.italic() {
                style = style.add_modifier(Modifier::ITALIC);
            }
            if cell.underline() {
                style = style.add_modifier(Modifier::UNDERLINED);
            }
            if cell.inverse() {
                style = style.add_modifier(Modifier::REVERSED);
            }
            let text = cell.contents();
            let text = if text.is_empty() || cell.is_wide_continuation() {
                " "
            } else if cell.is_wide() && col + 1 >= inner.width {
                "?"
            } else {
                &text
            };
            if let Some(target) = frame.buffer_mut().cell_mut((inner.x + col, inner.y + row)) {
                target.set_symbol(text).set_style(style);
            }
        }
    }
    if focused && !screen.hide_cursor() {
        let (row, col) = screen.cursor_position();
        if row < inner.height && col < inner.width {
            frame.set_cursor_position((inner.x + col, inner.y + row));
        }
    }
}

fn native_review(
    frame: &mut Frame,
    app: &App,
    buffer: &crate::review_editor::Buffer,
    area: Rect,
    theme: Theme,
) {
    let focused = app.panel == Panel::Reader && matches!(app.mode, Mode::Browse);
    let label = if buffer.insert { "INSERT" } else { "NORMAL" };
    let comparison = if buffer.human_diff {
        "人工 vs 固定交付"
    } else {
        "草稿 vs 节点基线"
    };
    let title = format!(
        " {} · {}{} · {}:{} · {} ",
        safe_text(&buffer.path),
        label,
        if buffer.changed() {
            " · 人工草稿未发布"
        } else {
            " · 固定交付"
        },
        buffer.line() + 1,
        buffer.column() + 1,
        comparison
    );
    let block = theme.block(title, focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let height = inner.height.saturating_sub(1);
    let gutter = 13_u16.min(inner.width);
    let starts = std::iter::once(0)
        .chain(buffer.text.match_indices('\n').map(|(p, _)| p + 1))
        .collect::<Vec<_>>();
    for (row, data) in buffer
        .rows
        .iter()
        .skip(buffer.top)
        .take(height as usize)
        .enumerate()
    {
        let background = match data.tag {
            similar::ChangeTag::Insert => {
                if app.light {
                    Color::Rgb(222, 244, 230)
                } else {
                    Color::Rgb(25, 49, 40)
                }
            }
            similar::ChangeTag::Delete => {
                if app.light {
                    Color::Rgb(253, 228, 233)
                } else {
                    Color::Rgb(59, 30, 40)
                }
            }
            _ => theme.bg,
        };
        let base = Style::default().bg(background).fg(theme.text);
        let marker = match data.tag {
            similar::ChangeTag::Insert => '+',
            similar::ChangeTag::Delete => '-',
            _ => ' ',
        };
        let number = |line: Option<usize>| line.map(|n| (n + 1).to_string()).unwrap_or_default();
        let prefix = format!(
            "{:>4} {:>4} {}{} ",
            number(data.old),
            number(data.new),
            marker,
            if data.human { '*' } else { ' ' }
        );
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(prefix, base.fg(theme.muted)))),
            Rect::new(inner.x, inner.y + row as u16, gutter, 1),
        );
        let mut cells = 0;
        let mut offset = data.new.and_then(|n| starts.get(n).copied());
        let mut spans = Vec::new();
        for (text, role) in crate::review_editor::tokens(&data.text) {
            let fg = match role {
                1 => theme.accent,
                2 => theme.good,
                3 => theme.muted,
                4 => theme.warning,
                _ => theme.text,
            };
            for grapheme in text.graphemes(true) {
                let display = if grapheme == "\t" {
                    "    ".to_string()
                } else {
                    safe_text(grapheme)
                };
                let width = display.width();
                let end = cells + width;
                let selected = offset.is_some_and(|at| {
                    buffer
                        .selected()
                        .is_some_and(|(a, b)| at < b && at + grapheme.len() > a)
                });
                let style = if selected {
                    base.fg(fg).add_modifier(Modifier::REVERSED)
                } else {
                    base.fg(fg)
                };
                if cells >= buffer.left && end <= buffer.left + (inner.width - gutter) as usize {
                    spans.push(Span::styled(display, style));
                }
                if let Some(at) = &mut offset {
                    *at += grapheme.len();
                }
                cells = end;
                if cells >= buffer.left + (inner.width - gutter) as usize {
                    break;
                }
            }
        }
        frame.render_widget(
            Paragraph::new(Line::from(spans)).style(base),
            Rect::new(
                inner.x + gutter,
                inner.y + row as u16,
                inner.width - gutter,
                1,
            ),
        );
    }
    if inner.height > 0 {
        let status = if app.helix_review {
            if app.review_compare {
                "只读对比 · j/k 滚动 · Ctrl+d 返回 Helix".into()
            } else {
                "只读快照 · 2 / Tab 打开 Helix；未初始化时不接受编辑/批准快捷键".into()
            }
        } else {
            buffer.command.as_ref().map(|(search,text)|format!("{}{}",if *search{"/"}else{":"},safe_text(text))).unwrap_or_else(||"i 编辑 · v/Shift方向 选区 · Ctrl+g/:ask 讨论 · Ctrl+s 发布 · Ctrl+d 对比 · Ctrl+] 工作台".into())
        };
        frame.render_widget(
            Paragraph::new(status).style(Style::default().fg(theme.accent)),
            Rect::new(inner.x, inner.y + inner.height - 1, inner.width, 1),
        );
    }
    if focused && !app.helix_review {
        let row = buffer.cursor_row().saturating_sub(buffer.top);
        let start = starts.get(buffer.line()).copied().unwrap_or(0);
        let col = buffer.text[start..buffer.cursor]
            .replace('\t', "    ")
            .width()
            .saturating_sub(buffer.left);
        if row < height as usize && col < (inner.width - gutter) as usize {
            frame.set_cursor_position((inner.x + gutter + col as u16, inner.y + row as u16));
        }
    }
}
fn review_chat_panel(frame: &mut Frame, app: &App, area: Rect, theme: Theme) {
    let block = theme.block(
        if app.review_chat_busy {
            " 代码讨论 · AI 回复中 "
        } else {
            " 代码讨论 · i 追问 · Esc 回代码 · Home/End "
        },
        app.panel == Panel::Evidence,
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let context = app.review_chat_context.as_ref();
    let mut lines = Vec::new();
    for message in app
        .review_messages
        .iter()
        .filter(|m| context.is_some_and(|c| m.task_id == c.task_id && m.path == c.path))
        .rev()
        .take(24)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        lines.push(Line::from(Span::styled(
            if message.role == "user" { "你" } else { "AI" },
            Style::default()
                .fg(if message.role == "user" {
                    theme.accent
                } else {
                    theme.good
                })
                .add_modifier(Modifier::BOLD),
        )));
        lines.extend(style_markdown(
            &natural_block(
                &message.content,
                inner.width as usize,
                if message.role == "user" {
                    Role::User
                } else {
                    Role::Assistant
                },
            )
            .lines,
            app.light,
        ));
    }
    if !app.review_chat_text.is_empty() {
        lines.extend(style_markdown(
            &natural_block(&app.review_chat_text, inner.width as usize, Role::Assistant).lines,
            app.light,
        ));
    }
    if let Mode::ReviewChat { text } = &app.mode {
        lines.push(Line::from(Span::styled(
            format!("> {}▏", safe_text(text)),
            Style::default().fg(theme.accent),
        )));
        lines.push(Line::from("Enter 直接发送当前引用 · Esc 返回代码"));
    }
    if app.review_proposal.is_some() {
        lines.push(Line::from(Span::styled(
            "有局部建议：代码区 Ctrl+p 查看/确认应用",
            Style::default().fg(theme.warning),
        )));
    }
    let skip = app
        .review_chat_scroll
        .unwrap_or_else(|| lines.len().saturating_sub(inner.height as usize))
        .min(lines.len().saturating_sub(1));
    frame.render_widget(
        Paragraph::new(lines.into_iter().skip(skip).collect::<Vec<_>>()),
        inner,
    );
}

fn reader(frame: &mut Frame, app: &App, area: Rect, theme: Theme, cache: &mut ReaderCache) {
    if let Some(screen) = &app.editor_screen
        && !app.review_compare
        && (app.editor_conflict
            || ((!app.editor_project && app.view == View::Review)
                || (app.editor_project && app.view == View::Project)))
    {
        let area = if app.view == View::Review
            && app.review_chat_open
            && (app.fullscreen || frame.area().width < 116)
        {
            let a = Layout::vertical([Constraint::Percentage(65), Constraint::Percentage(35)])
                .split(area);
            review_chat_panel(frame, app, a[1], theme);
            a[0]
        } else {
            area
        };
        render_editor(
            frame,
            screen,
            area,
            app.editor_focused,
            theme,
            app.config.helix_keepalive && !app.editor_conflict,
        );
        return;
    }
    if app.view == View::Review
        && let Some(buffer) = app.current_buffer()
    {
        if app.review_chat_open && (app.fullscreen || frame.area().width < 116) {
            let areas = Layout::vertical([Constraint::Percentage(65), Constraint::Percentage(35)])
                .split(area);
            native_review(frame, app, buffer, areas[0], theme);
            review_chat_panel(frame, app, areas[1], theme);
        } else {
            native_review(frame, app, buffer, area, theme);
        }
        return;
    }
    cache.prepare(app, area.width.saturating_sub(2));
    let scroll = if app.view == View::Tasks || app.showing_node_contract() {
        app.task_scroll
    } else {
        app.current().map_or(0, |item| item.scroll)
    } as usize;
    let content: Vec<Line> = if let Some(lines) = &cache.contract_lines {
        lines
            .iter()
            .skip(scroll)
            .take(area.height.saturating_sub(2) as usize)
            .map(|line| {
                Line::from(
                    line.fragments
                        .iter()
                        .map(|fragment| {
                            Span::styled(
                                fragment.text.as_str(),
                                theme.markdown_style(line.context, fragment.style),
                            )
                        })
                        .collect::<Vec<_>>(),
                )
            })
            .collect()
    } else {
        cache
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

type EvidenceKey = (Vec<(String, bool)>, u16, View, usize, Option<String>);
#[derive(Default)]
struct EvidenceCache {
    key: Option<EvidenceKey>,
    lines: Vec<SemanticLine>,
}

fn evidence(frame: &mut Frame, app: &App, area: Rect, theme: Theme, cache: &mut EvidenceCache) {
    if app.view == View::Review && app.review_chat_open {
        review_chat_panel(frame, app, area, theme);
        return;
    }
    let mut blocks = app.node_evidence_blocks().unwrap_or_else(|| {
        let mut blocks = Vec::new();
        if app.view == View::Tasks {
            if let Some(plan) = app.selected_task().and_then(|t| t.plan.as_ref()) {
                blocks.push(("总体目标".into(), true));
                blocks.push((plan.objective.clone(), false));
                blocks.push(("任务约束".into(), true));
                blocks.extend(plan.constraints.iter().cloned().map(|s| (s, false)));
                for (index, acceptance) in plan.acceptance.iter().enumerate() {
                    blocks.push((format!("□ 未验证 · 验收条件 {}", index + 1), true));
                    blocks.push((acceptance.clone(), false));
                }
            }
        } else if let Some(item) = app.current() {
            blocks.push((
                "说明 / 验证记录；文档复选框不代表真实验证或人工批准。".into(),
                true,
            ));
            if app.explain && !item.explanation.is_empty() {
                blocks.push((item.explanation.clone(), false));
            }
            if !item.evidence.is_empty() {
                blocks.push((item.evidence.clone(), false));
            }
        }
        blocks
    });
    if blocks.is_empty() {
        blocks.push(("暂无记录".into(), true));
    }
    let selection = app
        .task_selection
        .as_ref()
        .map(|s| format!("{}:{}", s.task_id, s.node_id.as_deref().unwrap_or("")));
    let key = (
        blocks,
        area.width.saturating_sub(2),
        app.view,
        app.selected,
        selection,
    );
    if cache.key.as_ref() != Some(&key) {
        cache.lines.clear();
        for (source, literal) in &key.0 {
            cache.lines.extend(if *literal {
                contract_literal(source, key.1 as usize, Role::Constraint, Status::None, true)
            } else {
                natural_block(source, key.1 as usize, Role::Body).lines
            });
        }
        cache.key = Some(key);
    }
    frame.render_widget(
        Paragraph::new(style_markdown(&cache.lines, app.light))
            .block(theme.block(
                app.tr(
                    " 验收 / 测试证据 · v 测试 ",
                    " Acceptance / test evidence · v ",
                ),
                app.panel == Panel::Evidence,
            ))
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

/// Literal input display only: the source in Mode is never changed. Cursor
/// coordinates describe the sanitized, cell-wrapped display, not byte offsets.
#[derive(Debug)]
struct InputLayout {
    lines: Vec<String>,
    cursor_column: usize,
}

impl InputLayout {
    fn new(text: &str, prefix: &str, width: usize) -> Self {
        if width == 0 {
            return Self {
                lines: Vec::new(),
                cursor_column: 0,
            };
        }
        let display = safe_text(&format!("{prefix}{text}"));
        let mut lines = Vec::new();
        let mut line = String::new();
        let mut column = 0;
        for glyph in display.graphemes(true) {
            if glyph == "\n" {
                lines.push(std::mem::take(&mut line));
                column = 0;
                continue;
            }
            let cells = glyph.width();
            if column + cells > width && !line.is_empty() {
                lines.push(std::mem::take(&mut line));
                column = 0;
            }
            // Same narrow-cell fallback as literal readers. Actual input
            // popups have at least 22 cells under the existing terminal guard.
            if cells > width {
                line.push('�');
                column += 1;
            } else {
                line.push_str(glyph);
                column += cells;
            }
        }
        lines.push(line);
        // A full last row needs a real next row for the insertion cursor.
        if column == width {
            lines.push(String::new());
            column = 0;
        }
        Self {
            lines,
            cursor_column: column,
        }
    }

    fn window_start(&self, height: usize) -> usize {
        self.lines.len().saturating_sub(height)
    }
}

fn input_popup(
    frame: &mut Frame,
    theme: Theme,
    title: &str,
    text: &str,
    prefix: &str,
    preferred_width: u16,
    hints: &[&str],
) {
    let terminal = frame.area();
    let width = preferred_width.min(terminal.width);
    let inner_width = width.saturating_sub(2) as usize;
    let layout = InputLayout::new(text, prefix, inner_width);
    // Hints have their own literal region: only the input scrolls. Essential
    // mode/Enter/Esc hints precede the optional runtime explanation.
    let mut footer = Vec::new();
    let footer_capacity = terminal.height.saturating_sub(3) as usize;
    for hint in hints {
        let rows = wrap_cells(&safe_text(hint), inner_width);
        if footer.len() + rows.len() > footer_capacity {
            break;
        }
        footer.extend(rows);
    }
    let height = (layout
        .lines
        .len()
        .saturating_add(footer.len())
        .saturating_add(2))
    .max(5)
    .min(terminal.height as usize) as u16;
    let area = centered(terminal, width, height);
    let block = theme.block(title, true);
    let inner = block.inner(area);
    let input_height = inner.height.saturating_sub(footer.len() as u16);
    let start = layout.window_start(input_height as usize);
    let input_area = Rect::new(inner.x, inner.y, inner.width, input_height);
    let footer_area = Rect::new(
        inner.x,
        inner.y + input_height,
        inner.width,
        footer.len() as u16,
    );
    frame.render_widget(Clear, area);
    frame.render_widget(
        block.style(Style::default().bg(theme.bg).fg(theme.text)),
        area,
    );
    frame.render_widget(
        Paragraph::new(
            layout
                .lines
                .iter()
                .skip(start)
                .take(input_height as usize)
                .map(|line| Line::from(line.as_str()))
                .collect::<Vec<_>>(),
        ),
        input_area,
    );
    frame.render_widget(
        Paragraph::new(footer.into_iter().map(Line::from).collect::<Vec<_>>())
            .style(Style::default().fg(theme.warning)),
        footer_area,
    );
    if input_height > 0 && inner.width > 0 {
        let row = layout.lines.len().saturating_sub(1).saturating_sub(start) as u16;
        frame.set_cursor_position((inner.x + layout.cursor_column as u16, inner.y + row));
    }
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
        lines
            .iter()
            .map(SemanticLine::text)
            .collect::<Vec<_>>()
            .join("\n")
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
        for expected in [
            "契约标题",
            "节点 ID",
            "任务 ID",
            "执行状态",
            "验收状态",
            "人工审核",
            "目标",
            "文件范围",
            "边界条件",
            "实现方法",
            "任务约束",
            "节点约束",
            "依赖",
            "第二条验收",
            "开始版本",
            "完成版本",
            "src/[literal]_file.rs",
            "dependency_literal",
            "base_[literal]",
            "completion_*literal*",
        ] {
            assert!(output.contains(expected), "missing {expected}");
        }
        assert!(!output.contains("**契约标题**"));
        assert!(
            lines
                .iter()
                .flat_map(|l| &l.fragments)
                .any(|f| f.style.heading > 0)
        );
        for light in [false, true] {
            let theme = Theme::new(light);
            let normal = theme.markdown_style(MarkdownContext::default(), InlineStyle::default());
            for (role, color) in [
                (Role::Field, theme.accent),
                (Role::Constraint, theme.warning),
                (Role::System, theme.muted),
            ] {
                let style = theme.markdown_style(
                    MarkdownContext {
                        role,
                        status: Status::None,
                    },
                    InlineStyle::default(),
                );
                assert_eq!(style.fg, Some(color));
                assert_ne!(style.fg, normal.fg);
            }
        }
    }

    #[test]
    fn wrapped_body_preserves_inline_and_warning_semantics() {
        let field = contract_field(
            "目标",
            &[content().contract.objective],
            12,
            Role::Body,
            false,
        );
        assert!(field.lines.iter().all(|l| l.width() <= 12));
        let strong: Vec<_> = field
            .lines
            .iter()
            .skip(1)
            .filter(|l| l.fragments.iter().any(|f| f.style.strong))
            .collect();
        assert!(strong.len() > 1);
        let joined: String = field.lines.iter().skip(1).map(SemanticLine::text).collect();
        assert!(joined.contains("👩‍💻 e\u{301}"));
        assert!(!joined.contains("**"));
        let warning = contract_field(
            "边界",
            &["# 标题\n\n[链接](url) `code`".into()],
            12,
            Role::Constraint,
            false,
        );
        for light in [false, true] {
            assert!(
                style_markdown(&warning.lines[1..], light)
                    .iter()
                    .flat_map(|l| &l.spans)
                    .all(|s| s.style.fg == Some(Theme::new(light).warning))
            );
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
        assert!(
            lines
                .iter()
                .any(|l| l.text() == "验收状态：通过" && l.context.status == Status::Success)
        );
        assert!(
            lines
                .iter()
                .any(|l| l.text() == "人工审核：已批准" && l.context.status == Status::Success)
        );
        // No per-condition evidence exists: even an aggregate Passed status
        // cannot silently check individual conditions or document markers.
        assert!(text(&lines).contains("□ 未验证 · 验收条件 2"));
    }

    #[test]
    fn search_matches_source_and_rendered_text_before_wrapping() {
        for filter in [
            "正文 **强调",
            "正文 强调",
            "src/[literal]_file.rs",
            "completion_*literal*",
        ] {
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
        app.select_task_entry(TaskSelection {
            task_id: "task_literal".into(),
            node_id: Some("second_node".into()),
        });
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
        assert_eq!(
            app.task_lines(),
            description
                .lines()
                .map(|l| (l.to_owned(), 2))
                .collect::<Vec<_>>()
        );
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
            let context = MarkdownContext {
                role: Role::Assistant,
                status: Status::None,
            };
            let semantic = render(
                "**bold** *italic* ~~strike~~ [link](url) `code`",
                80,
                context,
            );
            let styled = style_markdown(&semantic, light);
            let spans: Vec<_> = styled.iter().flat_map(|l| &l.spans).collect();
            assert!(
                spans
                    .iter()
                    .any(|s| s.style.add_modifier.contains(Modifier::BOLD))
            );
            assert!(
                spans
                    .iter()
                    .any(|s| s.style.add_modifier.contains(Modifier::ITALIC))
            );
            assert!(
                spans
                    .iter()
                    .any(|s| s.style.add_modifier.contains(Modifier::CROSSED_OUT))
            );
            assert!(
                spans
                    .iter()
                    .any(|s| s.style.add_modifier.contains(Modifier::UNDERLINED))
            );
            assert!(
                spans
                    .iter()
                    .any(|s| s.content == "bold" && s.style.fg == Some(theme.good))
            );
            assert_eq!(
                semantic.iter().map(SemanticLine::text).collect::<Vec<_>>(),
                styled
                    .iter()
                    .map(|l| l
                        .spans
                        .iter()
                        .map(|s| s.content.as_ref())
                        .collect::<String>())
                    .collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn source_checkbox_is_not_application_success() {
        for light in [false, true] {
            let semantic = render("- [x] document only", 80, MarkdownContext::default());
            assert!(semantic.iter().all(|l| l.context.status == Status::None));
            let theme = Theme::new(light);
            let style = theme.markdown_style(
                MarkdownContext {
                    role: Role::Body,
                    status: Status::Success,
                },
                InlineStyle::default(),
            );
            assert!(
                style
                    .add_modifier
                    .contains(Modifier::BOLD | Modifier::UNDERLINED)
            );
            let mapped = style_markdown(&semantic, light);
            assert!(
                !mapped
                    .iter()
                    .flat_map(|l| &l.spans)
                    .any(|s| s.style.fg == Some(theme.good))
            );
        }
    }

    #[test]
    fn final_theme_boundary_sanitizes_fragments() {
        let lines = [SemanticLine {
            fragments: vec![Fragment {
                text: "\x1b[2J\u{009b}".into(),
                style: InlineStyle::default(),
            }],
            context: MarkdownContext::default(),
        }];
        let mapped = style_markdown(&lines, false);
        assert!(mapped[0].spans[0].content.chars().all(|c| !c.is_control()));
    }
}

/// Human-started regression (NOT executed by this node):
/// `cargo test --lib ui::text_surface_tests`
/// Also run existing `ui::contract_tests` and `ui::markdown_tests` explicitly.
/// Manual: in both themes, inspect current/history/streaming messages containing
/// lists, fences, links, tables and CJK/emoji; search visible prose and `**source**`;
/// confirm role labels survive, the final reply is not duplicated, and resizing
/// reflows without changing input. Add two notes with an unfinished fence in the
/// first and a heading in the second. Open README.md and a Rust file from the
/// existing project snapshot, then review both fixed diffs (including merge
/// preview without confirming). Inspect evidence with an unfinished summary
/// fence and scope violations: unverified/authority warnings must remain literal.
/// Scroll and switch theme without rebuilding, then edit content/search/selection
/// or resize and confirm the cache updates. Cancel every authorization dialog.
#[cfg(test)]
mod text_surface_tests {
    use super::*;
    use crate::keymap::Keymap;

    fn text(lines: &[SemanticLine]) -> String {
        lines
            .iter()
            .map(SemanticLine::text)
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn complete_messages_keep_fences_roles_and_search_mapping() {
        let records = vec![
            ("┌─ 你".into(), 7),
            (
                "**visible phrase**\n\n```rust\n*a_literal*\n# source\n```\n\n- second".into(),
                3,
            ),
            ("┌─ AI".into(), 8),
            ("**answer**".into(), 4),
        ];
        for filter in ["visible phrase", "**visible phrase**", "*a_literal*"] {
            let lines = record_layout(&records, 40, filter);
            let output = text(&lines);
            assert!(output.contains("┌─ 你"));
            assert!(output.contains("│ *a_literal*"));
            assert!(output.contains("│ # source"));
            assert!(!output.contains("┌─ AI"));
            assert!(
                lines
                    .iter()
                    .any(|l| l.fragments.iter().any(|f| f.style.code))
            );
        }
        let lines = record_layout(&records, 12, "");
        assert!(lines.iter().all(|l| l.width() <= 12));
        for light in [false, true] {
            let styled = style_markdown(&lines, light);
            assert!(
                styled
                    .iter()
                    .flat_map(|l| &l.spans)
                    .any(|s| s.content.contains("你")
                        && s.style.fg == Some(Theme::new(light).accent))
            );
            assert!(styled.iter().flat_map(|l| &l.spans).any(|s| s.content.contains("AI") && s.style.fg == Some(Theme::new(light).good)));
        }
    }

    #[test]
    fn notes_are_independent_and_risk_metadata_stays_literal() {
        let records = vec![("```\nunfinished".into(), 0), ("# next note".into(), 0)];
        let lines = record_layout(&records, 50, "");
        assert!(lines.iter().any(|l| {
            l.fragments
                .iter()
                .any(|f| f.style.heading > 0 && f.text.contains("next note"))
        }));
        let records = vec![
            ("验收：未验证 · 人工审核：待审".into(), 6),
            ("```\n- [x] document only".into(), 0),
            ("仓内越界：src/outside.rs".into(), 6),
        ];
        let lines = record_layout(&records, 60, "document only");
        assert!(text(&lines).contains("验收：未验证"));
        assert!(text(&lines).contains("仓内越界"));
        assert!(lines.iter().all(|l| l.context.status == Status::None));
    }

    #[test]
    fn compact_titles_do_not_parse_status_and_truncate_explicitly() {
        let line = queue_label("**中文标题 👩‍💻** · 未验证", true, 22, Theme::new(false));
        let output: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(output.contains("未验证"));
        assert!(!output.contains("**"));
        let line = compact(
            "**very long title** [link](https://example.test)",
            8,
            Theme::new(false),
            Role::Body,
        );
        let output: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(output.ends_with('…'));
        assert!(output.width() <= 8);
    }

    #[test]
    fn cache_tracks_stream_content_width_search_but_not_scroll_theme() {
        let mut app = App::empty(Keymap::default());
        app.view = View::Tasks;
        app.task = Some("**request**".into());
        app.run.streaming_text = true;
        app.run.text = "**partial**".into();
        let mut cache = ReaderCache::default();
        cache.prepare(&app, 40);
        let initial = cache.builds;
        assert!(text(cache.contract_lines.as_ref().unwrap()).contains("partial"));
        app.light = !app.light;
        app.task_scroll = 2;
        cache.prepare(&app, 40);
        assert_eq!(cache.builds, initial);
        app.run.text = "**completed**".into();
        cache.prepare(&app, 40);
        assert_eq!(cache.builds, initial + 1);
        assert!(!text(cache.contract_lines.as_ref().unwrap()).contains("partial"));
        cache.prepare(&app, 12);
        assert_eq!(cache.builds, initial + 2);
        app.filter = "completed".into();
        cache.prepare(&app, 12);
        assert_eq!(cache.builds, initial + 3);
        assert!(text(cache.contract_lines.as_ref().unwrap()).contains("AI"));
    }

    #[test]
    fn final_reply_is_not_rendered_twice_and_historical_records_are_complete() {
        let mut app = App::empty(Keymap::default());
        app.view = View::Tasks;
        app.task = Some("request".into());
        app.run.text = "**final reply**".into();
        app.run.streaming_text = false;
        app.history.push(crate::storage::Message {
            session: "session".into(),
            role: "assistant".into(),
            content: app.run.text.clone(),
        });
        assert_eq!(
            text(&record_layout(&app.task_text_blocks(), 80, ""))
                .matches("final reply")
                .count(),
            1
        );
        let mut old = crate::nodes::TaskRecord::legacy("old", "**old request**", "session");
        old.history = app.history.clone();
        app.tasks.push(old);
        app.tasks.push(crate::nodes::TaskRecord::legacy(
            "live",
            "live request",
            "session",
        ));
        app.task_selection = Some(crate::app::TaskSelection {
            task_id: "old".into(),
            node_id: None,
        });
        let output = text(&record_layout(&app.task_text_blocks(), 80, ""));
        assert!(output.contains("old request"));
        assert!(output.contains("┌─ AI"));
        assert_eq!(output.matches("final reply").count(), 1);
    }

    #[test]
    fn project_sources_and_fixed_review_diffs_stay_literal() {
        let mut app = App::empty(Keymap::default());
        app.items.push(crate::app::ReadingItem {
            title: "README.md".into(),
            version: "fixed_snapshot".into(),
            intent: "**intent**".into(),
            code: "**prose**\n\n- entry".into(),
            diff: "+**literal_added**\n-# literal_removed".into(),
            change: Some(crate::checkpoint::ChangeKind::Modified),
            evidence: "**evidence**".into(),
            explanation: "**explanation**".into(),
            read: false,
            scroll: 0,
            evidence_scroll: 0,
        });
        app.view = View::Project;
        let mut cache = ReaderCache::default();
        cache.prepare(&app, 60);
        assert!(!text(cache.contract_lines.as_ref().unwrap()).contains("**prose**"));
        app.items[0].title = "src/literal.rs".into();
        cache.prepare(&app, 60);
        assert!(cache.contract_lines.is_none());
        assert!(cache.lines.iter().any(|(s, _)| s == "**prose**"));
        app.view = View::Review;
        app.page = Page::Code;
        cache.prepare(&app, 60);
        assert!(cache.contract_lines.is_none());
        assert!(cache.lines.contains(&("+**literal_added**".into(), 1)));
        assert!(cache.lines.contains(&("-# literal_removed".into(), -1)));
        app.page = Page::Intent;
        cache.prepare(&app, 60);
        let output = text(cache.contract_lines.as_ref().unwrap());
        assert!(output.contains("fixed_snapshot"));
        assert!(output.ends_with("intent"));
    }

    #[test]
    fn compact_labels_use_inline_not_block_syntax() {
        for source in ["# literal heading", "- literal item", "---", "```rust"] {
            let line = compact(source, 80, Theme::new(false), Role::Body);
            let output: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            assert_eq!(output, source);
        }
        let line = compact(
            "**bold**\n[link](https://example.test)",
            80,
            Theme::new(false),
            Role::Body,
        );
        let output: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(output.contains("bold"));
        assert!(output.contains("https://example.test"));
        assert!(!output.contains("**"));
        assert!(!output.contains('\n'));
    }

    #[test]
    fn chat_inline_styles_preserve_roles_and_search_keeps_workflow_metadata() {
        let records = vec![
            ("状态：已暂停 · 未验证".into(), 2),
            ("┌─ 你".into(), 7),
            (
                "# heading\n\n[link](https://example.test) and `code`".into(),
                3,
            ),
            ("┌─ AI".into(), 8),
            ("# answer\n\n**reply**".into(), 4),
        ];
        let filtered = record_layout(&records, 60, "heading");
        assert!(text(&filtered).contains("状态：已暂停 · 未验证"));
        assert!(text(&filtered).contains("┌─ 你"));
        assert!(!text(&filtered).contains("┌─ AI"));
        for light in [false, true] {
            let theme = Theme::new(light);
            for line in record_layout(&records, 60, "") {
                let expected = match line.context.role {
                    Role::User => theme.accent,
                    Role::Assistant => theme.good,
                    _ => continue,
                };
                for fragment in line.fragments {
                    assert_eq!(
                        theme.markdown_style(line.context, fragment.style).fg,
                        Some(expected)
                    );
                }
            }
        }
    }

    #[test]
    fn project_markdown_extensions_are_explicit() {
        for path in ["README.md", "notes.MARKDOWN", "a.mdown", "a.mkd"] {
            assert!(is_markdown_path(path));
        }
        for path in ["src/a.rs", "a.md.rs", "Makefile", "a.txt", "a.mdx"] {
            assert!(!is_markdown_path(path));
        }
        assert!(block_matches(
            &natural_block("some **visible** phrase", 4, Role::Body),
            "some visible phrase"
        ));
    }
}

/// Human-started regression only (not executed by this node):
/// `cargo test --lib ui::input_viewport_tests`
/// Manual: open command, search, read-only discussion, coding discussion and
/// paused correction inputs in both themes. Type past one row, exactly fill a
/// row, then backspace across its boundary. Paste CJK, emoji, combining marks,
/// Markdown source and enough text to exceed terminal height. Resize down to
/// 24x8 and back up; confirm the tail/cursor stay inside the input, mode warnings
/// and Enter/Esc hints remain visible. Submit a harmless search/AI message and
/// verify its original source; cancel commands with Esc without executing them.
#[cfg(test)]
mod input_viewport_tests {
    use super::*;
    use ratatui::{
        Terminal,
        backend::{Backend, TestBackend},
    };

    #[test]
    fn exact_fill_wrap_and_delete_follow_the_insertion_point() {
        let empty = InputLayout::new("", "> ", 6);
        assert_eq!(empty.lines, vec!["> "]);
        assert_eq!(empty.cursor_column, 2);
        let exact = InputLayout::new("abcd", "> ", 6);
        assert_eq!(exact.lines, vec!["> abcd", ""]);
        assert_eq!(exact.cursor_column, 0);
        let continued = InputLayout::new("abcde", "> ", 6);
        assert_eq!(continued.lines, vec!["> abcd", "e"]);
        assert_eq!(continued.cursor_column, 1);
        let deleted = InputLayout::new("abc", "> ", 6);
        assert_eq!(deleted.lines, vec!["> abc"]);
        assert_eq!(deleted.cursor_column, 5);
    }

    #[test]
    fn literal_graphemes_sanitization_and_resize_share_one_layout() {
        let source = "中英👩‍💻e\u{301}**_#`[link](url)";
        for width in [4, 7, 22, 74] {
            let layout = InputLayout::new(source, "> ", width);
            assert_eq!(layout.lines.concat(), format!("> {source}"));
            assert!(layout.lines.iter().all(|line| line.width() <= width));
            assert!(layout.cursor_column < width);
            assert!(layout.lines.iter().any(|line| line.contains("👩‍💻")));
            assert!(layout.lines.iter().any(|line| line.contains("e\u{301}")));
        }
        let layout = InputLayout::new("a\t中\u{1b}\nend", "", 22);
        assert_eq!(layout.lines, vec!["a    中�", "end"]);
        assert_eq!(layout.cursor_column, 3);
        let layout = InputLayout::new("abcdef\n", "", 6);
        assert_eq!(layout.lines, vec!["abcdef", ""]);
        assert_eq!(layout.cursor_column, 0);
    }

    #[test]
    fn long_input_window_always_contains_the_cursor_row() {
        let layout = InputLayout::new(&"中文👩‍💻abc".repeat(100), "> ", 22);
        for height in [1, 2, 6, 1000] {
            let start = layout.window_start(height);
            let cursor_row = layout.lines.len() - 1 - start;
            assert!(cursor_row < height);
            assert_eq!(layout.lines[start..].last(), layout.lines.last());
        }
    }

    #[test]
    fn popups_keep_tail_cursor_hints_and_mode_notices_inside_terminal() {
        let source = format!("{}尾部XYZ", "中👩‍💻**source**".repeat(100));
        for (width, height) in [(24, 8), (36, 12), (100, 30)] {
            for notice in [
                "只读讨论 · 代码可能外发",
                "已暂停 · 继续后才发送",
                "仍有编码权限",
            ] {
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                terminal
                    .draw(|frame| {
                        input_popup(
                            frame,
                            Theme::new(false),
                            " 消息 ",
                            &source,
                            "",
                            76,
                            &[notice, "Enter 提交 · Esc 返回"],
                        )
                    })
                    .unwrap();
                let cursor = terminal.backend_mut().get_cursor_position().unwrap();
                assert!(cursor.x > 0 && cursor.x < width - 1);
                assert!(cursor.y > 0 && cursor.y < height - 1);
                let buffer = terminal.backend().buffer();
                let output: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
                // The marker itself may wrap; inspect the cell immediately
                // before the insertion cursor rather than flattening borders.
                assert_eq!(buffer[(cursor.x - 1, cursor.y)].symbol(), "Z");
                assert!(output.contains("Enter"));
                assert!(output.contains("Esc"));
                let visible = output.replace(' ', "");
                assert!(
                    visible.contains("只读讨论")
                        || visible.contains("已暂停")
                        || visible.contains("仍有编码权限")
                );
            }
        }
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
