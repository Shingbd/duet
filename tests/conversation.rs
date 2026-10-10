use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use duet_workbench::{
    app::{App, Mode, Panel, RunControl, View},
    keymap::{Action, Keymap},
    rpc::{RunState, RunStatus, message_text},
    storage::{Message, WorkspaceStore},
    ui,
};
use ratatui::{Terminal, backend::TestBackend};
use serde_json::json;

#[test]
fn chat_roles_are_distinct_empty_messages_and_duplicate_final_text_are_hidden() {
    let mut app = App::empty(Keymap::default());
    app.task = Some("实现功能".into());
    app.view = View::Tasks;
    for (role, content) in [
        ("user", "ΩUSER_CONTENT"),
        ("assistant", ""),
        ("assistant", "   \n"),
        ("assistant", "λAI_CONTENT"),
    ] {
        app.history.push(Message {
            session: "s".into(),
            role: role.into(),
            content: content.into(),
        });
    }
    app.run.text = "AI_CONTENT".into(); // Authoritative final text is already in history.
    let body = app.body();
    assert_eq!(body.matches("AI_CONTENT").count(), 1);
    assert_eq!(body.matches("┌─ AI").count(), 1);
    assert!(body.contains("┌─ 你"));
    assert!(!body.contains("assistant"));
    let mut terminal = Terminal::new(TestBackend::new(120, 32)).unwrap();
    terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
    let buffer = terminal.backend().buffer();
    let user = buffer
        .content
        .iter()
        .find(|cell| cell.symbol() == "Ω")
        .unwrap();
    let ai = buffer
        .content
        .iter()
        .find(|cell| cell.symbol() == "λ")
        .unwrap();
    assert_ne!(user.fg, ai.fg);
    app.run.streaming_text = true;
    app.run.text = "LIVE_RESPONSE".into();
    assert!(app.body().contains("AI · 正在回复"));
    assert!(app.body().contains("LIVE_RESPONSE"));
}

