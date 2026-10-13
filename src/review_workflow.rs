//! Fixed Git revisions published from human review buffers, without changing original worktrees.
use crate::{
    app::{App, View},
    checkpoint::Repository,
    nodes::{Attempt, Execution, Review, TaskState, Verification},
    review_editor::Buffer,
    storage::WorkspaceStore,
};
use anyhow::{Context, Result, ensure};
use std::{
    fs,
    time::{SystemTime, UNIX_EPOCH},
};
pub fn prepare(app: &mut App, repo: &Repository) -> Result<()> {
    if app.view != View::Review || app.current_buffer().is_some() {
        return Ok(());
    }
    let Some(item) = app.current() else {
        return Ok(());
    };
    let Some(task) = app.selected_task() else {
        return Ok(());
    };
    ensure!(app.review_buffers.len() < 64, "最多保留 64 个审核草稿");
    let snapshot = repo.resolve_snapshot(&item.version)?;
    let original = if item.change == Some(crate::checkpoint::ChangeKind::Deleted) {
        String::new()
    } else {
        repo.read(&snapshot, &item.title)?
    };
    let base = if let Some(parent) = &snapshot.parent {
        let parent = repo.resolve_snapshot(parent)?;
        if repo.snapshot_has_path(&parent, &item.title)? {
            repo.read(&parent, &item.title)?
        } else {
            String::new()
        }
    } else {
        String::new()
    };
    ensure!(
        app.review_buffers
            .iter()
            .map(|b| b.base.len() + b.original.len() + b.text.len())
            .sum::<usize>()
            + base.len()
            + original.len() * 2
            <= 24 * 1024 * 1024,
        "审核草稿总量超过 24 MiB；旧草稿保留"
    );
    let previous = app
        .review_buffers
        .iter()
        .rev()
        .find(|b| b.task_id == task.id && b.path == item.title && !b.changed())
        .map(|b| (b.cursor, b.top, b.left));
    let mut buffer = Buffer::new(
        task.id.clone(),
        app.task_selection.as_ref().and_then(|s| s.node_id.clone()),
        item.title.clone(),
        item.version.clone(),
        base,
        original,
    );
    buffer.top = item.scroll as usize;
    if let Some((cursor, top, left)) = previous {
        buffer.cursor = cursor.min(buffer.text.len());
        while !buffer.text.is_char_boundary(buffer.cursor) {
            buffer.cursor -= 1;
        }
        buffer.top = top;
        buffer.left = left;
    }
    app.review_buffers.push(buffer);
    app.review_revision = app.review_revision.wrapping_add(1);
    Ok(())
}
pub fn publish(app: &mut App, store: &mut WorkspaceStore) -> Result<()> {
    let buffer = app.current_buffer().context("没有审核缓冲区")?.clone();
    ensure!(buffer.changed(), "缓冲区没有修改；未创建修订");
    let index = app
        .tasks
        .iter()
        .position(|t| t.id == buffer.task_id)
        .context("任务不存在")?;
    let task = &app.tasks[index];
    ensure!(
        task.state != TaskState::Merged && task.merged.is_none(),
        "已合入任务仅保留本地草稿，不改写历史"
    );
    ensure!(
        task.nodes
            .iter()
            .all(|n| n.execution == Execution::Implemented),
        "本地草稿已保存；发布修订目前要求全部节点实现结束，避免覆盖后续执行"
    );
    let node_index = task
        .nodes
        .iter()
        .position(|n| Some(n.contract.id.as_str()) == buffer.node_id.as_deref())
        .context("选择节点不存在")?;
    ensure!(
        task.checkpoint.as_deref() == Some(buffer.version.as_str())
            && task.nodes[node_index].checkpoint() == Some(buffer.version.as_str()),
        "本地草稿已保存；当前只支持最终交付节点的新修订，历史节点需另行返工"
    );
    let changed = app
        .review_buffers
        .iter()
        .filter(|b| {
            b.task_id == buffer.task_id
                && b.node_id == buffer.node_id
                && b.version == buffer.version
                && b.changed()
        })
        .cloned()
        .collect::<Vec<_>>();
    ensure!(
        changed
            .iter()
            .all(|b| task.nodes[node_index].contract.contains_path(&b.path)),
        "草稿修改超出当前节点范围；未发布或改写契约"
    );
    let original_attempt = task.nodes[node_index]
        .attempts
        .last()
        .context("节点无交付")?
        .clone();
    let repo = Repository::open(store.directory().parent().unwrap())?;
    let snapshot = repo.resolve_snapshot(&buffer.version)?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_nanos()
        .to_string();
    let folder = store.directory().join("review-revisions").join(&nonce);
    crate::conflict::private_dir(&folder)?;
    let worktree = folder.join("worktree");
    let branch = format!("duet/review-{nonce}");
    fs::write(
        folder.join("owner.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"task_id":buffer.task_id,"node_id":buffer.node_id,"original":buffer.version,"branch":branch,"worktree":worktree}),
        )?,
    )?;
    repo.create_worktree_at(&worktree, &branch, &snapshot.commit)?;
    for b in &changed {
        crate::checkpoint::validate_path(&b.path)?;
        let path = worktree.join(&b.path);
        if let Some(parent) = path.parent() {
            crate::conflict::private_dir(parent)?;
        }
        if let Ok(meta) = fs::symlink_metadata(&path) {
            ensure!(
                meta.is_file() && !meta.file_type().is_symlink(),
                "审核路径不是普通文件"
            );
        }
        fs::write(path, &b.text)?;
    }
    let fixed = Repository::open(&worktree)?
        .capture_worktree_from(&repo.resolve_snapshot(&original_attempt.base)?)?;
    ensure!(
        repo.snapshot_changes(&fixed)?
            .iter()
            .all(|c| task.nodes[node_index].contract.contains_path(&c.path)),
        "固定修订超出契约；工作区保留，未更新任务"
    );
    let changed_files = repo.snapshot_changes(&fixed)?.len();
    let execution = if changed_files == 0 {
        Execution::NoChanges
    } else {
        Execution::Implemented
    };
    // Commit the task record before replacing UI state. Old attempts and original candidates remain.
    let mut updated = task.clone();
    let node = &mut updated.nodes[node_index];
    node.execution = execution;
    node.review = Review::Pending;
    node.reviewed_checkpoint = None;
    node.verification = Verification::Unverified;
    node.attempts.push(Attempt {
        base: original_attempt.base,
        checkpoint: Some(fixed.commit.clone()),
        changed_files: Some(changed_files),
        previews: Vec::new(),
        execution,
        summary: "人工审核修订；未测试、未批准，原候选/尝试保留".into(),
        scope_violations: Vec::new(),
        verification: Verification::Unverified,
        review: Review::Pending,
    });
    updated.checkpoint = Some(fixed.commit.clone());
    store.save_task(&updated)?;
    app.tasks[index] = updated;
    app.tasks_revision = app.tasks_revision.wrapping_add(1);
    app.append_node_checkpoint(&repo, &fixed, &buffer.task_id, node_index)?;
    app.review_version = Some(fixed.commit.clone());
    if let Some(selected) = app
        .items
        .iter()
        .position(|i| i.version == fixed.commit && i.title == buffer.path)
    {
        app.selected = selected;
    }
    app.panel = crate::app::Panel::Queue;
    app.status =
        "人工修改已发布为新固定修订 · 原批准未继承 · a 审核新版本；原候选保留，不自动测试/合入"
            .into();
    Ok(())
}
