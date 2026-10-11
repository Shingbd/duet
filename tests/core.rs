use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use duet_workbench::{
    app::{App, Mode, Page, Panel, ReadingItem},
    keymap::{Action, Keymap},
    rpc::{MAX_RECORD_BYTES, RunState, RunStatus, read_record},
    ui,
};
use ratatui::{Terminal, backend::TestBackend};
use serde_json::json;
use std::{collections::BTreeMap, io::Cursor};

#[test]
fn approval_shortcut_is_configurable_contextual_and_never_fires_in_input_modes() {
    let event = |code| KeyEvent::new(code, KeyModifiers::NONE);
    assert_eq!(
        Keymap::default().resolve(event(KeyCode::Char('a'))),
        Some(Action::ApproveNode)
    );
    let custom = Keymap::from_overrides(BTreeMap::from([(
        "approve_node".into(),
        vec!["ctrl+a".into()],
    )]))
    .unwrap();
    assert_eq!(custom.resolve(event(KeyCode::Char('a'))), None);
    assert_eq!(
        custom.resolve(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL)),
        Some(Action::ApproveNode)
    );
    assert!(
        Keymap::from_overrides(BTreeMap::from([("approve_node".into(), vec!["q".into()])]))
            .is_err()
    );
    let mut app = App::empty(Keymap::default());
    app.act(Action::Tasks);
    app.handle_key(event(KeyCode::Char('a')));
    assert_eq!(app.mode, Mode::Browse);
    assert!(app.status.contains("审核"));
    for mode in [
        Mode::Input {
            search: false,
            text: String::new(),
        },
        Mode::Message {
            text: String::new(),
        },
    ] {
        app.mode = mode;
        app.handle_key(event(KeyCode::Char('a')));
        match &app.mode {
            Mode::Input { text, .. } | Mode::Message { text } => assert_eq!(text, "a"),
            _ => panic!("typing a must not approve"),
        }
    }
}

#[test]
fn pi_label_describes_current_host_session_not_external_pi_availability() {
    let mut app = App::empty(Keymap::default());
    assert_eq!(app.pi_session_label(), "Pi 会话未启动");
    app.agent_connected = true;
    app.run.status = RunStatus::Running;
    assert_eq!(app.pi_session_label(), "Pi 会话运行中");
    app.run.status = RunStatus::Settled;
    assert!(app.pi_session_label().contains("空闲"));
    app.agent_connected = false;
    assert_eq!(app.pi_session_label(), "Pi 会话已结束");
    app.run.status = RunStatus::Failed;
    assert_eq!(app.pi_session_label(), "Pi 会话失败");
    app.tasks.push(duet_workbench::nodes::TaskRecord::legacy(
        "history",
        "old task",
        "old session",
    ));
    app.select_task_entry(duet_workbench::app::TaskSelection {
        task_id: "history".into(),
        node_id: None,
    });
    app.act(Action::Tasks);
    assert_eq!(app.pi_session_label(), "Pi 会话失败");
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal.draw(|f| ui::draw(f, &app)).unwrap();
    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect::<String>()
        .replace(' ', "");
    assert!(rendered.contains("Pi会话失败"));
    assert!(!rendered.contains("Pi未连接"));
}

