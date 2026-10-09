use anyhow::{Context, Result, ensure};
use clap::{Parser, Subcommand};
use crossterm::{
    event::{self, DisableBracketedPaste, EnableBracketedPaste, Event},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use duet_workbench::{
    app::{App, Mode},
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
    time::Duration,
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
            let mut coding = CodingRun::start(
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
                    if coding.is_finished() {
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
    loop {
        if let Some(coding) = &mut coding {
            coding.poll(&mut app, &mut store)?;
        }
        terminal.draw(|frame| ui::draw(frame, &app))?;
        if app.quit {
            break;
        }
        if event::poll(Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(key) => app.handle_key(key),
                Event::Paste(text) => {
                    if let Mode::Input { text: input, .. } = &mut app.mode {
                        input.push_str(&text);
                    }
                }
                _ => {}
            }
            if std::mem::take(&mut app.cancel_requested)
                && let Some(coding) = &mut coding
            {
                coding.cancel(&mut app)?;
            }
            if let Some(prompt) = app.requested_task.take() {
                let project = store
                    .directory()
                    .parent()
                    .context("项目目录无效")?
                    .to_owned();
                match CodingRun::start(
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
                        app.run.status = duet_workbench::rpc::RunStatus::Failed;
                        app.status = format!("无法开始：{error}");
                    }
                }
            }
            store.save(&app)?;
            if app.status == "笔记已记录 · 仅本次会话" {
                app.status = "笔记已保存".into();
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
