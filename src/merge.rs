//! Whole-task integration: immutable review, explicit confirmation, no model/test/hook calls.
use crate::{
    checkpoint::Repository,
    nodes::{Execution, MergeReceipt, TaskRecord, TaskState, Verification},
    storage::WorkspaceStore,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergePreview {
    pub task_id: String,
    pub signature: String,
    pub source_head: String,
    pub branch: String,
    pub delivery: String,
    pub commit: String,
    pub diff: String,
    pub paths: Vec<String>,
    pub unverified: usize,
}
pub fn prepare(
    repo: &Repository,
    store: &WorkspaceStore,
    task: &TaskRecord,
) -> Result<MergePreview> {
    task.merge_ready()?;
    store.candidate(&task.id)?;
    repo.merge_guard()?;
    let branch = repo.branch_ref()?;
    let source_head = repo.baseline()?.commit;
    let previous = validate_delivery(repo, task)?;
    let snapshot = repo.prepare_integration(&source_head, &previous, &task.id)?;
    preview(repo, task, source_head, branch, previous, snapshot.commit)
}
pub fn validate_delivery(repo: &Repository, task: &TaskRecord) -> Result<String> {
    task.merge_ready()?;
    let order = task.plan.as_ref().unwrap().order()?;
    let mut previous = task.baseline.clone().context("任务原始基线缺失")?;
    for &index in &order {
        let node = &task.nodes[index];
        let base = &node
            .attempts
            .iter()
            .find(|a| a.execution != Execution::NoChanges)
            .context("节点基线缺失")?
            .base;
        ensure!(
            repo.tree(base)? == repo.tree(&previous)?,
            "节点 {} 开始基线包含未审/前置版本不一致的改动，拒绝合并",
            node.contract.id
        );
        let snapshot = repo.resolve_snapshot(node.checkpoint().unwrap())?;
        ensure!(
            snapshot.parent.as_deref() == Some(base),
            "节点检查点不属于记录的审核基线"
        );
        let changes = repo.snapshot_changes(&snapshot)?;
        ensure!(
            !changes.is_empty() && changes.iter().all(|c| node.contract.contains_path(&c.path)),
            "节点 {} 无有效范围内交付",
            node.contract.id
        );
        previous = snapshot.commit;
    }
    ensure!(
        task.checkpoint.as_deref() == Some(&previous),
        "任务最终版本与批准节点链不一致"
    );
    Ok(previous)
}
pub fn preview(
    repo: &Repository,
    task: &TaskRecord,
    source_head: String,
    branch: String,
    previous: String,
    commit: String,
) -> Result<MergePreview> {
    let snapshot = repo.resolve_snapshot(&commit)?;
    let paths = repo
        .snapshot_changes(&snapshot)?
        .into_iter()
        .map(|c| c.path)
        .collect();
    let diff = repo.integration_diff(&source_head, &snapshot.commit)?;
    Ok(MergePreview {
        task_id: task.id.clone(),
        signature: serde_json::to_string(task)?,
        source_head,
        branch,
        delivery: previous,
        commit: snapshot.commit,
        diff,
        paths,
        unverified: task
            .nodes
            .iter()
            .filter(|n| {
                n.verification != Verification::Passed
                    || n.attempts
                        .last()
                        .is_none_or(|a| a.verification != Verification::Passed)
            })
            .count(),
    })
}
pub fn apply(
    repo: &Repository,
    store: &mut WorkspaceStore,
    task: &mut TaskRecord,
    preview: &MergePreview,
) -> Result<String> {
    task.merge_ready()?;
    ensure!(
        task.id == preview.task_id && serde_json::to_string(task)? == preview.signature,
        "节点/批准/计划在预览后改变，请重新预览"
    );
    store.candidate(&task.id)?;
    repo.merge_guard()?;
    ensure!(
        repo.baseline()?.commit == preview.source_head && repo.branch_ref()? == preview.branch,
        "主项目 HEAD/分支在预览后改变，请重新预览"
    );
    let integration = repo.resolve_snapshot(&preview.commit)?;
    ensure!(
        integration.parent.as_deref() == Some(&preview.source_head)
            && repo.is_ancestor(&preview.delivery, &preview.commit)?,
        "无效集成提交"
    );
    ensure!(
        repo.integration_diff(&preview.source_head, &preview.commit)? == preview.diff,
        "预览与集成提交不一致"
    );
    let backups = store.directory().join("backups");
    std::fs::create_dir_all(&backups)?;
    ensure!(
        !std::fs::symlink_metadata(&backups)?
            .file_type()
            .is_symlink(),
        "备份目录不允许符号链接"
    );
    let dir = backups.join(format!(
        "merge-{}-{}",
        task.id,
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    std::fs::create_dir(&dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    }
    store.save_task(task)?;
    store.backup_database(&dir.join("workspace.db"))?;
    std::fs::write(
        dir.join("confirmation.json"),
        serde_json::to_vec_pretty(preview)?,
    )?;
    repo.bundle_candidate(
        &dir.join("before-merge.bundle"),
        &preview.branch,
        &HashSet::from([preview.delivery.clone()]),
    )?;
    // Persist authorization intent BEFORE Git; restart only reconciles, never applies it.
    store.save_merge_intent(preview)?;
    repo.fast_forward_integration(&preview.source_head, &preview.branch, &preview.commit)?;
    record(store, task, preview)?;
    Ok(preview.commit.clone())
}
fn record(store: &mut WorkspaceStore, task: &mut TaskRecord, p: &MergePreview) -> Result<()> {
    task.merged = Some(MergeReceipt {
        source_head: p.source_head.clone(),
        delivery: p.delivery.clone(),
        commit: p.commit.clone(),
    });
    task.state = TaskState::Merged;
    store.save_task(task)?;
    store.finish_run(&task.id, "Merged", task.checkpoint.as_deref())?;
    store.clear_merge_intent(&task.id)
}
pub fn recover(store: &mut WorkspaceStore) -> Result<()> {
    let intents = store.merge_intents()?;
    if intents.is_empty() {
        return Ok(());
    }
    let repo = Repository::open(store.directory().parent().context("主项目缺失")?)?;
    let head = repo.baseline()?.commit;
    for p in intents {
        if repo.is_ancestor(&p.commit, &head)? {
            if let Some(mut task) = store.tasks()?.into_iter().find(|t| t.id == p.task_id) {
                ensure!(
                    task.merged.is_some() || serde_json::to_string(&task)? == p.signature,
                    "Git 已合并但任务记录已变化，请检查备份与合并记录"
                );
                record(store, &mut task, &p)?;
            }
        } else {
            store.clear_merge_intent(&p.task_id)?;
        }
    }
    Ok(())
}
