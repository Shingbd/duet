use duet_workbench::{
    app::App,
    checkpoint::Repository,
    helix_workspace::{Binding, Frame, Identity, Meta, Workspace, open_command, position},
    keymap::Keymap,
    review_editor::{Buffer, Proposal},
    storage::WorkspaceStore,
};
use std::{
    io::Write,
    path::Path,
    process::{Command, Stdio},
};
fn fixture() -> (tempfile::TempDir, WorkspaceStore, Workspace, App) {
    let dir = tempfile::tempdir().unwrap();
    let store = WorkspaceStore::open(dir.path()).unwrap();
    let w = Workspace::new(dir.path(), &store, false).unwrap();
    let mut app = App::empty(Keymap::default());
    app.helix_review = true;
    app.review_buffers.push(Buffer::new(
        "task".into(),
        Some("node".into()),
        "main.txt".into(),
        "a".repeat(40),
        String::new(),
        "aa aa 文é\n".into(),
    ));
    (dir, store, w, app)
}
fn bind(w: &mut Workspace, app: &App) -> std::path::PathBuf {
    let f = w.directory.join("main.txt");
    std::fs::write(&f, &app.review_buffers[0].text).unwrap();
    let b = &app.review_buffers[0];
    w.manifest.bindings.push(Binding {
        file: f.clone(),
        identity: Some(Identity {
            task: b.task_id.clone(),
            node: b.node_id.clone(),
            path: b.path.clone(),
            version: b.version.clone(),
        }),
    });
    f
}
fn frame(
    file: &Path,
    text: &str,
    selection: &str,
    cursor: usize,
    anchor: usize,
    select: bool,
) -> Frame {
    Frame {
        meta: Meta {
            action: "ask".into(),
            select,
            line: 1,
            column: cursor,
            path: file.to_str().unwrap().into(),
            selection: selection.into(),
            anchor_line: 1,
            anchor_column: anchor,
            multiple: false,
        },
        text: text.into(),
    }
}
#[test]
fn actual_anchor_disambiguates_repeated_words_and_unsaved_unicode() {
    let (_dir, _store, mut w, mut app) = fixture();
    let f = bind(&mut w, &app);
    let x = frame(&f, "aa aa 文é\n", "aa", 5, 4, true);
    let c = w.absorb(&x, &mut app, Path::new("/")).unwrap();
    assert_eq!((c.from, c.to, c.excerpt.as_str()), (3, 5, "aa"));
    let p = Proposal {
        context: c,
        old: "aa".into(),
        new: "bb".into(),
    };
    app.review_buffers[0].apply(&p).unwrap();
    assert_eq!(app.review_buffers[0].text, "aa bb 文é\n");
    assert_eq!(position("文é\n", 1, 2).unwrap(), 3);
    assert_eq!(position("文é\n", 1, 3).unwrap(), 6);
    assert_eq!(
        duet_workbench::helix_workspace::cursor_excerpt("文é_word x\n", 3),
        (0, 11)
    );
    assert_eq!(
        duet_workbench::helix_workspace::cursor_excerpt("aa aa\n", 2),
        (0, 5)
    );
    let x = frame(&f, "aa aa 文é\n", "a", 4, 4, false);
    let c = w.absorb(&x, &mut app, Path::new("/")).unwrap();
    assert_eq!((c.from, c.to, c.excerpt.as_str()), (3, 5, "aa"));
}
#[test]
fn stale_or_unowned_or_multiple_selection_is_rejected() {
    let (_dir, _store, mut w, mut app) = fixture();
    let f = bind(&mut w, &app);
    let x = frame(&f, "aa aa 文é\n", "aa", 5, 4, true);
    let c = w.absorb(&x, &mut app, Path::new("/")).unwrap();
    let p = Proposal {
        context: c,
        old: "aa".into(),
        new: "bb".into(),
    };
    let mut changed = frame(&f, "aa cc 文é\n", "cc", 5, 4, true);
    w.absorb(&changed, &mut app, Path::new("/")).unwrap();
    assert!(
        w.stage_apply(&p, &changed, &mut app, Path::new("/"))
            .is_err()
    );
    changed.meta.multiple = true;
    assert!(w.absorb(&changed, &mut app, Path::new("/")).is_err());
    changed.meta.multiple = false;
    changed.meta.path = "/etc/passwd".into();
    assert!(w.absorb(&changed, &mut app, Path::new("/")).is_err());
    assert!(!w.directory.join("replacement.json").exists());
}
#[test]
fn shell_metacharacters_are_metadata_or_stdin_not_commands() {
    let (_dir, _store, w, _app) = fixture();
    let marker = w.directory.join("must-not-exist");
    let payload = format!("text'; touch {}; $(echo danger)", marker.display());
    let raw = format!(
        "__duet_meta|ask|1|1|1|bad'; touch {}; echo '",
        marker.display()
    );
    let mut child = Command::new(env!("CARGO_BIN_EXE_duet-workbench"))
        .arg("helix-bridge")
        .arg(&w.directory)
        .arg(raw)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(payload.as_bytes())
        .unwrap();
    assert!(child.wait().unwrap().success());
    let m: Meta =
        serde_json::from_slice(&std::fs::read(w.directory.join("pending.json")).unwrap()).unwrap();
    assert_eq!(m.selection, payload);
    assert!(!marker.exists());
    let open = open_command(Path::new("/tmp/z %sh{touch marker}; $.txt")).unwrap();
    assert!(
        String::from_utf8(open)
            .unwrap()
            .contains(":open '/tmp/z %sh{touch marker}; $.txt'")
    );
    assert!(open_command(Path::new("/tmp/a\nb")).is_err());
}
#[test]
fn private_review_keeps_main_index_head_and_original_objects() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("main.txt"), "base\n").unwrap();
    for args in [
        vec!["init", "-q"],
        vec!["add", "."],
        vec![
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
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
    let repo = Repository::open(dir.path()).unwrap();
    let base = repo.baseline().unwrap();
    std::fs::write(dir.path().join("main.txt"), "delivery\n").unwrap();
    let snapshot = repo.capture(&["main.txt".into()]).unwrap();
    std::fs::write(dir.path().join("main.txt"), "main dirty\n").unwrap();
    let index = std::fs::read(dir.path().join(".git/index")).unwrap();
    let store = WorkspaceStore::open(dir.path()).unwrap();
    let mut w = Workspace::new(dir.path(), &store, true).unwrap();
    let b = Buffer::new(
        "task".into(),
        Some("node".into()),
        "main.txt".into(),
        snapshot.commit.clone(),
        "base\n".into(),
        "delivery\n".into(),
    );
    let f = w.review(&b, &repo).unwrap();
    std::fs::write(f, "human draft\n").unwrap();
    assert_eq!(repo.baseline().unwrap().commit, base.commit);
    assert_eq!(std::fs::read(dir.path().join(".git/index")).unwrap(), index);
    assert_eq!(repo.read(&snapshot, "main.txt").unwrap(), "delivery\n");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("main.txt")).unwrap(),
        "main dirty\n"
    );
    let config = std::fs::read_to_string(w.config_file()).unwrap();
    assert!(config.contains("duet-light"));
    assert!(config.contains("auto-format = false"));
    assert!(config.contains("helix-bridge"));
}
#[test]
fn readonly_failed_review_focus_never_invokes_workbench_actions() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use duet_workbench::app::{Panel, View};
    let mut app = App::empty(Keymap::default());
    app.helix_review = true;
    app.view = View::Review;
    app.panel = Panel::Reader;
    // Explicit comparison is readonly; editable native-buffer dispatch stays disabled.
    app.review_compare = true;
    let mut task = duet_workbench::nodes::TaskRecord::legacy("t", "goal", "session");
    task.checkpoint = Some("a".repeat(40));
    app.tasks.push(task);
    app.task_selection = Some(duet_workbench::app::TaskSelection {
        task_id: "t".into(),
        node_id: None,
    });
    app.items.push(duet_workbench::app::ReadingItem {
        title: "x.txt".into(),
        version: "a".repeat(40),
        intent: String::new(),
        code: "x".into(),
        diff: String::new(),
        change: Some(duet_workbench::checkpoint::ChangeKind::Modified),
        evidence: String::new(),
        explanation: String::new(),
        read: false,
        scroll: 0,
        evidence_scroll: 0,
    });
    app.review_buffers.push(Buffer::new(
        "t".into(),
        None,
        "x.txt".into(),
        "a".repeat(40),
        "x".into(),
        "x".into(),
    ));
    for k in ['i', 'a', 's', 'd', 'g'] {
        app.handle_key(KeyEvent::new(KeyCode::Char(k), KeyModifiers::NONE));
    }
    assert!(
        app.requested_start.is_none()
            && app.requested_merge.is_none()
            && app.requested_delete.is_none()
    );
    assert_eq!(app.review_buffers[0].text, "x");
    assert!(!app.review_buffers[0].insert);
    app.open_review_chat();
    assert!(app.requested_review_chat.is_none() && app.review_chat_context.is_none());
}
#[test]
fn observing_one_buffer_does_not_clear_uncertainty_for_another() {
    let (_dir, _store, mut w, mut app) = fixture();
    let first = bind(&mut w, &app);
    let mut b = app.review_buffers[0].clone();
    b.path = "other.txt".into();
    b.text = "z\n".into();
    b.original = b.text.clone();
    app.review_buffers.push(b);
    let second = w.directory.join("other.txt");
    std::fs::write(&second, "z\n").unwrap();
    w.manifest.bindings.push(Binding {
        file: second.clone(),
        identity: Some(Identity {
            task: "task".into(),
            node: Some("node".into()),
            path: "other.txt".into(),
            version: "a".repeat(40),
        }),
    });
    w.active = Some(first.clone());
    w.mark_input(false);
    w.absorb(
        &frame(&second, "z\n", "z", 1, 1, false),
        &mut app,
        Path::new("/"),
    )
    .unwrap();
    assert!(app.helix_unknown);
    w.absorb(
        &frame(&first, "aa aa 文é\n", "a", 1, 1, false),
        &mut app,
        Path::new("/"),
    )
    .unwrap();
    assert!(!app.helix_unknown);
}
#[test]
#[cfg(unix)]
fn real_helix_uses_duet_palette_in_cells_and_tracks_theme_changes() {
    let exe =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("target/tools/helix-25.07.1-x86_64-linux/hx");
    if !exe.exists() {
        return;
    }
    let (_dir, _store, w, mut app) = fixture();
    let file = w.directory.join("theme.txt");
    std::fs::write(&file, "theme text\n").unwrap();
    let terminal = duet_workbench::embedded::PtyEditor::spawn_managed(
        &exe,
        &w.directory,
        &file,
        20,
        80,
        Some(&w.config_home()),
    )
    .unwrap();
    let mut s = duet_workbench::editing::Session {
        cwd: w.directory.clone(),
        target: duet_workbench::editing::Target::Workspace {
            directory: w.directory.clone(),
        },
        terminal,
        workspace: w,
    };
    let wait = |s: &mut duet_workbench::editing::Session, color| {
        let end = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while std::time::Instant::now() < end {
            s.terminal.poll().unwrap();
            if s.terminal.screen().cell(2, 10).unwrap().bgcolor() == color {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        panic!("Helix theme cell did not match {:?}", color)
    };
    wait(&mut s, vt100::Color::Rgb(17, 21, 30));
    app.light = true;
    duet_workbench::editing::poll_bridge(&mut s, &mut app).unwrap();
    wait(&mut s, vt100::Color::Rgb(245, 246, 250));
    s.terminal.stop().unwrap();
}
