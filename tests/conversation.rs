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

// Display/input regressions are independent of the controlled RPC module below.
mod display_regression {
    use super::*;
    use ratatui::{backend::Backend, style::Modifier};
    use unicode_width::UnicodeWidthStr;

    fn screen(terminal: &Terminal<TestBackend>) -> String {
        let buffer = terminal.backend().buffer();
        (0..buffer.area.height).map(|y| {
            let mut row = String::new();
            let mut x = 0;
            while x < buffer.area.width {
                let symbol = buffer[(x, y)].symbol();
                row.push_str(symbol);
                x += symbol.width().max(1) as u16;
            }
            row
        }).collect::<Vec<_>>().join("\n")
    }

    fn set_input(app: &mut App, kind: usize, text: &str) {
        app.mode = match kind {
            0 => Mode::Input { search: false, text: text.into() },
            1 => Mode::Input { search: true, text: text.into() },
            _ => Mode::Message { text: text.into() },
        };
    }

    fn source(app: &App) -> &str {
        match &app.mode {
            Mode::Input { text, .. } | Mode::Message { text } => text,
            _ => panic!("input unexpectedly closed"),
        }
    }

    #[test]
    fn all_input_surfaces_follow_unicode_tail_resize_and_keep_original_source() {
        for light in [false, true] {
            for kind in 0..3 {
                let mut app = App::empty(Keymap::default());
                app.light = light;
                let text = format!("{}TAILZ", "中ASCII👩‍💻e\u{301}**_#`".repeat(100));
                set_input(&mut app, kind, &text);
                let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
                for (width, height) in [(100, 30), (24, 8), (40, 16), (100, 30)] {
                    terminal.backend_mut().resize(width, height);
                    terminal.resize(ratatui::layout::Rect::new(0, 0, width, height)).unwrap();
                    terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
                    let popup_width = width.min(if kind == 2 { 76 } else { 72 });
                    let inner_width = popup_width as usize - 2;
                    let display = format!("{}{text}", if kind == 2 { "" } else { "> " });
                    let mut rows = ui::wrap_cells(&display, inner_width);
                    let last_width = rows.last().unwrap().width();
                    let column = if last_width == inner_width { rows.push(String::new()); 0 } else { last_width };
                    let cursor = terminal.backend_mut().get_cursor_position().unwrap();
                    assert_eq!(cursor.x, (width - popup_width) / 2 + 1 + column as u16);
                    assert!(cursor.y > 0 && cursor.y < height - 1);
                    if column > 0 {
                        assert_eq!(terminal.backend().buffer()[(cursor.x - 1, cursor.y)].symbol(), "Z");
                    }
                    let output = screen(&terminal);
                    assert!(output.contains("Enter"));
                    assert!(output.contains("Esc"));
                    if kind == 2 { assert!(output.contains("仍有编码权限")); }
                    assert_eq!(source(&app), text);
                    // The tail window contains a preceding input row, not just
                    // a clipped first line. At minimum height it can be one row.
                    assert!(rows.len() > height as usize);
                }
            }
        }
    }

    #[test]
    fn second_input_row_is_visible_and_unicode_deletion_restores_cursor() {
        for light in [false, true] {
            for kind in 0..3 {
                let mut app = App::empty(Keymap::default());
                app.light = light;
                let inner_width = if kind == 2 { 74 } else { 70 };
                let prefix_width = if kind == 2 { 0 } else { 2 };
                let input = format!("{}SECOND", "a".repeat(inner_width - prefix_width));
                set_input(&mut app, kind, &input);
                let mut terminal = Terminal::new(TestBackend::new(100, 20)).unwrap();
                terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
                let cursor = terminal.backend_mut().get_cursor_position().unwrap();
                let left = (100 - if kind == 2 { 76 } else { 72 }) / 2 + 1;
                let buffer = terminal.backend().buffer();
                assert_eq!(buffer[(left, cursor.y)].symbol(), "S");
                assert_eq!(buffer[(left + 5, cursor.y)].symbol(), "D");
                assert_eq!(buffer[(left + prefix_width as u16, cursor.y - 1)].symbol(), "a");
                assert_eq!(cursor.x, left + 6);
                // Keyboard events remain scalar input. Display must cluster the
                // resulting CJK, ZWJ emoji and combining accent without changing it.
                let suffix = "中👩‍💻e\u{301}";
                for ch in suffix.chars() {
                    app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
                }
                terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
                assert!(screen(&terminal).contains(suffix));
                assert_eq!(source(&app), format!("{input}{suffix}"));
                for _ in suffix.chars() {
                    app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
                }
                terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
                assert_eq!(source(&app), input);
                assert_eq!(terminal.backend_mut().get_cursor_position().unwrap(), cursor);
            }
        }
    }

