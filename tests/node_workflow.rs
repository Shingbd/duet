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
    if mode == "live" {
        fs::write(dir.path().join("file1.txt"), "old baseline\n").unwrap();
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
import json,os,pathlib,sys,time
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
  with pathlib.Path(__file__).parent.joinpath('.duet/request-log').open('a') as log: log.write(json.dumps(r)+'\n')
  response['data']={{'disposition':'started'}}; emit(response); emit({{'type':'agent_start'}})
  if planner:
   if mode=='quota' and not pathlib.Path('quota-marker').exists():
    pathlib.Path('quota-marker').write_text('failed')
    emit({{'type':'message_end','message':{{'role':'assistant','content':[],'stopReason':'error','errorMessage':'Codex error: The usage limit has been reached'}}}})
    emit({{'type':'agent_settled','aborted':False}});continue
   nodes=[{{'id':'n'+str(i),'title':'node '+str(i),'objective':'implement '+str(i),'scope':['file'+str(i)+'.txt'],'boundaries':['only implement this node; preserve other behavior'],'implementation':'write the requested file','constraints':['no tests or merge'],'dependencies':[] if i==1 else ['n'+str(i-1)],'acceptance':['file content is correct']}} for i in range(1,count+1)]
   output=json.dumps({{'schema_version':1,'objective':'goal','constraints':['keep baseline'],'acceptance':['all files correct'],'nodes':nodes}}) if mode!='invalid' else 'not json'
   if mode=='clarify' and 'Rust' not in r['message']: output=json.dumps({{'reply':'请明确语言和架构边界。','plan':None}})
  else:
   node=json.loads(r['message'].split('\n当前节点契约：')[1].split('\n明确补充意见：')[0])
   code_turn+=1
   if mode not in ('nochange','timeout_resume') and (mode!='nochange_once' or code_turn>1): pathlib.Path(node['scope'][0]).write_text(node['id']+'\n')
   if mode=='timeout_resume':
    path=pathlib.Path(node['scope'][0])
    if '续写交接：' in r['message']:
     assert path.read_text()==node['id']+' partial\n','host must preserve unfinished file before prompt'
     path.write_text(path.read_text()+'resumed\n')
    else:
     path.write_text(node['id']+' partial\n');emit({{'type':'tool_execution_end','toolName':'write'}});time.sleep(.45)
   if mode=='violate': pathlib.Path('outside.txt').write_text('outside')
   if mode=='live':
    pathlib.Path(node['scope'][0]).write_text(node['id']+' first')
    emit({{'type':'tool_execution_end','toolName':'write'}});time.sleep(1.15)
    pathlib.Path(node['scope'][0]).write_text(node['id']+' second')
    emit({{'type':'tool_execution_end','toolName':'edit'}});time.sleep(1.15)
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
    // Cancellation stops execution, not the contract. Backpressure still applies.
    assert!(app.tasks[0].startable().is_err());
    for node in &mut app.tasks[0].nodes[..2] {
        let cp = node.checkpoint().unwrap().to_owned();
        node.approve(&cp).unwrap();
    }
    assert!(app.tasks[0].startable().is_ok());
    assert_eq!(app.tasks[0].resume_index().unwrap(), 2);
    let prior_attempts = app.tasks[0].nodes[..2]
        .iter()
        .map(|n| n.attempts.len())
        .collect::<Vec<_>>();
    app.act(duet_workbench::keymap::Action::Tasks);
    app.act(duet_workbench::keymap::Action::StartTask);
    assert!(matches!(
        app.mode,
        duet_workbench::app::Mode::ConfirmStart { .. }
    ));
    assert!(app.requested_start.is_none());
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 32)).unwrap();
    terminal
        .draw(|f| duet_workbench::ui::draw(f, &app))
        .unwrap();
    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect::<String>();
    let rendered = rendered.replace(' ', "");
    assert!(rendered.contains("▶node3"));
    assert!(rendered.contains("✓node1"));
    assert!(rendered.contains("✓node2"));
    assert!(!rendered.contains("已实现·跳过"));
    assert!(!rendered.contains("后续顺序执行"));
    assert!(!run.worktree.join("file3.txt").exists());
    app.handle_key(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Enter,
        crossterm::event::KeyModifiers::NONE,
    ));
    assert_eq!(app.requested_start.take().unwrap().0, run.id);
    let id = run.id.clone();
    run.stop(&mut app, &mut store).unwrap();
    drop(run);
    let mut continued = CodingRun::open_existing(
        &id,
        &fake,
        None,
        false,
        Duration::from_secs(15),
        &mut store,
        &mut app,
    )
    .unwrap();
    poll(&mut continued, &mut app, &mut store);
    assert_eq!(app.tasks.len(), 1);
    assert_eq!(app.tasks[0].state, TaskState::Implemented);
    assert_eq!(app.tasks[0].nodes[0].review, Review::Approved);
    assert_eq!(app.tasks[0].nodes[1].review, Review::Approved);
    assert_eq!(app.tasks[0].nodes[2].execution, Execution::Implemented);
    assert_eq!(
        app.tasks[0].nodes[..2]
            .iter()
            .map(|n| n.attempts.len())
            .collect::<Vec<_>>(),
        prior_attempts
    );
    continued.stop(&mut app, &mut store).unwrap();
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
fn node_rework_keeps_old_checkpoint_approval_and_automatically_follows_current_node() {
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
    assert_ne!(app.current().unwrap().version, old);
    assert_eq!(app.current().unwrap().scroll, 2);
    let node = app.selected_node().unwrap();
    assert_eq!(node.attempts.len(), 2);
    assert_eq!(node.attempts[0].review, Review::Approved);
    assert_eq!(node.review, Review::Pending);
    assert_eq!(node.verification, Verification::Unverified);
    assert_ne!(node.checkpoint(), Some(old.as_str()));
    assert!(app.node_evidence().unwrap().contains("待审"));
    assert!(
        Repository::open(&run.worktree)
            .unwrap()
            .resolve_snapshot(&old)
            .is_ok()
    );
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

#[test]
fn discussion_is_readonly_supports_questions_and_saved_plan_explicit_start() {
    use duet_workbench::{app::Mode, keymap::Action};
    let (dir, fake, mut store, mut app) = setup(2, "clarify");
    let mut run = CodingRun::start_drafting(
        dir.path(),
        &fake,
        None,
        "discuss a feature",
        Duration::from_secs(15),
        &mut store,
        &mut app,
    )
    .unwrap();
    poll(&mut run, &mut app, &mut store);
    assert!(app.tasks[0].plan.is_none());
    assert!(app.history.iter().any(|m| m.content.contains("明确语言")));
    assert!(!run.worktree.join("file1.txt").exists());
    assert!(run.resume(&mut app, &mut store).is_err());
    run.discuss(
        "用 Rust，实现这两个文件，边界严格按 scope",
        &mut app,
        &mut store,
    )
    .unwrap();
    poll(&mut run, &mut app, &mut store);
    assert_eq!(app.tasks[0].state, TaskState::Ready);
    assert!(
        app.tasks[0]
            .nodes
            .iter()
            .all(|n| n.execution == Execution::Planned)
    );
    app.act(Action::StartTask);
    assert!(matches!(app.mode, Mode::ConfirmStart { .. }));
    let id = run.id.clone();
    run.stop(&mut app, &mut store).unwrap();
    store.save(&app).unwrap();
    drop(run);
    drop(store);
    let mut store = WorkspaceStore::open(dir.path()).unwrap();
    store.restore(&mut app).unwrap();
    assert_eq!(app.tasks[0].state, TaskState::Ready);
    assert!(!app.agent_connected);
    let mut code = CodingRun::open_existing(
        &id,
        &fake,
        None,
        false,
        Duration::from_secs(15),
        &mut store,
        &mut app,
    )
    .unwrap();
    poll(&mut code, &mut app, &mut store);
    assert_eq!(code.id, id);
    assert_eq!(app.tasks.len(), 1);
    assert!(
        app.tasks[0]
            .nodes
            .iter()
            .all(|n| n.execution == Execution::Implemented)
    );
    assert!(!dir.path().join("file1.txt").exists());
}

#[test]
fn running_diff_automatically_follows_selected_node_and_preserves_position() {
    use duet_workbench::keymap::Action;
    let (dir, fake, mut store, mut app) = setup(1, "live");
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
    let deadline = Instant::now() + Duration::from_secs(8);
    while app.tasks[0].nodes.is_empty() {
        assert!(Instant::now() < deadline);
        run.poll(&mut app, &mut store).unwrap();
        std::thread::sleep(Duration::from_millis(5));
    }
    app.act(Action::Review);
    while app.tasks[0].nodes[0].preview_checkpoint().is_none() {
        assert!(Instant::now() < deadline);
        run.poll(&mut app, &mut store).unwrap();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(app.tasks[0].nodes[0].execution, Execution::Running);
    assert!(app.body().contains("+n1 first"));
    assert!(app.body().contains("-old baseline"));
    let first = app.review_checkpoint().unwrap().to_owned();
    app.items[app.selected].scroll = 3;
    app.act(Action::ApproveNode);
    assert_eq!(app.mode, duet_workbench::app::Mode::Browse);
    assert!(
        app.keymap
            .resolve(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char('v'),
                crossterm::event::KeyModifiers::NONE
            ))
            .is_none()
    );
    poll(&mut run, &mut app, &mut store);
    assert_ne!(app.review_checkpoint(), Some(first.as_str()));
    assert!(app.body().contains("+n1 second"));
    assert_eq!(app.items[app.selected].scroll, 3);
    assert!(app.tasks[0].nodes[0].attempts[0].previews.len() >= 2);
    assert!(app.body().contains("+n1 second"));
    assert_eq!(app.review_checkpoint(), app.tasks[0].nodes[0].checkpoint());
    assert!(
        Repository::open(&run.worktree)
            .unwrap()
            .diff_file(
                &Repository::open(&run.worktree)
                    .unwrap()
                    .resolve_snapshot(&first)
                    .unwrap(),
                "file1.txt"
            )
            .unwrap()
            .contains("n1 first")
    );
    run.stop(&mut app, &mut store).unwrap();
    store.save(&app).unwrap();
    drop(run);
    drop(store);
    let store = WorkspaceStore::open(dir.path()).unwrap();
    store.restore(&mut app).unwrap();
    assert!(app.body().contains("+n1 second"));
}

#[test]
fn delete_node_cascades_downstream_reverts_changes_and_removes_owned_discussion() {
    use duet_workbench::app::TaskSelection;
    let (dir, fake, mut store, mut app) = setup(3, "normal");
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
    let cp = app.tasks[0].nodes[0].checkpoint().unwrap().to_owned();
    app.tasks[0].nodes[0].approve(&cp).unwrap();
    run.resume(&mut app, &mut store).unwrap();
    poll(&mut run, &mut app, &mut store);
    run.stop(&mut app, &mut store).unwrap();
    let candidate = run.worktree.clone();
    let id = run.id.clone();
    drop(run);
    fs::write(candidate.join(".duet-fake-pi"), "ignored generated file").unwrap();
    duet_workbench::task_ops::delete_selection(
        &TaskSelection {
            task_id: id,
            node_id: Some("n2".into()),
        },
        &mut app,
        &mut store,
    )
    .unwrap();
    assert_eq!(app.tasks[0].nodes.len(), 1);
    assert_eq!(
        fs::read_to_string(candidate.join("file1.txt")).unwrap(),
        "n1\n"
    );
    assert!(!candidate.join("file2.txt").exists() && !candidate.join("file3.txt").exists());
    assert!(!candidate.join(".duet-fake-pi").exists());
    assert!(
        app.tasks[0]
            .history
            .iter()
            .any(|m| m.content.contains("implemented n1"))
    );
    assert!(
        !app.tasks[0]
            .history
            .iter()
            .any(|m| m.content.contains("implemented n2") || m.content.contains("implemented n3"))
    );
    assert_eq!(app.tasks[0].nodes[0].review, Review::Approved);
    assert_eq!(
        Repository::open(dir.path())
            .unwrap()
            .read(
                &Repository::open(dir.path())
                    .unwrap()
                    .resolve_snapshot(&cp)
                    .unwrap(),
                "file1.txt"
            )
            .unwrap(),
        "n1\n"
    );
    assert!(
        dir.path()
            .join(".duet/backups")
            .read_dir()
            .unwrap()
            .next()
            .is_some()
    );
}

#[test]
fn delete_task_preserves_other_task_and_main_dirty_files_and_index() {
    use duet_workbench::app::TaskSelection;
    let (dir, fake, mut store, mut app) = setup(1, "normal");
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
    let id = run.id.clone();
    let candidate = run.worktree.clone();
    drop(run);
    fs::write(dir.path().join("base.txt"), "manual uncommitted source").unwrap();
    let index = fs::read(dir.path().join(".git/index")).unwrap();
    let mut other = CodingRun::start_drafting(
        dir.path(),
        &fake,
        None,
        "other plan",
        Duration::from_secs(15),
        &mut store,
        &mut app,
    )
    .unwrap();
    poll(&mut other, &mut app, &mut store);
    other.stop(&mut app, &mut store).unwrap();
    let other_id = other.id.clone();
    let other_candidate = other.worktree.clone();
    drop(other);
    duet_workbench::task_ops::delete_selection(
        &TaskSelection {
            task_id: id,
            node_id: None,
        },
        &mut app,
        &mut store,
    )
    .unwrap();
    assert_eq!(app.tasks.len(), 1);
    assert_eq!(app.tasks[0].id, other_id);
    assert_eq!(app.tasks[0].state, TaskState::Ready);
    assert!(!candidate.exists() && other_candidate.exists());
    assert_eq!(
        fs::read_to_string(dir.path().join("base.txt")).unwrap(),
        "manual uncommitted source"
    );
    assert_eq!(fs::read(dir.path().join(".git/index")).unwrap(), index);
}

#[test]
fn deletion_refuses_tampered_candidate_that_points_to_main_project() {
    use duet_workbench::app::TaskSelection;
    let (dir, fake, mut store, mut app) = setup(1, "normal");
    let mut run = CodingRun::start_drafting(
        dir.path(),
        &fake,
        None,
        "plan",
        Duration::from_secs(15),
        &mut store,
        &mut app,
    )
    .unwrap();
    poll(&mut run, &mut app, &mut store);
    run.stop(&mut app, &mut store).unwrap();
    let id = run.id.clone();
    drop(run);
    let db = rusqlite::Connection::open(dir.path().join(".duet/workspace.db")).unwrap();
    db.execute(
        "UPDATE runs SET worktree=?1 WHERE id=?2",
        rusqlite::params![dir.path().to_str().unwrap(), id],
    )
    .unwrap();
    drop(db);
    assert!(
        duet_workbench::task_ops::delete_selection(
            &TaskSelection {
                task_id: id,
                node_id: None
            },
            &mut app,
            &mut store
        )
        .is_err()
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("base.txt")).unwrap(),
        "baseline\n"
    );
    assert_eq!(app.tasks.len(), 1);
}