#[test]
fn tool_only_events_do_not_create_visible_chat_and_streams_reset_between_messages() {
    let tool = json!({"role":"assistant","content":[{"type":"thinking","thinking":"private"},{"type":"toolCall","name":"read"}]});
    assert!(message_text(&tool).is_empty());
    assert_eq!(message_text(&json!({"content":"plain"})), "plain");
    let mut run = RunState::default();
    run.apply(&json!({"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"first"}}));
    assert!(run.streaming_text);
    run.apply(&json!({"type":"message_end","message":{"role":"assistant","content":[{"type":"text","text":"final"}]}}));
    assert!(!run.streaming_text);
    assert_eq!(run.text, "final");
    run.apply(&json!({"type":"message_start","message":{"role":"assistant"}}));
    run.apply(&json!({"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"second"}}));
    assert_eq!(run.text, "second");
}

#[test]
fn task_scroll_is_independent_and_reply_has_dedicated_input_mode() {
    let mut app = App::empty(Keymap::default());
    app.view = View::Tasks;
    app.panel = Panel::Reader;
    app.act(Action::Down);
    assert_eq!(app.task_scroll, 1); // Works even in a project without reading items.
    app.act(Action::Reply);
    for c in "jk纠正".chars() {
        app.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.controls, vec![RunControl::Say("jk纠正".into())]);
    assert_eq!(app.task_scroll, 1);
    assert_eq!(app.mode, Mode::Browse);
    app.act(Action::Pause);
    assert_eq!(app.controls.last(), Some(&RunControl::TogglePause));
}

#[test]
fn restore_filters_legacy_empty_chat_and_paused_runs_never_auto_resume() {
    let folder = tempfile::tempdir().unwrap();
    let mut store = WorkspaceStore::open(folder.path()).unwrap();
    store
        .begin_run("r", "s", folder.path(), "branch", "task")
        .unwrap();
    store.append_message("s", "assistant", "").unwrap();
    store.append_message("s", "assistant", " \n").unwrap();
    store.append_message("s", "user", "真实问题").unwrap();
    store.finish_run("r", "Paused", None).unwrap();
    let mut app = App::empty(Keymap::default());
    app.task_scroll = 7;
    store.save(&app).unwrap();
    drop(store);
    let store = WorkspaceStore::open(folder.path()).unwrap();
    store.restore(&mut app).unwrap();
    assert_eq!(app.history.len(), 1);
    assert_eq!(app.history[0].role, "user");
    assert_eq!(app.task_scroll, 7);
    assert_eq!(app.run.status, RunStatus::Aborted);
    assert!(!app.agent_connected);
    assert_eq!(store.history("s").unwrap().len(), 3); // Do not delete historical evidence.
}

#[cfg(unix)]
mod controlled {
    use super::*;
    use duet_workbench::{checkpoint::Repository, coding::CodingRun};
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        process::Command,
        time::{Duration, Instant},
    };

    fn project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("main.rs"), "baseline").unwrap();
        for args in [
            vec!["init", "-q"],
            vec!["add", "main.rs"],
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
        dir
    }

    fn fake(folder: &std::path::Path) -> std::path::PathBuf {
        let path = folder.join("fake-pi");
        fs::write(&path, r#"#!/usr/bin/env python3
import sys,json,pathlib,time
log=pathlib.Path(__file__).with_suffix('.log')
active=False
prompts=0
confirmations=0
def emit(value): print(json.dumps(value),flush=True)
def response(r,data=None):
    value={'type':'response','id':r['id'],'success':True}
    if data is not None: value['data']=data
    emit(value)
for line in sys.stdin.buffer:
    r=json.loads(line)
    with log.open('a') as f: f.write(json.dumps(r)+'\n')
    if r['type']=='get_state':
        busy=False
        if prompts==1 and not active:
            confirmations+=1
            busy=confirmations==1  # First idle probe is deliberately not quiescent.
        response(r,{'sessionId':'session','model':{'id':'fake'},'isStreaming':active,
                    'isCompacting':busy,'pendingMessageCount':0})
    elif r['type']=='clear_queue':
        time.sleep(0.03)
        if pathlib.Path('main.rs').read_text()=='clear fail':
            emit({'type':'response','id':r['id'],'success':False,'error':'clear rejected'})
        else: response(r,{'steering':[],'followUp':[]})
    elif r['type']=='abort':
        active=False
        emit({'type':'agent_settled','aborted':True})
        response(r)
    elif r['type']=='prompt':
        prompts+=1
        active=True
        if r['message']=='running correction':
            emit({'type':'agent_settled','aborted':False})
            time.sleep(0.08)  # Previous completion races with prompt acceptance.
        response(r,{'disposition':'started'})
        emit({'type':'agent_start'})
        emit({'type':'message_end','message':{'role':'user','content':r['message']}})
        emit({'type':'message_start','message':{'role':'assistant'}})
        emit({'type':'message_end','message':{'role':'assistant','content':[{'type':'thinking','thinking':'internal'}]}})
        emit({'type':'message_end','message':{'role':'assistant','content':[{'type':'toolCall','name':'read'}]}})
        emit({'type':'message_end','message':{'role':'assistant','content':[{'type':'text','text':'  \n'}]}})
        if prompts==1:
            pathlib.Path('main.rs').write_text('clear fail' if r['message']=='clear fail' else 'before pause')
            emit({'type':'tool_execution_start','toolName':'write'})
            emit({'type':'message_end','message':{'role':'assistant','content':[{'type':'text','text':'plan'}]}})
        else:
            pathlib.Path('main.rs').write_text('corrected' if prompts==2 else 'follow-up')
            emit({'type':'message_end','message':{'role':'assistant','content':[{'type':'text','text':'done'}]}})
            active=False
            emit({'type':'agent_end'})
            emit({'type':'agent_settled','aborted':False})
"#).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        path
    }

    fn until(
        run: &mut CodingRun,
        app: &mut App,
        store: &mut WorkspaceStore,
        predicate: impl Fn(&App) -> bool,
    ) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            run.poll(app, store).unwrap();
            if predicate(app) {
                break;
            }
            assert!(Instant::now() < deadline, "state {:?}", app.run.status);
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn pause_requires_idle_confirmation_and_corrections_resume_same_session_without_duplicates() {
        let project = project();
        let fixture = tempfile::tempdir().unwrap();
        let executable = fake(fixture.path());
        let repo = Repository::open(project.path()).unwrap();
        let mut store = WorkspaceStore::open(project.path()).unwrap();
        let mut app = App::browse(&repo, repo.baseline().unwrap(), Keymap::default()).unwrap();
        let mut run = CodingRun::start(
            project.path(),
            &executable,
            None,
            "initial task",
            Duration::from_secs(1),
            &mut store,
            &mut app,
        )
        .unwrap();
        until(&mut run, &mut app, &mut store, |a| a.history.len() == 2);
        run.pause(&mut app).unwrap();
        assert_eq!(app.run.status, RunStatus::Pausing);
        assert!(run.resume(&mut app, &mut store).is_err());
        until(&mut run, &mut app, &mut store, |a| {
            a.run.status == RunStatus::Paused
        });
        assert!(!run.is_finished());
        assert!(run.checkpoint.is_none());
        let before = fs::read_to_string(run.worktree.join("main.rs")).unwrap();
        std::thread::sleep(Duration::from_millis(1100)); // Paused time does not exhaust 1s budget.
        run.poll(&mut app, &mut store).unwrap();
        assert_eq!(app.run.status, RunStatus::Paused);
        assert_eq!(
            fs::read_to_string(run.worktree.join("main.rs")).unwrap(),
            before
        );
        run.interact(
            RunControl::Say("change approach".into()),
            &mut app,
            &mut store,
        )
        .unwrap();
        run.interact(
            RunControl::Say("keep scope small".into()),
            &mut app,
            &mut store,
        )
        .unwrap();
        assert_eq!(app.queued_corrections, 2);
        assert_eq!(app.run.status, RunStatus::Paused);
        assert_eq!(
            fs::read_to_string(run.worktree.join("main.rs")).unwrap(),
            before
        );
        run.resume(&mut app, &mut store).unwrap();
        until(&mut run, &mut app, &mut store, |a| {
            a.run.status == RunStatus::Settled
        });
        assert!(run.is_finished());
        assert_eq!(app.queued_corrections, 0);
        assert_eq!(store.history(&run.session).unwrap().len(), 5); // 3 user + 2 visible AI, no echo/empty rows.
        assert_eq!(app.history.iter().filter(|m| m.role == "user").count(), 3);
        let checkpoint = repo
            .resolve_snapshot(run.checkpoint.as_ref().unwrap())
            .unwrap();
        assert_eq!(repo.read(&checkpoint, "main.rs").unwrap(), "corrected");
        run.interact(
            RunControl::Say("follow-up question".into()),
            &mut app,
            &mut store,
        )
        .unwrap();
        until(&mut run, &mut app, &mut store, |a| {
            a.run.status == RunStatus::Settled
        });
        assert_eq!(
            fs::read_to_string(run.worktree.join("main.rs")).unwrap(),
            "follow-up"
        );
        assert_eq!(repo.read(&checkpoint, "main.rs").unwrap(), "corrected");
        assert_eq!(
            fs::read_to_string(project.path().join("main.rs")).unwrap(),
            "baseline"
        );
        let commands: Vec<serde_json::Value> = fs::read_to_string(executable.with_extension("log"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        let clear = commands
            .iter()
            .position(|r| r["type"] == "clear_queue")
            .unwrap();
        assert_eq!(commands[clear + 1]["type"], "abort");
        assert_eq!(commands[clear + 2]["type"], "get_state");
        let prompts: Vec<_> = commands.iter().filter(|r| r["type"] == "prompt").collect();
        assert_eq!(prompts[1]["message"], "change approach\n\nkeep scope small");
        assert_eq!(prompts[2]["streamingBehavior"], "steer");
        run.stop(&mut app, &mut store).unwrap();
    }

    #[test]
    fn running_correction_survives_previous_settle_racing_with_acceptance() {
        let project = project();
        let fixture = tempfile::tempdir().unwrap();
        let executable = fake(fixture.path());
        let mut store = WorkspaceStore::open(project.path()).unwrap();
        let mut app = App::empty(Keymap::default());
        let mut run = CodingRun::start(
            project.path(),
            &executable,
            None,
            "initial task",
            Duration::from_secs(5),
            &mut store,
            &mut app,
        )
        .unwrap();
        until(&mut run, &mut app, &mut store, |a| a.history.len() == 2);
        run.interact(
            RunControl::Say("running correction".into()),
            &mut app,
            &mut store,
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(30));
        run.poll(&mut app, &mut store).unwrap();
        assert!(!run.is_finished());
        assert_eq!(app.run.status, RunStatus::Running);
        assert!(run.checkpoint.is_none());
        until(&mut run, &mut app, &mut store, |a| {
            a.run.status == RunStatus::Settled
        });
        assert!(run.is_finished());
        assert_eq!(store.history(&run.session).unwrap().len(), 4);
        assert_eq!(
            fs::read_to_string(run.worktree.join("main.rs")).unwrap(),
            "corrected"
        );
    }

    #[test]
    fn failed_pause_request_is_not_reported_as_paused() {
        let project = project();
        let fixture = tempfile::tempdir().unwrap();
        let executable = fake(fixture.path());
        let mut store = WorkspaceStore::open(project.path()).unwrap();
        let mut app = App::empty(Keymap::default());
        let mut run = CodingRun::start(
            project.path(),
            &executable,
            None,
            "clear fail",
            Duration::from_secs(5),
            &mut store,
            &mut app,
        )
        .unwrap();
        until(&mut run, &mut app, &mut store, |a| a.history.len() == 2);
        run.pause(&mut app).unwrap();
        until(&mut run, &mut app, &mut store, |a| {
            a.run.status == RunStatus::Failed
        });
        assert!(run.is_finished());
        assert!(!app.agent_connected);
        assert_eq!(app.run.error.as_deref(), Some("clear rejected"));
    }

    #[test]
    fn cancelling_a_paused_task_closes_it_without_resuming_or_losing_files() {
        let project = project();
        let fixture = tempfile::tempdir().unwrap();
        let executable = fake(fixture.path());
        let mut store = WorkspaceStore::open(project.path()).unwrap();
        let mut app = App::empty(Keymap::default());
        let mut run = CodingRun::start(
            project.path(),
            &executable,
            None,
            "initial task",
            Duration::from_secs(5),
            &mut store,
            &mut app,
        )
        .unwrap();
        until(&mut run, &mut app, &mut store, |a| a.history.len() == 2);
        run.pause(&mut app).unwrap();
        until(&mut run, &mut app, &mut store, |a| {
            a.run.status == RunStatus::Paused
        });
        run.cancel(&mut app).unwrap();
        until(&mut run, &mut app, &mut store, |a| {
            a.run.status == RunStatus::Aborted
        });
        assert!(run.is_finished());
        assert!(run.checkpoint.is_some());
        assert!(!app.agent_connected);
        assert!(
            run.interact(RunControl::Say("retry".into()), &mut app, &mut store)
                .is_err()
        );
        assert_eq!(
            fs::read_to_string(run.worktree.join("main.rs")).unwrap(),
            "before pause"
        );
    }
}
