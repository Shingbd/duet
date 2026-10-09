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

fn item() -> ReadingItem {
    ReadingItem {
        title: "test.rs".into(),
        version: "test".into(),
        intent: String::new(),
        code: "fn main() {}".into(),
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
    assert!(app.enqueue(item()));
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
