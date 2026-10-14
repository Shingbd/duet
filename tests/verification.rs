#![cfg(unix)]
use duet_workbench::{
    app::{App, Mode, TaskSelection},
    checkpoint::Repository,
    keymap::Keymap,
    nodes::*,
    storage::WorkspaceStore,
    verification::{self, Outcome, Run},
};
use std::{
    fs,
    path::Path,
    process::Command,
    time::{Duration, Instant},
};
fn git(root: &Path, args: &[&str]) -> Vec<u8> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}
fn fixture() -> (tempfile::TempDir, App, WorkspaceStore) {
    let t = tempfile::tempdir().unwrap();
    let root = t.path();
    git(root, &["init", "-q"]);
    fs::write(root.join("main.txt"), "baseline\n").unwrap();
    git(root, &["add", "."]);
    git(
        root,
        &[
            "-c",
            "user.name=test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "-qm",
            "base",
        ],
    );
    let repo = Repository::open(root).unwrap();
    let base = repo.baseline().unwrap();
    fs::write(root.join("main.txt"), "delivery\n").unwrap();
    let snap = repo.capture_from(&["main.txt".into()], &base).unwrap();
    fs::write(root.join("main.txt"), "human dirty\n").unwrap();
    let mut app = App::empty(Keymap::default());
    let mut task = TaskRecord::legacy("task", "goal", "session");
    task.install_plan(TaskPlan {
        schema_version: 1,
        objective: "goal".into(),
        constraints: vec![],
        acceptance: vec!["overall".into()],
        nodes: vec![NodeContract {
            id: "node".into(),
            title: "node".into(),
            objective: "goal".into(),
            scope: vec!["main.txt".into()],
            boundaries: vec!["keep main".into()],
            implementation: "method".into(),
            constraints: vec![],
            dependencies: vec![],
            acceptance: vec!["first".into(), "second".into()],
        }],
    })
    .unwrap();
    task.nodes[0].begin(&base.commit);
    let node = &mut task.nodes[0];
    node.execution = Execution::Implemented;
    node.attempts[0].execution = Execution::Implemented;
    node.attempts[0].checkpoint = Some(snap.commit);
    task.state = TaskState::Implemented;
    app.tasks.push(task);
    app.task_selection = Some(TaskSelection {
        task_id: "task".into(),
        node_id: Some("node".into()),
    });
    let store = WorkspaceStore::open(root).unwrap();
    (t, app, store)
}
fn start(
    app: &mut App,
    store: &mut WorkspaceStore,
    argv: Vec<String>,
    criteria: Vec<usize>,
    timeout: u64,
) -> Run {
    let spec = serde_json::json!({"argv":argv,"criteria":criteria,"timeout_secs":timeout});
    let req = verification::request(app, &spec.to_string()).unwrap();
    Run::start(req, app, store).unwrap()
}
fn finish(run: &mut Run, app: &mut App, store: &mut WorkspaceStore) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while !run.finished() {
        run.poll(app, store).unwrap();
        assert!(Instant::now() < deadline, "runner stuck");
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn py(code: &str) -> Vec<String> {
    vec!["python3".into(), "-c".into(), code.into()]
}
#[test]
fn immutable_delivery_dirty_main_and_approval_are_independent() {
    let (t, mut app, mut store) = fixture();
    let head = git(t.path(), &["rev-parse", "HEAD"]);
    let index = fs::read(t.path().join(".git/index")).unwrap();
    let cp = app.tasks[0].nodes[0].checkpoint().unwrap().to_owned();
    let mut r = start(
        &mut app,
        &mut store,
        py(
            "from pathlib import Path; assert Path('main.txt').read_text()=='delivery\\n'; print('actual fixed input')",
        ),
        vec![1, 2],
        3,
    );
    finish(&mut r, &mut app, &mut store);
    assert_eq!(r.record.outcome, Outcome::Passed);
    assert_eq!(app.tasks[0].nodes[0].verification, Verification::Passed);
    assert_eq!(app.tasks[0].nodes[0].review, Review::Pending);
    assert_eq!(app.tasks[0].nodes[0].checkpoint(), Some(cp.as_str()));
    assert_eq!(
        fs::read_to_string(t.path().join("main.txt")).unwrap(),
        "human dirty\n"
    );
    assert_eq!(git(t.path(), &["rev-parse", "HEAD"]), head);
    assert_eq!(fs::read(t.path().join(".git/index")).unwrap(), index);
    assert!(r.record.output.contains("actual fixed input"));
    assert!(r.record.directory.join("record.json").exists());
    let mut restored = App::empty(Keymap::default());
    store.restore(&mut restored).unwrap();
    assert_eq!(restored.tasks[0].nodes[0].verification_runs[0], r.record);
}
#[test]
fn unmapped_partial_and_failed_general_checks_do_not_fake_complete_acceptance() {
    let (_temp, mut a, mut s) = fixture();
    let mut r = start(&mut a, &mut s, py("print('general')"), vec![], 3);
    finish(&mut r, &mut a, &mut s);
    assert_eq!(a.tasks[0].nodes[0].verification, Verification::Unverified);
    for i in [1, 2] {
        let mut r = start(&mut a, &mut s, py("print('specific')"), vec![i], 3);
        finish(&mut r, &mut a, &mut s);
        assert_eq!(
            a.tasks[0].nodes[0].verification,
            if i == 1 {
                Verification::Unverified
            } else {
                Verification::Passed
            }
        );
    }
    let mut r = start(&mut a, &mut s, py("raise SystemExit(2)"), vec![], 3);
    finish(&mut r, &mut a, &mut s);
    assert_eq!(a.tasks[0].nodes[0].verification, Verification::Failed);
    let mut r = start(&mut a, &mut s, py("print('different test')"), vec![1], 3);
    finish(&mut r, &mut a, &mut s);
    assert_eq!(
        a.tasks[0].nodes[0].verification,
        Verification::Failed,
        "different mapped command cannot clear failed general check"
    );
}
#[test]
fn source_changes_even_hidden_index_flags_invalidate_success() {
    for code in [
        "from pathlib import Path;Path('main.txt').write_text('modified')",
        "import subprocess;subprocess.run(['git','update-index','--assume-unchanged','main.txt'],check=True)",
    ] {
        let (_temp, mut a, mut s) = fixture();
        let mut r = start(&mut a, &mut s, py(code), vec![1, 2], 3);
        finish(&mut r, &mut a, &mut s);
        assert_eq!(r.record.outcome, Outcome::SourceChanged);
        assert_eq!(a.tasks[0].nodes[0].verification, Verification::Failed);
    }
}
#[test]
fn cancel_timeout_spawn_error_and_process_group_cleanup() {
    let (t, mut a, mut s) = fixture();
    let marker = t.path().join("descendant-marker");
    let child = format!(
        "import time;from pathlib import Path;time.sleep(2);Path({:?}).write_text('bad')",
        marker.to_string_lossy()
    );
    let code = format!(
        "import subprocess,time;subprocess.Popen(['python3','-c',{}]);time.sleep(10)",
        serde_json::to_string(&child).unwrap()
    );
    let mut r = start(&mut a, &mut s, py(&code), vec![1], 1);
    finish(&mut r, &mut a, &mut s);
    assert_eq!(r.record.outcome, Outcome::TimedOut);
    std::thread::sleep(Duration::from_millis(1300));
    assert!(!marker.exists());
    let mut r = start(&mut a, &mut s, py("import time;time.sleep(10)"), vec![1], 3);
    r.cancel();
    finish(&mut r, &mut a, &mut s);
    assert_eq!(r.record.outcome, Outcome::Cancelled);
    assert_eq!(
        a.tasks[0].nodes[0].verification,
        Verification::Failed,
        "cancellation cannot erase earlier failed evidence"
    );
    let r = start(
        &mut a,
        &mut s,
        vec!["/nonexistent/duet-test-program".into()],
        vec![1],
        3,
    );
    assert_eq!(r.record.outcome, Outcome::Error);
}
#[test]
fn old_version_and_contract_results_never_update_new_version() {
    let (t, mut a, mut s) = fixture();
    let mut r = start(
        &mut a,
        &mut s,
        py("import time;time.sleep(.1)"),
        vec![1, 2],
        3,
    );
    let repo = Repository::open(t.path()).unwrap();
    let snap = repo.capture(&["main.txt".into()]).unwrap();
    a.tasks[0].nodes[0].attempts[0].checkpoint = Some(snap.commit);
    finish(&mut r, &mut a, &mut s);
    assert_eq!(r.record.outcome, Outcome::Passed);
    assert_eq!(a.tasks[0].nodes[0].verification, Verification::Unverified);
    assert_eq!(a.tasks[0].nodes[0].verification_runs.len(), 1);
    let req = verification::request(&a, &serde_json::json!({"argv":["true"]}).to_string()).unwrap();
    a.tasks[0].nodes[0].contract.acceptance.push("new".into());
    assert!(Run::start(req, &mut a, &mut s).is_err());
}
#[test]
fn restart_marks_running_interrupted_without_execution() {
    let (_temp, mut a, mut s) = fixture();
    let mut r = start(
        &mut a,
        &mut s,
        py("import time;time.sleep(10)"),
        vec![1, 2],
        30,
    );
    let running = r.record.clone();
    r.stop(&mut a, &mut s).unwrap();
    a.tasks[0].nodes[0].verification_runs[0] = running;
    a.tasks_revision += 1;
    s.save(&a).unwrap();
    let mut b = App::empty(Keymap::default());
    s.restore(&mut b).unwrap();
    assert_eq!(
        b.tasks[0].nodes[0].verification_runs[0].outcome,
        Outcome::Interrupted
    );
    assert_eq!(b.tasks[0].nodes[0].verification, Verification::Unverified);
}
#[test]
fn authorization_cancel_stale_criteria_and_literal_argv() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let (t, mut a, mut s) = fixture();
    let marker = t.path().join("marker");
    let argv = vec![
        "python3".into(),
        "-c".into(),
        "import sys;print(sys.argv[1])".into(),
        format!("%sh{{touch {}}}", marker.display()),
    ];
    let req = verification::request(&a, &serde_json::json!({"argv":argv}).to_string()).unwrap();
    a.mode = Mode::ConfirmVerification {
        request: Box::new(req.clone()),
        scroll: 0,
    };
    a.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(a.requested_verification.is_none());
    assert!(!marker.exists());
    let mut r = Run::start(req, &mut a, &mut s).unwrap();
    finish(&mut r, &mut a, &mut s);
    assert!(!marker.exists());
    for text in [
        "{\"argv\":[]}",
        "{\"argv\":[\"true\"],\"criteria\":[0]}",
        "{\"argv\":[\"true\"],\"criteria\":[1,1]}",
        "{\"argv\":[\"true\"],\"timeout_secs\":0}",
        "{\"argv\":[\"--api-key=credential\"]}",
    ] {
        assert!(verification::request(&a, text).is_err());
    }
}
#[test]
fn bounded_redacted_logs_and_no_credential_environment() {
    let (_temp, mut a, mut s) = fixture();
    let code = "import os;assert not any(k in os.environ for k in ['OPENAI_API_KEY','ANTHROPIC_API_KEY','GITHUB_TOKEN']);print('password: hidden-value');print('safe-line');print(('x'*1000+'\\n')*300)";
    let mut r = start(&mut a, &mut s, py(code), vec![], 3);
    finish(&mut r, &mut a, &mut s);
    assert_eq!(r.record.outcome, Outcome::Passed);
    assert!(!r.record.output.contains("hidden-value"));
    assert!(r.record.output.contains("safe-line"));
    assert!(r.record.truncated);
    assert!(r.record.output.len() <= 128 * 1024);
}
