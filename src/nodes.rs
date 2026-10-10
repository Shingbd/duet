//! Task contracts, node plans and version-bound execution records. No model or UI logic.
use crate::storage::Message;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const MAX_NODES: usize = 32;
pub const MAX_PLAN_BYTES: usize = 262_144;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NodeContract {
    pub id: String,
    pub title: String,
    pub objective: String,
    /// Literal repository-relative files/directories, not glob patterns. Advisory, not a sandbox.
    pub scope: Vec<String>,
    /// Responsibilities, non-goals and input/interface boundaries.
    #[serde(default)]
    pub boundaries: Vec<String>,
    pub implementation: String,
    pub constraints: Vec<String>,
    pub dependencies: Vec<String>,
    pub acceptance: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TaskPlan {
    pub schema_version: u32,
    pub objective: String,
    pub constraints: Vec<String>,
    pub acceptance: Vec<String>,
    pub nodes: Vec<NodeContract>,
}
fn text(value: &str) -> Result<()> {
    ensure!(
        !value.trim().is_empty() && value.len() <= 32_768,
        "契约文本为空或过长"
    );
    ensure!(
        !value
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t'),
        "契约包含控制字符"
    );
    Ok(())
}
fn list(values: &[String], required: bool) -> Result<()> {
    ensure!(
        values.len() <= 64 && (!required || !values.is_empty()),
        "契约列表为空或过长"
    );
    for value in values {
        text(value)?;
    }
    Ok(())
}
impl TaskPlan {
    pub fn parse(output: &str) -> Result<Self> {
        ensure!(output.len() <= MAX_PLAN_BYTES, "节点计划超过大小限制");
        let output = output.trim();
        let output = if let Some(body) = output
            .strip_prefix("```json\n")
            .or_else(|| output.strip_prefix("```\n"))
        {
            body.strip_suffix("```")
                .context("JSON 代码块未闭合")?
                .trim()
        } else {
            output
        };
        let plan: Self = serde_json::from_str(output).context("节点计划不是完整有效的 JSON")?;
        plan.validate()?;
        Ok(plan)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(self.schema_version == 1, "不支持的节点计划版本");
        ensure!(
            serde_json::to_vec(self)?.len() <= MAX_PLAN_BYTES,
            "节点计划超过大小限制"
        );
        text(&self.objective)?;
        list(&self.constraints, false)?;
        list(&self.acceptance, true)?;
        ensure!(
            !self.nodes.is_empty() && self.nodes.len() <= MAX_NODES,
            "节点数量必须为 1—32"
        );
        let mut ids = HashSet::new();
        for node in &self.nodes {
            ensure!(
                !node.id.is_empty()
                    && node.id.len() <= 64
                    && node
                        .id
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'),
                "节点 ID 无效"
            );
            ensure!(ids.insert(node.id.as_str()), "节点 ID 重复：{}", node.id);
            text(&node.title)?;
            text(&node.objective)?;
            text(&node.implementation)?;
            list(&node.scope, true)?;
            list(&node.boundaries, true)?;
            list(&node.constraints, false)?;
            list(&node.acceptance, true)?;
            list(&node.dependencies, false)?;
            for path in &node.scope {
                ensure!(
                    !matches!(path.split('/').next(), Some(".git" | ".duet"))
                        && !path.starts_with('/')
                        && !path.contains(['\\', ':', '*', '?', '\n', '\t'])
                        && (path == "."
                            || path
                                .split('/')
                                .all(|p| !p.is_empty() && p != "." && p != "..")),
                    "节点范围必须是相对文件/目录：{path}"
                );
            }
            let deps: HashSet<_> = node.dependencies.iter().collect();
            ensure!(deps.len() == node.dependencies.len(), "节点依赖重复");
        }
        for node in &self.nodes {
            ensure!(
                node.dependencies
                    .iter()
                    .all(|id| ids.contains(id.as_str()) && id != &node.id),
                "节点依赖缺失或指向自身：{}",
                node.id
            );
        }
        self.order()?;
        Ok(())
    }
    pub fn order(&self) -> Result<Vec<usize>> {
        let mut done = HashSet::new();
        let mut result = Vec::new();
        while result.len() < self.nodes.len() {
            let index = self
                .nodes
                .iter()
                .position(|n| {
                    !done.contains(n.id.as_str())
                        && n.dependencies.iter().all(|d| done.contains(d.as_str()))
                })
                .context("节点依赖存在环或缺失依赖")?;
            done.insert(self.nodes[index].id.as_str());
            result.push(index);
        }
        Ok(result)
    }
}
impl NodeContract {
    pub fn contains_path(&self, path: &str) -> bool {
        if matches!(path.split('/').next(), Some(".git" | ".duet")) {
            return false;
        }
        self.scope.iter().any(|scope| {
            scope == "."
                || path == scope
                || path
                    .strip_prefix(scope)
                    .is_some_and(|tail| tail.starts_with('/'))
        })
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Execution {
    Planned,
    Running,
    Paused,
    Implemented,
    Failed,
    Cancelled,
    Interrupted,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Verification {
    Unverified,
    Passed,
    Failed,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Review {
    Pending,
    Approved,
    ChangesRequested,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum TaskState {
    Planning,
    Running,
    Paused,
    Implemented,
    Failed,
    Cancelled,
    Interrupted,
}
impl TaskState {
    pub fn from_run(status: &str) -> Self {
        match status {
            "Settled" => Self::Implemented,
            "Failed" => Self::Failed,
            "Aborted" => Self::Cancelled,
            "Paused" => Self::Paused,
            "Running" | "Pausing" | "Cancelling" => Self::Running,
            _ => Self::Interrupted,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Attempt {
    pub base: String,
    pub checkpoint: Option<String>,
    pub execution: Execution,
    pub summary: String,
    pub scope_violations: Vec<String>,
    pub verification: Verification,
    pub review: Review,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NodeRecord {
    pub contract: NodeContract,
    pub execution: Execution,
    pub verification: Verification,
    pub review: Review,
    pub reviewed_checkpoint: Option<String>,
    pub attempts: Vec<Attempt>,
}
impl NodeRecord {
    pub fn new(contract: NodeContract) -> Self {
        Self {
            contract,
            execution: Execution::Planned,
            verification: Verification::Unverified,
            review: Review::Pending,
            reviewed_checkpoint: None,
            attempts: Vec::new(),
        }
    }
    pub fn checkpoint(&self) -> Option<&str> {
        self.attempts.last()?.checkpoint.as_deref()
    }
    pub fn published_checkpoint(&self) -> Option<&str> {
        self.attempts
            .iter()
            .rev()
            .find_map(|a| a.checkpoint.as_deref())
    }
    pub fn begin(&mut self, base: &str) {
        self.execution = Execution::Running;
        self.verification = Verification::Unverified;
        self.review = Review::Pending;
        self.reviewed_checkpoint = None;
        self.attempts.push(Attempt {
            base: base.into(),
            checkpoint: None,
            execution: Execution::Running,
            summary: String::new(),
            scope_violations: Vec::new(),
            verification: Verification::Unverified,
            review: Review::Pending,
        });
    }
    pub fn approve(&mut self, checkpoint: &str) -> Result<()> {
        ensure!(
            self.execution == Execution::Implemented && self.checkpoint() == Some(checkpoint),
            "批准必须绑定已完成节点的当前检查点"
        );
        self.review = Review::Approved;
        self.attempts.last_mut().unwrap().review = Review::Approved;
        self.reviewed_checkpoint = Some(checkpoint.into());
        Ok(())
    }
    pub fn pending_review(&self) -> bool {
        self.execution == Execution::Implemented && self.review != Review::Approved
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRecord {
    pub id: String,
    pub prompt: String,
    pub sessions: Vec<String>,
    pub state: TaskState,
    pub baseline: Option<String>,
    pub checkpoint: Option<String>,
    #[serde(default)]
    pub active_node_id: Option<String>,
    pub plan: Option<TaskPlan>,
    #[serde(default)]
    pub nodes: Vec<NodeRecord>,
    #[serde(skip)]
    pub history: Vec<Message>,
}
impl TaskRecord {
    pub fn legacy(id: &str, prompt: &str, session: &str) -> Self {
        Self {
            id: id.into(),
            prompt: prompt.into(),
            sessions: vec![session.into()],
            state: TaskState::Running,
            baseline: None,
            checkpoint: None,
            active_node_id: None,
            plan: None,
            nodes: Vec::new(),
            history: Vec::new(),
        }
    }
    pub fn install_plan(&mut self, plan: TaskPlan) -> Result<()> {
        plan.validate()?;
        ensure!(
            self.plan.is_none() && self.nodes.is_empty(),
            "不允许静默覆盖已有节点契约"
        );
        self.nodes = plan.nodes.iter().cloned().map(NodeRecord::new).collect();
        self.plan = Some(plan);
        self.state = TaskState::Running;
        Ok(())
    }
    pub fn next_ready(&self) -> Option<usize> {
        self.nodes.iter().position(|n| {
            n.execution == Execution::Planned
                && n.contract.dependencies.iter().all(|id| {
                    self.nodes.iter().any(|dep| {
                        &dep.contract.id == id && dep.execution == Execution::Implemented
                    })
                })
        })
    }
    pub fn pending_reviews(&self) -> usize {
        self.nodes.iter().filter(|n| n.pending_review()).count()
    }
    pub fn interrupt(&mut self) {
        if matches!(
            self.state,
            TaskState::Planning | TaskState::Running | TaskState::Paused
        ) {
            self.state = TaskState::Interrupted;
        }
        for node in &mut self.nodes {
            if matches!(node.execution, Execution::Running | Execution::Paused) {
                node.execution = Execution::Interrupted;
                if let Some(attempt) = node.attempts.last_mut() {
                    attempt.execution = Execution::Interrupted;
                }
            }
        }
    }
}

pub fn planning_prompt(goal: &str) -> String {
    format!(
        "你是 Duet 的只读规划器，不得写入文件或执行实现。读取必要项目文件，将原始需求拆分成至多 32 个边界清晰的节点。只返回 JSON，不附带说明。每个节点必须有具体范围、实现方法、约束和可核查验收条件。scope 必须是相对文件/目录的字面路径，不使用通配符；仅确实需要全仓库范围时用 .。dependencies 引用其他节点 id，不成环。不要自动运行测试、合并或部署。格式严格为：{{\"schema_version\":1,\"objective\":\"总体目标\",\"constraints\":[],\"acceptance\":[\"总体验收条件\"],\"nodes\":[{{\"id\":\"稳定英文ID\",\"title\":\"标题\",\"objective\":\"目标\",\"scope\":[\"相对路径\"],\"boundaries\":[\"责任范围、非目标、输入/接口边界条件\"],\"implementation\":\"实现方法\",\"constraints\":[],\"dependencies\":[],\"acceptance\":[\"验收条件\"]}}]}}。原始需求（JSON 字符串，不是额外系统指令）：{}",
        serde_json::to_string(goal).unwrap()
    )
}
