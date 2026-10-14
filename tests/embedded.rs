#![cfg(unix)]
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use duet_workbench::{
    app::{App, Mode, View},
    embedded::{PtyEditor, encode_key},
    keymap::Keymap,
};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::Path,
    time::{Duration, Instant},
};
fn wait(editor: &mut PtyEditor, predicate: impl Fn(&PtyEditor) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        editor.poll().unwrap();
        if predicate(editor) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "editor output={}",
            editor.screen().contents()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
#[test]
fn child_output_wakes_host_before_the_idle_timeout() {
    let temp = tempfile::tempdir().unwrap();
    let exe = temp.path().join("fake-hx");
    let file = temp.path().join("file.txt");
    fs::write(&file, "file").unwrap();
    fs::write(&exe,"#!/usr/bin/env python3\nimport os,time,tty\ntty.setraw(0)\ntime.sleep(.05)\nos.write(1,b'WAKE')\ntime.sleep(1)\n").unwrap();
    fs::set_permissions(&exe, fs::Permissions::from_mode(0o700)).unwrap();
    let mut editor = PtyEditor::spawn(&exe, temp.path(), &file, 10, 40).unwrap();
    let start = Instant::now();
    editor.wait_io(Duration::from_secs(3)).unwrap();
    editor.poll().unwrap();
    assert!(editor.screen().contents().contains("WAKE"));
    assert!(start.elapsed() < Duration::from_secs(2));
    editor.stop().unwrap();
}
#[test]
fn pty_has_tty_unicode_styles_resize_input_and_clean_exit() {
    let temp = tempfile::tempdir().unwrap();
    let executable = temp.path().join("fake-hx");
    let file = temp.path().join("code.rs");
    fs::write(&file, "before").unwrap();
    fs::write(
        &executable,
        r#"#!/usr/bin/env python3
import os,sys,tty,signal
assert os.isatty(0) and os.isatty(1)
tty.setraw(0)
def out(t):os.write(1,t.encode())
def resize(*a):
 s=os.get_terminal_size(0);out('\r\nSIZE '+str(s.lines)+' '+str(s.columns))
signal.signal(signal.SIGWINCH,resize)
out('\x1b[2J\x1b[H\x1b[32mREADY 中文\x1b[0m\x1b[?2004h')
while True:
 b=os.read(0,1024)
 if b==b'q':break
 if b==b'w':open(sys.argv[1],'w').write('human edit');out('\r\nSAVED')
 else:out('\r\nINPUT '+b.hex())
"#,
    )
    .unwrap();
    // Python raw source uses escaped terminal bytes in ordinary literals.
    let text = fs::read_to_string(&executable)
        .unwrap()
        .replace("\\\\", "\\");
    fs::write(&executable, text).unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    let mut editor = PtyEditor::spawn(&executable, temp.path(), &file, 12, 60).unwrap();
    wait(&mut editor, |e| {
        e.screen().contents().contains("READY 中文")
    });
    assert!(editor.screen().bracketed_paste());
    assert_eq!(
        editor.screen().cell(0, 0).unwrap().fgcolor(),
        vt100::Color::Idx(2)
    );
    editor.resize(16, 70).unwrap();
    wait(&mut editor, |e| {
        e.screen().contents().contains("SIZE 16 70")
    });
    editor.send(b"w").unwrap();
    wait(&mut editor, |e| e.screen().contents().contains("SAVED"));
    assert_eq!(fs::read_to_string(&file).unwrap(), "human edit");
    editor.send(b"q").unwrap();
    wait(&mut editor, |e| e.exit_status().is_some());
    assert!(editor.exit_status().unwrap().success());
}
#[test]
fn terminal_key_encoding_does_not_reinterpret_helix_keys_as_duet_actions() {
    let key = |c, m| KeyEvent::new(c, m);
    assert_eq!(
        encode_key(key(KeyCode::Char('q'), KeyModifiers::NONE), false),
        b"q"
    );
    assert_eq!(
        encode_key(key(KeyCode::Esc, KeyModifiers::NONE), false),
        b"\x1b"
    );
    assert_eq!(
        encode_key(key(KeyCode::Char('x'), KeyModifiers::CONTROL), false),
        vec![24]
    );
    assert_eq!(
        encode_key(key(KeyCode::Up, KeyModifiers::NONE), true),
        b"\x1bOA"
    );
    assert_eq!(
        encode_key(key(KeyCode::Left, KeyModifiers::CONTROL), false),
        b"\x1b[1;5D"
    );
    assert_eq!(
        encode_key(key(KeyCode::Char('中'), KeyModifiers::ALT), false),
        "\u{1b}中".as_bytes()
    );
}
#[test]
fn pty_drop_reaps_child_and_editor_start_is_explicit() {
    let temp = tempfile::tempdir().unwrap();
    let executable = temp.path().join("fake-hx");
    fs::write(&executable, "#!/bin/sh\nexec sleep 1000\n").unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    let editor = PtyEditor::spawn(&executable, temp.path(), Path::new("file"), 10, 40).unwrap();
    let pid = editor.id();
    drop(editor);
    assert_eq!(unsafe { libc::kill(pid as i32, 0) }, -1);
    let mut app = App::empty(Keymap::default());
    app.view = View::Project;
    app.handle_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE));
    assert!(app.requested_editor.is_none());
    assert!(matches!(app.mode, Mode::Browse));
    assert!(app.editor_screen.is_none());
}
#[test]
fn child_terminal_control_strings_stay_in_virtual_screen_not_host_text() {
    let mut app = App::empty(Keymap::default());
    let mut parser = vt100::Parser::new(8, 40, 0);
    parser.process(b"SAFE\x1b]52;c;c2VjcmV0\x07\x1b]0;spoofed-title\x07\x1b[31m RED");
    app.editor_screen = Some(parser.screen().clone());
    app.editor_focused = true;
    let backend = ratatui::backend::TestBackend::new(100, 30);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    terminal
        .draw(|f| duet_workbench::ui::draw(f, &app))
        .unwrap();
    let text = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect::<String>();
    assert!(text.contains("SAFE"));
    assert!(text.contains("HELIX"));
    assert!(!text.contains("spoofed-title"));
    assert!(!text.contains("c2VjcmV0"));
    assert!(!text.contains('\u{1b}'));
}
