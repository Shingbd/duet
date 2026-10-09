use duet_workbench::{
    app::{App, ReadingItem},
    keymap::Keymap,
    storage::WorkspaceStore,
};
use std::{fs, process::Command};

fn app(version: &str) -> App {
    let mut app = App::empty(Keymap::default());
    app.items.push(ReadingItem {
        title: "file.rs".into(),
        version: version.into(),
        intent: String::new(),
        code: String::new(),
        evidence: String::new(),
        explanation: String::new(),
        read: false,
        scroll: 0,
        evidence_scroll: 0,
    });
    app
}

#[test]
fn first_open_creates_local_data_and_reopen_restores_notes_and_progress() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = WorkspaceStore::open(dir.path()).unwrap();
    assert!(dir.path().join(".duet/workspace.db").is_file());
    assert_eq!(
        fs::read_to_string(dir.path().join(".duet/.gitignore")).unwrap(),
        "*\n"
    );
    let mut original = app("sha-one");
    original.notes.push("真实用户笔记".into());
    original.items[0].read = true;
    original.items[0].scroll = 8;
    original.light = true;
    store.save(&original).unwrap();
    drop(store);
    let store = WorkspaceStore::open(dir.path()).unwrap();
    let mut restored = app("sha-one");
    store.restore(&mut restored).unwrap();
    assert_eq!(restored.notes, original.notes);
    assert!(restored.current().unwrap().read);
    assert_eq!(restored.current().unwrap().scroll, 8);
    assert!(restored.light);
    let mut changed = app("sha-two");
    store.restore(&mut changed).unwrap();
    assert!(!changed.current().unwrap().read);
    assert_eq!(changed.current().unwrap().scroll, 0);
}

#[test]
fn conversation_survives_restart_and_is_scoped_to_session_and_project() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = WorkspaceStore::open(dir.path()).unwrap();
    store.append_message("one", "user", "你好").unwrap();
    store.append_message("one", "assistant", "回应").unwrap();
    store.append_message("two", "user", "另一个会话").unwrap();
    assert!(store.append_message("one", "invalid", "bad").is_err());
    drop(store);
    let store = WorkspaceStore::open(dir.path()).unwrap();
    let history = store.history("one").unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].role, "user");
    assert_eq!(history[1].content, "回应");
    let other = tempfile::tempdir().unwrap();
    assert!(
        WorkspaceStore::open(other.path())
            .unwrap()
            .history("one")
            .unwrap()
            .is_empty()
    );
}

#[test]
fn data_is_ignored_without_modifying_root_gitignore() {
    let dir = tempfile::tempdir().unwrap();
    Command::new("git")
        .arg("-C")
        .arg(dir.path())
        .args(["init", "-q"])
        .status()
        .unwrap();
    fs::write(dir.path().join(".gitignore"), "target/\n").unwrap();
    let _store = WorkspaceStore::open(dir.path()).unwrap();
    let output = Command::new("git")
        .arg("-C")
        .arg(dir.path())
        .args(["status", "--porcelain", "--untracked-files=all"])
        .output()
        .unwrap();
    assert!(!String::from_utf8(output.stdout).unwrap().contains(".duet"));
    assert_eq!(
        fs::read_to_string(dir.path().join(".gitignore")).unwrap(),
        "target/\n"
    );
}

#[test]
fn project_lock_prevents_concurrent_instances() {
    let dir = tempfile::tempdir().unwrap();
    let store = WorkspaceStore::open(dir.path()).unwrap();
    assert!(WorkspaceStore::open(dir.path()).is_err());
    drop(store);
    assert!(WorkspaceStore::open(dir.path()).is_ok());
}

#[cfg(unix)]
#[test]
fn symlinked_storage_is_rejected() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    symlink(outside.path(), dir.path().join(".duet")).unwrap();
    assert!(WorkspaceStore::open(dir.path()).is_err());
}
