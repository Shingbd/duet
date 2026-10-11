use duet_workbench::{
    app::{App, Panel, View},
    checkpoint::{ChangeKind, Repository},
    keymap::{Action, Keymap},
    storage::WorkspaceStore,
    ui,
};
use ratatui::{Terminal, backend::TestBackend};
use std::{fs, process::Command};

// In-memory snapshots isolate display regressions from Git/workflow operations.
mod display_regression {
    use super::*;
    use duet_workbench::app::{Mode, Page, ReadingItem, TaskSelection};
    use ratatui::style::Modifier;
    use unicode_width::UnicodeWidthStr;

    fn fixture() -> App {
        let mut app = App::empty(Keymap::default());
        app.items.push(ReadingItem {
            title: "README.md".into(), version: "fixed_snapshot".into(),
            intent: "# ΩIntent\n\n**visible intent**".into(),
            code: "# λDocument\n\n**visible prose**\n\n- entry".into(),
            diff: "+**Ωadded_literal**\n-# λremoved_literal".into(),
            change: Some(ChangeKind::Modified),
            evidence: "## ξEvidence\n\n- [x] document only".into(),
            explanation: "**ψExplanation**".into(),
            read: false, scroll: 0, evidence_scroll: 0,
        });
        app.panel = Panel::Reader;
        app.fullscreen = true;
        app
    }

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

    #[test]
    fn markdown_project_selection_is_explicit_and_fixed_diffs_keep_literals_and_colors() {
        let mut app = fixture();
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        for light in [false, true] {
            app.light = light;
            app.view = View::Project;
            for path in ["README.md", "notes.MARKDOWN", "a.mdown", "a.mkd"] {
                app.items[0].title = path.into();
                terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
                let output = screen(&terminal);
                assert!(output.contains("visible prose"));
                assert!(!output.contains("**visible prose**"));
                let cell = terminal.backend().buffer().content.iter().find(|cell| cell.symbol() == "λ").unwrap();
                assert!(cell.modifier.contains(Modifier::BOLD));
            }
            for path in ["src/a.rs", "a.md.rs", "notes.txt", "notes.mdx", "Makefile"] {
                app.items[0].title = path.into();
                terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
                let output = screen(&terminal);
                assert!(output.contains("**visible prose**"));
                assert!(output.contains("# λDocument"));
            }
            app.view = View::Review;
            app.page = Page::Code;
            app.items[0].title = "README.md".into();
            terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
            let output = screen(&terminal);
            assert!(output.contains("+**Ωadded_literal**"));
            assert!(output.contains("-# λremoved_literal"));
            let cells = &terminal.backend().buffer().content;
            let added = cells.iter().find(|cell| cell.symbol() == "Ω").unwrap();
            let removed = cells.iter().find(|cell| cell.symbol() == "λ").unwrap();
            assert_ne!(added.fg, removed.fg);
            assert!(!added.modifier.contains(Modifier::BOLD));
            // Render only the preview; never prepare/confirm/execute a merge.
            app.mode = Mode::MergePreview { scroll: 0, preview: duet_workbench::merge::MergePreview {
                task_id: "task".into(), signature: "fixture".into(),
                source_head: "a".repeat(40), delivery: "b".repeat(40),
                branch: "main".into(), commit: "c".repeat(40),
                diff: app.items[0].diff.clone(), paths: vec!["README.md".into()], unverified: 1,
            } };
            terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
            assert!(screen(&terminal).contains("+**Ωadded_literal**"));
            let cells = &terminal.backend().buffer().content;
            assert_ne!(cells.iter().find(|cell| cell.symbol() == "Ω").unwrap().fg,
                cells.iter().find(|cell| cell.symbol() == "λ").unwrap().fg);
            assert!(app.requested_merge.is_none());
            app.mode = Mode::Browse;
        }
    }

