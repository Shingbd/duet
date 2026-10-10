#![cfg(unix)]
use duet_workbench::{
    app::{App, RunControl},
    checkpoint::Repository,
    coding::CodingRun,
    keymap::Keymap,
    nodes::{Execution, Review, Verification},
    storage::WorkspaceStore,
};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    process::Command,
    time::{Duration, Instant},
};
fn setup(count: usize, mode: &str) -> (tempfile::TempDir, std::path::PathBuf, WorkspaceStore, App) {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("base.txt"), "baseline\n").unwrap();
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
            "base",
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
    let fake = dir.path().join(".duet-fake-pi");
    let script = format!(
        r#"#!/usr/bin/env python3
import json,os,pathlib,sys
count={count}
mode={mode:?}
code_turn=0
planner=sys.argv[sys.argv.index('--tools')+1]=='read,grep,find,ls'
def emit(r): print(json.dumps(r),flush=True)
for line in sys.stdin:
 r=json.loads(line); response={{'type':'response','id':r['id'],'success':True}}
 if r['type']=='get_state':
  response['data']={{'sessionId':'session-'+str(os.getpid()),'model':{{'id':'fake'}},'isStreaming':False,'isCompacting':False,'pendingMessageCount':0}}; emit(response)
 elif r['type']=='prompt':
  response['data']={{'disposition':'started'}}; emit(response); emit({{'type':'agent_start'}})
  if planner:
   nodes=[{{'id':'n'+str(i),'title':'node '+str(i),'objective':'implement '+str(i),'scope':['file'+str(i)+'.txt'],'boundaries':['only implement this node; preserve other behavior'],'implementation':'write the requested file','constraints':['no tests or merge'],'dependencies':[] if i==1 else ['n'+str(i-1)],'acceptance':['file content is correct']}} for i in range(1,count+1)]
   output=json.dumps({{'schema_version':1,'objective':'goal','constraints':['keep baseline'],'acceptance':['all files correct'],'nodes':nodes}}) if mode!='invalid' else 'not json'
  else:
   node=json.loads(r['message'].split('\n当前节点契约：')[1].split('\n明确补充意见：')[0])
   code_turn+=1
   if mode!='nochange' and (mode!='nochange_once' or code_turn>1): pathlib.Path(node['scope'][0]).write_text(node['id']+'\n')
   if mode=='violate': pathlib.Path('outside.txt').write_text('outside')
   output='implemented '+node['id']+'; tests not run'
  emit({{'type':'message_end','message':{{'role':'assistant','content':[{{'type':'text','text':output}}]}}}})
  emit({{'type':'agent_settled','aborted':False}})
 else: emit(response)
"#
    );
    fs::write(&fake, script).unwrap();
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o700)).unwrap();
    // The fake executable is test infrastructure, not an authorized source change.
    fs::write(dir.path().join(".git/info/exclude"), ".duet-fake-pi\n").unwrap();
    let repo = Repository::open(dir.path()).unwrap();
    let app = App::browse(&repo, repo.baseline().unwrap(), Keymap::default()).unwrap();
    let store = WorkspaceStore::open(dir.path()).unwrap();
    (dir, fake, store, app)
}
fn poll(run: &mut CodingRun, app: &mut App, store: &mut WorkspaceStore) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while !run.is_finished() && !run.is_waiting() {
        assert!(
            Instant::now() < deadline,
            "{} {:?}",
            app.status,
            app.run.status
        );
        run.poll(app, store).unwrap();
        std::thread::sleep(Duration::from_millis(5));
    }
}
#[test]
fn readonly_planning_runs_nodes_with_separate_immutable_diffs_and_restores_history() {
    let (dir, fake, mut store, mut app) = setup(2, "normal");
    let mut run = CodingRun::start_planning(
        dir.path(),
        &fake,
        None,
        "whole requirement",
        Duration::from_secs(15),
        &mut store,
        &mut app,
    )
    .unwrap();
    poll(&mut run, &mut app, &mut store);
    assert!(run.is_finished(), "{}", app.status);
    let task = app.tasks.last().unwrap();
    assert_eq!(task.nodes.len(), 2);
    for node in &task.nodes {
        assert_eq!(node.execution, Execution::Implemented);
        assert_eq!(node.verification, Verification::Unverified);
        assert_eq!(node.review, Review::Pending);
    }
    let repo = Repository::open(&run.worktree).unwrap();
    let first = repo
        .resolve_snapshot(task.nodes[0].checkpoint().unwrap())
        .unwrap();
    let second = repo
        .resolve_snapshot(task.nodes[1].checkpoint().unwrap())
        .unwrap();
    assert_eq!(second.parent.as_deref(), Some(first.commit.as_str()));
    assert_eq!(repo.snapshot_changes(&first).unwrap()[0].path, "file1.txt");
    let changes = repo.snapshot_changes(&second).unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].path, "file2.txt");
    assert!(repo.read(&second, "file1.txt").unwrap().contains("n1"));
    assert!(!dir.path().join("file1.txt").exists());
    let id = task.id.clone();
    run.stop(&mut app, &mut store).unwrap();
    store.save(&app).unwrap();
    drop(run);
    drop(store);
    let store = WorkspaceStore::open(dir.path()).unwrap();
    let mut restored = App::empty(Keymap::default());
    store.restore(&mut restored).unwrap();
    assert_eq!(restored.tasks[0].id, id);
    assert_eq!(restored.tasks[0].nodes.len(), 2);
    assert_eq!(restored.history[0].content, "whole requirement");
    assert!(
        restored
            .items
            .iter()
            .any(|i| i.version == first.commit && i.title == "file1.txt")
    );
    assert!(
        restored
            .items
            .iter()
            .any(|i| i.version == second.commit && i.title == "file2.txt")
    );
}
#[test]
fn real_node_backpressure_requires_explicit_approval_and_resume() {
    let (dir, fake, mut store, mut app) = setup(3, "normal");
    let mut run = CodingRun::start_planning(
        dir.path(),
        &fake,
        None,
        "three nodes",
        Duration::from_secs(15),
        &mut store,
        &mut app,
    )
    .unwrap();
    poll(&mut run, &mut app, &mut store);
    assert!(run.is_waiting());
    assert_eq!(app.tasks[0].nodes[2].execution, Execution::Planned);
    assert!(!run.worktree.join("file3.txt").exists());
    assert!(run.resume(&mut app, &mut store).is_err());
    let node = &mut app.tasks[0].nodes[0];
    let checkpoint = node.checkpoint().unwrap().to_owned();
    node.approve(&checkpoint).unwrap();
    assert!(!run.worktree.join("file3.txt").exists());
    run.resume(&mut app, &mut store).unwrap();
    poll(&mut run, &mut app, &mut store);
    assert!(run.is_finished());
    assert_eq!(app.tasks[0].nodes[2].execution, Execution::Implemented);
}
#[test]
fn invalid_plan_and_scope_violations_stop_without_starting_later_nodes() {
    for mode in ["invalid", "violate"] {
        let (dir, fake, mut store, mut app) = setup(2, mode);
        let mut run = CodingRun::start_planning(
            dir.path(),
            &fake,
            None,
            "requirement",
            Duration::from_secs(15),
            &mut store,
            &mut app,
        )
        .unwrap();
        poll(&mut run, &mut app, &mut store);
        assert!(run.is_finished());
        assert_eq!(app.run.status, duet_workbench::rpc::RunStatus::Failed);
        assert!(!run.worktree.join("file2.txt").exists());
        if mode == "invalid" {
            assert!(!run.worktree.join("file1.txt").exists());
        } else {
            assert_eq!(
                app.tasks[0].nodes[0].attempts[0].scope_violations,
                vec!["outside.txt"]
            );
        }
    }
}
#[test]
fn cancelling_at_node_boundary_does_not_resume_pending_nodes() {
    let (dir, fake, mut store, mut app) = setup(3, "normal");
    let mut run = CodingRun::start_planning(
        dir.path(),
        &fake,
        None,
        "three nodes",
        Duration::from_secs(15),
        &mut store,
        &mut app,
    )
    .unwrap();
    poll(&mut run, &mut app, &mut store);
    run.cancel(&mut app).unwrap();
    run.poll(&mut app, &mut store).unwrap();
    assert!(run.is_finished());
    assert_eq!(app.tasks[0].state, TaskState::Cancelled);
    assert!(!run.worktree.join("file3.txt").exists());
    assert!(
        run.interact(RunControl::Resume, &mut app, &mut store)
            .is_err()
    );
}
#[test]
fn task_sidebar_contains_nodes_not_files_and_review_is_node_scoped() {
    use duet_workbench::{
        app::{Panel, TaskSelection},
        keymap::Action,
        ui,
    };
    use ratatui::{Terminal, backend::TestBackend};
    let (dir, fake, mut store, mut app) = setup(2, "normal");
    let mut run = CodingRun::start_planning(
        dir.path(),
        &fake,
        None,
        "whole requirement",
        Duration::from_secs(15),
        &mut store,
        &mut app,
    )
    .unwrap();
    poll(&mut run, &mut app, &mut store);
    let id = run.id.clone();
    app.act(Action::Tasks);
    app.panel = Panel::Queue;
    for (width, height) in [(64, 20), (120, 32)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|f| ui::draw(f, &app)).unwrap();
        let buffer = terminal.backend().buffer();
        let sidebar = (4..height - 2)
            .map(|y| (0..25).map(|x| buffer[(x, y)].symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(sidebar.contains("node 1"));
        assert!(sidebar.contains("node 2"));
        assert!(!sidebar.contains("base.txt"));
        assert!(!sidebar.contains("file1.txt"));
    }
    app.panel = Panel::Queue;
    app.act(Action::Down);
    assert_eq!(app.selected_node().unwrap().contract.id, "n1");
    assert!(app.body().contains("实现方法"));
    app.act(Action::Review);
    assert_eq!(
        app.changed_indices()
            .iter()
            .map(|&i| app.items[i].title.as_str())
            .collect::<Vec<_>>(),
        vec!["file1.txt"]
    );
    let pinned = app.current().unwrap().version.clone();
    app.act(Action::NextNode);
    assert_eq!(app.selected_node().unwrap().contract.id, "n2");
    assert!(app.body().contains("+n2"));
    assert!(!app.body().contains("file1.txt"));
    app.act(Action::PreviousNode);
    assert_eq!(app.current().unwrap().version, pinned);
    app.act(Action::Project);
    assert_eq!(
        app.visible_file_indices()
            .iter()
            .map(|&i| app.items[i].title.as_str())
            .collect::<Vec<_>>(),
        vec!["base.txt"]
    );
    assert_eq!(app.current().unwrap().code, "baseline\n");
    app.select_task_entry(TaskSelection {
        task_id: id.clone(),
        node_id: Some("n1".into()),
    });
    app.act(Action::Review);
    app.current().unwrap();
    store.save(&app).unwrap();
    drop(run);
    drop(store);
    let store = WorkspaceStore::open(dir.path()).unwrap();
    let mut restored = App::empty(Keymap::default());
    store.restore(&mut restored).unwrap();
    assert_eq!(
        restored.task_selection.clone().unwrap(),
        TaskSelection {
            task_id: id,
            node_id: Some("n1".into())
        }
    );
    assert_eq!(restored.current().unwrap().version, pinned);
}

#[test]
fn explicit_node_approval_is_confirmed_persisted_and_does_not_verify_or_merge() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use duet_workbench::{app::Mode, keymap::Action};
    let (dir, fake, mut store, mut app) = setup(2, "normal");
    let mut run = CodingRun::start_planning(
        dir.path(),
        &fake,
        None,
        "whole requirement",
        Duration::from_secs(15),
        &mut store,
        &mut app,
    )
    .unwrap();
    poll(&mut run, &mut app, &mut store);
    app.act(Action::Review);
    app.act(Action::MarkRead);
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.selected_node().unwrap().review, Review::Pending);
    let command = |app: &mut App| {
        app.act(Action::Palette);
        for c in "approve-node".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    };
    command(&mut app);
    assert!(matches!(app.mode, Mode::ConfirmNodeApproval { .. }));
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.selected_node().unwrap().review, Review::Pending);
    app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
    assert!(matches!(app.mode, Mode::ConfirmNodeApproval { .. }));
    assert_eq!(app.selected_node().unwrap().review, Review::Pending);
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.selected_node().unwrap().review, Review::Approved);
    assert_eq!(
        app.selected_node().unwrap().verification,
        Verification::Unverified
    );
    assert!(app.node_evidence().unwrap().contains("已批准"));
    store.save(&app).unwrap();
    assert_eq!(store.tasks().unwrap()[0].nodes[0].review, Review::Approved);
    assert!(!dir.path().join("file1.txt").exists());
}

