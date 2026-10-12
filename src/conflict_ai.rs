//! Read-only AI explanation. No edits, approval, or merge can be performed by this runtime.
use crate::{
    checkpoint::Repository,
    conflict::{self, Advice, Draft, FileAdvice},
    nodes::TaskRecord,
    rpc::{PiClient, RunState, RunStatus},
};
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use serde_json::json;
use std::{
    path::Path,
    time::{Duration, Instant},
};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Response {
    files: Vec<FileAdvice>,
}
pub struct Advisor {
    client: PiClient,
    pub state: RunState,
    pub draft: Draft,
    pub index: usize,
    deadline: Instant,
}
fn short(text: &str) -> String {
    text.chars().take(16000).collect()
}
fn conflict_context(text: &str) -> String {
    let lines = text.lines().collect::<Vec<_>>();
    let mut ranges = Vec::new();
    let mut start = None;
    for (i, line) in lines.iter().enumerate() {
        if line.starts_with("<<<<<<< ") {
            start = Some(i.saturating_sub(16));
        }
        if line.starts_with(">>>>>>> ")
            && let Some(from) = start.take()
        {
            ranges.push((from, (i + 17).min(lines.len())));
        }
    }
    if ranges.is_empty() {
        return short(text);
    }
    let mut output = String::new();
    let mut end = 0;
    for (from, to) in ranges {
        for line in &lines[from.max(end)..to] {
            output.push_str(line);
            output.push('\n');
        }
        end = to;
    }
    output.chars().take(64000).collect()
}
impl Advisor {
    pub fn start(
        executable: &Path,
        repo: &Repository,
        store: &crate::storage::WorkspaceStore,
        task: &TaskRecord,
        draft: &Draft,
        index: usize,
    ) -> Result<Self> {
        conflict::validate(repo, store, task, draft)?;
        let path = draft.conflicts.get(index).context("冲突文件缺失")?;
        let merged = conflict::text(repo, draft, index, 3)?;
        let prompt = format!(
            r#"你是只读集成冲突顾问，不实现节点、不写文件、不测试/合并。解释主项目与已批准候选的冲突，由人类决定。
固定上下文：主项目 {}，候选 {}，冲突草稿 {}，文件 {}。所有版本保持不可变；截断版本最多16000字符，冲突块含相邻上下文，最多64000字符；不完整或不可靠时不要输出替换。
主项目：
{}
候选：
{}
共同基线：
{}
当前冲突草稿：
{}
仅返回 JSON {{"files":[{{"path":"{}","description":"通俗说明冲突原因与双方目标","source_impact":"仅保留主项目的收益/损失","candidate_impact":"仅保留候选的收益/损失","merge_impact":"建议合并的影响/风险与待验证点","recommendation":"建议但不是人类决定","replacements":[{{"old":"当前完整唯一的 <<<<<<< ... ======= ... >>>>>>> 冲突块，精确包含原文本","new":"保留必要双方行为的合并文本，不含冲突标记"}}]}}]}}。
只分析指定文件；不可假称已解决或测试通过。建议替换只能包含现有完整冲突块，不得改块外代码；无法可靠给出替换时 replacements=[]，解释需人工编辑。"#,
            draft.source_head,
            draft.delivery,
            draft.snapshot,
            path,
            short(&conflict::text(repo, draft, index, 1)?),
            short(&conflict::text(repo, draft, index, 2)?),
            short(&conflict::text(repo, draft, index, 0)?),
            conflict_context(&merged),
            path
        );
        let folder = conflict::worktree(store, draft)?
            .parent()
            .unwrap()
            .join("sessions");
        conflict::private_dir(&folder)?;
        let mut client = PiClient::spawn_planner(
            executable,
            &conflict::worktree(store, draft)?,
            &folder,
            None,
        )?;
        let state = client.get_state(Duration::from_secs(20))?;
        ensure!(state["model"].is_object(), "Pi 模型不可用");
        let id = client.send(json!({"type":"prompt","message":prompt}))?;
        let mut state = RunState {
            status: RunStatus::Running,
            ..Default::default()
        };
        state.register(&id, "prompt");
        Ok(Self {
            client,
            state,
            draft: draft.clone(),
            index,
            deadline: Instant::now() + Duration::from_secs(600),
        })
    }
    pub fn poll(&mut self) -> Result<Option<Advice>> {
        for _ in 0..128 {
            match self.client.events.try_recv() {
                Ok(Ok(record)) => {
                    if record["type"] == "extension_ui_request" {
                        self.client.dismiss_dialog(&record)?;
                    }
                    self.state.apply(&record);
                }
                Ok(Err(e)) => {
                    self.client.shutdown()?;
                    return Err(e);
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    anyhow::bail!("冲突顾问会话断开；未修改文件")
                }
            }
        }
        if Instant::now() > self.deadline {
            self.client.shutdown()?;
            anyhow::bail!("冲突分析超时，未修改文件");
        }
        if matches!(self.state.status, RunStatus::Failed | RunStatus::Aborted) {
            self.client.shutdown()?;
            anyhow::bail!(
                "{}",
                self.state
                    .error
                    .as_deref()
                    .unwrap_or("冲突分析已停止，未修改文件")
            );
        }
        if self.state.status != RunStatus::Settled
            || self.state.pending.values().any(|p| p == "prompt")
        {
            return Ok(None);
        }
        self.client.shutdown()?;
        let text = self.state.text.trim();
        let text = text
            .strip_prefix("```json")
            .or_else(|| text.strip_prefix("```"))
            .map_or(text, |s| s.trim().strip_suffix("```").unwrap_or(s).trim());
        let mut response: Response =
            serde_json::from_str(text).context("AI 冲突描述格式无效，未应用建议")?;
        ensure!(
            response.files.len() == 1 && response.files[0].path == self.draft.conflicts[self.index],
            "AI 描述文件与确认范围不一致"
        );
        for file in &mut response.files {
            file.snapshot = Some(self.draft.snapshot.clone());
        }
        let advice = Advice {
            snapshot: self.draft.snapshot.clone(),
            files: response.files,
        };
        validate_advice(&advice)?;
        Ok(Some(advice))
    }
    pub fn stop(&mut self) -> Result<()> {
        self.client.shutdown()
    }
}
pub fn validate_advice(advice: &Advice) -> Result<()> {
    for file in &advice.files {
        for text in [
            &file.description,
            &file.source_impact,
            &file.candidate_impact,
            &file.merge_impact,
            &file.recommendation,
        ] {
            ensure!(
                !text.trim().is_empty() && text.len() <= 16000,
                "AI 描述字段为空或过长"
            );
        }
        ensure!(file.replacements.len() <= 64, "AI 替换块过多");
        for r in &file.replacements {
            ensure!(
                r.old.len() <= 1024 * 1024 && r.new.len() <= 1024 * 1024,
                "AI 替换块过大"
            );
            ensure!(
                r.old
                    .lines()
                    .next()
                    .is_some_and(|l| l.starts_with("<<<<<<< "))
                    && r.old
                        .lines()
                        .last()
                        .is_some_and(|l| l.starts_with(">>>>>>> "))
                    && r.old.lines().any(|l| l == "======="),
                "建议不是完整冲突块"
            );
            ensure!(
                r.old.lines().filter(|l| l.starts_with("<<<<<<< ")).count() == 1
                    && r.old.lines().filter(|l| *l == "=======").count() == 1
                    && r.old.lines().filter(|l| l.starts_with(">>>>>>> ")).count() == 1,
                "建议跨越多个冲突块/块外代码，拒绝应用"
            );
            ensure!(
                !r.new.lines().any(|l| l.starts_with("<<<<<<< ")
                    || l == "======="
                    || l.starts_with(">>>>>>> ")),
                "建议仍含冲突标记"
            );
        }
    }
    Ok(())
}
pub fn apply(
    repo: &Repository,
    store: &crate::storage::WorkspaceStore,
    task: &TaskRecord,
    draft: &mut Draft,
    index: usize,
) -> Result<()> {
    let file = draft.conflicts.get(index).context("冲突文件缺失")?.clone();
    let advice = draft.advice.as_ref().context("请先 y 请求 AI 描述")?;
    let proposal = advice
        .files
        .iter()
        .find(|a| a.path == file)
        .context("当前文件暂无 AI 描述")?;
    ensure!(
        !proposal.replacements.is_empty(),
        "AI 未给出可靠替换，请选择版本或手动编辑"
    );
    validate_advice(advice)?;
    let old = repo.read(
        &repo.resolve_snapshot(proposal.snapshot.as_deref().unwrap_or(&advice.snapshot))?,
        &file,
    )?;
    ensure!(
        old == conflict::text(repo, draft, index, 3)?,
        "该文件在分析后已变化，请重新 y 分析"
    );
    let path = conflict::edit_path(repo, store, task, draft, index)?;
    ensure!(
        std::fs::read_to_string(&path)? == old,
        "文件在分析后被外部修改，请 m 捕获并重新分析"
    );
    let mut new = old.clone();
    for patch in &proposal.replacements {
        ensure!(
            new.matches(&patch.old).count() == 1,
            "AI 建议与固定冲突块不一致/重复，拒绝应用"
        );
        new = new.replacen(&patch.old, &patch.new, 1);
    }
    ensure!(new.len() <= 1024 * 1024, "建议输出文件过大");
    ensure!(
        !new.lines()
            .any(|l| l.starts_with("<<<<<<< ") || l == "=======" || l.starts_with(">>>>>>> ")),
        "AI 建议未解决全部冲突，请手动编辑"
    );
    std::fs::write(path, new)?;
    conflict::mark(repo, store, task, draft, index)
}