    #[test]
    fn intent_evidence_explanation_and_independent_notes_share_markdown_styles() {
        let mut app = fixture();
        app.view = View::Review;
        let mut terminal = Terminal::new(TestBackend::new(132, 36)).unwrap();
        for light in [false, true] {
            app.light = light;
            app.fullscreen = true;
            for (page, expected, symbol) in [(Page::Intent, "visible intent", "Ω"), (Page::Evidence, "document only", "ξ")] {
                app.page = page;
                terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
                assert!(screen(&terminal).contains(expected));
                let cell = terminal.backend().buffer().content.iter().find(|cell| cell.symbol() == symbol).unwrap();
                assert!(cell.modifier.contains(Modifier::BOLD));
            }
            app.fullscreen = false;
            app.explain = true;
            app.page = Page::Code;
            terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
            assert!(screen(&terminal).contains("ψExplanation"));
            assert!(!screen(&terminal).contains("**ψExplanation**"));
            assert!(screen(&terminal).contains("document only"));
            let cell = terminal.backend().buffer().content.iter().find(|cell| cell.symbol() == "ψ").unwrap();
            assert!(cell.modifier.contains(Modifier::BOLD));
            app.view = View::Knowledge;
            app.fullscreen = true;
            app.notes = vec!["```\nΩunfinished code".into(), "# λNext note\n\n**body**".into()];
            terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
            assert!(screen(&terminal).contains("λNext note"));
            let cell = terminal.backend().buffer().content.iter().find(|cell| cell.symbol() == "λ").unwrap();
            assert!(cell.modifier.contains(Modifier::BOLD));
            app.view = View::Review;
        }
    }

    #[test]
    fn evidence_search_and_unfinished_summary_cannot_hide_true_risk_metadata() {
        use duet_workbench::nodes::{NodeContract, NodeRecord, TaskRecord};
        let contract: NodeContract = serde_json::from_value(serde_json::json!({
            "id":"n", "title":"title", "objective":"body", "scope":["src/a.rs"],
            "boundaries":["scope"], "implementation":"step", "constraints":[],
            "dependencies":[], "acceptance":["- [x] document only"]
        })).unwrap();
        let mut node = NodeRecord::new(contract);
        node.begin("base");
        node.attempts[0].checkpoint = Some("fixed_snapshot".into());
        node.attempts[0].scope_violations = vec!["src/outside.rs".into()];
        node.attempts[0].summary = "```\nsummary **not status**".into();
        let mut task = TaskRecord::legacy("task", "request", "session");
        task.nodes.push(node);
        let mut app = fixture();
        app.tasks.push(task);
        app.task_selection = Some(TaskSelection { task_id: "task".into(), node_id: Some("n".into()) });
        app.view = View::Review;
        app.page = Page::Evidence;
        app.explain = true;
        app.filter = "summary".into();
        for light in [false, true] {
            app.light = light;
            let mut terminal = Terminal::new(TestBackend::new(100, 36)).unwrap();
            terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
            let output = screen(&terminal);
            for expected in ["未验证", "待审", "仓内越界", "src/outside.rs", "summary **not status**"] {
                assert!(output.contains(expected), "missing {expected}");
            }
        }
    }
}

