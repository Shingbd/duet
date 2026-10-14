use anyhow::{Context, Result, ensure};
use clap::{Parser, Subcommand};
use crossterm::{
    event::{self, DisableBracketedPaste, EnableBracketedPaste, Event},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use duet_workbench::{
    app::{App, Mode, RunControl},
    checkpoint::Repository,
    coding::CodingRun,
    config::{Config, Language, Theme},
    keymap::Keymap,
    rpc::PiClient,
    storage::WorkspaceStore,
    ui,
};
use ratatui::{Terminal, backend::CrosstermBackend};
use std::{
    io::{self, IsTerminal},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[derive(Parser)]
#[command(version, about = "Duet: local-first human-AI development workbench")]
struct Cli {
    #[arg(long, global = true)]
    keymap: Option<PathBuf>,
    #[arg(long, global = true)]
    helix: Option<PathBuf>,
    #[arg(long, global = true, value_enum)]
    review_editor: Option<duet_workbench::config::ReviewEditor>,
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[arg(long, global = true, value_enum)]
    language: Option<Language>,
    #[arg(long, global = true, value_enum)]
    theme: Option<Theme>,
    #[command(subcommand)]
    command: Option<Command>,
}
#[derive(Subcommand)]
enum ConfigCommand {
    /// Explicitly create the default preferences without overwriting a file.
    Init {
        #[arg(default_value = "duet.json")]
        path: PathBuf,
    },
    /// Print effective preferences; does not open a workspace or call Pi.
    Show {
        #[arg(default_value = ".")]
        project: PathBuf,
    },
}
#[derive(Subcommand)]
enum Command {
    #[command(hide = true)]
    HelixBridge {
        directory: PathBuf,
        #[arg(allow_hyphen_values = true)]
        command: String,
    },
    /// Preferences only: no implicit model calls or write permissions.
    Config {
        #[command(subcommand)]
        action: ConfigCommand,
    },
    /// 打开当前文件夹。
    #[command(alias = "demo")]
    Workspace,
    /// 打开项目；首次自动创建 .duet。
    Open {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// 初始化项目本地数据（无需交互终端）。
    Init {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// 浏览固定 Git 版本。
    Browse {
        #[arg(long)]
        repo: PathBuf,
        #[arg(long)]
        snapshot: Option<String>,
    },
    /// 在独立 worktree 中执行 AI 编码；不会自动合并。
    Code {
        #[arg(long)]
        repo: PathBuf,
        #[arg(long)]
        prompt: String,
        /// Model may read/write with this OS account's permissions.
        #[arg(long)]
        allow_write: bool,
        #[arg(long)]
        pi: Option<PathBuf>,
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        timeout_secs: Option<u64>,
        #[arg(long)]
        headless: bool,
    },
    Snapshot {
        #[arg(long)]
        repo: PathBuf,
        #[arg(long,required=true,num_args=1..)]
        paths: Vec<String>,
    },
    /// Pi RPC connectivity only; never sends a prompt.
    PiProbe {
        #[arg(long)]
        repo: PathBuf,
        #[arg(long)]
        pi: Option<PathBuf>,
    },
}
fn main() -> Result<()> {
    let cli = Cli::parse();
    let command = cli.command.unwrap_or(Command::Workspace);
    if let Command::HelixBridge { directory, command } = &command {
        return duet_workbench::helix_workspace::helper(directory, command);
    }
    if let Command::Config {
        action: ConfigCommand::Init { path },
    } = &command
    {
        Config::initialize(path)?;
        println!("{}", path.display());
        return Ok(());
    }
    let project = match &command {
        Command::Open { path } | Command::Init { path } => path.as_path(),
        Command::Browse { repo, .. }
        | Command::Code { repo, .. }
        | Command::Snapshot { repo, .. }
        | Command::PiProbe { repo, .. } => repo.as_path(),
        Command::Config {
            action: ConfigCommand::Show { project },
        } => project.as_path(),
        _ => Path::new("."),
    };
    let mut config = Config::load(project, cli.config.as_deref())?;
    if let Some(editor) = cli.review_editor {
        config.review_editor = editor;
    }
    if let Some(language) = cli.language {
        config.language = language;
    }
    if let Some(theme) = cli.theme {
        config.theme = theme;
    }
    if let Some(helix) = cli.helix {
        config.helix_path = if helix.is_absolute() || helix.components().count() == 1 {
            helix
        } else {
            std::env::current_dir()?.join(helix)
        };
    }
    if let Some(keymap) = cli.keymap {
        config.keymap_file = Some(keymap);
    }
    match &command {
        Command::Code {
            pi,
            model,
            timeout_secs,
            ..
        } => {
            if let Some(pi) = pi {
                config.agent.pi_path = pi.clone();
            }
            if let Some(model) = model {
                config.agent.model = Some(model.clone());
            }
            if let Some(timeout) = timeout_secs {
                config.agent.timeout_secs = *timeout;
            }
        }
        Command::PiProbe { pi: Some(pi), .. } => config.agent.pi_path = pi.clone(),
        _ => {}
    }
    config.validate()?;
    if matches!(command, Command::Config { .. }) {
        println!("{}", serde_json::to_string_pretty(&config)?);
        return Ok(());
    }
    let keymap = match &config.keymap_file {
        Some(path) => Keymap::load(path)?,
        None => Keymap::default(),
    };
    match command {
        Command::Workspace => open_project(Path::new("."), keymap, config),
        Command::Open { path } => open_project(&path, keymap, config),
        Command::Init { path } => {
            let store = WorkspaceStore::open(&path)?;
            println!("{}", store.directory().display());
            Ok(())
        }
        Command::Browse { repo, snapshot } => {
            let repo = Repository::open(&repo)?;
            let snapshot = match snapshot {
                Some(revision) => repo.resolve_snapshot(&revision)?,
                None => repo.baseline()?,
            };
            let store = WorkspaceStore::open(repo.root())?;
            let mut app = App::browse(&repo, snapshot, keymap)?;
            store.restore(&mut app)?;
            config.apply(&mut app);
            run(app, store, None)
        }
        Command::Code {
            repo,
            prompt,
            allow_write,
            headless,
            ..
        } => {
            ensure!(
                allow_write,
                "编码需要 --allow-write 明确授权。模型请求可能外发代码；worktree 不是安全沙箱。"
            );
            if !headless {
                require_terminal()?;
            }
            let repo = Repository::open(&repo)?;
            let mut store = WorkspaceStore::open(repo.root())?;
            let mut app = App::browse(&repo, repo.baseline()?, keymap)?;
            store.restore(&mut app)?;
            config.apply(&mut app);
            let mut coding = CodingRun::start_planning(
                repo.root(),
                &config.agent.pi_path,
                config.agent.model.as_deref(),
                &prompt,
                Duration::from_secs(config.agent.timeout_secs),
                &mut store,
                &mut app,
            )?;
            if headless {
                loop {
                    coding.poll(&mut app, &mut store)?;
                    if coding.is_finished() || coding.is_waiting() {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                store.save(&app)?;
                println!(
                    "status={:?}\nbranch={}\nworktree={}\ncheckpoint={}",
                    app.run.status,
                    coding.branch,
                    coding.worktree.display(),
                    coding.checkpoint.as_deref().unwrap_or("none")
                );
                ensure!(
                    app.run.status == duet_workbench::rpc::RunStatus::Settled,
                    "{}",
                    app.run.error.as_deref().unwrap_or("编码未完成")
                );
                Ok(())
            } else {
                run(app, store, Some(coding))
            }
        }
        Command::Snapshot { repo, paths } => {
            let repo = Repository::open(&repo)?;
            let snapshot = repo.capture(&paths)?;
            println!("{}", serde_json::to_string_pretty(&snapshot)?);
            Ok(())
        }
        Command::PiProbe { repo, .. } => {
            let repo = Repository::open(&repo)?;
            let mut client = PiClient::spawn(&config.agent.pi_path, repo.root())?;
            let state = client.get_state(Duration::from_secs(20))?;
            println!(
                "Pi RPC connected; session={} streaming={}; no prompt sent",
                state["sessionId"].as_str().unwrap_or("unknown"),
                state["isStreaming"].as_bool().unwrap_or(false)
            );
            client.shutdown()
        }
        Command::Config { .. } | Command::HelixBridge { .. } => unreachable!(),
    }
}

fn open_project(path: &Path, keymap: Keymap, config: Config) -> Result<()> {
    require_terminal()?;
    let root = path.canonicalize()?;
    let store = WorkspaceStore::open(&root)?;
    let mut app = match Repository::open(&root) {
        Ok(repo) => match repo.baseline() {
            Ok(snapshot) => App::browse(&repo, snapshot, keymap)?,
            Err(_) => App::empty(keymap),
        },
        Err(_) => App::empty(keymap),
    };
    app.name = root
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into();
    store.restore(&mut app)?;
    config.apply(&mut app);
    run(app, store, None)
}

fn require_terminal() -> Result<()> {
    ensure!(
        io::stdin().is_terminal() && io::stdout().is_terminal(),
        "TUI 需要交互终端"
    );
    Ok(())
}

struct TerminalGuard;
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), DisableBracketedPaste, LeaveAlternateScreen);
    }
}

fn run(mut app: App, mut store: WorkspaceStore, mut coding: Option<CodingRun>) -> Result<()> {
    require_terminal()?;
    let config = app.config.clone();
    enable_raw_mode().context("无法进入终端 raw mode")?;
    let _guard = TerminalGuard;
    execute!(io::stdout(), EnterAlternateScreen, EnableBracketedPaste)?;
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), DisableBracketedPaste, LeaveAlternateScreen);
        previous(info);
    }));
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let mut renderer = ui::Renderer::default();
    let mut redraw = true;
    let mut save_since: Option<Instant> = None;
    let mut last_input = Instant::now();
    let mut advisor: Option<duet_workbench::conflict_ai::Advisor> = None;
    let mut editor: Option<duet_workbench::editing::Session> = None;
    let mut verification: Option<duet_workbench::verification::Run> = None;
    duet_workbench::verification::recover(&mut app);
    app.helix_review = config.review_editor == duet_workbench::config::ReviewEditor::Helix;
    let mut review_runtime = duet_workbench::review_chat::Runtime::default();
    let review_repo = Repository::open(store.directory().parent().unwrap()).ok();
    if let Err(error) = app
        .project_tree
        .refresh(store.directory().parent().unwrap())
    {
        app.status = format!("文件树不可用：{error}");
    }
    if config.helix_startup && app.helix_review && coding.as_ref().is_none_or(|c| c.is_finished()) {
        if let Some(repo) = &review_repo {
            let _ = duet_workbench::review_workflow::prepare(&mut app, repo);
        }
        let size = terminal.size()?;
        let (rows, cols) = ui::editor_size_for(
            &app,
            ratatui::layout::Rect::new(0, 0, size.width, size.height),
        );
        match duet_workbench::editing::startup(&mut app, &store, rows, cols) {
            Ok(s) => {
                app.editor_screen = Some(s.terminal.screen().clone());
                editor = Some(s);
            }
            Err(e) => {
                app.status = format!("Helix 启动失败（不重试）：{e}");
                app.editor_focused = false;
            }
        }
    }
    loop {
        if std::mem::take(&mut app.requested_verification_cancel)
            && let Some(run) = &mut verification
        {
            run.cancel();
        }
        if let Some(run) = &mut verification {
            if run.poll(&mut app, &mut store)? {
                redraw = true;
            }
            if run.finished() {
                app.status = format!("测试 {} · 不自动批准/合入", run.record.outcome.label());
                verification = None;
                redraw = true;
            }
        }
        if let Some(request) = app.requested_verification.take() {
            let result = (|| -> Result<duet_workbench::verification::Run> {
                ensure!(
                    verification.is_none()
                        && advisor.is_none()
                        && review_runtime.chat.is_none()
                        && coding.as_ref().is_none_or(|c| c.is_finished()),
                    "请先停止当前测试/AI会话；没有执行新命令"
                );
                duet_workbench::verification::Run::start(request, &mut app, &mut store)
            })();
            match result {
                Ok(run) => {
                    app.status =
                        "测试运行中 · Ctrl+X / :verify-stop 停止 · 固定交付，不含草稿".into();
                    verification = Some(run);
                }
                Err(e) => app.status = format!("未运行测试：{e}"),
            };
            redraw = true;
        }
        app.verification_active = verification.is_some();
        if review_runtime.poll(&mut app, &mut store)? {
            redraw = true;
            save_since.get_or_insert_with(Instant::now);
        }
        if let Some(session) = &mut editor {
            let size = terminal.size()?;
            let (rows, cols) = ui::editor_size_for(
                &app,
                ratatui::layout::Rect::new(0, 0, size.width, size.height),
            );
            match duet_workbench::editing::poll_bridge(session, &mut app) {
                Ok(changed) => {
                    if changed {
                        redraw = true;
                        save_since.get_or_insert_with(Instant::now);
                    }
                }
                Err(error) => {
                    app.status = format!("Helix 桥接拒绝：{error}");
                    app.editor_focused = false;
                    app.panel = duet_workbench::app::Panel::Queue;
                    app.helix_unknown = true;
                    redraw = true;
                }
            }
            let activity = session
                .terminal
                .resize(rows, cols)
                .and_then(|changed| session.terminal.poll().map(|data| changed || data));
            match activity {
                Ok(changed) => {
                    if changed && session.terminal.exit_status().is_none() {
                        app.editor_screen = Some(session.terminal.screen().clone());
                        redraw = true;
                    }
                }
                Err(error) => {
                    app.status = format!("内嵌编辑器通信失败：{error}；已保存改动保留");
                    let _ = session.terminal.stop();
                }
            }
            if let Some(status) = session.terminal.exit_status() {
                let keep = app.config.helix_keepalive
                    && status.success()
                    && !matches!(
                        session.target,
                        duet_workbench::editing::Target::Integration { .. }
                    );
                let resumed = if keep {
                    match duet_workbench::editing::reopen(session, &mut app, rows, cols) {
                        Ok(()) => true,
                        Err(error) => {
                            app.status = format!("Helix 重建失败：{error}；不会重试");
                            false
                        }
                    }
                } else {
                    false
                };
                if !resumed {
                    let session = editor.take().unwrap();
                    app.requested_review_helix = false;
                    app.editor_screen = None;
                    app.editor_focused = false;
                    if let Err(error) = duet_workbench::editing::finish(
                        session.target,
                        status.success(),
                        &mut app,
                        &store,
                    ) {
                        app.status = format!(
                            "编辑器已退出，固定版本捕获未完成：{error}；草稿/备份保留，不会自动批准或合入"
                        );
                    }
                }
                redraw = true;
            }
        }
        if app.requested_review_publish || app.requested_review_chat.is_some() {
            review_runtime.interact(
                &mut app,
                &mut store,
                &config.agent,
                advisor.is_some()
                    || editor.as_ref().is_some_and(|s| {
                        matches!(
                            s.target,
                            duet_workbench::editing::Target::Integration { .. }
                        )
                    })
                    || coding.as_ref().is_some_and(|c| !c.is_finished()),
            )?;
            redraw = true;
            save_since.get_or_insert_with(Instant::now);
        }
        if app.quit && editor.is_some() {
            app.quit = false;
            app.mode = Mode::ConfirmStopEditor;
            redraw = true;
        }
        if let Some(a) = &mut advisor {
            let result = a.poll();
            app.run = a.state.clone();
            redraw = true;
            match result {
                Ok(None) => {}
                outcome => {
                    let mut a = advisor.take().unwrap();
                    let _ = a.stop();
                    app.agent_connected = false;
                    let repo = Repository::open(store.directory().parent().unwrap())?;
                    let mut draft = duet_workbench::conflict::load(&store, &a.draft.task_id)?
                        .context("集成草稿不存在")?;
                    let task = app
                        .tasks
                        .iter()
                        .find(|t| t.id == draft.task_id)
                        .context("任务不存在")?;
                    match outcome {
                        Ok(Some(report)) => {
                            if let Err(e) =
                                duet_workbench::conflict::validate(&repo, &store, task, &draft)
                            {
                                draft.advice_error = Some(e.to_string());
                            } else if draft.snapshot != a.draft.snapshot {
                                draft.advice_error =
                                    Some("草稿在分析期间变化，建议未应用，请重新分析".into());
                            } else {
                                if let Some(previous) = &mut draft.advice {
                                    previous
                                        .files
                                        .retain(|f| !report.files.iter().any(|n| n.path == f.path));
                                }
                                let mut report = report;
                                if let Some(previous) = draft.advice.take() {
                                    report.files.extend(previous.files);
                                }
                                draft.advice = Some(report);
                                draft.advice_error = None;
                            }
                        }
                        Err(e) => {
                            draft.advice_error = Some(e.to_string().chars().take(2000).collect());
                            app.run.status = duet_workbench::rpc::RunStatus::Failed;
                        }
                        _ => {}
                    }
                    duet_workbench::conflict::save(&store, &draft)?;
                    if matches!(
                        app.mode,
                        Mode::Conflict { .. } | Mode::ConfirmConflict { .. }
                    ) {
                        let body = duet_workbench::conflict::display(&repo, &draft, a.index, 4)?;
                        app.mode = Mode::Conflict {
                            draft,
                            index: a.index,
                            side: 4,
                            scroll: 0,
                            body,
                        };
                    }
                    app.status = "AI 分析已结束 · 只提出建议，未修改文件 · 人类决定".into();
                }
            }
        }
        if let Some(coding) = &mut coding
            && coding.poll(&mut app, &mut store)?
        {
            redraw = true;
            save_since.get_or_insert_with(Instant::now);
        }
        if redraw {
            if let Some(repo) = &review_repo
                && let Err(error) = duet_workbench::review_workflow::prepare(&mut app, repo)
            {
                app.status = format!("审核编辑器不可用：{error}");
            }
            if app.helix_review
                && app.requested_review_helix
                && app.view == duet_workbench::app::View::Review
                && matches!(app.mode, Mode::Browse)
                && !app.editor_focused
                && app.current_buffer().is_some()
            {
                let result = (|| -> Result<()> {
                    ensure!(
                        advisor.is_none() && coding.as_ref().is_none_or(|c| c.is_finished()),
                        "AI 写入/顾问仍在运行；当前固定代码只读，请结束后按 2 打开 Helix"
                    );
                    let repo = review_repo.as_ref().context("审核需要 Git")?;
                    if let Some(s) = &mut editor {
                        if s.workspace.pending.is_none() {
                            duet_workbench::editing::switch_review(s, &mut app, repo)?;
                        }
                    } else {
                        let focus = app.panel == duet_workbench::app::Panel::Reader;
                        let size = terminal.size()?;
                        let (rows, cols) = ui::editor_size_for(
                            &app,
                            ratatui::layout::Rect::new(0, 0, size.width, size.height),
                        );
                        let s = duet_workbench::editing::start(
                            duet_workbench::embedded::Selection::ReviewBuffer,
                            &mut app,
                            &store,
                            rows,
                            cols,
                        )?;
                        app.editor_screen = Some(s.terminal.screen().clone());
                        app.editor_project = false;
                        app.editor_focused = focus;
                        if !focus {
                            app.panel = duet_workbench::app::Panel::Queue;
                        }
                        editor = Some(s);
                    }
                    Ok(())
                })();
                if let Err(error) = result {
                    app.status = format!("Helix 审核不可用：{error}");
                    app.requested_review_helix = false;
                    app.editor_focused = false;
                }
            }
            let size = terminal.size()?;
            let (rows, cols) = ui::editor_size_for(
                &app,
                ratatui::layout::Rect::new(0, 0, size.width, size.height),
            );
            let chat_inline = app.review_chat_open && (app.fullscreen || size.width < 116);
            let compare = app.review_compare;
            if !compare && let Some(buffer) = app.current_buffer_mut() {
                buffer.keep_visible(
                    if chat_inline {
                        (rows as usize * 65 / 100).saturating_sub(2)
                    } else {
                        (rows as usize).saturating_sub(1)
                    },
                    (cols as usize).saturating_sub(13),
                );
            }
            terminal.draw(|frame| renderer.draw(frame, &app))?;
            redraw = false;
        }
        // Paint first; persist batched navigation only after idle or a bounded delay.
        if save_since.is_some_and(|since| {
            since.elapsed() >= Duration::from_millis(config.performance.save_max_delay_ms)
                || last_input.elapsed() >= Duration::from_millis(config.performance.save_delay_ms)
        }) {
            store.save(&app)?;
            save_since = None;
            if app.status == "笔记已记录 · 仅本次会话" {
                app.status = "笔记已保存".into();
                redraw = true;
            }
        }
        if app.quit {
            break;
        }
        let wait = if verification.is_some()
            || review_runtime.chat.is_some()
            || editor.is_some()
            || advisor.is_some()
            || coding.as_ref().is_some_and(|run| !run.is_finished())
                && app.run.status != duet_workbench::rpc::RunStatus::Paused
        {
            Duration::from_millis(config.performance.active_poll_ms)
        } else if save_since.is_some() {
            Duration::from_millis(50)
        } else {
            Duration::from_millis(config.performance.idle_poll_ms)
        };
        let input_ready = if let Some(s) = &editor {
            if event::poll(Duration::ZERO)? {
                true
            } else {
                s.terminal.wait_io(wait)?;
                event::poll(Duration::ZERO)?
            }
        } else {
            event::poll(wait)?
        };
        if input_ready {
            let mut host_changed = false;
            // Drain queued input before drawing so key repeats/pastes don't queue frames.
            for _ in 0..64 {
                let mut changed = true;
                match event::read()? {
                    Event::Key(key)
                        if verification.is_some()
                            && !app.editor_focused
                            && key
                                .modifiers
                                .contains(crossterm::event::KeyModifiers::CONTROL)
                            && key.code == crossterm::event::KeyCode::Char('x') =>
                    {
                        app.requested_verification_cancel = true;
                    }
                    Event::Key(_)
                        if editor
                            .as_ref()
                            .is_some_and(|s| s.workspace.pending.is_some()) =>
                    {
                        changed = false;
                    }
                    Event::Key(key)
                        if matches!(app.mode, Mode::ConfirmReviewPatch { .. })
                            && editor.is_some()
                            && key.code == crossterm::event::KeyCode::Enter =>
                    {
                        if let Mode::ConfirmReviewPatch { proposal } = &app.mode {
                            let s = editor.as_mut().unwrap();
                            s.workspace.confirm = Some(*proposal.clone());
                            s.workspace.begin(&mut s.terminal, 0x10)?;
                            app.mode = Mode::Browse;
                        }
                    }
                    Event::Key(key) if app.editor_focused && editor.is_some() => {
                        changed = false;
                        editor
                            .as_mut()
                            .unwrap()
                            .workspace
                            .mark_input(app.view == duet_workbench::app::View::Project);
                        app.helix_unknown =
                            !editor.as_ref().unwrap().workspace.unobserved.is_empty();
                        if key
                            .modifiers
                            .contains(crossterm::event::KeyModifiers::CONTROL)
                            && matches!(key.code, crossterm::event::KeyCode::Char(']' | '5'))
                        {
                            let s = editor.as_mut().unwrap();
                            if !matches!(
                                s.target,
                                duet_workbench::editing::Target::Integration { .. }
                            ) && !s.workspace.manifest.bindings.is_empty()
                            {
                                s.workspace.begin(&mut s.terminal, 0x11)?;
                            }
                            changed = true;
                            app.editor_focused = false;
                            app.fullscreen = false;
                            app.panel = duet_workbench::app::Panel::Queue;
                            app.status =
                                "焦点回 Duet · Tab 返回 Helix · 实现/合入前 :editor-stop 明确停止"
                                    .into();
                        } else if app.view == duet_workbench::app::View::Review
                            && key
                                .modifiers
                                .contains(crossterm::event::KeyModifiers::CONTROL)
                            && matches!(
                                key.code,
                                crossterm::event::KeyCode::Char('g' | 's' | 'p' | 'd')
                            )
                        {
                            changed = true;
                            let n = match key.code {
                                crossterm::event::KeyCode::Char('g') => 7,
                                crossterm::event::KeyCode::Char('s') => 19,
                                crossterm::event::KeyCode::Char('p') => 16,
                                _ => 4,
                            };
                            let s = editor.as_mut().unwrap();
                            s.workspace.begin(&mut s.terminal, n)?;
                        } else if let Err(error) = editor.as_mut().unwrap().terminal.key(key) {
                            changed = true;
                            app.status = error.to_string();
                        }
                    }
                    Event::Paste(_)
                        if editor
                            .as_ref()
                            .is_some_and(|s| s.workspace.pending.is_some()) =>
                    {
                        changed = false;
                    }
                    Event::Paste(text) if app.editor_focused && editor.is_some() => {
                        changed = false;
                        editor
                            .as_mut()
                            .unwrap()
                            .workspace
                            .mark_input(app.view == duet_workbench::app::View::Project);
                        app.helix_unknown =
                            !editor.as_ref().unwrap().workspace.unobserved.is_empty();
                        if let Err(error) = editor.as_mut().unwrap().terminal.paste(&text) {
                            changed = true;
                            app.status = error.to_string();
                        }
                    }
                    Event::Key(key) => app.handle_key(key),
                    Event::Paste(text) => {
                        if let Mode::Input { text: input, .. }
                        | Mode::Message { text: input }
                        | Mode::ReviewChat { text: input } = &mut app.mode
                        {
                            input.push_str(&text);
                        } else if app.view == duet_workbench::app::View::Review
                            && app.panel == duet_workbench::app::Panel::Reader
                            && app.editor_screen.is_none()
                            && matches!(app.mode, Mode::Browse)
                            && let Some(buffer) = app.current_buffer_mut()
                        {
                            if let Err(error) = buffer.paste(&text) {
                                app.status = error.to_string();
                            }
                            app.review_revision = app.review_revision.wrapping_add(1);
                        }
                    }
                    _ => {}
                }
                host_changed |= changed;
                if app.quit || !event::poll(Duration::ZERO)? {
                    break;
                }
            }
            last_input = Instant::now();
            redraw |= host_changed;
            if host_changed {
                save_since.get_or_insert(last_input);
            }
            if std::mem::take(&mut app.requested_helix_context)
                && let Some(s) = &mut editor
                && let Err(e) = s.workspace.begin(&mut s.terminal, 7)
            {
                app.status = format!("未读取上下文：{e}");
            }
            review_runtime.interact(
                &mut app,
                &mut store,
                &config.agent,
                advisor.is_some()
                    || editor.as_ref().is_some_and(|s| {
                        matches!(
                            s.target,
                            duet_workbench::editing::Target::Integration { .. }
                        )
                    })
                    || coding.as_ref().is_some_and(|c| !c.is_finished()),
            )?;
            if std::mem::take(&mut app.requested_editor_stop)
                && let Some(mut session) = editor.take()
            {
                app.requested_editor_stop = false;
                app.requested_review_helix = false;
                session.terminal.stop()?;
                app.editor_screen = None;
                app.editor_focused = false;
                if let Err(error) =
                    duet_workbench::editing::finish(session.target, false, &mut app, &store)
                {
                    app.status = format!("编辑器停止，改动保留，捕获未完成：{error}");
                }
            }
            if std::mem::take(&mut app.requested_tree_refresh)
                && let Err(error) = app
                    .project_tree
                    .refresh(store.directory().parent().unwrap())
            {
                app.status = format!("文件树刷新失败：{error}");
            }
            if let Some(mut path) = app.requested_project_open.take() {
                if path.is_empty()
                    || !app
                        .project_tree
                        .entries
                        .iter()
                        .any(|e| e.path == path && !e.directory)
                {
                    path = app
                        .project_tree
                        .entries
                        .iter()
                        .find(|e| !e.directory && e.status != 'D')
                        .map(|e| e.path.clone())
                        .unwrap_or_default();
                }
                if !path.is_empty() {
                    app.project_tree.reveal(&path);
                    let focus = app.panel == duet_workbench::app::Panel::Reader;
                    if let Some(session) = &mut editor {
                        match duet_workbench::editing::switch_project(session, &store, &path) {
                            Ok(()) => {
                                app.status =
                                    format!("Helix 打开 {path} · 原缓冲区保留，未自动保存");
                                app.panel = if focus {
                                    duet_workbench::app::Panel::Reader
                                } else {
                                    duet_workbench::app::Panel::Queue
                                };
                                app.editor_focused = focus;
                                app.editor_project = true;
                                app.review_compare = false;
                            }
                            Err(error) => app.status = format!("未切换文件：{error}"),
                        }
                    } else {
                        app.requested_editor =
                            Some(duet_workbench::embedded::Selection::ProjectFile { path });
                    }
                }
            }
            if let Some(selection) = app.requested_editor.take() {
                let seamless = matches!(
                    &selection,
                    duet_workbench::embedded::Selection::ProjectFile { .. }
                        | duet_workbench::embedded::Selection::ReviewBuffer
                );
                let focus = app.panel == duet_workbench::app::Panel::Reader;
                let result = (|| -> Result<()> {
                    ensure!(editor.is_none(), "编辑器已在运行，Tab 返回");
                    ensure!(
                        advisor.is_none()
                            && review_runtime.chat.is_none()
                            && coding.as_ref().is_none_or(|c| c.is_finished()),
                        "请先结束 AI 执行/讨论；人工编辑器与模型运行互斥"
                    );
                    let size = terminal.size()?;
                    let (rows, cols) = ui::editor_size_for(
                        &app,
                        ratatui::layout::Rect::new(0, 0, size.width, size.height),
                    );
                    let session =
                        duet_workbench::editing::start(selection, &mut app, &store, rows, cols)?;
                    app.editor_screen = Some(session.terminal.screen().clone());
                    editor = Some(session);
                    if seamless {
                        app.panel = if focus {
                            duet_workbench::app::Panel::Reader
                        } else {
                            duet_workbench::app::Panel::Queue
                        };
                        app.editor_focused = focus;
                    }
                    Ok(())
                })();
                if let Err(error) = result {
                    app.status = format!("未打开内嵌 Helix：{error}");
                }
            }
            if (editor.is_some() || verification.is_some())
                && (app.requested_delete.is_some()
                    || app.requested_start.is_some()
                    || app.requested_retry.is_some()
                    || app.requested_merge_preview.is_some()
                    || app.requested_merge.is_some()
                    || app.requested_task.is_some()
                    || !app.controls.is_empty()
                    || app.requested_conflict.is_some()
                    || app.cancel_requested)
            {
                app.requested_delete = None;
                app.requested_start = None;
                app.requested_retry = None;
                app.requested_merge_preview = None;
                app.requested_merge = None;
                app.requested_task = None;
                app.controls.clear();
                app.requested_conflict = None;
                app.cancel_requested = false;
                app.status = if verification.is_some() {
                    "测试仍运行：Ctrl+X / :verify-stop 明确停止后再实现/删除/合入".into()
                } else {
                    "Helix 仍在运行：Tab 返回；实现/处理草稿/合入前，:editor-stop 明确停止（先保存）".into()
                };
            }
            if std::mem::take(&mut app.cancel_requested)
                && let Some(coding) = &mut coding
            {
                coding.cancel(&mut app)?;
            }
            if let Some(selection) = app.requested_delete.take() {
                let result = (|| -> Result<()> {
                    ensure!(advisor.is_none(), "请先 x 停止冲突分析再删除任务");
                    if coding.as_ref().is_some_and(|c| c.id == selection.task_id)
                        && let Some(mut c) = coding.take()
                    {
                        c.stop(&mut app, &mut store)?;
                    }
                    duet_workbench::task_ops::delete_selection(&selection, &mut app, &mut store)
                })();
                if let Err(e) = result {
                    app.status = format!("删除未完成：{e}");
                }
            }
            if let Some((id, fingerprint)) = app.requested_retry.take() {
                let result = (|| -> Result<()> {
                    ensure!(advisor.is_none(), "请先 x 停止冲突顾问");
                    ensure!(
                        coding.as_ref().is_none_or(|c| c.is_finished()),
                        "仍有活动任务，请等待结束后重试"
                    );
                    let task = app
                        .tasks
                        .iter()
                        .find(|t| t.id == id)
                        .context("任务不存在")?;
                    ensure!(
                        task.state == duet_workbench::nodes::TaskState::Failed
                            && serde_json::to_string(&task.failure)? == fingerprint,
                        "失败状态已变化，请重新确认"
                    );
                    let failure = task.failure.clone().context("失败记录缺失")?;
                    ensure!(
                        failure.stage == duet_workbench::nodes::FailureStage::Discussion,
                        "编码重试需重新确认写入计划"
                    );
                    let message = task
                        .history
                        .iter()
                        .rev()
                        .find(|m| m.role == "user" && !m.content.trim().is_empty())
                        .map(|m| m.content.clone())
                        .unwrap_or_else(|| task.prompt.clone());
                    if let Some(mut c) = coding.take() {
                        c.stop(&mut app, &mut store)?;
                    }
                    app.select_task_entry(duet_workbench::app::TaskSelection {
                        task_id: id.clone(),
                        node_id: failure.node_id,
                    });
                    let mut c = CodingRun::open_existing(
                        &id,
                        &config.agent.pi_path,
                        config.agent.model.as_deref(),
                        true,
                        Duration::from_secs(config.agent.timeout_secs),
                        &mut store,
                        &mut app,
                    )?;
                    if let Err(e) = c.discuss(&message, &mut app, &mut store) {
                        let _ = c.stop(&mut app, &mut store);
                        return Err(e);
                    }
                    coding = Some(c);
                    Ok(())
                })();
                if let Err(e) = result {
                    app.status = format!("重试未完成：{e}");
                    app.run.status = duet_workbench::rpc::RunStatus::Failed;
                    app.run.error = Some(e.to_string());
                    if let Some(task) = app.tasks.iter_mut().find(|t| t.id == id) {
                        task.state = duet_workbench::nodes::TaskState::Failed;
                        task.failure = Some(duet_workbench::nodes::TaskFailure {
                            stage: duet_workbench::nodes::FailureStage::Discussion,
                            reason: e.to_string().chars().take(2000).collect(),
                            node_id: task.failure.as_ref().and_then(|f| f.node_id.clone()),
                        });
                        store.save_task(task)?;
                        store.finish_run(&id, "Failed", task.checkpoint.as_deref())?;
                        app.content_revision = app.content_revision.wrapping_add(1);
                    }
                }
            }
            if let Some(id) = app.requested_merge_preview.take() {
                let result = (|| -> Result<duet_workbench::conflict::Preparation> {
                    ensure!(advisor.is_none(), "请先结束冲突分析");
                    ensure!(
                        coding.as_ref().is_none_or(|c| c.is_finished()),
                        "还有活动编码/讨论，请等任务完全结束"
                    );
                    let repo = Repository::open(store.directory().parent().context("主项目缺失")?)?;
                    let task = app
                        .tasks
                        .iter()
                        .find(|t| t.id == id)
                        .context("任务不存在")?;
                    duet_workbench::conflict::prepare(&repo, &store, task, false)
                })();
                match result {
                    Ok(duet_workbench::conflict::Preparation::Preview(preview)) => {
                        app.mode = Mode::MergePreview { preview, scroll: 0 }
                    }
                    Ok(duet_workbench::conflict::Preparation::Conflict(draft)) => {
                        let repo = Repository::open(store.directory().parent().unwrap())?;
                        let body = duet_workbench::conflict::display(&repo, &draft, 0, 4)?;
                        app.mode = Mode::Conflict {
                            draft,
                            index: 0,
                            side: 4,
                            scroll: 0,
                            body,
                        };
                    }
                    Err(e) => app.status = format!("不能预览合并：{e}"),
                }
            }
            if let Some(request) = app.requested_conflict.take() {
                use duet_workbench::app::ConflictKind;
                let mut draft = request.draft;
                let index = request.index;
                let mut side = 4;
                let result = (|| -> Result<Option<Mode>> {
                    if request.kind == ConflictKind::StopAdvisor {
                        if let Some(mut a) = advisor.take() {
                            a.stop()?;
                            app.agent_connected = false;
                            app.run.status = duet_workbench::rpc::RunStatus::Aborted;
                        }
                    } else {
                        ensure!(advisor.is_none(), "AI 正在只读分析；请等待或 x 停止");
                    }
                    ensure!(
                        coding.as_ref().is_none_or(|c| c.is_finished()),
                        "仍有活动编码，不能处理集成冲突"
                    );
                    if request.kind == ConflictKind::Advise
                        && let Some(mut c) = coding.take()
                    {
                        c.stop(&mut app, &mut store)?;
                    }
                    let repo = Repository::open(store.directory().parent().unwrap())?;
                    let task = app
                        .tasks
                        .iter()
                        .find(|t| t.id == draft.task_id)
                        .context("任务不存在")?;
                    match request.kind {
                        ConflictKind::Lockfile => duet_workbench::conflict::authorize_lockfile(
                            &repo, &store, task, &mut draft,
                        )?,
                        ConflictKind::View(n) => {
                            side = n;
                        }
                        ConflictKind::Choose(n) => duet_workbench::conflict::choose(
                            &repo, &store, task, &mut draft, index, n,
                        )?,
                        ConflictKind::Mark => {
                            duet_workbench::conflict::mark(&repo, &store, task, &mut draft, index)?
                        }
                        ConflictKind::ApplyAI => duet_workbench::conflict_ai::apply(
                            &repo, &store, task, &mut draft, index,
                        )?,
                        ConflictKind::Finish => {
                            let preview =
                                duet_workbench::conflict::finish(&repo, &store, task, &mut draft)?;
                            return Ok(Some(Mode::MergePreview { preview, scroll: 0 }));
                        }
                        ConflictKind::Advise => {
                            advisor = Some(duet_workbench::conflict_ai::Advisor::start_configured(
                                &config.agent,
                                &repo,
                                &store,
                                task,
                                &draft,
                                index,
                            )?);
                            app.agent_connected = true;
                            app.run = advisor.as_ref().unwrap().state.clone();
                            app.status = "AI 只读分析当前冲突 · 未应用任何方案 · x 可停止".into();
                        }
                        ConflictKind::Rebuild => {
                            match duet_workbench::conflict::prepare(&repo, &store, task, true)? {
                                duet_workbench::conflict::Preparation::Conflict(new) => {
                                    draft = new;
                                }
                                duet_workbench::conflict::Preparation::Preview(preview) => {
                                    return Ok(Some(Mode::MergePreview { preview, scroll: 0 }));
                                }
                            }
                        }
                        ConflictKind::Edit => {
                            let path = duet_workbench::conflict::edit_path(
                                &repo, &store, task, &draft, index,
                            )?;
                            let editor = std::env::var_os("DUET_EDITOR")
                                .unwrap_or_else(|| config.editor.clone().into_os_string());
                            ensure!(
                                !editor.is_empty()
                                    && !editor.to_string_lossy().chars().any(char::is_control),
                                "DUET_EDITOR 仅接受可执行文件路径，不接受 shell 参数"
                            );
                            disable_raw_mode()?;
                            execute!(io::stdout(), DisableBracketedPaste, LeaveAlternateScreen)?;
                            let status = std::process::Command::new(editor).arg(&path).status();
                            enable_raw_mode()?;
                            execute!(io::stdout(), EnterAlternateScreen, EnableBracketedPaste)?;
                            terminal.clear()?;
                            ensure!(status?.success(), "编辑器未正常退出，原草稿保留");
                            duet_workbench::conflict::refresh(&repo, &store, task, &mut draft)?;
                        }
                        ConflictKind::StopAdvisor => {}
                    }
                    let body = duet_workbench::conflict::display(&repo, &draft, index, side)?;
                    Ok(Some(Mode::Conflict {
                        draft: draft.clone(),
                        index,
                        side,
                        scroll: 0,
                        body,
                    }))
                })();
                match result {
                    Ok(Some(mode)) => app.mode = mode,
                    Ok(None) => {}
                    Err(e) => {
                        app.status = format!("冲突操作未完成：{e}");
                        if request.kind == ConflictKind::Advise {
                            draft.advice_error = Some(e.to_string().chars().take(2000).collect());
                            duet_workbench::conflict::save(&store, &draft)?;
                        }
                        let repo = Repository::open(store.directory().parent().unwrap())?;
                        let body = duet_workbench::conflict::display(&repo, &draft, index, side)
                            .unwrap_or_else(|e| e.to_string());
                        app.mode = Mode::Conflict {
                            draft,
                            index,
                            side,
                            scroll: 0,
                            body,
                        };
                    }
                }
            }
            if let Some(preview) = app.requested_merge.take() {
                let result = (|| -> Result<String> {
                    ensure!(advisor.is_none(), "请先结束冲突 AI 分析");
                    ensure!(
                        coding.as_ref().is_none_or(|c| c.is_finished()),
                        "仍有活动编码，拒绝合并"
                    );
                    if let Some(mut c) = coding.take() {
                        c.stop(&mut app, &mut store)?;
                    }
                    let repo = Repository::open(store.directory().parent().context("主项目缺失")?)?;
                    let task = app
                        .tasks
                        .iter_mut()
                        .find(|t| t.id == preview.task_id)
                        .context("任务不存在")?;
                    let commit = duet_workbench::merge::apply(&repo, &mut store, task, &preview)?;
                    app.tasks_revision = app.tasks_revision.wrapping_add(1);
                    app.refresh_project(&repo)?;
                    Ok(commit)
                })();
                match result {
                    Ok(commit) => {
                        app.status =
                            format!("已合并到主项目 {} · 未自动测试、推送或部署", &commit[..12])
                    }
                    Err(e) => app.status = format!("合并未完成：{e}"),
                }
            }
            if let Some((id, confirmed_plan)) = app.requested_start.take() {
                let result = (|| -> Result<CodingRun> {
                    ensure!(advisor.is_none(), "请先 x 停止冲突顾问");
                    ensure!(
                        coding.as_ref().is_none_or(|c| c.is_finished()),
                        "请先暂停并退出当前执行；不能同时启动第二个编码会话"
                    );
                    let task = app
                        .tasks
                        .iter()
                        .find(|t| t.id == id)
                        .context("任务不存在")?;
                    ensure!(
                        serde_json::to_string(&task.plan)? == confirmed_plan,
                        "计划在确认期间改变，请重新 s 确认"
                    );
                    task.startable()?;
                    if let Some(mut c) = coding.take() {
                        c.stop(&mut app, &mut store)?;
                    }
                    CodingRun::open_existing(
                        &id,
                        &config.agent.pi_path,
                        config.agent.model.as_deref(),
                        false,
                        Duration::from_secs(config.agent.timeout_secs),
                        &mut store,
                        &mut app,
                    )
                })();
                match result {
                    Ok(c) => coding = Some(c),
                    Err(e) => {
                        if coding.is_none() {
                            app.agent_connected = false;
                            app.run.status = duet_workbench::rpc::RunStatus::Failed;
                        }
                        app.status = format!("无法启动：{e}");
                    }
                }
            }
            for control in std::mem::take(&mut app.controls) {
                let draft = if let RunControl::Say(text) = &control {
                    Some(text.clone())
                } else {
                    None
                };
                let result = (|| -> Result<()> {
                    if let RunControl::Say(message) = &control {
                        let selection = app.selected_task().map(|t| t.id.clone());
                        let current = coding
                            .as_ref()
                            .is_some_and(|c| selection.as_deref() == Some(&c.id));
                        if current
                            && coding.as_ref().is_some_and(|c| {
                                !c.is_finished() || app.planning_only || app.agent_connected
                            })
                        {
                            return coding
                                .as_mut()
                                .unwrap()
                                .interact(control, &mut app, &mut store);
                        }
                        ensure!(
                            coding.as_ref().is_none_or(|c| c.is_finished()),
                            "另一个任务正在执行，未发送讨论"
                        );
                        if let Some(mut c) = coding.take() {
                            c.stop(&mut app, &mut store)?;
                        }
                        if let Some(id) = selection {
                            let mut c = CodingRun::open_existing(
                                &id,
                                &config.agent.pi_path,
                                config.agent.model.as_deref(),
                                true,
                                Duration::from_secs(config.agent.timeout_secs),
                                &mut store,
                                &mut app,
                            )?;
                            c.discuss(message, &mut app, &mut store)?;
                            coding = Some(c);
                        } else {
                            let project = store
                                .directory()
                                .parent()
                                .context("项目目录无效")?
                                .to_owned();
                            coding = Some(CodingRun::start_drafting(
                                &project,
                                &config.agent.pi_path,
                                config.agent.model.as_deref(),
                                message,
                                Duration::from_secs(config.agent.timeout_secs),
                                &mut store,
                                &mut app,
                            )?);
                        }
                        Ok(())
                    } else if let Some(c) = &mut coding {
                        c.interact(control, &mut app, &mut store)
                    } else {
                        Err(anyhow::anyhow!("当前无执行会话；已有计划请按 s 启动"))
                    }
                })();
                if let Err(error) = result {
                    app.status = format!("操作未完成：{error}");
                    if let Some(text) = draft {
                        app.mode = Mode::Message { text };
                    }
                }
            }
            if let Some(prompt) = app.requested_task.take() {
                if advisor.is_some() {
                    app.status = "请先 x 停止冲突顾问再讨论新任务".into();
                    continue;
                }
                let project = store
                    .directory()
                    .parent()
                    .context("项目目录无效")?
                    .to_owned();
                if let Some(mut c) = coding.take() {
                    c.stop(&mut app, &mut store)?;
                }
                match CodingRun::start_drafting(
                    &project,
                    &config.agent.pi_path,
                    config.agent.model.as_deref(),
                    &prompt,
                    Duration::from_secs(config.agent.timeout_secs),
                    &mut store,
                    &mut app,
                ) {
                    Ok(new_run) => coding = Some(new_run),
                    Err(error) => {
                        app.agent_connected = false;
                        app.run.status = duet_workbench::rpc::RunStatus::Failed;
                        app.status = format!("无法开始：{error}");
                    }
                }
            }
        }
    }
    if let Some(run) = &mut verification {
        run.stop(&mut app, &mut store)?;
    }
    review_runtime.stop()?;
    if let Some(mut a) = advisor.take() {
        a.stop()?;
        app.agent_connected = false;
    }
    if let Some(coding) = &mut coding {
        coding.stop(&mut app, &mut store)?;
    }
    store.save(&app)?;
    terminal.show_cursor()?;
    Ok(())
}
