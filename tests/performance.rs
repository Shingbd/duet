use duet_workbench::{
    app::{App, Page, ReadingItem},
    keymap::{Action, Keymap},
    storage::WorkspaceStore,
    ui::{Renderer, wrap_cells},
};
use ratatui::{Terminal, backend::TestBackend};
use rusqlite::Connection;
use unicode_width::UnicodeWidthStr;

fn fixture() -> App {
    let mut app = App::empty(Keymap::default());
    app.enqueue(ReadingItem {
        title: "large.rs".into(),
        version: "fixed".into(),
        code: (0..6000)
            .map(|n| format!("let value_{n} = process({n});\n"))
            .collect(),
        intent: String::new(),
        evidence: String::new(),
        explanation: String::new(),
        read: false,
        scroll: 0,
        evidence_scroll: 0,
    });
    app
}

#[test]
fn scrolling_theme_and_agent_activity_reuse_immutable_reader_layout() {
    let mut app = fixture();
    let mut renderer = Renderer::default();
    let mut terminal = Terminal::new(TestBackend::new(120, 32)).unwrap();
    terminal.draw(|frame| renderer.draw(frame, &app)).unwrap();
    assert_eq!(renderer.layout_builds(), 1);
    for _ in 0..20 {
        app.act(Action::Down);
        app.act(Action::Theme);
        app.content_revision += 1; // Agent streams without changing fixed code.
        terminal.draw(|frame| renderer.draw(frame, &app)).unwrap();
    }
    assert_eq!(renderer.layout_builds(), 1);
    app.filter = "value_123".into();
    terminal.draw(|frame| renderer.draw(frame, &app)).unwrap();
    assert_eq!(renderer.layout_builds(), 2);
    terminal.backend_mut().resize(80, 24);
    terminal
        .resize(ratatui::layout::Rect::new(0, 0, 80, 24))
        .unwrap();
    terminal.draw(|frame| renderer.draw(frame, &app)).unwrap();
    assert_eq!(renderer.layout_builds(), 3);
    app.page = Page::Intent;
    terminal.draw(|frame| renderer.draw(frame, &app)).unwrap();
    assert_eq!(renderer.layout_builds(), 4);
}

#[test]
fn wrapping_preserves_graphemes_and_terminal_cell_width() {
    let source = "中文👩‍💻e\u{301}abc";
    let lines = wrap_cells(source, 4);
    assert_eq!(lines.concat(), source);
    assert!(lines.iter().all(|line| line.width() <= 4));
    assert!(lines.iter().any(|line| line.contains("👩‍💻")));
    assert!(lines.iter().any(|line| line.contains("e\u{301}")));
    assert!(wrap_cells("中文", 0).is_empty());
    assert_eq!(wrap_cells("", 4), vec![""]);
}

#[test]
fn unchanged_state_does_not_commit_and_changed_progress_survives_restart() {
    let folder = tempfile::tempdir().unwrap();
    let mut app = fixture();
    let mut store = WorkspaceStore::open(folder.path()).unwrap();
    store.save(&app).unwrap();
    let observer = Connection::open(store.directory().join("workspace.db")).unwrap();
    let version = || {
        observer
            .pragma_query_value(None, "data_version", |row| row.get::<_, u64>(0))
            .unwrap()
    };
    let before = version();
    for _ in 0..20 {
        store.save(&app).unwrap();
    }
    assert_eq!(version(), before, "unchanged saves must not write SQLite");
    app.act(Action::Down);
    store.save(&app).unwrap();
    assert_ne!(version(), before);
    let after = version();
    store.save(&app).unwrap();
    assert_eq!(version(), after);
    drop(observer);
    drop(store);
    let store = WorkspaceStore::open(folder.path()).unwrap();
    let mut restored = fixture();
    store.restore(&mut restored).unwrap();
    assert_eq!(restored.current().unwrap().scroll, 1);
}