use duet_workbench::nodes::TaskState;

#[test]
fn timeout_preserves_partial_code_and_reopen_hands_it_to_new_session() {
    let (dir, fake, mut store, mut app) = setup(1, "timeout_resume");
    let mut run = CodingRun::start_planning(
        dir.path(),
        &fake,
        None,
        "requirement",
        Duration::from_millis(150),
        &mut store,
        &mut app,
    )
    .unwrap();
    poll(&mut run, &mut app, &mut store);
    assert_eq!(app.tasks[0].state, TaskState::Failed);
    assert_eq!(app.tasks[0].nodes[0].execution, Execution::Failed);
    assert!(
        app.tasks[0]
            .failure
            .as_ref()
            .unwrap()
            .reason
            .contains("时间预算耗尽")
    );
    assert!(app.status.contains("超时停止"));
    let id = run.id.clone();
    let checkpoint = run.checkpoint.clone().unwrap();
    assert_eq!(
        fs::read_to_string(run.worktree.join("file1.txt")).unwrap(),
        "n1 partial\n"
    );
    let candidate = run.worktree.clone();
    run.stop(&mut app, &mut store).unwrap();
    drop(run);
    let mut continued = CodingRun::open_existing(
        &id,
        &fake,
        None,
        false,
        Duration::from_secs(15),
        &mut store,
        &mut app,
    )
    .unwrap();
    poll(&mut continued, &mut app, &mut store);
    let prompts = fs::read_to_string(dir.path().join(".duet/request-log")).unwrap();
    let last: serde_json::Value = serde_json::from_str(prompts.lines().last().unwrap()).unwrap();
    let message = last["message"].as_str().unwrap();
    assert!(message.contains("续写交接：") && message.contains("不是从零重做"));
    assert!(message.contains(&checkpoint) && message.contains("file1.txt"));
    assert_eq!(
        fs::read_to_string(candidate.join("file1.txt")).unwrap(),
        "n1 partial\nresumed\n"
    );
    assert_eq!(app.tasks.len(), 1);
    assert_eq!(app.tasks[0].nodes[0].execution, Execution::Implemented);
    assert_eq!(app.tasks[0].nodes[0].attempts.len(), 2);
    assert_eq!(
        app.tasks[0].nodes[0].attempts[0].checkpoint.as_deref(),
        Some(checkpoint.as_str())
    );
    continued.stop(&mut app, &mut store).unwrap();
}

