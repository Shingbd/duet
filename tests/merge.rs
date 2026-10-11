#![cfg(unix)]
use duet_workbench::{
    checkpoint::Repository,
    merge,
    nodes::{Execution, NodeContract, TaskPlan, TaskRecord, TaskState, Verification},
    storage::WorkspaceStore,
};
use std::{fs, os::unix::fs::PermissionsExt, path::Path, process::Command};

fn git(path: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().into()
}
fn commit(path: &Path) {
    git(path, &["add", "."]);
    git(
        path,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "-qm",
            "human change",
        ],
    );
}
fn fixture() -> (
    tempfile::TempDir,
    WorkspaceStore,
    TaskRecord,
    std::path::PathBuf,
) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git(root, &["init", "-q"]);
    fs::write(root.join("base.txt"), "baseline\n").unwrap();
    fs::write(root.join("remove.txt"), "remove me\n").unwrap();
    fs::write(root.join(".gitignore"), "local.txt\n").unwrap();
    commit(root);
    let mut store = WorkspaceStore::open(root).unwrap();
    let worktree = store.directory().join("worktrees/1001");
    fs::create_dir_all(worktree.parent().unwrap()).unwrap();
    let repo = Repository::open(root).unwrap();
    let baseline = repo.baseline().unwrap();
    repo.create_worktree(&worktree, "duet/task-1001").unwrap();
    store
        .begin_run("1001", "session", &worktree, "duet/task-1001", "whole task")
        .unwrap();
    let contract = |id: &str, scope: Vec<&str>, deps: Vec<&str>| NodeContract {
        id: id.into(),
        title: id.into(),
        objective: "implement".into(),
        scope: scope.into_iter().map(str::to_owned).collect(),
        boundaries: vec!["only own scope".into()],
        implementation: "modify source".into(),
        constraints: vec!["no tests".into()],
        dependencies: deps.into_iter().map(str::to_owned).collect(),
        acceptance: vec!["review diff".into()],
    };
    let mut task = TaskRecord::legacy("1001", "whole task", "session");
    task.baseline = Some(baseline.commit.clone());
    task.install_plan(TaskPlan {
        schema_version: 1,
        objective: "goal".into(),
        constraints: vec![],
        acceptance: vec!["all changes".into()],
        nodes: vec![
            contract("one", vec!["one.txt", "remove.txt"], vec![]),
            contract("two", vec!["two.txt"], vec!["one"]),
        ],
    })
    .unwrap();
    let candidate = Repository::open(&worktree).unwrap();
    fs::write(worktree.join("one.txt"), "approved one\n").unwrap();
    fs::remove_file(worktree.join("remove.txt")).unwrap();
    let first = candidate
        .capture_from(&["one.txt".into(), "remove.txt".into()], &baseline)
        .unwrap();
    fs::write(worktree.join("two.txt"), "approved two\n").unwrap();
    let second = candidate.capture_from(&["two.txt".into()], &first).unwrap();
    for (index, base, cp, count) in [
        (0, baseline.commit, first.commit, 2),
        (1, second.parent.clone().unwrap(), second.commit.clone(), 1),
    ] {
        let node = &mut task.nodes[index];
        node.begin(&base);
        node.execution = Execution::Implemented;
        let a = node.attempts.last_mut().unwrap();
        a.execution = Execution::Implemented;
        a.checkpoint = Some(cp.clone());
        a.changed_files = Some(count);
        node.approve(&cp).unwrap();
    }
    task.checkpoint = Some(second.commit);
    task.state = TaskState::Implemented;
    store.save_task(&task).unwrap();
    (dir, store, task, worktree)
}
#[test]
fn merge_uses_approved_immutable_tree_preserves_candidate_and_never_runs_hooks() {
    let (dir, mut store, mut task, candidate) = fixture();
    let repo = Repository::open(dir.path()).unwrap();
    let head = repo.baseline().unwrap().commit;
    let index = fs::read(dir.path().join(".git/index")).unwrap();
    let hook = dir.path().join(".git/hooks/post-merge");
    fs::write(
        &hook,
        format!(
            "#!/bin/sh\ntouch '{}'\n",
            dir.path().join("hook-ran").display()
        ),
    )
    .unwrap();
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o700)).unwrap();
    let preview = merge::prepare(&repo, &store, &task).unwrap();
    assert_eq!(repo.baseline().unwrap().commit, head);
    assert_eq!(fs::read(dir.path().join(".git/index")).unwrap(), index);
    assert!(dir.path().join("remove.txt").exists());
    assert!(!dir.path().join("one.txt").exists());
    assert_eq!(preview.unverified, 2);
    assert!(
        preview.diff.contains("approved one")
            && preview.diff.contains("approved two")
            && preview.diff.contains("-remove me")
    );
    fs::write(candidate.join("one.txt"), "unapproved mutable content").unwrap();
    let result = merge::apply(&repo, &mut store, &mut task, &preview).unwrap();
    assert_eq!(result, repo.baseline().unwrap().commit);
    assert_eq!(task.state, TaskState::Merged);
    assert_eq!(
        fs::read_to_string(dir.path().join("one.txt")).unwrap(),
        "approved one\n"
    );
    assert!(!dir.path().join("remove.txt").exists());
    assert_eq!(
        fs::read_to_string(candidate.join("one.txt")).unwrap(),
        "unapproved mutable content"
    );
    assert!(!dir.path().join("hook-ran").exists());
    assert!(task.startable().is_err());
    assert!(merge::prepare(&repo, &store, &task).is_err());
    assert!(store.merge_intents().unwrap().is_empty());
}
#[test]
fn merge_blocks_pending_stale_approval_failed_verification_and_dirty_main() {
    let (dir, store, task, _) = fixture();
    let repo = Repository::open(dir.path()).unwrap();
    let mut changed = task.clone();
    changed.nodes[0].reviewed_checkpoint = changed.baseline.clone();
    assert!(merge::prepare(&repo, &store, &changed).is_err());
    changed = task.clone();
    changed.nodes[0].verification = Verification::Failed;
    assert!(merge::prepare(&repo, &store, &changed).is_err());
    changed = task.clone();
    changed.nodes[1].execution = Execution::Planned;
    assert!(merge::prepare(&repo, &store, &changed).is_err());
    let head = repo.baseline().unwrap().commit;
    fs::write(dir.path().join("base.txt"), "personal edit").unwrap();
    assert!(merge::prepare(&repo, &store, &task).is_err());
    assert_eq!(
        fs::read_to_string(dir.path().join("base.txt")).unwrap(),
        "personal edit"
    );
    assert_eq!(repo.baseline().unwrap().commit, head);
}
#[test]
fn merge_preserves_new_main_commits_and_refuses_three_way_conflicts() {
    let (dir, mut store, mut task, _) = fixture();
    fs::write(dir.path().join("human.txt"), "human addition").unwrap();
    commit(dir.path());
    let repo = Repository::open(dir.path()).unwrap();
    let newer = repo.baseline().unwrap().commit;
    let preview = merge::prepare(&repo, &store, &task).unwrap();
    merge::apply(&repo, &mut store, &mut task, &preview).unwrap();
    assert_eq!(
        fs::read_to_string(dir.path().join("human.txt")).unwrap(),
        "human addition"
    );
    assert_eq!(
        repo.resolve_snapshot(&preview.commit).unwrap().parent,
        Some(newer)
    );
    let (other, store, task, _) = fixture();
    fs::write(other.path().join("one.txt"), "human add/add conflict").unwrap();
    commit(other.path());
    let repo = Repository::open(other.path()).unwrap();
    let head = repo.baseline().unwrap().commit;
    let index = fs::read(other.path().join(".git/index")).unwrap();
    let error = merge::prepare(&repo, &store, &task)
        .unwrap_err()
        .to_string();
    assert!(error.contains("冲突"));
    assert_eq!(repo.baseline().unwrap().commit, head);
    assert_eq!(fs::read(other.path().join(".git/index")).unwrap(), index);
    assert!(!other.path().join(".git/MERGE_HEAD").exists());
}
#[test]
fn confirmed_preview_rejects_changed_main_branch_head_and_task_version() {
    let (dir, mut store, mut task, _) = fixture();
    let repo = Repository::open(dir.path()).unwrap();
    let p = merge::prepare(&repo, &store, &task).unwrap();
    task.nodes[0].attempts[0].summary = "changed evidence".into();
    assert!(merge::apply(&repo, &mut store, &mut task, &p).is_err());
    task.nodes[0].attempts[0].summary.clear();
    git(dir.path(), &["checkout", "-qb", "other"]);
    assert!(merge::apply(&repo, &mut store, &mut task, &p).is_err());
    git(
        dir.path(),
        &[
            "checkout",
            "-q",
            p.branch.strip_prefix("refs/heads/").unwrap(),
        ],
    );
    fs::write(dir.path().join("new.txt"), "new head").unwrap();
    commit(dir.path());
    let head = repo.baseline().unwrap().commit;
    assert!(merge::apply(&repo, &mut store, &mut task, &p).is_err());
    assert_eq!(repo.baseline().unwrap().commit, head);
    assert!(!dir.path().join("one.txt").exists());
}
#[test]
fn ignored_personal_files_and_custom_drivers_are_not_overwritten_or_executed() {
    let (dir, mut store, mut task, candidate) = fixture();
    let repo = Repository::open(dir.path()).unwrap();
    let head = repo.baseline().unwrap().commit;
    fs::write(dir.path().join("local.txt"), "personal ignored file").unwrap();
    let base = Repository::open(&candidate)
        .unwrap()
        .resolve_snapshot(task.nodes[0].checkpoint().unwrap())
        .unwrap();
    fs::write(candidate.join(".gitignore"), "").unwrap();
    fs::write(candidate.join("local.txt"), "candidate tracked file").unwrap();
    let cp = Repository::open(&candidate)
        .unwrap()
        .capture_from(&[".gitignore".into(), "local.txt".into()], &base)
        .unwrap();
    task.nodes[1].contract.scope = vec![".gitignore".into(), "local.txt".into()];
    task.plan.as_mut().unwrap().nodes[1] = task.nodes[1].contract.clone();
    let node = &mut task.nodes[1];
    node.attempts.clear();
    node.begin(&base.commit);
    node.execution = Execution::Implemented;
    let a = node.attempts.last_mut().unwrap();
    a.execution = Execution::Implemented;
    a.checkpoint = Some(cp.commit.clone());
    a.changed_files = Some(2);
    node.approve(&cp.commit).unwrap();
    task.checkpoint = Some(cp.commit);
    store.save_task(&task).unwrap();
    let p = merge::prepare(&repo, &store, &task).unwrap();
    assert!(merge::apply(&repo, &mut store, &mut task, &p).is_err());
    assert_eq!(repo.baseline().unwrap().commit, head);
    assert_eq!(
        fs::read_to_string(dir.path().join("local.txt")).unwrap(),
        "personal ignored file"
    );
    git(
        dir.path(),
        &["config", "merge.unsafe.driver", "touch should-not-run"],
    );
    assert!(merge::prepare(&repo, &store, &task).is_err());
    assert!(!dir.path().join("should-not-run").exists());
}
#[test]
fn restart_reconciles_completed_merge_intent_but_never_applies_pending_intent() {
    let (dir, store, task, _) = fixture();
    let repo = Repository::open(dir.path()).unwrap();
    let p = merge::prepare(&repo, &store, &task).unwrap();
    store.save_merge_intent(&p).unwrap();
    repo.fast_forward_integration(&p.source_head, &p.branch, &p.commit)
        .unwrap();
    drop(store);
    let store = WorkspaceStore::open(dir.path()).unwrap();
    assert_eq!(store.tasks().unwrap()[0].state, TaskState::Merged);
    assert_eq!(
        store.tasks().unwrap()[0].merged.as_ref().unwrap().commit,
        p.commit
    );
    assert!(store.merge_intents().unwrap().is_empty());
    drop(store);
    let (other, store, task, _) = fixture();
    let repo = Repository::open(other.path()).unwrap();
    let p = merge::prepare(&repo, &store, &task).unwrap();
    store.save_merge_intent(&p).unwrap();
    drop(store);
    let store = WorkspaceStore::open(other.path()).unwrap();
    assert_eq!(repo.baseline().unwrap().commit, p.source_head);
    assert!(store.tasks().unwrap()[0].merged.is_none());
    assert!(store.merge_intents().unwrap().is_empty());
}
