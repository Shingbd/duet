//! Explicit, fixed-version local commands. A worktree is not an OS sandbox.
use crate::{
    app::App,
    checkpoint::Repository,
    nodes::{Execution, Verification},
    storage::WorkspaceStore,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Read,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Spec {
    pub argv: Vec<String>,
    #[serde(default)]
    pub criteria: Vec<usize>,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
}
fn default_timeout() -> u64 {
    300
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub task: String,
    pub node: String,
    pub snapshot: String,
    pub contract: String,
    pub spec: Spec,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Outcome {
    Running,
    Passed,
    Failed,
    Cancelled,
    TimedOut,
    Interrupted,
    Error,
    SourceChanged,
}
impl Outcome {
    pub fn label(self) -> &'static str {
        match self {
            Self::Running => "◐ 运行中",
            Self::Passed => "✓ 命令通过",
            Self::Failed => "× 命令失败",
            Self::Cancelled => "⊘ 已取消",
            Self::TimedOut => "! 超时",
            Self::Interrupted => "! 中断",
            Self::Error => "× 启动/采集失败",
            Self::SourceChanged => "× 源码改变，证据无效",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    pub id: String,
    pub task: String,
    pub node: String,
    pub snapshot: String,
    pub contract: String,
    pub spec: Spec,
    pub outcome: Outcome,
    pub started_ms: u64,
    pub finished_ms: Option<u64>,
    pub exit_code: Option<i32>,
    pub directory: PathBuf,
    pub output: String,
    pub truncated: bool,
    pub environment: String,
}
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}
pub fn request(app: &App, text: &str) -> Result<Request> {
    ensure!(text.len() <= 16384, "测试配置最多 16 KiB");
    let spec:Spec=serde_json::from_str(text).context("请输入 JSON，例如 {\"argv\":[\"cargo\",\"test\",\"--offline\"],\"criteria\":[1],\"timeout_secs\":300}")?;
    ensure!(
        !spec.argv.is_empty()
            && spec.argv.len() <= 64
            && spec
                .argv
                .iter()
                .all(|s| !s.is_empty() && s.len() <= 4096 && !s.chars().any(char::is_control)),
        "argv 需要 1—64 个非空参数，不允许控制字符"
    );
    ensure!(
        (1..=3600).contains(&spec.timeout_secs),
        "超时范围 1—3600 秒"
    );
    ensure!(
        spec.argv.iter().all(|s| !argument_sensitive(s)),
        "不要在测试参数中填写凭据/密钥；参数会作为证据保存"
    );
    let task = app.selected_task().context("请选择任务节点")?;
    let node = app.selected_node().context("请选择节点，不是任务根行")?;
    ensure!(
        node.execution == Execution::Implemented,
        "只验证已交付的节点，不验证活动预览/空交付"
    );
    let snapshot = node.checkpoint().context("无固定交付")?.to_owned();
    ensure!(
        app.review_checkpoint() == Some(snapshot.as_str()),
        "请选择当前节点最终固定版本，不是历史/预览"
    );
    let mut indices = spec.criteria.clone();
    indices.sort_unstable();
    indices.dedup();
    ensure!(
        indices.len() == spec.criteria.len()
            && indices
                .iter()
                .all(|i| *i > 0 && *i <= node.contract.acceptance.len()),
        "验收序号应唯一且在节点条件范围内（从 1 开始）；空数组只记命令结果"
    );
    ensure!(
        node.verification_runs.len() < 128,
        "本节点已有 128 条验证记录；不会自动覆盖历史证据"
    );
    Ok(Request {
        task: task.id.clone(),
        node: node.contract.id.clone(),
        snapshot,
        contract: serde_json::to_string(&node.contract)?,
        spec,
    })
}
pub fn validate(app: &App, req: &Request) -> Result<()> {
    let node = app
        .tasks
        .iter()
        .find(|t| t.id == req.task)
        .and_then(|t| t.nodes.iter().find(|n| n.contract.id == req.node))
        .context("节点已删除")?;
    ensure!(
        node.execution == Execution::Implemented
            && node.checkpoint() == Some(req.snapshot.as_str())
            && serde_json::to_string(&node.contract)? == req.contract,
        "交付/契约已变化，请重新选择并授权测试"
    );
    Ok(())
}
pub fn criterion<'a>(
    node: &'a crate::nodes::NodeRecord,
    snapshot: &str,
    index: usize,
) -> Option<&'a Record> {
    let contract = serde_json::to_string(&node.contract).ok()?;
    node.verification_runs.iter().rev().find(|r| {
        r.snapshot == snapshot && r.contract == contract && r.spec.criteria.contains(&index)
    })
}
fn failed(outcome: Outcome) -> bool {
    matches!(
        outcome,
        Outcome::Failed | Outcome::TimedOut | Outcome::Error | Outcome::SourceChanged
    )
}
fn retained_failure(
    node: &crate::nodes::NodeRecord,
    snapshot: &str,
    contract: &str,
    index: Option<usize>,
) -> bool {
    node.verification_runs
        .iter()
        .rev()
        .find(|r| {
            r.snapshot == snapshot
                && r.contract == contract
                && index.map_or(r.spec.criteria.is_empty(), |i| r.spec.criteria.contains(&i))
                && !matches!(
                    r.outcome,
                    Outcome::Running | Outcome::Cancelled | Outcome::Interrupted
                )
        })
        .is_some_and(|r| failed(r.outcome))
}
pub fn summarize(node: &mut crate::nodes::NodeRecord) {
    let Some(snapshot) = node.checkpoint().map(str::to_owned) else {
        return;
    };
    let contract = serde_json::to_string(&node.contract).unwrap_or_default();
    let latest = node
        .verification_runs
        .iter()
        .rev()
        .find(|r| r.snapshot == snapshot && r.contract == contract);
    let general =
        node.verification_runs.iter().rev().find(|r| {
            r.snapshot == snapshot && r.contract == contract && r.spec.criteria.is_empty()
        });
    let result = if latest.is_some_and(|r| r.outcome == Outcome::Running) {
        Verification::Unverified
    } else if latest.is_some_and(|r| {
        matches!(
            r.outcome,
            Outcome::Failed | Outcome::TimedOut | Outcome::Error | Outcome::SourceChanged
        )
    }) || retained_failure(node, &snapshot, &contract, None)
        || general.is_some_and(|r| {
            matches!(
                r.outcome,
                Outcome::Failed | Outcome::TimedOut | Outcome::Error | Outcome::SourceChanged
            )
        })
    {
        Verification::Failed
    } else if general.is_none_or(|r| r.outcome == Outcome::Passed)
        && !node.contract.acceptance.is_empty()
        && (1..=node.contract.acceptance.len())
            .all(|i| criterion(node, &snapshot, i).is_some_and(|r| r.outcome == Outcome::Passed))
    {
        Verification::Passed
    } else if (1..=node.contract.acceptance.len()).any(|i| {
        retained_failure(node, &snapshot, &contract, Some(i))
            || criterion(node, &snapshot, i).is_some_and(|r| {
                matches!(
                    r.outcome,
                    Outcome::Failed | Outcome::TimedOut | Outcome::Error | Outcome::SourceChanged
                )
            })
    }) {
        Verification::Failed
    } else {
        Verification::Unverified
    };
    node.verification = result;
    if let Some(a) = node.attempts.last_mut() {
        a.verification = result;
    }
}
pub fn recover(app: &mut App) {
    let mut recovered = false;
    for t in &mut app.tasks {
        for n in &mut t.nodes {
            let mut changed = false;
            for r in &mut n.verification_runs {
                if r.outcome == Outcome::Running {
                    r.outcome = Outcome::Interrupted;
                    r.finished_ms = Some(now());
                    r.output
                        .push_str("\n工作台重启：未恢复/重跑命令，遗留运行标为中断。\n");
                    changed = true;
                }
            }
            if changed {
                recovered = true;
                summarize(n);
            }
        }
    }
    if recovered {
        app.tasks_revision = app.tasks_revision.wrapping_add(1);
    }
}
fn argument_sensitive(s: &str) -> bool {
    let s = s.to_ascii_lowercase();
    [
        "--password",
        "--passwd",
        "password=",
        "passwd=",
        "secret=",
        "api_key=",
        "--api-key",
        "--access-token",
        "--token",
        "access_token=",
        "authorization:",
        "bearer ",
    ]
    .iter()
    .any(|p| s.contains(p))
}
fn sensitive(s: &str) -> bool {
    let s = s.to_ascii_lowercase();
    [
        "password",
        "passwd",
        "secret",
        "api_key",
        "api-key",
        "access_token",
        "access-token",
        "authorization:",
        "bearer ",
        "token=",
    ]
    .iter()
    .any(|p| s.contains(p))
}
fn scrub(text: &str) -> String {
    text.lines()
        .map(|line| {
            if sensitive(line) {
                "[已隐藏可能包含凭据的日志行]"
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}
const MAX_LOG: usize = 128 * 1024;
enum Output {
    Data(Vec<u8>),
    End,
}
pub struct Run {
    pub request: Request,
    pub record: Record,
    child: Option<Child>,
    rx: Receiver<Output>,
    start: Instant,
    last_save: Instant,
    dirty: bool,
    ended: Option<Instant>,
    ends: usize,
    forced: Option<Outcome>,
    exit_success: bool,
    pending_text: Vec<u8>,
}
impl Run {
    pub fn start(req: Request, app: &mut App, store: &mut WorkspaceStore) -> Result<Self> {
        ensure!(cfg!(unix), "测试 Runner 当前需要 Unix 进程组");
        validate(app, &req)?;
        ensure!(
            !req.spec.argv.is_empty() && (1..=3600).contains(&req.spec.timeout_secs),
            "命令/超时无效"
        );
        let root = store.directory().parent().context("缺少项目目录")?;
        let repo = Repository::open(root)?;
        let snapshot = repo.resolve_snapshot(&req.snapshot)?;
        ensure!(snapshot.commit == req.snapshot, "交付不是完整固定对象 ID");
        repo.verification_guard()?;
        let id = format!(
            "{}",
            SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
        );
        let dir = store.directory().join("verifications").join(&id);
        crate::conflict::private_dir(&dir)?;
        let worktree = dir.join("worktree");
        repo.create_worktree_at(&worktree, &format!("duet/verify-{id}"), &snapshot.commit)?;
        let record = Record {
            id,
            task: req.task.clone(),
            node: req.node.clone(),
            snapshot: req.snapshot.clone(),
            contract: req.contract.clone(),
            spec: req.spec.clone(),
            outcome: Outcome::Running,
            started_ms: now(),
            finished_ms: None,
            exit_code: None,
            directory: dir.clone(),
            output: String::new(),
            truncated: false,
            environment: format!(
                "{} / {} · credentials-free environment allowlist · no sandbox",
                std::env::consts::OS,
                std::env::consts::ARCH
            ),
        };
        let (tx, rx) = mpsc::sync_channel(32);
        let mut run = Self {
            request: req,
            record,
            child: None,
            rx,
            start: Instant::now(),
            last_save: Instant::now(),
            dirty: false,
            ended: None,
            ends: 0,
            forced: None,
            exit_success: false,
            pending_text: vec![],
        };
        run.save(app, store)?;
        let mut cmd = Command::new(&run.request.spec.argv[0]);
        cmd.args(&run.request.spec.argv[1..])
            .current_dir(&worktree)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env_clear();
        for key in [
            "PATH",
            "HOME",
            "USER",
            "LOGNAME",
            "LANG",
            "LC_ALL",
            "CARGO_HOME",
            "RUSTUP_HOME",
            "RUSTUP_TOOLCHAIN",
            "LD_LIBRARY_PATH",
            "LIBRARY_PATH",
            "SDKROOT",
        ] {
            if let Some(value) = std::env::var_os(key) {
                cmd.env(key, value);
            }
        }
        cmd.env("TERM", "dumb")
            .env("NO_COLOR", "1")
            .env("GIT_TERMINAL_PROMPT", "0");
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        match cmd.spawn() {
            Ok(mut child) => {
                let streams: Vec<Box<dyn Read + Send>> = vec![
                    Box::new(child.stdout.take().unwrap()),
                    Box::new(child.stderr.take().unwrap()),
                ];
                for mut stream in streams {
                    let tx = tx.clone();
                    std::thread::spawn(move || {
                        let mut buf = [0u8; 8192];
                        loop {
                            match stream.read(&mut buf) {
                                Ok(0) | Err(_) => break,
                                Ok(n) => {
                                    if tx.send(Output::Data(buf[..n].to_vec())).is_err() {
                                        return;
                                    }
                                }
                            }
                        }
                        let _ = tx.send(Output::End);
                    });
                }
                run.child = Some(child);
            }
            Err(e) => {
                run.record.outcome = Outcome::Error;
                run.record.finished_ms = Some(now());
                run.record.output = scrub(&e.to_string());
                run.save(app, store)?;
            }
        }
        Ok(run)
    }
    fn node_mut<'a>(&self, app: &'a mut App) -> Option<&'a mut crate::nodes::NodeRecord> {
        app.tasks
            .iter_mut()
            .find(|t| t.id == self.request.task)?
            .nodes
            .iter_mut()
            .find(|n| n.contract.id == self.request.node)
    }
    fn publish(&self, app: &mut App) -> Result<()> {
        if let Some(n) = self.node_mut(app) {
            if let Some(r) = n
                .verification_runs
                .iter_mut()
                .find(|r| r.id == self.record.id)
            {
                *r = self.record.clone();
            } else {
                n.verification_runs.push(self.record.clone());
            }
            if n.checkpoint() == Some(self.record.snapshot.as_str())
                && serde_json::to_string(&n.contract)? == self.record.contract
            {
                summarize(n);
            }
            app.tasks_revision = app.tasks_revision.wrapping_add(1);
        }
        Ok(())
    }
    fn save(&self, app: &mut App, store: &mut WorkspaceStore) -> Result<()> {
        self.publish(app)?;
        fs::write(
            self.record.directory.join("record.json"),
            serde_json::to_vec_pretty(&self.record)?,
        )?;
        store.save(app)?;
        Ok(())
    }
    fn kill_group(&mut self) {
        if let Some(c) = &mut self.child {
            #[cfg(unix)]
            unsafe {
                libc::kill(-(c.id() as i32), libc::SIGKILL);
            }
            let _ = c.kill();
        }
    }
    pub fn cancel(&mut self) {
        if !self.finished() {
            self.forced = Some(Outcome::Cancelled);
            self.kill_group();
        }
    }
    pub fn finished(&self) -> bool {
        self.record.outcome != Outcome::Running
    }
    fn append(&mut self, bytes: &[u8]) {
        if self.record.output.len() >= MAX_LOG {
            self.record.truncated = true;
            return;
        }
        self.pending_text.extend_from_slice(bytes);
        while let Some(end) = self.pending_text.iter().position(|b| *b == b'\n') {
            let line: Vec<_> = self.pending_text.drain(..=end).collect();
            self.append_line(&line);
        }
        if self.pending_text.len() > 16384 {
            self.pending_text.clear();
            self.record.truncated = true;
            self.append_line(b"[overlong log line omitted]");
        }
    }
    fn append_line(&mut self, line: &[u8]) {
        let s = crate::ui::safe_text(&scrub(&String::from_utf8_lossy(line)));
        let left = MAX_LOG.saturating_sub(self.record.output.len());
        let mut end = s.len().min(left.saturating_sub(1));
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        self.record.output.push_str(&s[..end]);
        if left > 0 {
            self.record.output.push('\n');
        }
        if end < s.len() {
            self.record.truncated = true;
        }
    }
    pub fn poll(&mut self, app: &mut App, store: &mut WorkspaceStore) -> Result<bool> {
        if self.finished() {
            return Ok(false);
        }
        let mut changed = false;
        for _ in 0..32 {
            match self.rx.try_recv() {
                Ok(Output::Data(bytes)) => {
                    self.append(&bytes);
                    changed = true;
                }
                Ok(Output::End) => {
                    self.ends += 1;
                }
                Err(_) => break,
            }
        }
        if self.start.elapsed() >= Duration::from_secs(self.request.spec.timeout_secs)
            && self.forced.is_none()
        {
            self.forced = Some(Outcome::TimedOut);
            self.kill_group();
        }
        if self.ended.is_none()
            && let Some(child) = &mut self.child
            && let Some(status) = child.try_wait()?
        {
            self.record.exit_code = status.code();
            self.exit_success = status.success();
            self.ended = Some(Instant::now());
            self.kill_group();
        }
        if self
            .ended
            .is_some_and(|t| self.ends >= 2 || t.elapsed() >= Duration::from_millis(500))
        {
            if self.ends < 2 {
                self.record.truncated = true;
            }
            let tail = std::mem::take(&mut self.pending_text);
            if !tail.is_empty() {
                self.append_line(&tail);
            }
            self.record.outcome = self.forced.unwrap_or(if self.exit_success {
                Outcome::Passed
            } else {
                Outcome::Failed
            });
            if self.forced.is_none() {
                let worktree = self.record.directory.join("worktree");
                match Repository::open(&worktree)
                    .and_then(|r| r.verification_clean(&self.request.snapshot))
                {
                    Ok(true) => {}
                    Ok(false) => {
                        self.record.outcome = Outcome::SourceChanged;
                        self.append_line(b"Tracked source/HEAD/index changed; result invalid.");
                    }
                    Err(e) => {
                        self.record.outcome = Outcome::Error;
                        self.append_line(e.to_string().as_bytes());
                    }
                }
            }
            self.record.finished_ms = Some(now());
            self.child.take();
            changed = true;
        }
        if changed {
            self.publish(app)?;
            self.dirty = true;
        }
        if self.dirty && (self.finished() || self.last_save.elapsed() >= Duration::from_millis(200))
        {
            self.save(app, store)?;
            self.last_save = Instant::now();
            self.dirty = false;
        }
        Ok(changed)
    }
    pub fn stop(&mut self, app: &mut App, store: &mut WorkspaceStore) -> Result<()> {
        self.cancel();
        if let Some(c) = &mut self.child {
            let _ = c.wait();
        }
        if !self.finished() {
            self.record.outcome = Outcome::Cancelled;
            self.record.finished_ms = Some(now());
            self.save(app, store)?;
        }
        Ok(())
    }
}
impl Drop for Run {
    fn drop(&mut self) {
        self.kill_group();
        if let Some(c) = &mut self.child {
            let _ = c.wait();
        }
    }
}
