use duet_workbench::checkpoint::{Repository, validate_path};
use std::{fs, path::Path, process::Command};
use tempfile::TempDir;

fn git(path: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}
#[test]
fn node_checkpoint_can_delete_an_untracked_file_added_by_a_previous_node() {
    let dir = fixture();
    let repo = Repository::open(dir.path()).unwrap();
    let head = git(dir.path(), &["rev-parse", "HEAD"]);
    let index = git(dir.path(), &["diff", "--cached"]);
    fs::write(dir.path().join("new-node.txt"), "new content").unwrap();
    let first = repo.capture(&["new-node.txt".into()]).unwrap();
    fs::remove_file(dir.path().join("new-node.txt")).unwrap();
    let second = repo.capture_worktree_from(&first).unwrap();
    assert_eq!(second.parent.as_deref(), Some(first.commit.as_str()));
    let changes = repo.snapshot_changes(&second).unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].path, "new-node.txt");
    assert!(changes[0].deleted);
    assert_eq!(git(dir.path(), &["rev-parse", "HEAD"]), head);
    assert_eq!(git(dir.path(), &["diff", "--cached"]), index);
}

fn fixture() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q"]);
    git(dir.path(), &["config", "user.name", "Test"]);
    git(dir.path(), &["config", "user.email", "test@localhost"]);
    fs::write(dir.path().join("register.rs"), "baseline\n").unwrap();
    fs::write(dir.path().join("other.rs"), "other baseline\n").unwrap();
    fs::write(dir.path().join(".gitignore"), ".env\n").unwrap();
    git(dir.path(), &["add", "."]);
    git(dir.path(), &["commit", "-qm", "baseline"]);
    dir
}

#[test]
fn snapshot_preserves_user_index_head_and_working_files() {
    let dir = fixture();
    let repo = Repository::open(dir.path()).unwrap();
    fs::write(dir.path().join("register.rs"), "staged version\n").unwrap();
    git(dir.path(), &["add", "register.rs"]);
    fs::write(dir.path().join("register.rs"), "working version\n").unwrap();
    fs::write(dir.path().join("other.rs"), "unselected dirty\n").unwrap();
    fs::write(dir.path().join("new.rs"), "explicit new file\n").unwrap();
    let index_before = fs::read(dir.path().join(".git/index")).unwrap();
    let head_before = git(dir.path(), &["rev-parse", "HEAD"]);
    let snapshot = repo
        .capture(&["register.rs".into(), "new.rs".into()])
        .unwrap();
    assert_eq!(
        repo.read(&snapshot, "register.rs").unwrap(),
        "working version\n"
    );
    assert_eq!(
        repo.read(&snapshot, "new.rs").unwrap(),
        "explicit new file\n"
    );
    assert_eq!(
        repo.read(&snapshot, "other.rs").unwrap(),
        "other baseline\n"
    );
    assert_eq!(
        fs::read(dir.path().join(".git/index")).unwrap(),
        index_before
    );
    assert_eq!(git(dir.path(), &["rev-parse", "HEAD"]), head_before);
    assert_eq!(
        fs::read_to_string(dir.path().join("other.rs")).unwrap(),
        "unselected dirty\n"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("register.rs")).unwrap(),
        "working version\n"
    );
    assert!(
        snapshot
            .reference
            .as_ref()
            .unwrap()
            .starts_with("refs/duet/checkpoints/")
    );
    assert!(repo.diff(&snapshot).unwrap().contains("+working version"));
    let reopened = repo
        .resolve_snapshot(snapshot.reference.as_ref().unwrap())
        .unwrap();
    assert_eq!(reopened.commit, snapshot.commit);
    assert_eq!(
        repo.read(&reopened, "register.rs").unwrap(),
        "working version\n"
    );
    // Later AI edits must not change the human's material.
    fs::write(dir.path().join("register.rs"), "AI changed again\n").unwrap();
    assert_eq!(
        repo.read(&snapshot, "register.rs").unwrap(),
        "working version\n"
    );
}

#[test]
fn baseline_does_not_mix_uncommitted_changes() {
    let dir = fixture();
    let repo = Repository::open(dir.path()).unwrap();
    fs::write(dir.path().join("register.rs"), "dirty\n").unwrap();
    let baseline = repo.baseline().unwrap();
    assert!(repo.is_dirty().unwrap());
    assert_eq!(repo.read(&baseline, "register.rs").unwrap(), "baseline\n");
}

#[test]
fn selected_deleted_file_is_removed_only_in_snapshot() {
    let dir = fixture();
    let repo = Repository::open(dir.path()).unwrap();
    fs::remove_file(dir.path().join("register.rs")).unwrap();
    let snapshot = repo.capture(&["register.rs".into()]).unwrap();
    assert!(
        !repo
            .files(&snapshot)
            .unwrap()
            .contains(&"register.rs".into())
    );
    assert!(
        repo.files(&repo.baseline().unwrap())
            .unwrap()
            .contains(&"register.rs".into())
    );
}

#[test]
fn ignored_missing_directory_and_traversal_paths_are_rejected() {
    let dir = fixture();
    let repo = Repository::open(dir.path()).unwrap();
    fs::write(dir.path().join(".env"), "secret").unwrap();
    for path in [".env", "missing.rs", "../outside", "/etc/passwd", ".git"] {
        assert!(repo.capture(&[path.into()]).is_err(), "{path}");
    }
    assert!(repo.capture(&[]).is_err());
    assert!(validate_path(".").is_err());
}

#[cfg(unix)]
#[test]
fn symlinks_and_symlink_ancestors_are_rejected() {
    use std::os::unix::fs::symlink;
    let dir = fixture();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("secret"), "outside").unwrap();
    symlink(outside.path(), dir.path().join("linked")).unwrap();
    symlink("register.rs", dir.path().join("alias")).unwrap();
    let repo = Repository::open(dir.path()).unwrap();
    assert!(repo.capture(&["linked/secret".into()]).is_err());
    assert!(repo.capture(&["alias".into()]).is_err());
}

#[test]
fn read_rejects_large_and_non_utf8_files_without_crashing() {
    let dir = fixture();
    fs::write(dir.path().join("binary"), [0xff, 0xfe]).unwrap();
    fs::write(dir.path().join("large"), vec![b'x'; 1_048_577]).unwrap();
    git(dir.path(), &["add", "."]);
    git(dir.path(), &["commit", "-qm", "more files"]);
    let repo = Repository::open(dir.path()).unwrap();
    let baseline = repo.baseline().unwrap();
    assert!(repo.read(&baseline, "large").is_err());
    assert!(repo.read(&baseline, "binary").is_err());
}
