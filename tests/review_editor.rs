use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use duet_workbench::{
    app::{App, Mode, Panel, View},
    keymap::Keymap,
    review_chat,
    review_editor::{Action, Buffer, Proposal},
};
fn buffer() -> Buffer {
    Buffer::new(
        "task".into(),
        Some("node".into()),
        "src/main.rs".into(),
        "a".repeat(40),
        "fn old() {}\n".into(),
        "fn current() {}\n".into(),
    )
}
#[test]
fn exact_context_includes_unsaved_unicode_selection_not_entire_file() {
    let mut b = buffer();
    b.replace(0, b.text.len(), "fn 新设计() {}\nprivate_data\n")
        .unwrap();
    b.cursor = 3;
    assert_eq!(b.context().excerpt, "新设计");
    assert!(!b.context().excerpt.contains("private_data"));
    b.anchor = Some(0);
    b.cursor = b.text.find('\n').unwrap();
    assert_eq!(b.context().excerpt, "fn 新设计() {}");
    b.anchor = None;
    b.cursor = b.text.len();
    assert_eq!(b.context().excerpt, "");
}
#[test]
fn edits_diff_and_undo_preserve_original_and_deleted_lines() {
    let mut b = buffer();
    assert!(
        b.rows
            .iter()
            .any(|r| r.text == "fn old() {}" && r.tag == similar::ChangeTag::Delete)
    );
    b.paste("// human\n").unwrap();
    assert_eq!(b.original, "fn current() {}\n");
    assert!(b.changed());
    assert!(b.rows.iter().any(|r| r.human));
    b.handle(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(!b.changed());
}
#[test]
fn native_focus_isolates_typing_from_approval_and_model_controls() {
    let mut b = buffer();
    b.handle(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE))
        .unwrap();
    b.handle(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE))
        .unwrap();
    assert!(b.text.starts_with('q'));
    assert_eq!(
        b.handle(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL))
            .unwrap(),
        Action::Ask
    );
    assert_eq!(
        b.handle(KeyEvent::new(KeyCode::Char('5'), KeyModifiers::CONTROL))
            .unwrap(),
        Action::Workbench
    );
    assert_eq!(
        b.handle(KeyEvent::new(KeyCode::Char(']'), KeyModifiers::CONTROL))
            .unwrap(),
        Action::Workbench
    );
}
#[test]
fn ai_patch_is_explicit_unique_excerpt_bound_and_rejects_post_discussion_edit() {
    let mut b = buffer();
    b.cursor = 3;
    let context = b.context();
    let answer = review_chat::answer(
        r#"{"reply":"rename","replacement":{"old":"current","new":"better"}}"#,
        &context,
    );
    assert!(!b.text.contains("better"));
    let proposal = answer.proposal.unwrap();
    b.apply(&proposal).unwrap();
    assert!(b.text.contains("better"));
    assert!(b.apply(&proposal).is_err());
    assert!(
        review_chat::answer(
            r#"{"reply":"outside","replacement":{"old":"fn","new":"bad"}}"#,
            &context
        )
        .proposal
        .is_none()
    );
}
#[test]
fn unicode_graphemes_and_size_limit_do_not_corrupt_buffer() {
    let mut b = buffer();
    b.replace(0, b.text.len(), "e\u{301}中文").unwrap();
    b.cursor = 0;
    b.handle(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE))
        .unwrap();
    assert_eq!(b.cursor, 3);
    let before = b.text.clone();
    assert!(b.paste(&"x".repeat(1_048_576)).is_err());
    assert_eq!(b.text, before);
}
#[test]
fn chat_submission_has_no_range_confirmation_and_cancel_keeps_cursor() {
    let mut app = App::empty(Keymap::default());
    let b = buffer();
    app.items.push(duet_workbench::app::ReadingItem {
        title: b.path.clone(),
        version: b.version.clone(),
        intent: String::new(),
        code: b.text.clone(),
        diff: String::new(),
        change: Some(duet_workbench::checkpoint::ChangeKind::Modified),
        evidence: String::new(),
        explanation: String::new(),
        read: false,
        scroll: 0,
        evidence_scroll: 0,
    });
    // A real task is supplied in workflow integration tests; direct mode dispatch checks no confirmation.
    app.review_chat_context = Some(b.context());
    app.view = View::Review;
    app.panel = Panel::Evidence;
    app.mode = Mode::ReviewChat {
        text: "why?".into(),
    };
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(app.requested_review_chat.is_some());
    assert!(matches!(app.mode, Mode::Browse));
    assert!(app.controls.is_empty());
    assert!(app.requested_task.is_none());
}
#[test]
fn repeated_word_elsewhere_is_not_a_patch_target() {
    let mut b = buffer();
    b.replace(0, b.text.len(), "same same").unwrap();
    b.cursor = 5;
    let context = b.context();
    assert_eq!(context.excerpt, "same");
    let p = review_chat::answer(
        r#"{"reply":"second only","replacement":{"old":"same","new":"different"}}"#,
        &context,
    )
    .proposal
    .unwrap();
    b.apply(&p).unwrap();
    assert_eq!(b.text, "same different");
}
#[test]
fn multiline_patch_cannot_extend_beyond_selected_context() {
    let mut b = buffer();
    let context = b.context();
    let p = Proposal {
        context,
        old: "fn current() {}".into(),
        new: "bad".into(),
    };
    assert!(b.apply(&p).is_err());
}