#[test]
fn start_delete_follow_shortcuts_are_configurable_contextual_and_confirmed() {
    use duet_workbench::{
        app::{TaskSelection, View},
        nodes::{NodeContract, TaskPlan, TaskRecord},
    };
    let mut app = App::empty(Keymap::default());
    app.tasks
        .push(TaskRecord::legacy("saved", "goal", "session"));
    app.tasks[0]
        .install_plan(TaskPlan {
            schema_version: 1,
            objective: "goal".into(),
            constraints: vec![],
            acceptance: vec!["review".into()],
            nodes: vec![NodeContract {
                id: "n".into(),
                title: "node".into(),
                objective: "goal".into(),
                scope: vec!["src".into()],
                boundaries: vec!["own source only".into()],
                implementation: "write".into(),
                constraints: vec![],
                dependencies: vec![],
                acceptance: vec!["inspect".into()],
            }],
        })
        .unwrap();
    app.select_task_entry(TaskSelection {
        task_id: "saved".into(),
        node_id: None,
    });
    app.view = View::Tasks;
    app.tasks[0].discussion_pending = true;
    app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
    assert_eq!(app.mode, Mode::Browse);
    app.tasks[0].discussion_pending = false;
    app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
    assert!(matches!(app.mode, Mode::ConfirmStart { .. }));
    assert!(app.requested_start.is_none());
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(app.requested_start.is_some());
    app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
    assert!(matches!(app.mode, Mode::ConfirmDelete { .. }));
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.requested_delete.is_none() && app.tasks.len() == 1);
    app.view = View::Project;
    app.act(Action::StartTask);
    assert_eq!(app.mode, Mode::Browse);
    app.mode = Mode::Message {
        text: String::new(),
    };
    for c in ['s', 'd', 'v'] {
        app.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
    assert_eq!(app.mode, Mode::Message { text: "sdv".into() });
    let keys = Keymap::from_overrides(BTreeMap::from([(
        "start_task".into(),
        vec!["ctrl+s".into()],
    )]))
    .unwrap();
    assert_eq!(
        keys.resolve(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)),
        Some(Action::StartTask)
    );
    assert!(
        keys.resolve(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE))
            .is_none()
    );
}

fn item() -> ReadingItem {
    ReadingItem {
        title: "test.rs".into(),
        version: "test".into(),
        intent: String::new(),
        code: "fn main() {}".into(),
        diff: String::new(),
        change: None,
        evidence: String::new(),
        explanation: String::new(),
        read: false,
        scroll: 0,
        evidence_scroll: 0,
    }
}
fn fixture_app() -> App {
    let mut app = App::empty(Keymap::default());
    app.enqueue(item());
    app
}

#[test]
fn lowercase_jk_navigate_and_uppercase_keys_remain_unbound() {
    let mut app = fixture_app();
    assert_eq!(app.keymap.resolve(key(KeyCode::Char('J'))), None);
    assert_eq!(
        app.keymap
            .resolve(KeyEvent::new(KeyCode::Char('K'), KeyModifiers::SHIFT)),
        None
    );
    app.handle_key(key(KeyCode::Char('j')));
    assert_eq!(app.current().unwrap().scroll, 1);
    for c in ['J', 'K'] {
        app.handle_key(key(KeyCode::Char(c)));
    }
    assert_eq!(app.current().unwrap().scroll, 1);
    app.handle_key(key(KeyCode::Char('k')));
    assert_eq!(app.current().unwrap().scroll, 0);
    let custom = Keymap::from_overrides(BTreeMap::from([
        ("palette".into(), vec!["J".into()]),
        ("theme".into(), vec!["K".into()]),
    ]))
    .unwrap();
    assert_eq!(
        custom.resolve(key(KeyCode::Char('J'))),
        Some(Action::Palette)
    );
    assert_eq!(custom.resolve(key(KeyCode::Char('K'))), Some(Action::Theme));
    app.mode = Mode::Input {
        search: false,
        text: String::new(),
    };
    for c in "jJkK".chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }
    assert_eq!(
        app.mode,
        Mode::Input {
            search: false,
            text: "jJkK".into()
        }
    );
    assert_eq!(app.current().unwrap().scroll, 0);
}

#[test]
fn fullscreen_navigation_cannot_target_a_hidden_panel() {
    let mut app = fixture_app();
    app.panel = Panel::Queue;
    app.act(Action::Fullscreen);
    assert!(app.fullscreen);
    assert_eq!(app.panel, Panel::Reader);
    app.handle_key(key(KeyCode::Char('j')));
    assert_eq!(app.current().unwrap().scroll, 1);
    app.act(Action::NextPanel);
    assert!(!app.fullscreen);
    assert_eq!(app.panel, Panel::Evidence);
}

#[test]
fn empty_workspace_has_no_sample_data_and_navigation_is_safe() {
    let mut app = App::empty(Keymap::default());
    for action in [
        Action::Up,
        Action::Down,
        Action::NextItem,
        Action::PreviousItem,
        Action::Open,
        Action::Search,
        Action::MarkRead,
    ] {
        app.act(action);
    }
    assert!(app.current().is_none());
    assert!(app.items.is_empty());
    assert!(app.notes.is_empty());
    app.mode = Mode::Browse;
    let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
    terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
    let output: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(!output.contains("邀请码"));
    assert!(!output.contains("DEMO"));
    assert_eq!(app.body(), "暂无内容");
    assert!(output.contains("DUET"));
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::empty())
}

