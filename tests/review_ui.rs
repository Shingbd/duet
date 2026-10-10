use duet_workbench::{
    app::{App, Panel, View},
    checkpoint::{ChangeKind, Repository},
    keymap::{Action, Keymap},
    storage::WorkspaceStore,
    ui,
};
use ratatui::{Terminal, backend::TestBackend};
use std::{fs, process::Command};

fn setup() -> (
    tempfile::TempDir,
    Repository,
    duet_workbench::checkpoint::Snapshot,
    App,
) {
    let dir = tempfile::tempdir().unwrap();
    for (path, text) in [
        ("a.rs", "old a\n"),
        ("b.rs", "unchanged\n"),
        ("d.rs", "deleted\n"),
    ] {
        fs::write(dir.path().join(path), text).unwrap();
    }
    for args in [
        vec!["init", "-q"],
        vec!["add", "."],
        vec![
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@localhost",
            "commit",
            "-qm",
            "baseline",
        ],
    ] {
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args(args)
                .status()
                .unwrap()
                .success()
        );
    }
    let repo = Repository::open(dir.path()).unwrap();
    let mut app = App::browse(&repo, repo.baseline().unwrap(), Keymap::default()).unwrap();
    app.items[0].scroll = 1;
    fs::write(dir.path().join("a.rs"), "new a\n").unwrap();
    fs::write(dir.path().join("n.rs"), "new file\n").unwrap();
    fs::remove_file(dir.path().join("d.rs")).unwrap();
    let snapshot = repo
        .capture(&["a.rs".into(), "d.rs".into(), "n.rs".into()])
        .unwrap();
    app.append_checkpoint(&repo, &snapshot).unwrap();
    (dir, repo, snapshot, app)
}

#[test]
fn review_lists_only_changed_paths_once_and_shows_file_scoped_colored_diff() {
    let (_dir, _repo, snapshot, mut app) = setup();
    assert_eq!(app.selected, 0); // Arrival does not replace an existing immutable reading item.
    assert_eq!(app.current().unwrap().code, "old a\n");
    assert_eq!(app.current().unwrap().scroll, 1);
    assert_eq!(app.file_indices().len(), 4); // One sidebar path despite retaining old versions.
    assert_eq!(app.changed_indices().len(), 3);
    app.act(Action::Tasks);
    app.act(Action::Review);
    assert_eq!(app.view, View::Review);
    assert_eq!(app.panel, Panel::Reader);
    assert_eq!(app.current().unwrap().title, "a.rs");
    assert_eq!(app.current().unwrap().version, snapshot.commit);
    assert_eq!(app.current().unwrap().change, Some(ChangeKind::Modified));
    assert!(app.body().contains("-old a"));
    assert!(app.body().contains("+new a"));
    assert!(!app.body().contains("n.rs"));
    let mut terminal = Terminal::new(TestBackend::new(120, 32)).unwrap();
    for light in [false, true] {
        app.light = light;
        terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
        let buffer = terminal.backend().buffer();
        let mut colors = Vec::new();
        let sidebar: Vec<String> = (4..30)
            .map(|y| (0..28).map(|x| buffer[(x, y)].symbol()).collect())
            .collect();
        assert!(!sidebar.iter().any(|line| line.contains("b.rs")));
        assert!(!sidebar.iter().any(|line| line.contains("待审 ·")));
        for (path, marker) in [("a.rs", "M"), ("d.rs", "D"), ("n.rs", "A")] {
            let row = sidebar.iter().position(|line| line.contains(path)).unwrap() as u16 + 4;
            let cell = (0..28)
                .map(|x| &buffer[(x, row)])
                .find(|cell| cell.symbol() == marker)
                .unwrap();
            colors.push(cell.fg);
        }
        assert_ne!(colors[0], colors[1]);
        assert_ne!(colors[0], colors[2]);
        assert_ne!(colors[1], colors[2]);
    }
    app.act(Action::NextItem);
    assert_eq!(app.current().unwrap().title, "d.rs");
    assert!(app.body().contains("-deleted"));
    app.act(Action::NextItem);
    assert_eq!(app.current().unwrap().title, "n.rs");
    assert!(app.body().contains("+new file"));
}

#[test]
fn legacy_prefixed_selection_and_progress_survive_plain_git_style_paths() {
    let (dir, repo, snapshot, mut app) = setup();
    app.act(Action::Review);
    app.items[app.selected].read = true;
    app.items[app.selected].scroll = 2;
    let mut store = WorkspaceStore::open(dir.path()).unwrap();
    store
        .begin_run("r", "s", dir.path(), "branch", "task")
        .unwrap();
    store
        .finish_run("r", "Settled", Some(&snapshot.commit))
        .unwrap();
    store.save(&app).unwrap();
    drop(store);
    let db = rusqlite::Connection::open(dir.path().join(".duet/workspace.db")).unwrap();
    let encoded: String = db
        .query_row(
            "SELECT value FROM ui_state WHERE key='workspace'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let mut saved: serde_json::Value = serde_json::from_str(&encoded).unwrap();
    saved["selected"][1] = "待审 · a.rs".into();
    db.execute(
        "UPDATE ui_state SET value=?1 WHERE key='workspace'",
        [saved.to_string()],
    )
    .unwrap();
    db.execute(
        "UPDATE reading_progress SET path='待审 · a.rs' WHERE version=?1 AND path='a.rs'",
        [&snapshot.commit],
    )
    .unwrap();
    drop(db);
    let store = WorkspaceStore::open(dir.path()).unwrap();
    let mut restored = App::browse(&repo, repo.baseline().unwrap(), Keymap::default()).unwrap();
    store.restore(&mut restored).unwrap();
    assert_eq!(restored.current().unwrap().title, "a.rs");
    assert_eq!(restored.current().unwrap().version, snapshot.commit);
    assert_eq!(restored.current().unwrap().scroll, 2);
    assert!(restored.current().unwrap().read);
}
