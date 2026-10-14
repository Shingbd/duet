#[test]
fn rust_syntax_preference_is_strict_and_defaults_fast() {
    use duet_workbench::config::{Config, HelixRustSyntax};
    assert_eq!(Config::default().helix_rust_syntax, HelixRustSyntax::Fast);
    let config: Config = serde_json::from_str(r#"{"helix_rust_syntax":"full"}"#).unwrap();
    assert_eq!(config.helix_rust_syntax, HelixRustSyntax::Full);
    assert!(serde_json::from_str::<Config>(r#"{"helix_rust_syntax":"off"}"#).is_err());
}
use duet_workbench::{
    app::{App, View},
    config::{Config, DEFAULT_JSON, DefaultView, Language, Theme},
    keymap::Keymap,
    ui,
};
use std::{fs, path::Path, process::Command};

#[test]
fn shipped_defaults_match_runtime_defaults_and_grant_no_permissions() {
    let defaults: Config = serde_json::from_str(DEFAULT_JSON).unwrap();
    defaults.validate().unwrap();
    assert_eq!(defaults, Config::default());
    let temp = tempfile::tempdir().unwrap();
    assert_eq!(
        Config::load_layers(temp.path(), None, None).unwrap(),
        defaults
    );
    assert_eq!(defaults.agent.timeout_secs, 600);
    assert_eq!(defaults.theme, Theme::Restore);
    assert!(!DEFAULT_JSON.contains("allow_write"));
    assert!(!temp.path().join(".duet").exists());
}
#[test]
fn precedence_deep_merge_and_relative_paths_follow_each_file() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let global = root.join("global");
    let project = root.join("project");
    let explicit = root.join("explicit");
    for dir in [&global, &project, &explicit] {
        fs::create_dir(dir).unwrap();
    }
    fs::write(
        global.join("config.json"),
        r#"{"language":"en","agent":{"model":"provider/model","timeout_secs":900}}"#,
    )
    .unwrap();
    fs::write(
        project.join("duet.json"),
        r#"{"theme":"light","agent":{"timeout_secs":300},"keymap_file":"keys.json"}"#,
    )
    .unwrap();
    fs::write(
        explicit.join("config.json"),
        r#"{"language":"zh-CN","agent":{"pi_path":"./bin/pi","model":null}}"#,
    )
    .unwrap();
    let cfg = Config::load_layers(
        &project,
        Some(&global.join("config.json")),
        Some(&explicit.join("config.json")),
    )
    .unwrap();
    assert_eq!(cfg.language, Language::Chinese);
    assert_eq!(cfg.theme, Theme::Light);
    assert_eq!(cfg.agent.timeout_secs, 300);
    assert!(cfg.agent.model.is_none());
    assert_eq!(cfg.keymap_file, Some(project.join("keys.json")));
    assert_eq!(cfg.agent.pi_path, explicit.join("./bin/pi"));
}
#[test]
fn invalid_and_unsafe_settings_fail_before_initialization() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("duet.json");
    for text in [
        r#"{"language":"fr"}"#,
        r#"{"theme":"unknown"}"#,
        r#"{"schema_version":2}"#,
        r#"{"agent":{"timeout_secs":0}}"#,
        r#"{"agent":{"timeout_secs":3601}}"#,
        r#"{"auto_merge":true}"#,
        r#"{"agent":{"allow_write":true}}"#,
        r#"{"performance":{"idle_poll_ms":1}}"#,
        r#"{"performance":{"save_delay_ms":2500,"save_max_delay_ms":2000}}"#,
        r#"{"agent":{"model":""}}"#,
        "null",
        "{broken",
    ] {
        fs::write(&file, text).unwrap();
        assert!(
            Config::load_layers(temp.path(), None, None).is_err(),
            "accepted {text}"
        );
        assert!(!temp.path().join(".duet").exists());
    }
    fs::write(&file, " ".repeat(65537)).unwrap();
    assert!(Config::load_layers(temp.path(), None, None).is_err());
    fs::remove_file(file).unwrap();
    assert!(
        Config::load_layers(
            temp.path(),
            None,
            Some(Path::new("/nonexistent/explicit-duet-config"))
        )
        .is_err()
    );
}
#[cfg(unix)]
#[test]
fn config_symlinks_are_rejected() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("real.json"), DEFAULT_JSON).unwrap();
    std::os::unix::fs::symlink(temp.path().join("real.json"), temp.path().join("duet.json"))
        .unwrap();
    assert!(Config::load_layers(temp.path(), None, None).is_err());
}
#[test]
fn default_generation_never_overwrites_or_starts_a_workspace() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("duet.json");
    Config::initialize(&path).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), DEFAULT_JSON);
    assert!(Config::initialize(&path).is_err());
    assert_eq!(fs::read_to_string(&path).unwrap(), DEFAULT_JSON);
    assert!(!temp.path().join(".duet").exists());
}
#[test]
fn preferences_apply_after_restore_without_touching_task_permissions() {
    let mut app = App::empty(Keymap::default());
    app.light = true;
    app.view = View::Knowledge;
    Config::default().apply(&mut app);
    assert!(app.light);
    assert_eq!(app.view, View::Knowledge);
    let cfg = Config {
        language: Language::English,
        theme: Theme::Dark,
        default_view: DefaultView::Project,
        ..Default::default()
    };
    cfg.apply(&mut app);
    assert!(!app.light);
    assert_eq!(app.view, View::Project);
    assert_eq!(app.pi_session_label(), "Pi session not started");
    assert!(app.requested_task.is_none());
    assert!(app.requested_start.is_none());
    assert!(app.requested_merge.is_none());
    let backend = ratatui::backend::TestBackend::new(120, 32);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    terminal.draw(|f| ui::draw(f, &app)).unwrap();
    let text = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect::<String>();
    assert!(text.contains("Tasks"));
    assert!(text.contains("Review"));
    assert!(text.contains("Pi session not started"));
    assert!(text.contains("English UI: partial"));
}
#[test]
fn cli_show_and_init_are_side_effect_free_and_cli_overrides_win() {
    let temp = tempfile::tempdir().unwrap();
    let binary = env!("CARGO_BIN_EXE_duet-workbench");
    let cfg = temp.path().join("custom.json");
    fs::write(&cfg, r#"{"language":"en","theme":"light"}"#).unwrap();
    let output = Command::new(binary)
        .env("XDG_CONFIG_HOME", temp.path().join("global"))
        .args([
            "--config",
            cfg.to_str().unwrap(),
            "--language",
            "zh-CN",
            "config",
            "show",
            temp.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{:?}", output.stderr);
    let parsed: Config = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(parsed.language, Language::Chinese);
    assert_eq!(parsed.theme, Theme::Light);
    assert!(!temp.path().join(".duet").exists());
    let target = temp.path().join("duet.json");
    let status = Command::new(binary)
        .args(["config", "init", target.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(status.status.success());
    let status = Command::new(binary)
        .args(["config", "init", target.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!status.status.success());
    assert!(!temp.path().join(".duet").exists());
}
#[test]
fn configuration_never_replaces_explicit_write_authorization() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(
        temp.path().join("duet.json"),
        r#"{"agent":{"pi_path":"./must-not-be-started","model":"provider/model"}}"#,
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_duet-workbench"))
        .env("XDG_CONFIG_HOME", temp.path().join("global"))
        .args([
            "code",
            "--repo",
            temp.path().to_str().unwrap(),
            "--prompt",
            "do not execute",
            "--headless",
        ])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("--allow-write"));
    assert!(!temp.path().join(".duet").exists());
}
