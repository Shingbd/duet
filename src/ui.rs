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

pub fn draw(frame: &mut Frame, app: &App) {
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
    } else if app.agent_connected {
        "独立工作区"
    } else {
        "只读"
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
                    if app.agent_connected {
                        format!("Pi {:?}", app.run.status)
                    } else {
                        "Pi 未连接".into()
                    },
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
        reader(frame, app, body, theme);
    } else if body.width >= 116 {
        let cols = Layout::horizontal([
            Constraint::Length(28),
            Constraint::Min(36),
            Constraint::Length(32),
        ])
        .split(body);
        queue(frame, app, cols[0], theme);
        reader(frame, app, cols[1], theme);
        evidence(frame, app, cols[2], theme);
    } else if body.width >= 78 {
        let cols = Layout::horizontal([Constraint::Length(25), Constraint::Min(30)]).split(body);
        queue(frame, app, cols[0], theme);
        if app.panel == Panel::Evidence {
            evidence(frame, app, cols[1], theme);
        } else {
            reader(frame, app, cols[1], theme);
        }
    } else {
        match app.panel {
            Panel::Queue => queue(frame, app, body, theme),
            Panel::Reader => reader(frame, app, body, theme),
            Panel::Evidence => evidence(frame, app, body, theme),
        }
    }
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                format!(" {}", safe_text(&app.status)),
                Style::default().fg(theme.warning),
            )),
            Line::from(Span::styled(
                format!(
                    " {} 切面板 · {} 解释 · {} 全屏 · {} 已读 · {} 命令 · {} 帮助",
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
        Mode::Browse => {}
    }
}

fn queue(frame: &mut Frame, app: &App, area: Rect, theme: Theme) {
    let items: Vec<ListItem> = app
        .items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let marker = if item.read { "✓" } else { "○" };
            let style = if i == app.selected {
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.muted)
            };
            ListItem::new(Line::from(format!(
                "{} {marker} {}",
                if i == app.selected { "›" } else { " " },
                safe_text(&item.title)
            )))
            .style(style)
        })
        .collect();
    // Window follows selection only when the user moves selection, never new delivery.
    let visible = area.height.saturating_sub(2).max(1) as usize;
    let start = app.selected.saturating_sub(visible - 1);
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

fn reader(frame: &mut Frame, app: &App, area: Rect, theme: Theme) {
    let page = match app.page {
        Page::Intent => "意图",
        Page::Code => "代码",
        Page::Evidence => "证据",
    };
    let mut content = Vec::new();
    if let Some(item) = app.current() {
        content.push(Line::from(Span::styled(
            format!("{} · {}", page, safe_text(&item.version)),
            Style::default().fg(theme.muted),
        )));
        content.push(Line::from(""));
    }
    let body = app.body();
    for line in body
        .split('\n')
        .filter(|line| app.filter.is_empty() || line.contains(&app.filter))
    {
        let line = safe_text(line);
        let color = if line.starts_with('+') {
            theme.good
        } else if line.starts_with('-') {
            theme.removed
        } else {
            theme.text
        };
        content.push(Line::from(Span::styled(line, Style::default().fg(color))));
    }
    frame.render_widget(
        Paragraph::new(content)
            .block(theme.block(
                format!(
                        " {} ",
                        match app.view {
                            View::Tasks => "任务详情".to_owned(),
                            View::Knowledge => "笔记".to_owned(),
                            View::Project => "项目".to_owned(),
                            View::Review => app
                                .current()
                                .map(|item| safe_text(&item.title))
                                .unwrap_or_else(|| "代码 / 变更".into()),
                        }
                    ),
                app.panel == Panel::Reader,
            ))
            .wrap(Wrap { trim: false })
            .scroll((app.current().map_or(0, |item| item.scroll), 0)),
        area,
    );
}

fn evidence(frame: &mut Frame, app: &App, area: Rect, theme: Theme) {
    let text = app
        .current()
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
        .unwrap_or_default();
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
            .scroll((app.current().map_or(0, |item| item.evidence_scroll), 0)),
        area,
    );
}

/// Never emit terminal control sequences from repository or model content.
pub fn safe_text(text: &str) -> String {
    text.chars()
        .map(|ch| {
            if ch == '\t' {
                "    ".to_owned()
            } else if ch == '\n' {
                "\n".to_owned()
            } else if ch.is_control() {
                "�".to_owned()
            } else {
                ch.to_string()
            }
        })
        .collect()
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