#[test]
fn node_rework_keeps_old_checkpoint_approval_and_pinned_reading_version() {
    use duet_workbench::{
        app::{Panel, TaskSelection},
        keymap::Action,
        nodes::Verification,
    };
    let (dir, fake, mut store, mut app) = setup(2, "normal");
    let mut run = CodingRun::start_planning(
        dir.path(),
        &fake,
        None,
        "whole requirement",
        Duration::from_secs(15),
        &mut store,
        &mut app,
    )
    .unwrap();
    poll(&mut run, &mut app, &mut store);
    app.select_task_entry(TaskSelection {
        task_id: run.id.clone(),
        node_id: Some("n2".into()),
    });
    app.act(Action::Review);
    let old = app.current().unwrap().version.clone();
    app.tasks[0].nodes[1].approve(&old).unwrap();
    app.current().unwrap();
    app.items[app.selected].scroll = 2;
    app.panel = Panel::Reader;
    run.interact(
        RunControl::Say("refine implementation".into()),
        &mut app,
        &mut store,
    )
    .unwrap();
    poll(&mut run, &mut app, &mut store);
    assert_eq!(app.current().unwrap().version, old);
    assert_eq!(app.current().unwrap().scroll, 2);
    let node = app.selected_node().unwrap();
    assert_eq!(node.attempts.len(), 2);
    assert_eq!(node.attempts[0].review, Review::Approved);
    assert_eq!(node.review, Review::Pending);
    assert_eq!(node.verification, Verification::Unverified);
    assert_ne!(node.checkpoint(), Some(old.as_str()));
    assert!(app.node_evidence().unwrap().contains("已批准"));
    app.select_task_entry(TaskSelection {
        task_id: run.id.clone(),
        node_id: Some("n2".into()),
    });
    assert!(app.body().contains("+n2"));
    assert!(!app.body().contains("file1.txt"));
    assert!(app.node_evidence().unwrap().contains("待审"));
}

