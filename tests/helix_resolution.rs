use duet_workbench::editing::helix_candidates;
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};
#[test]
fn default_helix_prefers_path_then_trusted_executable_relative_bundle() {
    let c = helix_candidates(
        Path::new("hx"),
        Some(OsStr::new("/usr/bin:/usr/local/bin")),
        Path::new("/opt/duet/target/release/duet-workbench"),
    );
    assert_eq!(c[0], PathBuf::from("/usr/bin/hx"));
    assert!(c.contains(&PathBuf::from(
        "/opt/duet/target/tools/helix-25.07.1-x86_64-linux/hx"
    )));
    assert!(!c.iter().any(|p| p.is_relative()));
}
#[test]
fn explicit_missing_editor_is_not_replaced_by_bundle() {
    assert_eq!(
        helix_candidates(
            Path::new("/chosen/missing-hx"),
            None,
            Path::new("/host/target/release/duet")
        ),
        vec![PathBuf::from("/chosen/missing-hx")]
    );
    assert!(
        helix_candidates(
            Path::new("custom-hx"),
            None,
            Path::new("/host/target/release/duet")
        )
        .is_empty()
    );
}
#[test]
fn footer_has_no_h_helix_hint_and_uses_tab_for_editor_focus() {
    use duet_workbench::{
        app::{App, View},
        keymap::Keymap,
        ui,
    };
    let mut app = App::empty(Keymap::default());
    app.view = View::Project;
    app.editor_project = true;
    let p = vt100::Parser::new(20, 80, 0);
    app.editor_screen = Some(p.screen().clone());
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(200, 36)).unwrap();
    terminal.draw(|f| ui::draw(f, &app)).unwrap();
    let b = terminal.backend().buffer();
    let footer = b
        .content
        .iter()
        .skip(200 * 34)
        .map(|c| c.symbol())
        .collect::<String>();
    assert!(!footer.contains("Helix"));
    let all = b.content.iter().map(|c| c.symbol()).collect::<String>();
    assert!(all.contains("Tab"));
    assert!(!all.contains("h Helix"));
}
