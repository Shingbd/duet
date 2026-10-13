use duet_workbench::file_tree::Tree;
use std::{fs, path::Path, process::Command};
fn git(root: &Path, args: &[&str]) {
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .status()
            .unwrap()
            .success()
    );
}
#[test]
fn hierarchical_sort_expand_collapse_git_status_ignore_and_refresh_position() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    git(r, &["init", "-q"]);
    fs::create_dir(r.join("src")).unwrap();
    fs::write(r.join("src/a.rs"), "old").unwrap();
    fs::write(r.join("z.txt"), "old").unwrap();
    fs::write(r.join(".gitignore"), "ignored/\n").unwrap();
    git(r, &["add", "."]);
    git(
        r,
        &[
            "-c",
            "user.name=T",
            "-c",
            "user.email=t@example.test",
            "commit",
            "-qm",
            "base",
        ],
    );
    fs::write(r.join("src/a.rs"), "new").unwrap();
    fs::remove_file(r.join("z.txt")).unwrap();
    fs::write(r.join("new.txt"), "new").unwrap();
    fs::create_dir(r.join("ignored")).unwrap();
    fs::write(r.join("ignored/file"), "secret").unwrap();
    fs::create_dir(r.join(".duet")).unwrap();
    fs::write(r.join(".duet/private"), "never list").unwrap();
    let index = fs::read(r.join(".git/index")).unwrap();
    let mut tree = Tree::default();
    tree.refresh(r).unwrap();
    assert_eq!(fs::read(r.join(".git/index")).unwrap(), index);
    assert_eq!(tree.visible()[0].path, "src");
    assert!(!tree.visible().iter().any(|e| e.path == "src/a.rs"));
    tree.selected = "src".into();
    tree.expand();
    tree.move_by(1);
    assert_eq!(tree.selected, "src/a.rs");
    assert_eq!(tree.entry().unwrap().status, 'M');
    tree.refresh(r).unwrap();
    assert_eq!(tree.selected, "src/a.rs");
    assert!(tree.expanded.contains("src"));
    tree.collapse();
    assert_eq!(tree.selected, "src");
    assert!(!tree.expanded.contains("src"));
    assert!(
        tree.entries
            .iter()
            .any(|e| e.path == "z.txt" && e.status == 'D')
    );
    assert!(
        tree.entries
            .iter()
            .any(|e| e.path == "new.txt" && e.status == '?')
    );
    assert!(!tree.entries.iter().any(|e| e.path.starts_with("ignored")));
    tree.show_ignored = true;
    tree.refresh(r).unwrap();
    assert!(
        tree.entries
            .iter()
            .any(|e| e.path == "ignored/file" && e.status == '!')
    );
    assert!(
        !tree
            .entries
            .iter()
            .any(|e| e.path.starts_with(".duet") || e.path.starts_with(".git/"))
    );
}
#[test]
fn plain_folder_tree_does_not_follow_symlink_or_show_private_metadata() {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join("src")).unwrap();
    fs::write(d.path().join("src/a"), "file").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("/tmp", d.path().join("outside")).unwrap();
    let mut t = Tree::default();
    t.refresh(d.path()).unwrap();
    t.reveal("src/a");
    assert_eq!(t.visible().len(), 2);
    assert!(t.entries.iter().all(|e| !e.path.starts_with("outside")));
}
#[test]
fn restored_tree_state_is_preferences_only_not_a_process_request() {
    use duet_workbench::{app::App, keymap::Keymap, storage::WorkspaceStore};
    let d = tempfile::tempdir().unwrap();
    let mut s = WorkspaceStore::open(d.path()).unwrap();
    let mut a = App::empty(Keymap::default());
    a.project_tree.reveal("src/a.rs");
    a.project_tree.show_ignored = true;
    s.save(&a).unwrap();
    let mut b = App::empty(Keymap::default());
    s.restore(&mut b).unwrap();
    assert_eq!(b.project_tree.selected, "src/a.rs");
    assert!(b.project_tree.expanded.contains("src"));
    assert!(b.project_tree.show_ignored);
    assert!(b.requested_editor.is_none() && b.requested_project_open.is_none());
}