#[test]
fn zero_file_delivery_is_not_implementation_and_requires_correction_before_continuing() {
    use duet_workbench::{app::Mode, keymap::Action};
    let (dir, fake, mut store, mut app) = setup(2, "nochange_once");
    let mut run = CodingRun::start_planning(
        dir.path(),
        &fake,
        None,
        "implement files",
        Duration::from_secs(15),
        &mut store,
        &mut app,
    )
    .unwrap();
    poll(&mut run, &mut app, &mut store);
    assert!(run.is_waiting());
    assert_eq!(app.tasks[0].nodes[0].execution, Execution::NoChanges);
    assert_eq!(app.tasks[0].nodes[0].attempts[0].changed_files, Some(0));
    assert_eq!(app.tasks[0].nodes[1].execution, Execution::Planned);
    app.act(Action::Review);
    assert!(app.body().contains("未交付"));
    app.act(Action::ApproveNode);
    assert_eq!(app.mode, Mode::Browse);
    assert!(app.status.contains("未交付"));
    assert!(run.resume(&mut app, &mut store).is_err());
    run.interact(
        RunControl::Say("请实际修改文件，不要只给方案".into()),
        &mut app,
        &mut store,
    )
    .unwrap();
    run.resume(&mut app, &mut store).unwrap();
    poll(&mut run, &mut app, &mut store);
    assert!(run.is_finished());
    assert_eq!(app.tasks[0].nodes[0].execution, Execution::Implemented);
    assert_eq!(
        app.tasks[0].nodes[0].attempts[0].execution,
        Execution::NoChanges
    );
    assert_eq!(app.tasks[0].nodes[0].attempts.len(), 2);
    assert_eq!(app.tasks[0].nodes[1].execution, Execution::Implemented);
}

