use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use duet_workbench::{
    app::{App, Mode, Panel, View},
    checkpoint::Repository,
    keymap::Keymap,
    storage::WorkspaceStore,
};
use std::{fs, process::Command};

fn git(path: &std::path::Path, args: &[&str]) {
    let result = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..90 {
        fs::write(dir.path().join(format!("b{i:02}.rs")), "baseline").unwrap();
    }
    fs::write(dir.path().join("gone 文件.rs"), "removed source").unwrap();
    git(dir.path(), &["init", "-q"]);
    git(dir.path(), &["add", "."]);
    git(
        dir.path(),
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@localhost",
            "commit",
            "-qm",
            "baseline",
        ],
    );
    dir
}

#[test]
fn restart_restores_all_changed_files_selection_and_progress_from_exact_snapshot() {
    let dir = project();
    let repo = Repository::open(dir.path()).unwrap();
    let mut paths = Vec::new();
    for i in 0..85 {
        let path = format!("z-contract-{i:02}.rs");
        fs::write(
            dir.path().join(&path),
            format!("fixed delivery {i}\nline two\nline three"),
        )
        .unwrap();
        paths.push(path);
    }
    fs::remove_file(dir.path().join("gone 文件.rs")).unwrap();
    paths.push("gone 文件.rs".into());
    let snapshot = repo.capture(&paths).unwrap();
    let mut app = App::browse(&repo, repo.baseline().unwrap(), Keymap::default()).unwrap();
    assert_eq!(app.items.len(), 80);
    let mut store = WorkspaceStore::open(dir.path()).unwrap();
    store
        .begin_run("run", "session", dir.path(), "task-branch", "contract task")
        .unwrap();
    store
        .finish_run("run", "Settled", Some(&snapshot.commit))
        .unwrap();
    app.task = Some("contract task".into());
    app.append_checkpoint(&repo, &snapshot).unwrap();
    assert_eq!(
        app.items
            .iter()
            .filter(|item| item.change.is_some())
            .count(),
        86
    );
    let selected = app
        .items
        .iter()
        .position(|item| item.title == "z-contract-84.rs")
        .unwrap();
    app.selected = selected;
    app.items[selected].read = true;
    app.items[selected].scroll = 2;
    app.view = View::Review;
    app.panel = Panel::Reader;
    store.save(&app).unwrap();
    drop(store);
    // Mutate the live files and advance HEAD. Neither may replace the reading version.
    fs::write(dir.path().join("z-contract-84.rs"), "later mutable edits").unwrap();
    fs::write(dir.path().join("b89.rs"), "new HEAD").unwrap();
    git(dir.path(), &["add", "b89.rs"]);
    git(
        dir.path(),
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@localhost",
            "commit",
            "-qm",
            "advance HEAD",
        ],
    );
    let store = WorkspaceStore::open(dir.path()).unwrap();
    let mut restored = App::browse(&repo, repo.baseline().unwrap(), Keymap::default()).unwrap();
    store.restore(&mut restored).unwrap();
    let current = restored.current().unwrap();
    assert_eq!(current.title, "z-contract-84.rs");
    assert_eq!(current.version, snapshot.commit);
    assert!(current.code.starts_with("fixed delivery 84"));
    assert!(current.read);
    assert_eq!(current.scroll, 2);
    assert!(
        restored
            .items
            .iter()
            .any(|item| item.title == "gone 文件.rs" && item.code.contains("已删除"))
    );
    let count = restored.items.len();
    store.restore(&mut restored).unwrap();
    assert_eq!(
        restored.items.len(),
        count,
        "recovery must not duplicate deliveries"
    );
    restored.mode = Mode::Input {
        search: false,
        text: "changes".into(),
    };
    restored.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(restored.current().unwrap().title, "gone 文件.rs");
    assert_eq!(restored.current().unwrap().version, snapshot.commit);
    assert!(restored.body().contains("-removed source"));
}

#[test]
fn missing_snapshot_is_reported_without_losing_conversation_or_faking_delivery() {
    let dir = project();
    let mut store = WorkspaceStore::open(dir.path()).unwrap();
    store
        .begin_run("run", "session", dir.path(), "branch", "task")
        .unwrap();
    store
        .append_message("session", "assistant", "result")
        .unwrap();
    store
        .finish_run("run", "Settled", Some(&"0".repeat(40)))
        .unwrap();
    let mut app = App::empty(Keymap::default());
    store.restore(&mut app).unwrap();
    assert_eq!(app.history.len(), 1);
    assert!(app.items.is_empty());
    assert!(app.status.contains("审核快照无法恢复"));
}
