//! Explicit Helix command/stdin bridge. No editor state is inferred from terminal cells.
use crate::{
    app::App,
    checkpoint::Repository,
    conflict,
    review_editor::{Buffer, Context, MAX_BYTES, Proposal},
};
use anyhow::{Context as _, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use unicode_segmentation::UnicodeSegmentation;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Identity {
    pub task: String,
    pub node: Option<String>,
    pub path: String,
    pub version: String,
}
impl Identity {
    fn matches(&self, b: &Buffer) -> bool {
        b.matches(&self.task, self.node.as_deref(), &self.path, &self.version)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Binding {
    pub file: PathBuf,
    pub identity: Option<Identity>,
}
#[derive(Serialize, Deserialize)]
pub struct Manifest {
    pub root: PathBuf,
    pub bindings: Vec<Binding>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Meta {
    pub action: String,
    pub select: bool,
    pub line: usize,
    pub column: usize,
    pub path: String,
    pub selection: String,
    pub anchor_line: usize,
    pub anchor_column: usize,
    pub multiple: bool,
}
#[derive(Serialize, Deserialize)]
pub struct Frame {
    pub meta: Meta,
    pub text: String,
}
#[derive(Serialize, Deserialize)]
struct Replacement {
    file: PathBuf,
    before: String,
    after: String,
}
pub struct Workspace {
    pub directory: PathBuf,
    pub manifest: Manifest,
    pub active: Option<PathBuf>,
    pub pending: Option<Instant>,
    pub light: bool,
    pub rust_syntax: crate::config::HelixRustSyntax,
    pub confirm: Option<Proposal>,
    pub applying: bool,
    pub unobserved: std::collections::HashSet<PathBuf>,
}
fn json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let temp = path.with_extension("tmp");
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)?;
    f.write_all(&serde_json::to_vec(value)?)?;
    drop(f);
    fs::rename(temp, path)?;
    Ok(())
}
fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let meta = fs::symlink_metadata(path)?;
    ensure!(
        meta.is_file() && !meta.file_type().is_symlink() && meta.len() <= 16 * MAX_BYTES as u64,
        "Invalid bridge record"
    );
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}
impl Workspace {
    pub fn new(root: &Path, store: &crate::storage::WorkspaceStore, light: bool) -> Result<Self> {
        Self::new_with_syntax(root, store, light, Default::default())
    }
    pub fn new_with_syntax(
        root: &Path,
        store: &crate::storage::WorkspaceStore,
        light: bool,
        rust_syntax: crate::config::HelixRustSyntax,
    ) -> Result<Self> {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let directory = store
            .directory()
            .join("helix-workspaces")
            .join(nonce.to_string());
        conflict::private_dir(&directory)?;
        let mut w = Self {
            directory,
            manifest: Manifest {
                root: root.canonicalize()?,
                bindings: vec![],
            },
            active: None,
            pending: None,
            light,
            rust_syntax,
            confirm: None,
            applying: false,
            unobserved: Default::default(),
        };
        w.save_manifest()?;
        w.write_config()?;
        Ok(w)
    }
    pub fn welcome(&mut self) -> Result<PathBuf> {
        let file = self.directory.join("welcome.txt");
        fs::write(
            &file,
            "Duet · Helix 已就绪\n在侧栏选择文件；Tab 进入编辑器。\n此启动页位于私有目录，不是项目文件，不可发送给 AI。\n",
        )?;
        self.manifest.bindings.push(Binding {
            file: file.clone(),
            identity: None,
        });
        self.save_manifest()?;
        Ok(file)
    }
    fn save_manifest(&self) -> Result<()> {
        json(&self.directory.join("manifest.json"), &self.manifest)
    }
    /// Internal Helix picker/buffer switches have no continuous document identity feed.
    /// Conservatively retain uncertainty for other buffers until captured or the editor exits.
    pub fn mark_input(&mut self, project: bool) {
        for b in &self.manifest.bindings {
            if b.identity.is_some() && (project || self.active.as_ref() == Some(&b.file)) {
                self.unobserved.insert(b.file.clone());
            }
        }
    }
    pub fn config_home(&self) -> PathBuf {
        self.directory.join("config")
    }
    pub fn config_file(&self) -> PathBuf {
        self.config_home().join("helix/config.toml")
    }
    pub fn refresh_config(&mut self) -> Result<()> {
        self.write_config()
    }
    fn write_config(&mut self) -> Result<()> {
        let dir = self.config_home().join("helix");
        conflict::private_dir(&dir.join("themes"))?;
        // Session-local query overlays, never edit hx runtime or user configuration.
        // Rust base highlights, grammar, indentation/textobjects and LSP stay upstream.
        let queries = dir.join("runtime/queries/rust");
        conflict::private_dir(&queries)?;
        for name in ["locals.scm", "injections.scm"] {
            let file = queries.join(name);
            if self.rust_syntax == crate::config::HelixRustSyntax::Fast {
                fs::write(
                    file,
                    "; Duet fast Rust: retain upstream base syntax; omit locals/injections.\n",
                )?;
            } else {
                match fs::remove_file(file) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.into()),
                }
            }
        }
        for light in [false, true] {
            fs::write(
                dir.join("themes").join(if light {
                    "duet-light.toml"
                } else {
                    "duet-dark.toml"
                }),
                theme(light),
            )?;
        }
        let shell = serde_json::to_string(&vec![
            std::env::current_exe()?.to_string_lossy().into_owned(),
            "helix-bridge".into(),
            self.directory.to_string_lossy().into_owned(),
        ])?;
        let mut config = format!(
            "theme = \"duet-{}\"\n[editor]\nauto-format = false\nmouse = false\nshell = {shell}\n",
            if self.light { "light" } else { "dark" }
        );
        for mode in ["normal", "select"] {
            config.push_str(&format!("\n[keys.{mode}]\n"));
            let apply = vec![
                "save_selection",
                "select_all",
                ":pipe \"__duet_replace|%{buffer_name}\"",
                "jump_backward",
            ];
            config.push_str(&format!("A-p = {}\n", serde_json::to_string(&apply)?));
            config.push_str(&format!(
                "F12 = {}\n",
                serde_json::to_string(&vec!["normal_mode", ":open \"%sh{__duet_open}\""])?
            ));
            config.push_str(&format!(
                "space.q = {}\n",
                serde_json::to_string(&capture_commands("save-back", mode == "select"))?
            ));
            for (key, action) in [
                ("C-g", "ask"),
                ("C-s", "publish"),
                ("C-p", "apply"),
                ("C-y", "sync"),
                ("C-q", "leave"),
                ("C-d", "diff"),
            ] {
                let commands = capture_commands(action, mode == "select");
                config.push_str(&format!("{key} = {}\n", serde_json::to_string(&commands)?));
            }
        }
        // Insert-mode bridge keys first leave insert mode; normal Helix bindings otherwise remain intact.
        config.push_str("\n[keys.insert]\n");
        // Host file navigation is also available while inserting; finish the edit
        // transaction before opening another buffer, without writing either one.
        config.push_str(&format!(
            "F12 = {}\n",
            serde_json::to_string(&vec!["normal_mode", ":open \"%sh{__duet_open}\""])?
        ));
        for (key, action) in [
            ("C-g", "ask"),
            ("C-s", "publish"),
            ("C-p", "apply"),
            ("C-y", "sync"),
            ("C-q", "leave"),
            ("C-d", "diff"),
        ] {
            let mut c = vec!["normal_mode".into()];
            c.extend(capture_commands(action, false));
            config.push_str(&format!("{key} = {}\n", serde_json::to_string(&c)?));
        }
        fs::write(self.config_file(), config)?;
        Ok(())
    }
    pub fn project(&mut self, path: &str) -> Result<PathBuf> {
        let file = crate::editing::safe_file(&self.manifest.root, path)?.canonicalize()?;
        if !self.manifest.bindings.iter().any(|b| b.file == file) {
            ensure!(self.manifest.bindings.len() < 128, "最多打开 128 个缓冲区");
            self.manifest.bindings.push(Binding {
                file: file.clone(),
                identity: None,
            });
            self.save_manifest()?;
        }
        Ok(file)
    }
    pub fn review(&mut self, b: &Buffer, repo: &Repository) -> Result<PathBuf> {
        let identity = Identity {
            task: b.task_id.clone(),
            node: b.node_id.clone(),
            path: b.path.clone(),
            version: b.version.clone(),
        };
        if let Some(binding) = self
            .manifest
            .bindings
            .iter()
            .find(|p| p.identity.as_ref() == Some(&identity))
        {
            return Ok(binding.file.clone());
        }
        ensure!(self.manifest.bindings.len() < 128, "最多打开 128 个缓冲区");
        let i = self.manifest.bindings.len();
        let parent = self.directory.join("reviews");
        conflict::private_dir(&parent)?;
        let cwd = parent.join(i.to_string());
        let snapshot = repo.resolve_snapshot(&b.version)?;
        let base = snapshot.parent.as_deref().unwrap_or(&snapshot.commit);
        repo.create_worktree_at(
            &cwd,
            &format!(
                "duet/helix-{}-{i}",
                self.directory.file_name().unwrap().to_string_lossy()
            ),
            base,
        )?;
        crate::checkpoint::validate_path(&b.path)?;
        let mut file = cwd.clone();
        for part in Path::new(&b.path).components() {
            file.push(part);
            if let Ok(m) = fs::symlink_metadata(&file) {
                ensure!(!m.file_type().is_symlink(), "草稿路径不允许符号链接");
            }
        }
        fs::create_dir_all(file.parent().unwrap())?;
        fs::write(&file, &b.text)?;
        let file = file.canonicalize()?;
        self.manifest.bindings.push(Binding {
            file: file.clone(),
            identity: Some(identity),
        });
        self.save_manifest()?;
        Ok(file)
    }
    pub fn binding<'a>(&'a self, path: &str, cwd: &Path) -> Result<&'a Binding> {
        let file = if Path::new(path).is_absolute() {
            PathBuf::from(path)
        } else {
            cwd.join(path)
        };
        let file = file.canonicalize().context("Helix 缓冲区路径不可验证")?;
        self.manifest
            .bindings
            .iter()
            .find(|b| b.file == file)
            .context("此 Helix 缓冲区不属于宿主管理的审核/项目文件；未发送代码")
    }
    pub fn request_open(
        &mut self,
        terminal: &mut crate::embedded::PtyEditor,
        file: &Path,
    ) -> Result<()> {
        ensure!(
            self.manifest.bindings.iter().any(|b| b.file == file),
            "文件未绑定"
        );
        json(&self.directory.join("open.json"), &file.to_path_buf())?;
        terminal.send(b"\x1b[24~")
    }
    pub fn begin(&mut self, terminal: &mut crate::embedded::PtyEditor, key: u8) -> Result<()> {
        ensure!(self.pending.is_none(), "正在读取编辑器，请稍候");
        for name in ["pending.json", "pending.tmp", "frame.json", "frame.tmp"] {
            match fs::remove_file(self.directory.join(name)) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
        self.pending = Some(Instant::now());
        terminal.send(&[key])
    }
    pub fn take_frame(&mut self) -> Result<Option<Frame>> {
        let file = self.directory.join("frame.json");
        if !file.exists() {
            if self.pending.is_some_and(|p| p.elapsed().as_secs() > 5) {
                self.pending = None;
                bail!("Helix 上下文读取超时；未发送/应用，不猜测代码位置");
            }
            return Ok(None);
        }
        let f = read_json(&file);
        fs::remove_file(file)?;
        self.pending = None;
        f.map(Some)
    }
    pub fn absorb(&mut self, frame: &Frame, app: &mut App, cwd: &Path) -> Result<Context> {
        ensure!(
            frame.text.len() <= MAX_BYTES && !frame.meta.multiple,
            "只支持单选区、最多 1 MiB；未发送代码"
        );
        let binding = self.binding(&frame.meta.path, cwd)?.clone();
        self.active = Some(binding.file.clone());
        let identity = binding
            .identity
            .context("项目 Helix 上下文讨论尚未开放；未发送代码")?;
        let bytes = app
            .review_buffers
            .iter()
            .map(|b| {
                b.base.len()
                    + b.original.len()
                    + if identity.matches(b) {
                        frame.text.len()
                    } else {
                        b.text.len()
                    }
            })
            .sum::<usize>();
        ensure!(
            bytes <= 24 * MAX_BYTES,
            "审核草稿总正文超过24 MiB；未捕获/发送，Helix 内容仍保留"
        );
        let b = app
            .review_buffers
            .iter_mut()
            .find(|b| identity.matches(b))
            .context("审核身份已失效")?;
        if b.text != frame.text {
            b.text = frame.text.clone();
            b.revision = b.revision.wrapping_add(1);
            b.rebuild();
        }
        let cursor = position(&b.text, frame.meta.line, frame.meta.column)?;
        let anchor = position(&b.text, frame.meta.anchor_line, frame.meta.anchor_column)?;
        let from = cursor.min(anchor);
        let end = cursor.max(anchor);
        let to = b.text[end..]
            .graphemes(true)
            .next()
            .map_or(end, |g| end + g.len());
        ensure!(
            b.text.get(from..to) == Some(frame.meta.selection.as_str()),
            "Helix 选区与全文不一致；未发送代码"
        );
        b.cursor = cursor;
        b.anchor = None;
        if frame.meta.select || frame.meta.selection.graphemes(true).count() > 1 {
            b.cursor = to;
            b.anchor = Some(from);
        }
        let mut context = b.context();
        if !frame.meta.select && frame.meta.selection.graphemes(true).count() <= 1 {
            let (from, to) = cursor_excerpt(&b.text, cursor);
            context.from = from;
            context.to = to;
            context.excerpt = b.text[from..to].into();
        }
        context.line = frame.meta.line;
        ensure!(
            frame.meta.action != "ask" || context.excerpt.len() <= 256 * 1024,
            "引用超过 256 KiB；未截断或发送"
        );
        app.review_revision = app.review_revision.wrapping_add(1);
        self.unobserved.remove(&binding.file);
        app.helix_unknown = !self.unobserved.is_empty();
        Ok(context)
    }
    pub fn stage_apply(
        &mut self,
        p: &Proposal,
        frame: &Frame,
        app: &mut App,
        cwd: &Path,
    ) -> Result<()> {
        let binding = self.binding(&frame.meta.path, cwd)?.clone();
        let id = binding.identity.as_ref().context("不是审核草稿")?;
        let b = app
            .review_buffers
            .iter()
            .find(|b| id.matches(b))
            .context("审核选择已改变")?;
        let mut next = b.clone();
        next.apply(p)?;
        ensure!(
            frame.text == p.context.buffer,
            "Helix 缓冲区已变化；未应用补丁"
        );
        json(
            &self.directory.join("replacement.json"),
            &Replacement {
                file: binding.file,
                before: frame.text.clone(),
                after: next.text,
            },
        )?;
        self.applying = true;
        Ok(())
    }
    pub fn sync_saved(directory: &Path, app: &mut App) -> Result<()> {
        let manifest: Manifest = read_json(&directory.join("manifest.json"))?;
        for binding in manifest.bindings {
            if let Some(id) = binding.identity {
                let file = binding.file;
                let m = fs::symlink_metadata(&file)?;
                ensure!(
                    m.is_file() && !m.file_type().is_symlink() && m.len() <= MAX_BYTES as u64,
                    "无效审核草稿文件"
                );
                let text = fs::read_to_string(file)?;
                if let Some(b) = app.review_buffers.iter_mut().find(|b| id.matches(b))
                    && b.text != text
                {
                    b.text = text;
                    b.revision = b.revision.wrapping_add(1);
                    b.rebuild();
                    app.review_revision = app.review_revision.wrapping_add(1);
                }
            }
        }
        Ok(())
    }
}
fn capture_commands(action: &str, select: bool) -> Vec<String> {
    let mut commands = vec![
        "save_selection".into(),
        format!(
            ":pipe-to \"__duet_meta|{action}|{}|%{{cursor_line}}|%{{cursor_column}}|%{{buffer_name}}\"",
            usize::from(select)
        ),
        "flip_selections".into(),
        ":pipe-to \"__duet_anchor|%{cursor_line}|%{cursor_column}\"".into(),
        "flip_selections".into(),
        "select_all".into(),
        ":pipe-to \"__duet_buffer|%{buffer_name}\"".into(),
        "jump_backward".into(),
    ];
    if matches!(action, "publish" | "save-back") {
        commands.insert(0, ":write".into());
    }
    commands
}
pub fn cursor_excerpt(text: &str, cursor: usize) -> (usize, usize) {
    use unicode_width::UnicodeWidthChar;
    let start = text[..cursor].rfind('\n').map_or(0, |p| p + 1);
    let end = text[cursor..].find('\n').map_or(text.len(), |p| cursor + p);
    let word = |g: &str| {
        g.chars().any(|c| c.is_alphanumeric() || c == '_')
            && g.chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c.width() == Some(0))
    };
    if !text[cursor..end].graphemes(true).next().is_some_and(word) {
        return (start, end);
    }
    let mut from = cursor;
    for (p, g) in text[start..cursor].grapheme_indices(true).rev() {
        if !word(g) {
            break;
        }
        from = start + p;
    }
    let mut to = cursor;
    for g in text[cursor..end].graphemes(true) {
        if !word(g) {
            break;
        }
        to += g.len();
    }
    (from, to)
}
pub fn position(text: &str, line: usize, column: usize) -> Result<usize> {
    ensure!(line > 0 && column > 0, "无效 Helix 光标");
    let start = if line == 1 {
        0
    } else {
        text.match_indices('\n')
            .nth(line - 2)
            .map(|(i, _)| i + 1)
            .context("光标行越界")?
    };
    let end = text[start..].find('\n').map_or(text.len(), |i| start + i);
    let p = text[start..end]
        .grapheme_indices(true)
        .nth(column - 1)
        .map(|(i, _)| start + i)
        .or_else(|| (column - 1 == text[start..end].graphemes(true).count()).then_some(end))
        .context("光标列越界")?;
    Ok(p)
}
/// Launched only by the child editor. Editor expansions are passed as one raw argv, NOT evaluated by a shell.
pub fn helper(directory: &Path, command: &str) -> Result<()> {
    let command = if command.starts_with("\"__duet_") {
        command
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .context("Invalid quoted bridge command")?
    } else {
        command
    };
    if !command.starts_with("__duet_") {
        let status = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(command)
            .status()?;
        ensure!(status.success(), "Helix shell command failed");
        return Ok(());
    }
    if command == "__duet_open" {
        let file: PathBuf = read_json(&directory.join("open.json"))?;
        let manifest: Manifest = read_json(&directory.join("manifest.json"))?;
        ensure!(
            manifest.bindings.iter().any(|b| b.file == file),
            "Open target not bound"
        );
        std::io::stdout().write_all(file.to_str().context("非UTF-8路径")?.as_bytes())?;
        return Ok(());
    }
    let pending = directory.join("pending.json");
    let mut input = String::new();
    std::io::stdin()
        .take(MAX_BYTES as u64 + 1)
        .read_to_string(&mut input)?;
    ensure!(input.len() <= MAX_BYTES, "Helix bridge input exceeds 1 MiB");
    let limit = if command.starts_with("__duet_meta|") {
        6
    } else if command.starts_with("__duet_anchor|") {
        3
    } else {
        2
    };
    let fields = command.splitn(limit, '|').collect::<Vec<_>>();
    match fields[0] {
        "__duet_meta" => {
            ensure!(fields.len() == 6, "Invalid metadata");
            if pending.exists() {
                let mut m: Meta = read_json(&pending)?;
                m.multiple = true;
                json(&pending, &m)?;
            } else {
                json(
                    &pending,
                    &Meta {
                        action: fields[1].into(),
                        select: fields[2] == "1",
                        line: fields[3].parse()?,
                        column: fields[4].parse()?,
                        path: fields[5].into(),
                        selection: input,
                        anchor_line: 0,
                        anchor_column: 0,
                        multiple: false,
                    },
                )?;
            }
        }
        "__duet_anchor" => {
            ensure!(fields.len() == 3, "Invalid anchor");
            let mut m: Meta = read_json(&pending)?;
            m.anchor_line = fields[1].parse()?;
            m.anchor_column = fields[2].parse()?;
            ensure!(input == m.selection, "Selection changed");
            json(&pending, &m)?;
        }
        "__duet_buffer" => {
            ensure!(fields.len() == 2, "Invalid buffer identity");
            let m: Meta = read_json(&pending)?;
            ensure!(m.path == fields[1], "Buffer changed during capture");
            json(
                &directory.join("frame.json"),
                &Frame {
                    meta: m,
                    text: input,
                },
            )?;
            fs::remove_file(pending)?;
        }
        "__duet_replace" => {
            ensure!(fields.len() == 2, "Invalid replacement identity");
            let r: Replacement = read_json(&directory.join("replacement.json"))?;
            let file = Path::new(fields[1]).canonicalize()?;
            ensure!(
                file == r.file && input == r.before,
                "Stale Helix buffer: patch rejected"
            );
            std::io::stdout().write_all(r.after.as_bytes())?;
            fs::remove_file(directory.join("replacement.json"))?;
        }
        _ => bail!("Unknown bridge command"),
    }
    Ok(())
}
pub fn open_command(file: &Path) -> Result<Vec<u8>> {
    let s = file.to_str().context("非 UTF-8 路径")?;
    ensure!(!s.chars().any(char::is_control), "路径含控制字符");
    let q = if !s.contains('\'') {
        format!("'{s}'")
    } else if !s.contains('`') {
        format!("`{s}`")
    } else {
        bail!("文件名同时含单引号和反引号，暂不支持宿主切换")
    };
    Ok(format!("\x1b\x1b:open {q}\r").into_bytes())
}
fn theme(light: bool) -> String {
    let (bg, fg, muted, accent, border, good, warn, removed) = if light {
        (
            "#f5f6fa", "#1f2937", "#5a6577", "#374daa", "#adb7cc", "#16704a", "#995b0c", "#a82d45",
        )
    } else {
        (
            "#11151e", "#dce2ed", "#8f9cb0", "#90a8ff", "#354157", "#7acda9", "#eabe74", "#e7889c",
        )
    };
    format!(
        "inherits = \"base16_default_{}\"\n\"ui.background\" = {{fg=\"fg\",bg=\"bg\"}}\n\"ui.text\" = \"fg\"\n\"ui.text.focus\" = \"accent\"\n\"ui.linenr\" = \"muted\"\n\"ui.linenr.selected\" = \"accent\"\n\"ui.statusline\" = {{fg=\"fg\",bg=\"border\"}}\n\"ui.statusline.inactive\" = {{fg=\"muted\",bg=\"bg\"}}\n\"ui.popup\" = {{fg=\"fg\",bg=\"bg\"}}\n\"ui.menu\" = {{fg=\"fg\",bg=\"bg\"}}\n\"ui.menu.selected\" = {{fg=\"bg\",bg=\"accent\"}}\n\"ui.selection\" = {{bg=\"border\"}}\n\"ui.cursor\" = {{fg=\"bg\",bg=\"accent\"}}\n\"ui.virtual.whitespace\" = \"border\"\n\"comment\" = \"muted\"\n\"keyword\" = \"accent\"\n\"function\" = \"accent\"\n\"string\" = \"good\"\n\"constant\" = \"warn\"\n\"type\" = \"warn\"\n\"diff.plus\" = \"good\"\n\"diff.minus\" = \"removed\"\n\"diff.delta\" = \"warn\"\n[palette]\nbg=\"{bg}\"\nfg=\"{fg}\"\nmuted=\"{muted}\"\naccent=\"{accent}\"\nborder=\"{border}\"\ngood=\"{good}\"\nwarn=\"{warn}\"\nremoved=\"{removed}\"\n",
        if light { "light" } else { "dark" }
    )
}