#[test]
fn legacy_failed_discussion_recovers_error_from_local_session_without_model_call() {
    let (dir, fake, mut store, mut app) = setup(1, "quota");
    let mut run = CodingRun::start_drafting(
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
    drop(run);
    let task = &mut app.tasks[0];
    task.failure = None;
    task.state = TaskState::Ready;
    store.save_task(task).unwrap();
    let session = task.sessions.last().unwrap();
    fs::write(store.directory().join("sessions").join(format!("test_{session}.jsonl")),r#"{"message":{"role":"assistant","stopReason":"error","errorMessage":"Codex error: The usage limit has been reached"}}"#).unwrap();
    drop(store);
    let store = WorkspaceStore::open(dir.path()).unwrap();
    let tasks = store.tasks().unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].state, TaskState::Failed);
    assert!(
        tasks[0]
            .failure
            .as_ref()
            .unwrap()
            .reason
            .contains("usage limit")
    );
    assert!(tasks[0].plan.is_none());
    assert_eq!(tasks[0].sessions.len(), 1);
}

#[test]
fn quota_failure_is_persisted_and_confirmed_retry_reuses_task_after_restart() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use duet_workbench::{
        app::Mode,
        keymap::Action,
        nodes::{FailureStage, TaskState},
    };
    let (dir, fake, mut store, mut app) = setup(1, "quota");
    let mut run = CodingRun::start_drafting(
        dir.path(),
        &fake,
        None,
        "original requirement",
        Duration::from_secs(15),
        &mut store,
        &mut app,
    )
    .unwrap();
    poll(&mut run, &mut app, &mut store);
    let id = run.id.clone();
    assert_eq!(app.tasks[0].state, TaskState::Failed);
    assert_eq!(
        app.tasks[0].failure.as_ref().unwrap().stage,
        FailureStage::Discussion
    );
    assert!(
        app.tasks[0]
            .failure
            .as_ref()
            .unwrap()
            .reason
            .contains("usage limit")
    );
    assert!(app.tasks[0].plan.is_none());
    assert!(
        app.task_lines()
            .iter()
            .any(|(l, _)| l.contains("usage limit"))
    );
    app.act(Action::RetryTask);
    assert!(matches!(app.mode, Mode::ConfirmRetry { .. }));
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.requested_retry.is_none());
    store.save(&app).unwrap();
    drop(run);
    drop(store);
    let mut store = WorkspaceStore::open(dir.path()).unwrap();
    store.restore(&mut app).unwrap();
    assert_eq!(app.tasks[0].state, TaskState::Failed);
    app.act(Action::RetryTask);
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.requested_retry.take().unwrap().0, id);
    let mut retry = CodingRun::open_existing(
        &id,
        &fake,
        None,
        true,
        Duration::from_secs(15),
        &mut store,
        &mut app,
    )
    .unwrap();
    retry
        .discuss("original requirement", &mut app, &mut store)
        .unwrap();
    poll(&mut retry, &mut app, &mut store);
    assert_eq!(app.tasks.len(), 1);
    assert_eq!(app.tasks[0].id, id);
    assert_eq!(app.tasks[0].state, TaskState::Ready);
    assert!(app.tasks[0].failure.is_none());
    assert!(app.tasks[0].plan.is_some());
    assert!(
        app.tasks[0]
            .nodes
            .iter()
            .all(|n| n.execution == Execution::Planned)
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("base.txt")).unwrap(),
        "baseline\n"
    );
    retry.stop(&mut app, &mut store).unwrap();
}
