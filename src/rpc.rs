use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Read, Write},
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant},
};

pub const MAX_RECORD_BYTES: usize = 1_048_576;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunStatus {
    Idle,
    Running,
    Pausing,
    Paused,
    Cancelling,
    Settled,
    Aborted,
    Failed,
}

#[derive(Debug, Clone)]
pub struct RunState {
    pub status: RunStatus,
    pub streaming_text: bool,
    pub text: String,
    pub error: Option<String>,
    pub tool_count: usize,
    pub pending: HashMap<String, String>,
}
impl Default for RunState {
    fn default() -> Self {
        Self {
            status: RunStatus::Idle,
            streaming_text: false,
            text: String::new(),
            error: None,
            tool_count: 0,
            pending: HashMap::new(),
        }
    }
}
impl RunState {
    pub fn register(&mut self, id: &str, command: &str) {
        self.pending.insert(id.into(), command.into());
    }

    pub fn apply(&mut self, record: &Value) {
        match record["type"].as_str().unwrap_or("") {
            "response" => {
                let Some(id) = record["id"].as_str() else {
                    return;
                };
                let Some(command) = self.pending.remove(id) else {
                    return;
                };
                if record["success"] == false {
                    self.status = RunStatus::Failed;
                    self.error = Some(record["error"].as_str().unwrap_or("RPC 命令失败").into());
                } else if command == "prompt" && record["data"]["disposition"] == "handled" {
                    self.status = RunStatus::Idle;
                }
                // A successful prompt response is acceptance, not completion.
            }
            "agent_start" => {
                self.status = RunStatus::Running;
                self.error = None;
            }
            "message_start" if record["message"]["role"] == "assistant" => {
                self.text.clear();
                self.streaming_text = false;
            }
            "message_update" => {
                let update = &record["assistantMessageEvent"];
                if update["type"] == "text_delta" {
                    self.streaming_text = true;
                    append_bounded(&mut self.text, update["delta"].as_str().unwrap_or(""));
                }
            }
            "message_end" if record["message"]["role"] == "assistant" => {
                self.streaming_text = false;
                let message = &record["message"];
                self.text = message_text(message);

                truncate_utf8(&mut self.text, MAX_RECORD_BYTES);
                if message["stopReason"] == "error" {
                    self.error = Some(
                        message["errorMessage"]
                            .as_str()
                            .unwrap_or("模型运行失败")
                            .into(),
                    );
                }
            }
            "tool_execution_start" => self.tool_count += 1,
            "agent_settled" => {
                self.status = if record["aborted"] == true {
                    RunStatus::Aborted
                } else if self.error.is_some() {
                    RunStatus::Failed
                } else {
                    RunStatus::Settled
                };
            }
            "agent_end" => {} // Retries / queued work may follow.
            _ => {}
        }
    }
}

