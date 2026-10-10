#[cfg(unix)]
#[test]
fn coding_loop_preserves_original_and_persists_conversation_and_checkpoint() {
    use duet_workbench::{
        app::App, checkpoint::Repository, coding::CodingRun, keymap::Keymap,
        storage::WorkspaceStore,
    };
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        process::Command,
        time::{Duration, Instant},
    };
    let dir = tempfile::tempdir().unwrap();
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&["init", "-q"]);
    git(&["config", "user.name", "Test"]);
    git(&["config", "user.email", "test@localhost"]);
    fs::write(dir.path().join("main.rs"), "baseline").unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "baseline"]);
    fs::write(dir.path().join("main.rs"), "user dirty").unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let executable = fixture.path().join("fake-pi");
    fs::write(&executable, r#"#!/usr/bin/env python3
import sys,json,pathlib
for line in sys.stdin.buffer:
    request=json.loads(line)
    def emit(value):
        print(json.dumps(value),flush=True)
    if request['type']=='get_state':
        emit({'type':'response','id':request['id'],'success':True,'data':{'sessionId':'test-session','model':{'id':'fake'}}})
    elif request['type']=='prompt':
        emit({'type':'response','id':request['id'],'success':True,'data':{'disposition':'started'}})
        emit({'type':'agent_start'})
        emit({'type':'message_end','message':{'role':'user','content':request['message']}})
        pathlib.Path('main.rs').write_text('agent changed')
        emit({'type':'message_end','message':{'role':'assistant','content':[{'type':'text','text':'changed main.rs'}],'stopReason':'stop'}})
        emit({'type':'agent_end'})
        emit({'type':'agent_settled','aborted':False})
"#).unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    let repo = Repository::open(dir.path()).unwrap();
    let mut store = WorkspaceStore::open(dir.path()).unwrap();
    let mut app = App::browse(&repo, repo.baseline().unwrap(), Keymap::default()).unwrap();
    let mut run = CodingRun::start(
        dir.path(),
        &executable,
        None,
        "change main.rs",
        Duration::from_secs(5),
        &mut store,
        &mut app,
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !run.is_finished() {
        run.poll(&mut app, &mut store).unwrap();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        fs::read_to_string(dir.path().join("main.rs")).unwrap(),
        "user dirty"
    );
    assert_eq!(
        fs::read_to_string(run.worktree.join("main.rs")).unwrap(),
        "agent changed"
    );
    assert!(run.checkpoint.is_some());
    assert_eq!(app.selected, 0);
    assert!(
        app.items
            .iter()
            .find(|item| item.change.is_some())
            .unwrap()
            .diff
            .contains("+agent changed")
    );
    assert!(app.items.iter().any(|item| item.title == "main.rs"
        && item.change.is_some()
        && item.code == "agent changed"));
    assert_eq!(store.history(&run.session).unwrap().len(), 2);
    store.save(&app).unwrap();
    drop(run);
    drop(store);
    let store = WorkspaceStore::open(dir.path()).unwrap();
    let mut restored = App::empty(Keymap::default());
    store.restore(&mut restored).unwrap();
    assert_eq!(restored.task.as_deref(), Some("change main.rs"));
    assert_eq!(restored.history.len(), 2);
}