fn setup() -> (
    tempfile::TempDir,
    Repository,
    duet_workbench::checkpoint::Snapshot,
    App,
) {
    let dir = tempfile::tempdir().unwrap();
    for (path, text) in [
        ("a.rs", "old a\n"),
        ("b.rs", "unchanged\n"),
        ("d.rs", "deleted\n"),
    ] {
        fs::write(dir.path().join(path), text).unwrap();
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
    let repo = Repository::open(dir.path()).unwrap();
    let mut app = App::browse(&repo, repo.baseline().unwrap(), Keymap::default()).unwrap();
    app.items[0].scroll = 1;
    fs::write(dir.path().join("a.rs"), "new a\n").unwrap();
    fs::write(dir.path().join("n.rs"), "new file\n").unwrap();
    fs::remove_file(dir.path().join("d.rs")).unwrap();
    let snapshot = repo
        .capture(&["a.rs".into(), "d.rs".into(), "n.rs".into()])
        .unwrap();
    app.append_checkpoint(&repo, &snapshot).unwrap();
    (dir, repo, snapshot, app)
}

#[test]
fn review_lists_only_changed_paths_once_and_shows_file_scoped_colored_diff() {
    let (_dir, _repo, snapshot, mut app) = setup();
    assert_eq!(app.selected, 0); // Arrival does not replace an existing immutable reading item.
    assert_eq!(app.current().unwrap().code, "old a\n");
    assert_eq!(app.current().unwrap().scroll, 1);
    assert_eq!(app.file_indices().len(), 4); // One sidebar path despite retaining old versions.
    assert_eq!(app.changed_indices().len(), 3);
    app.act(Action::Tasks);
    app.act(Action::Review);
    assert_eq!(app.view, View::Review);
    assert_eq!(app.panel, Panel::Reader);
    assert_eq!(app.current().unwrap().title, "a.rs");
    assert_eq!(app.current().unwrap().version, snapshot.commit);
    assert_eq!(app.current().unwrap().change, Some(ChangeKind::Modified));
    assert!(app.body().contains("-old a"));
    assert!(app.body().contains("+new a"));
    assert!(!app.body().contains("n.rs"));
    let mut terminal = Terminal::new(TestBackend::new(120, 32)).unwrap();
    for light in [false, true] {
        app.light = light;
        terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
        let buffer = terminal.backend().buffer();
        let mut colors = Vec::new();
        let sidebar: Vec<String> = (4..30)
            .map(|y| (0..28).map(|x| buffer[(x, y)].symbol()).collect())
            .collect();
        assert!(!sidebar.iter().any(|line| line.contains("b.rs")));
        assert!(!sidebar.iter().any(|line| line.contains("待审 ·")));
        for (path, marker) in [("a.rs", "M"), ("d.rs", "D"), ("n.rs", "A")] {
            let row = sidebar.iter().position(|line| line.contains(path)).unwrap() as u16 + 4;
            let cell = (0..28)
                .map(|x| &buffer[(x, row)])
                .find(|cell| cell.symbol() == marker)
                .unwrap();
            colors.push(cell.fg);
        }
        assert_ne!(colors[0], colors[1]);
        assert_ne!(colors[0], colors[2]);
        assert_ne!(colors[1], colors[2]);
    }
    app.act(Action::NextItem);
    assert_eq!(app.current().unwrap().title, "d.rs");
    assert!(app.body().contains("-deleted"));
    app.act(Action::NextItem);
    assert_eq!(app.current().unwrap().title, "n.rs");
    assert!(app.body().contains("+new file"));
}

#[test]
fn jk_follows_visible_review_files_and_project_navigation_keeps_all_files() {
    let (_dir, _repo, _snapshot, mut app) = setup();
    app.act(Action::Review);
    app.panel = Panel::Queue;
    let key = |c| {
        crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char(c),
            crossterm::event::KeyModifiers::NONE,
        )
    };
    for (c, expected) in [
        ('j', "d.rs"),
        ('j', "n.rs"),
        ('j', "n.rs"),
        ('k', "d.rs"),
        ('k', "a.rs"),
        ('k', "a.rs"),
    ] {
        app.handle_key(key(c));
        assert_eq!(app.current().unwrap().title, expected);
        assert!(app.visible_file_indices().contains(&app.selected));
    }
    app.act(Action::Project);
    app.handle_key(key('j'));
    assert_eq!(app.current().unwrap().title, "b.rs");
    app.act(Action::Review);
    app.panel = Panel::Queue;
    app.latest_checkpoint = None;
    let selected = app.selected;
    for c in ['j', 'k'] {
        app.handle_key(key(c));
    }
    assert_eq!(
        app.selected, selected,
        "empty review must not navigate hidden baseline files"
    );
}

#[test]
fn legacy_prefixed_selection_and_progress_survive_plain_git_style_paths() {
    let (dir, repo, snapshot, mut app) = setup();
    app.act(Action::Review);
    app.items[app.selected].read = true;
    app.items[app.selected].scroll = 2;
    let mut store = WorkspaceStore::open(dir.path()).unwrap();
    store
        .begin_run("r", "s", dir.path(), "branch", "task")
        .unwrap();
    store
        .finish_run("r", "Settled", Some(&snapshot.commit))
        .unwrap();
    store.save(&app).unwrap();
    drop(store);
    let db = rusqlite::Connection::open(dir.path().join(".duet/workspace.db")).unwrap();
    let encoded: String = db
        .query_row(
            "SELECT value FROM ui_state WHERE key='workspace'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let mut saved: serde_json::Value = serde_json::from_str(&encoded).unwrap();
    saved["selected"][1] = "待审 · a.rs".into();
    db.execute(
        "UPDATE ui_state SET value=?1 WHERE key='workspace'",
        [saved.to_string()],
    )
    .unwrap();
    db.execute(
        "UPDATE reading_progress SET path='待审 · a.rs' WHERE version=?1 AND path='a.rs'",
        [&snapshot.commit],
    )
    .unwrap();
    drop(db);
    let store = WorkspaceStore::open(dir.path()).unwrap();
    let mut restored = App::browse(&repo, repo.baseline().unwrap(), Keymap::default()).unwrap();
    store.restore(&mut restored).unwrap();
    assert_eq!(restored.current().unwrap().title, "a.rs");
    assert_eq!(restored.current().unwrap().version, snapshot.commit);
    assert_eq!(restored.current().unwrap().scroll, 2);
    assert!(restored.current().unwrap().read);
}