/// Only visible text belongs in the chat; thinking/tool blocks remain in raw Pi logs.
pub fn message_text(message: &Value) -> String {
    if let Some(text) = message["content"].as_str() {
        return text.into();
    }
    message["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|block| block["type"] == "text")
        .filter_map(|block| block["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn append_bounded(target: &mut String, delta: &str) {
    if target.len() < MAX_RECORD_BYTES {
        target.push_str(delta);
        truncate_utf8(target, MAX_RECORD_BYTES);
    }
}
fn truncate_utf8(target: &mut String, max: usize) {
    let mut end = target.len().min(max);
    while !target.is_char_boundary(end) {
        end -= 1;
    }
    target.truncate(end);
}

/// Strict LF JSONL; U+2028/U+2029 are not record separators.
pub fn read_record<R: BufRead>(reader: &mut R) -> Result<Option<Value>> {
    let mut bytes = Vec::new();
    let read = reader
        .take((MAX_RECORD_BYTES + 1) as u64)
        .read_until(b'\n', &mut bytes)?;
    if read == 0 {
        return Ok(None);
    }
    ensure!(bytes.len() <= MAX_RECORD_BYTES, "RPC 记录超过 1 MiB");
    ensure!(bytes.last() == Some(&b'\n'), "RPC 记录缺少 LF 终止符");
    bytes.pop();
    if bytes.last() == Some(&b'\r') {
        bytes.pop();
    }
    let value: Value = serde_json::from_slice(&bytes).context("无效 Pi JSONL")?;
    ensure!(value.is_object(), "RPC 记录必须是 JSON 对象");
    Ok(Some(value))
}

pub struct PiClient {
    child: Child,
    input: Option<ChildStdin>,
    pub events: Receiver<Result<Value>>,
    next_id: u64,
}

impl PiClient {
    /// Phase-0 read-only client. cwd is NOT a security sandbox.
    pub fn spawn(executable: &Path, cwd: &Path) -> Result<Self> {
        let mut command = Command::new(executable);
        command.current_dir(cwd).args([
            "--mode",
            "rpc",
            "--no-session",
            "--offline",
            "--no-approve",
            "--no-extensions",
            "--no-mcp",
            "--no-skills",
            "--no-prompt-templates",
            "--no-context-files",
            "--no-themes",
            "--tools",
            "read,grep,find,ls",
        ]);
        command.env("PI_TELEMETRY", "0");
        Self::spawn_command(command)
    }

    /// Explicitly authorized writer. No shell tool; worktree is NOT an OS sandbox.
    pub fn spawn_coder(
        executable: &Path,
        cwd: &Path,
        session_dir: &Path,
        model: Option<&str>,
    ) -> Result<Self> {
        let mut command = Command::new(executable);
        command.current_dir(cwd).args([
            "--mode", "rpc", "--offline", "--no-approve", "--no-extensions",
            "--no-mcp", "--no-skills", "--no-prompt-templates", "--no-context-files",
            "--no-themes", "--tools", "read,edit,write,grep,find,ls",
            "--append-system-prompt",
            "Work only on the user's requested task in the current workspace. Do not modify Git metadata, merge, deploy, or access credentials. Do not claim tests were run; no shell tool is available. Summarize changed files and remaining verification.",
        ]).arg("--session-dir").arg(session_dir).env("PI_TELEMETRY", "0");
        if let Some(model) = model {
            command.args(["--model", model]);
        }
        Self::spawn_command(command)
    }

    pub fn spawn_command(mut command: Command) -> Result<Self> {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("无法启动 Pi 子进程")?;
        let input = child.stdin.take().context("缺少 Pi stdin")?;
        let output = child.stdout.take().context("缺少 Pi stdout")?;
        let errors = child.stderr.take().context("缺少 Pi stderr")?;
        // Bounded channel propagates backpressure; receiver must be drained.
        let (tx, events) = mpsc::sync_channel(128);
        let error_tx = tx.clone();
        thread::spawn(move || {
            let mut reader = BufReader::new(output);
            loop {
                match read_record(&mut reader) {
                    Ok(Some(record)) => {
                        if tx.send(Ok(record)).is_err() {
                            break;
                        }
                    }
                    Ok(None) => {
                        let _ = tx.send(Err(anyhow::anyhow!("Pi stdout 已关闭")));
                        break;
                    }
                    Err(error) => {
                        let _ = tx.send(Err(error));
                        break;
                    }
                }
            }
        });
        thread::spawn(move || {
            let mut reader = BufReader::new(errors);
            // Do not retain diagnostic content, which may contain secrets.
            let mut buffer = [0; 4096];
            while let Ok(count) = reader.read(&mut buffer) {
                if count == 0 {
                    break;
                }
            }
            drop(error_tx);
        });
        Ok(Self {
            child,
            input: Some(input),
            events,
            next_id: 0,
        })
    }

    pub fn send(&mut self, mut command: Value) -> Result<String> {
        self.next_id += 1;
        let id = format!("duet-{}", self.next_id);
        command["id"] = json!(id);
        self.write(command)?;
        Ok(id)
    }

    fn write(&mut self, command: Value) -> Result<()> {
        let input = self.input.as_mut().context("Pi 输入已关闭")?;
        serde_json::to_writer(&mut *input, &command)?;
        input.write_all(b"\n")?;
        input.flush()?;
        Ok(())
    }

    pub fn get_state(&mut self, timeout: Duration) -> Result<Value> {
        let id = self.send(json!({"type":"get_state"}))?;
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let record = self
                .events
                .recv_timeout(remaining)
                .context("等待 Pi get_state 超时或连接中断")??;
            if record["type"] == "response" && record["id"] == id {
                if record["success"] != true {
                    bail!("Pi get_state 失败");
                }
                return Ok(record["data"].clone());
            }
            if record["type"] == "extension_ui_request" {
                self.dismiss_dialog(&record)?;
            }
        }
    }

    pub fn dismiss_dialog(&mut self, record: &Value) -> Result<()> {
        if matches!(
            record["method"].as_str(),
            Some("select" | "confirm" | "input" | "editor")
        ) {
            self.write(json!({"type":"extension_ui_response","id":record["id"],"cancelled":true}))?;
        }
        Ok(())
    }

    pub fn shutdown(&mut self) -> Result<()> {
        self.input.take(); // EOF requests orderly shutdown.
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            // Keep consuming stdout so closing cannot deadlock on pipe backpressure.
            while self.events.try_recv().is_ok() {}
            if self.child.try_wait()?.is_some() {
                return Ok(());
            }
            if Instant::now() >= deadline {
                self.child.kill()?;
                self.child.wait()?;
                return Ok(());
            }
            thread::sleep(Duration::from_millis(10));
        }
    }
}
impl Drop for PiClient {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}
