//! Preferences only: configuration never grants model, write, test, or merge permission.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

pub const DEFAULT_JSON: &str = include_str!("../duet.default.json");
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
pub enum Language {
    #[default]
    #[serde(rename = "zh-CN")]
    #[value(name = "zh-CN")]
    Chinese,
    #[serde(rename = "en")]
    #[value(name = "en")]
    English,
}
impl Language {
    pub fn text<'a>(self, chinese: &'a str, english: &'a str) -> &'a str {
        match self {
            Self::Chinese => chinese,
            Self::English => english,
        }
    }
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    #[default]
    Restore,
    Dark,
    Light,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DefaultView {
    #[default]
    Restore,
    Tasks,
    Review,
    Project,
    Knowledge,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AgentConfig {
    pub pi_path: PathBuf,
    pub model: Option<String>,
    pub timeout_secs: u64,
}
impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            pi_path: "pi".into(),
            model: None,
            timeout_secs: 600,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Performance {
    pub active_poll_ms: u64,
    pub idle_poll_ms: u64,
    pub save_delay_ms: u64,
    pub save_max_delay_ms: u64,
}
impl Default for Performance {
    fn default() -> Self {
        Self {
            active_poll_ms: 16,
            idle_poll_ms: 500,
            save_delay_ms: 300,
            save_max_delay_ms: 2000,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub schema_version: u32,
    pub language: Language,
    pub theme: Theme,
    pub default_view: DefaultView,
    pub keymap_file: Option<PathBuf>,
    pub editor: PathBuf,
    pub helix_path: PathBuf,
    pub agent: AgentConfig,
    pub performance: Performance,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            schema_version: 1,
            language: Language::Chinese,
            theme: Theme::Restore,
            default_view: DefaultView::Restore,
            keymap_file: None,
            editor: "vi".into(),
            helix_path: "hx".into(),
            agent: AgentConfig::default(),
            performance: Performance::default(),
        }
    }
}
impl Config {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == 1,
            "Unsupported config schema_version; expected 1"
        );
        ensure!(
            (1..=3600).contains(&self.agent.timeout_secs),
            "agent.timeout_secs must be 1..=3600"
        );
        ensure!(
            (16..=250).contains(&self.performance.active_poll_ms),
            "performance.active_poll_ms must be 16..=250"
        );
        ensure!(
            (100..=5000).contains(&self.performance.idle_poll_ms),
            "performance.idle_poll_ms must be 100..=5000"
        );
        ensure!(
            (50..=5000).contains(&self.performance.save_delay_ms),
            "performance.save_delay_ms must be 50..=5000"
        );
        ensure!(
            (self.performance.save_delay_ms..=10000).contains(&self.performance.save_max_delay_ms),
            "save_max_delay_ms must be >= save_delay_ms and <= 10000"
        );
        for path in [&self.agent.pi_path, &self.editor, &self.helix_path] {
            ensure!(
                !path.as_os_str().is_empty()
                    && !path.to_string_lossy().chars().any(char::is_control),
                "Executable path must be nonempty and contain no control characters"
            );
        }
        if let Some(model) = &self.agent.model {
            ensure!(
                !model.trim().is_empty()
                    && model.len() <= 256
                    && !model.chars().any(char::is_control),
                "agent.model must be a nonempty model identifier (<=256 bytes)"
            );
        }
        Ok(())
    }
    pub fn global_path() -> Option<PathBuf> {
        std::env::var_os("XDG_CONFIG_HOME")
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|s| PathBuf::from(s).join(".config")))
            .map(|p| p.join("duet/config.json"))
    }
    pub fn load(project: &Path, explicit: Option<&Path>) -> Result<Self> {
        Self::load_layers(project, Self::global_path().as_deref(), explicit)
    }
    pub fn load_layers(
        project: &Path,
        global: Option<&Path>,
        explicit: Option<&Path>,
    ) -> Result<Self> {
        let mut value: Value = serde_json::from_str(DEFAULT_JSON)?;
        if let Some(global) = global
            && global.exists()
        {
            merge_file(&mut value, global)?;
        }
        let project_file = project.join("duet.json");
        if project_file.exists() {
            merge_file(&mut value, &project_file)?;
        }
        if let Some(explicit) = explicit {
            merge_file(&mut value, explicit)?;
        }
        let config: Self = serde_json::from_value(value).context(
            "Invalid Duet configuration (unknown fields and unsupported values are rejected)",
        )?;
        config.validate()?;
        Ok(config)
    }
    pub fn apply(&self, app: &mut crate::app::App) {
        app.config = self.clone();
        match self.theme {
            Theme::Restore => {}
            Theme::Dark => app.light = false,
            Theme::Light => app.light = true,
        }
        app.view = match self.default_view {
            DefaultView::Restore => app.view,
            DefaultView::Tasks => crate::app::View::Tasks,
            DefaultView::Review => crate::app::View::Review,
            DefaultView::Project => crate::app::View::Project,
            DefaultView::Knowledge => crate::app::View::Knowledge,
        };
        app.content_revision = app.content_revision.wrapping_add(1);
    }
    /// Explicit generation only, using create_new: never overwrite an existing file.
    pub fn initialize(path: &Path) -> Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .with_context(|| {
                format!(
                    "Cannot create config {}; existing files are never overwritten",
                    path.display()
                )
            })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(fs::Permissions::from_mode(0o600))?;
        }
        file.write_all(DEFAULT_JSON.as_bytes())?;
        file.sync_all()?;
        Ok(())
    }
}
fn merge_file(target: &mut Value, path: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(path)
        .with_context(|| format!("Cannot read config {}", path.display()))?;
    ensure!(
        meta.is_file() && !meta.file_type().is_symlink() && meta.len() <= 65536,
        "Config must be a regular, non-symlink file <=64 KiB: {}",
        path.display()
    );
    let mut value: Value = serde_json::from_slice(&fs::read(path)?)
        .with_context(|| format!("Invalid JSON in {}", path.display()))?;
    ensure!(
        value.is_object(),
        "Config must be a JSON object: {}",
        path.display()
    );
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()?.join(path)
    };
    let base = absolute.parent().context("Config has no parent")?;
    for pointer in ["/keymap_file", "/editor", "/agent/pi_path", "/helix_path"] {
        if let Some(field) = value.pointer_mut(pointer)
            && let Some(text) = field.as_str()
        {
            let relative = Path::new(text);
            if !relative.is_absolute()
                && (pointer == "/keymap_file"
                    || relative.components().count() > 1
                    || text.starts_with('.'))
            {
                *field = Value::String(base.join(relative).to_string_lossy().into());
            }
        }
    }
    // Validate each layer separately so a later override cannot hide invalid fields.
    let mut check: Value = target.clone();
    merge(&mut check, value.clone());
    serde_json::from_value::<Config>(check)
        .with_context(|| format!("Invalid configuration in {}", path.display()))?
        .validate()?;
    merge(target, value);
    Ok(())
}
fn merge(target: &mut Value, value: Value) {
    match (target, value) {
        (Value::Object(target), Value::Object(value)) => {
            for (key, value) in value {
                if let Some(existing) = target.get_mut(&key) {
                    merge(existing, value);
                } else {
                    target.insert(key, value);
                }
            }
        }
        (target, value) => *target = value,
    }
}