    #[test]
    fn exact_fill_typing_and_backspace_cross_rows_in_all_three_inputs() {
        for kind in 0..3 {
            let mut app = App::empty(Keymap::default());
            let inner_width = if kind == 2 { 74 } else { 70 };
            let prefix = if kind == 2 { 0 } else { 2 };
            set_input(&mut app, kind, &"a".repeat(inner_width - prefix));
            let mut terminal = Terminal::new(TestBackend::new(100, 20)).unwrap();
            terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
            let exact = terminal.backend_mut().get_cursor_position().unwrap();
            let left = (100 - if kind == 2 { 76 } else { 72 }) / 2 + 1;
            assert_eq!(exact.x, left);
            app.handle_key(KeyEvent::new(KeyCode::Char('Z'), KeyModifiers::NONE));
            terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
            let typed = terminal.backend_mut().get_cursor_position().unwrap();
            assert_eq!((typed.x, typed.y), (left + 1, exact.y));
            app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
            terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
            assert_eq!(terminal.backend_mut().get_cursor_position().unwrap(), exact);
            app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
            terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
            let deleted = terminal.backend_mut().get_cursor_position().unwrap();
            assert_eq!(deleted.x, left + inner_width as u16 - 1);
            // Message height shrinks by one row and recenters; command/search
            // retain their five-row minimum popup height.
            assert_eq!(deleted.y, if kind == 2 { exact.y } else { exact.y - 1 });
            assert_eq!(source(&app).len(), inner_width - prefix - 1);
        }
    }

    #[test]
    fn enter_submits_original_text_and_escape_has_no_submission_side_effect() {
        let text = format!("{}end", "中文👩‍💻e\u{301}**source** ".repeat(20));
        for kind in 0..3 {
            let mut app = App::empty(Keymap::default());
            let input = if kind == 0 { format!("note {text}") } else { text.clone() };
            set_input(&mut app, kind, &input);
            let mut terminal = Terminal::new(TestBackend::new(24, 8)).unwrap();
            terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
            app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
            assert_eq!(app.mode, Mode::Browse);
            match kind {
                0 => assert_eq!(app.notes, vec![text.clone()]),
                1 => assert_eq!(app.filter, text),
                _ => assert_eq!(app.controls, vec![RunControl::Say(text.clone())]),
            }
            let (notes, filter, controls) = (app.notes.clone(), app.filter.clone(), app.controls.clone());
            set_input(&mut app, kind, "note must not submit");
            app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
            assert_eq!(app.mode, Mode::Browse);
            assert_eq!((app.notes, app.filter, app.controls), (notes, filter, controls));
        }
    }

    #[test]
    fn message_mode_notices_and_sanitized_input_remain_literal_on_small_terminal() {
        for (planning, paused, notice) in [(true, false, "只读讨论"), (false, true, "已暂停"), (false, false, "仍有编码权限")] {
            let mut app = App::empty(Keymap::default());
            app.planning_only = planning;
            if paused { app.run.status = RunStatus::Paused; }
            app.mode = Mode::Message { text: "**Ω**\t\x1b[2J".into() };
            let mut terminal = Terminal::new(TestBackend::new(24, 8)).unwrap();
            terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
            let output = screen(&terminal);
            assert!(output.contains(notice));
            assert!(output.contains("**Ω**"));
            assert!(output.contains('�'));
            assert!(output.contains("Enter") && output.contains("Esc"));
            assert_eq!(source(&app), "**Ω**\t\x1b[2J");
        }
    }

    #[test]
    fn current_historical_and_streaming_markdown_keep_roles_and_whole_message_search() {
        use duet_workbench::{app::TaskSelection, nodes::TaskRecord};
        let mut app = App::empty(Keymap::default());
        app.view = View::Tasks;
        app.fullscreen = true;
        app.task = Some("**request**".into());
        for (role, content) in [("user", "**Ωvisible phrase**\n\n```\n*a_literal*\n# source\n```\n\n- list"), ("assistant", "*λanswer*")] {
            app.history.push(Message { session: "s".into(), role: role.into(), content: content.into() });
        }
        let mut terminal = Terminal::new(TestBackend::new(80, 32)).unwrap();
        for light in [false, true] {
            app.light = light;
            terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
            let output = screen(&terminal);
            assert!(output.contains("┌─ 你") && output.contains("┌─ AI"));
            assert!(output.contains("*a_literal*") && output.contains("# source"));
            assert!(!output.contains("**Ωvisible phrase**"));
            let buffer = terminal.backend().buffer();
            let user = buffer.content.iter().find(|cell| cell.symbol() == "Ω").unwrap();
            let ai = buffer.content.iter().find(|cell| cell.symbol() == "λ").unwrap();
            assert_ne!(user.fg, ai.fg);
            assert!(user.modifier.contains(Modifier::BOLD));
            assert!(ai.modifier.contains(Modifier::ITALIC));
        }
        for filter in ["Ωvisible phrase", "**Ωvisible phrase**"] {
            app.filter = filter.into();
            terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
            let output = screen(&terminal);
            assert!(output.contains("┌─ 你") && output.contains("*a_literal*"));
            assert!(!output.contains("λanswer"));
        }
        app.filter.clear();
        app.run.streaming_text = true;
        app.run.text = "**streaming_tail**".into();
        terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
        assert!(screen(&terminal).contains("streaming_tail"));
        app.run.streaming_text = false;
        app.history.push(Message { session: "s".into(), role: "assistant".into(), content: app.run.text.clone() });
        terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
        assert_eq!(screen(&terminal).matches("streaming_tail").count(), 1);
        let mut old = TaskRecord::legacy("old", "**historical request**", "s");
        old.history = app.history.clone();
        app.tasks = vec![old, TaskRecord::legacy("live", "live", "s")];
        app.task_selection = Some(TaskSelection { task_id: "old".into(), node_id: None });
        terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
        assert!(screen(&terminal).contains("historical request"));
        assert_eq!(screen(&terminal).matches("streaming_tail").count(), 1);
    }
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