#[test]
fn all_approved_nodes_still_have_browsable_scoped_diffs() {
    use duet_workbench::{app::TaskSelection, keymap::Action};
    let (dir, fake, mut store, mut app) = setup(2, "normal");
    let mut run = CodingRun::start_planning(
        dir.path(),
        &fake,
        None,
        "implement files",
        Duration::from_secs(15),
        &mut store,
        &mut app,
    )
    .unwrap();
    poll(&mut run, &mut app, &mut store);
    for node in &mut app.tasks[0].nodes {
        let cp = node.checkpoint().unwrap().to_owned();
        node.approve(&cp).unwrap();
    }
    app.select_task_entry(TaskSelection {
        task_id: run.id.clone(),
        node_id: None,
    });
    app.act(Action::Review);
    assert!(app.body().contains("+n1"));
    app.act(Action::NextNode);
    assert!(app.body().contains("+n2"));
    app.act(Action::PreviousNode);
    assert!(app.body().contains("+n1"));
}

#[test]
fn schema_four_repairs_legacy_false_completion_using_immutable_checkpoint() {
    let (dir, fake, mut store, mut app) = setup(2, "nochange");
    let mut run = CodingRun::start_planning(
        dir.path(),
        &fake,
        None,
        "implement files",
        Duration::from_secs(15),
        &mut store,
        &mut app,
    )
    .unwrap();
    poll(&mut run, &mut app, &mut store);
    run.stop(&mut app, &mut store).unwrap();
    let node = &mut app.tasks[0].nodes[0];
    let cp = node.checkpoint().unwrap().to_owned();
    node.execution = Execution::Implemented;
    node.attempts[0].execution = Execution::Implemented;
    node.attempts[0].changed_files = None;
    store.save_task(&app.tasks[0]).unwrap();
    drop(run);
    drop(store);
    let db = rusqlite::Connection::open(dir.path().join(".duet/workspace.db")).unwrap();
    db.pragma_update(None, "user_version", 3).unwrap();
    drop(db);
    let store = WorkspaceStore::open(dir.path()).unwrap();
    let mut tasks = store.tasks().unwrap();
    let task = tasks.remove(0);
    assert_eq!(task.nodes[0].execution, Execution::NoChanges);
    assert_eq!(task.nodes[0].attempts[0].execution, Execution::NoChanges);
    assert_eq!(task.nodes[0].checkpoint(), Some(cp.as_str()));
    assert_eq!(task.nodes[0].review, Review::Pending);
    assert_eq!(task.nodes[1].execution, Execution::Planned);
}

use duet_workbench::nodes::TaskState;