#[test]
fn delivery_does_not_move_reading_position_or_focus() {
    let mut app = fixture_app();
    app.items[0].scroll = 3;
    assert!(app.enqueue(item()));
    assert_eq!(app.items.len(), 2);
    assert_eq!(app.selected, 0);
    assert_eq!(app.current().unwrap().scroll, 3);
    assert_eq!(app.panel, Panel::Reader);
    assert_eq!(app.page, Page::Code);
}

#[test]
fn unread_backpressure_pauses_until_human_finishes() {
    let mut app = fixture_app();
    let mut second = item();
    second.title = "second.rs".into();
    assert!(app.enqueue(second));
    assert!(!app.can_continue());
    assert!(!app.enqueue(item()));
    app.act(Action::MarkRead);
    app.handle_key(key(KeyCode::Enter));
    assert!(app.can_continue());
}

#[test]
fn input_mode_does_not_trigger_browse_shortcuts() {
    let mut app = fixture_app();
    app.act(Action::Palette);
    for c in "q1fem知识".chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }
    assert!(!app.quit);
    assert!(!app.fullscreen);
    assert!(!app.current().unwrap().read);
    assert!(matches!(&app.mode, Mode::Input { text, .. } if text == "q1fem知识"));
    app.handle_key(key(KeyCode::Esc));
    assert_eq!(app.mode, Mode::Browse);
}

#[test]
fn escape_does_not_abort_agent() {
    let mut app = fixture_app();
    app.run.apply(&json!({"type":"agent_start"}));
    app.act(Action::Help);
    app.handle_key(key(KeyCode::Esc));
    assert_eq!(app.run.status, RunStatus::Running);
}

#[test]
fn coding_task_requires_explicit_confirmation_and_can_be_cancelled() {
    let mut app = App::empty(Keymap::default());
    app.act(Action::Palette);
    for c in "code fix bug".chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }
    app.handle_key(key(KeyCode::Enter));
    assert!(matches!(app.mode, Mode::ConfirmCode { .. }));
    assert!(app.requested_task.is_none());
    app.handle_key(key(KeyCode::Esc));
    assert!(app.requested_task.is_none());
    app.mode = Mode::ConfirmCode {
        prompt: "fix bug".into(),
    };
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(app.requested_task.as_deref(), Some("fix bug"));
}

#[test]
fn mark_read_requires_confirmation() {
    let mut app = fixture_app();
    app.act(Action::MarkRead);
    assert!(!app.current().unwrap().read);
    app.handle_key(key(KeyCode::Esc));
    assert!(!app.current().unwrap().read);
    app.act(Action::MarkRead);
    app.handle_key(key(KeyCode::Enter));
    assert!(app.current().unwrap().read);
    assert!(app.status.contains("未批准"));
}

#[test]
fn keymap_conflicts_and_unknown_actions_are_rejected() {
    let config = BTreeMap::from([("quit".into(), vec!["e".into()])]);
    assert!(Keymap::from_overrides(config).is_err());
    assert!(Keymap::from_overrides(BTreeMap::from([("unknown".into(), vec![])])).is_err());
}

#[test]
fn configured_keys_are_used_in_help_and_resolution() {
    let map =
        Keymap::from_overrides(BTreeMap::from([("quit".into(), vec!["alt+x".into()])])).unwrap();
    assert_eq!(
        map.resolve(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::ALT)),
        Some(Action::Quit)
    );
    assert_eq!(map.resolve(key(KeyCode::Char('q'))), None);
    assert_eq!(map.hint(Action::Quit), "alt+x");
}

#[test]
fn shifted_punctuation_works() {
    let map = Keymap::default();
    assert_eq!(
        map.resolve(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::SHIFT)),
        Some(Action::Help)
    );
    assert_eq!(
        map.resolve(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT)),
        Some(Action::PreviousPanel)
    );
}

#[test]
fn lf_framing_preserves_unicode_line_separators() {
    let data = "{\"type\":\"message\",\"text\":\"中\u{2028}文\u{2029}字\"}\r\n";
    let mut reader = Cursor::new(data.as_bytes());
    assert_eq!(
        read_record(&mut reader).unwrap().unwrap()["text"],
        "中\u{2028}文\u{2029}字"
    );
    assert!(read_record(&mut reader).unwrap().is_none());
}

