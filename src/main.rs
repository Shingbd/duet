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
#[command(version, about = "Duet：本地 AI 协作工作台")]
struct Cli {
    #[arg(long, global = true)]
    keymap: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Command>,
}
#[derive(Subcommand)]
enum Command {
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
        /// Explicit consent: model may read/write files with this OS account's permissions.
        #[arg(long)]
        allow_write: bool,
        #[arg(long, default_value = "pi")]
        pi: PathBuf,
        #[arg(long)]
        model: Option<String>,
        #[arg(long, default_value_t = 600)]
        timeout_secs: u64,
        /// Non-interactive execution; still requires --allow-write.
        #[arg(long)]
        headless: bool,
    },
    Snapshot {
        #[arg(long)]
        repo: PathBuf,
        #[arg(long, required = true, num_args = 1..)]
        paths: Vec<String>,
    },
    /// Pi RPC 连通性检查，不调用模型。
    PiProbe {
        #[arg(long)]
        repo: PathBuf,
        #[arg(long, default_value = "pi")]
        pi: PathBuf,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let keymap = match cli.keymap {
        Some(path) => Keymap::load(&path)?,
        None => Keymap::default(),
    };
    match cli.command.unwrap_or(Command::Workspace) {
        Command::Workspace => open_project(Path::new("."), keymap),
        Command::Open { path } => open_project(&path, keymap),
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
            run(app, store, None)
        }
        Command::Code {
            repo,
            prompt,
            allow_write,
            pi,
            model,
            timeout_secs,
            headless,
        } => {
            ensure!(
                allow_write,
                "编码需要 --allow-write 明确授权。模型请求可能外发代码；worktree 不是安全沙箱。"
            );
            ensure!(
                timeout_secs > 0 && timeout_secs <= 3600,
                "超时必须为 1—3600 秒"
            );
            if !headless {
                require_terminal()?;
            }
            let repo = Repository::open(&repo)?;
            let mut store = WorkspaceStore::open(repo.root())?;
            let mut app = App::browse(&repo, repo.baseline()?, keymap)?;
            store.restore(&mut app)?;
            let mut coding = CodingRun::start_planning(
                repo.root(),
                &pi,
                model.as_deref(),
                &prompt,
                Duration::from_secs(timeout_secs),
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
        Command::PiProbe { repo, pi } => {
            let repo = Repository::open(&repo)?;
            let mut client = PiClient::spawn(&pi, repo.root())?;
            let state = client.get_state(Duration::from_secs(20))?;
            println!(
                "Pi RPC connected; session={} streaming={}; no prompt sent",
                state["sessionId"].as_str().unwrap_or("unknown"),
                state["isStreaming"].as_bool().unwrap_or(false)
            );
            client.shutdown()
        }
    }
}

fn open_project(path: &Path, keymap: Keymap) -> Result<()> {
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
    loop {
        if let Some(coding) = &mut coding
            && coding.poll(&mut app, &mut store)?
        {
            redraw = true;
            save_since.get_or_insert_with(Instant::now);
        }
        if redraw {
            terminal.draw(|frame| renderer.draw(frame, &app))?;
            redraw = false;
        }
        // Paint first; persist batched navigation only after idle or a bounded delay.
        if save_since.is_some_and(|since| {
            since.elapsed() >= Duration::from_secs(2)
                || last_input.elapsed() >= Duration::from_millis(300)
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
        let wait = if coding.as_ref().is_some_and(|run| !run.is_finished())
            && app.run.status != duet_workbench::rpc::RunStatus::Paused
        {
            Duration::from_millis(16)
        } else if save_since.is_some() {
            Duration::from_millis(50)
        } else {
            Duration::from_millis(500)
        };
        if event::poll(wait)? {
            // Drain queued input before drawing so key repeats/pastes don't queue frames.
            for _ in 0..64 {
                match event::read()? {
                    Event::Key(key) => app.handle_key(key),
                    Event::Paste(text) => {
                        if let Mode::Input { text: input, .. } | Mode::Message { text: input } =
                            &mut app.mode
                        {
                            input.push_str(&text);
                        }
                    }
                    _ => {}
                }
                if app.quit || !event::poll(Duration::ZERO)? {
                    break;
                }
            }
            last_input = Instant::now();
            redraw = true;
            save_since.get_or_insert(last_input);
            if std::mem::take(&mut app.cancel_requested)
                && let Some(coding) = &mut coding
            {
                coding.cancel(&mut app)?;
            }
            if let Some(selection) = app.requested_delete.take() {
                let result = (|| -> Result<()> {
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
            if let Some((id, confirmed_plan)) = app.requested_start.take() {
                let result = (|| -> Result<CodingRun> {
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
                        Path::new("pi"),
                        None,
                        false,
                        Duration::from_secs(600),
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
                                Path::new("pi"),
                                None,
                                true,
                                Duration::from_secs(600),
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
                                Path::new("pi"),
                                None,
                                message,
                                Duration::from_secs(600),
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
                    Path::new("pi"),
                    None,
                    &prompt,
                    Duration::from_secs(600),
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
    if let Some(coding) = &mut coding {
        coding.stop(&mut app, &mut store)?;
    }
    store.save(&app)?;
    terminal.show_cursor()?;
    Ok(())
}