#[test]
fn protocol_rejects_oversized_malformed_and_incomplete_frames() {
    let mut oversized = Cursor::new(vec![b'a'; MAX_RECORD_BYTES + 1]);
    assert!(read_record(&mut oversized).is_err());
    for bad in ["not-json\n", "[]\n", "{\"type\":\"x\"}"] {
        assert!(read_record(&mut Cursor::new(bad)).is_err());
    }
}

#[test]
fn command_acceptance_and_agent_end_are_not_completion() {
    let mut run = RunState::default();
    run.register("p1", "prompt");
    run.apply(&json!({"type":"agent_start"}));
    run.apply(&json!({"type":"response","id":"p1","command":"prompt","success":true,"data":{"disposition":"started"}}));
    run.apply(&json!({"type":"agent_end","willRetry":true}));
    assert_eq!(run.status, RunStatus::Running);
    run.apply(&json!({"type":"agent_settled","aborted":false}));
    assert_eq!(run.status, RunStatus::Settled);
}

#[test]
fn responses_correlate_by_id_not_arrival_order() {
    let mut run = RunState::default();
    run.register("p1", "prompt");
    run.register("s2", "get_state");
    run.apply(&json!({"type":"response","id":"s2","command":"get_state","success":true}));
    assert!(run.pending.contains_key("p1"));
    run.apply(&json!({"type":"response","id":"foreign","success":false}));
    assert_eq!(run.status, RunStatus::Idle);
    run.apply(&json!({"type":"response","id":"p1","success":false,"error":"rejected"}));
    assert_eq!(run.status, RunStatus::Failed);
}

#[test]
fn authoritative_message_replaces_deltas_and_failure_survives_settle() {
    let mut run = RunState::default();
    run.apply(&json!({"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"partial"}}));
    run.apply(&json!({"type":"message_end","message":{"role":"assistant","content":[{"type":"text","text":"final"}],"stopReason":"error","errorMessage":"provider failed"}}));
    run.apply(&json!({"type":"agent_settled","aborted":false}));
    assert_eq!(run.text, "final");
    assert_eq!(run.status, RunStatus::Failed);
}

#[test]
fn abort_and_handled_prompt_are_distinct() {
    let mut run = RunState::default();
    run.apply(&json!({"type":"agent_start"}));
    run.apply(&json!({"type":"agent_settled","aborted":true}));
    assert_eq!(run.status, RunStatus::Aborted);
    run.register("h1", "prompt");
    run.apply(
        &json!({"type":"response","id":"h1","success":true,"data":{"disposition":"handled"}}),
    );
    assert_eq!(run.status, RunStatus::Idle);
}

#[test]
fn repository_content_cannot_inject_terminal_control_sequences() {
    assert_eq!(
        ui::safe_text("\x1b[2J\x07hello\tworld\n"),
        "�[2J�hello    world\n"
    );
}

#[test]
fn evidence_panel_scrolls_independently() {
    let mut app = fixture_app();
    app.panel = Panel::Evidence;
    app.act(Action::Down);
    assert_eq!(app.current().unwrap().evidence_scroll, 1);
    assert_eq!(app.current().unwrap().scroll, 0);
}

#[test]
fn terminal_renders_chinese_at_multiple_sizes_modes_and_themes() {
    for (width, height) in [(20, 5), (40, 14), (80, 24), (120, 36), (160, 40)] {
        for light in [false, true] {
            let mut app = fixture_app();
            app.light = light;
            app.enqueue(item());
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            for panel in [Panel::Queue, Panel::Reader, Panel::Evidence] {
                app.panel = panel;
                for mode in [
                    Mode::Browse,
                    Mode::Help,
                    Mode::Input {
                        search: false,
                        text: "架构与事务".into(),
                    },
                    Mode::ConfirmRead,
                ] {
                    app.mode = mode;
                    terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
                }
            }
            app.mode = Mode::Browse;
            terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
            let output: String = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|c| c.symbol())
                .collect();
            assert!(output.contains("DUET") || output.contains("Duet"));
        }
    }
}
