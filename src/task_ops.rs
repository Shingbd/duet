//! Explicit destructive operations, restricted to project-owned candidate worktrees.
use crate::{
    app::{App, TaskSelection},
    checkpoint::Repository,
    nodes::TaskState,
    storage::WorkspaceStore,
};
use anyhow::{Context, Result, ensure};
use std::{
    collections::HashSet,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

pub fn delete_selection(
    selection: &TaskSelection,
    app: &mut App,
    store: &mut WorkspaceStore,
) -> Result<()> {
    let index = app
        .tasks
        .iter()
        .position(|t| t.id == selection.task_id)
        .context("任务不存在")?;
    let task = app.tasks[index].clone();
    ensure!(
        task.merged.is_none() || selection.node_id.is_none(),
        "该任务已经合并，节点删除不能撤回主项目代码；请手动 revert 合并提交"
    );
    let (worktree, branch) = store.candidate(&task.id)?;
    let source = Repository::open(store.directory().parent().context("主项目缺失")?)?;
    ensure!(source.root() != worktree, "拒绝操作主工作区");
    let ids: Vec<String> = if let Some(id) = &selection.node_id {
        let order = task.plan.as_ref().context("任务计划缺失")?.order()?;
        let start = order
            .iter()
            .position(|&i| task.nodes[i].contract.id == *id)
            .context("节点不存在")?;
        order[start..]
            .iter()
            .map(|&i| task.nodes[i].contract.id.clone())
            .collect()
    } else {
        task.nodes.iter().map(|n| n.contract.id.clone()).collect()
    };
    let removed: HashSet<_> = ids.iter().cloned().collect();
    let node = selection
        .node_id
        .as_ref()
        .and_then(|id| task.nodes.iter().find(|n| &n.contract.id == id));
    let base = node
        .and_then(|n| {
            n.attempts
                .iter()
                .find(|a| a.execution != crate::nodes::Execution::NoChanges)
                .or_else(|| n.attempts.first())
        })
        .map(|a| a.base.clone());
    let mut refs: HashSet<String> = task
        .nodes
        .iter()
        .filter(|n| selection.node_id.is_none() || removed.contains(&n.contract.id))
        .flat_map(|n| n.attempts.iter())
        .flat_map(|a| {
            a.checkpoint
                .iter()
                .cloned()
                .chain(a.previews.iter().cloned())
        })
        .chain(if selection.node_id.is_none() {
            task.checkpoint.clone()
        } else {
            None
        })
        .collect();
    for (_, nodes) in &task.plan_revisions {
        if selection.node_id.is_none() || nodes.iter().any(|n| removed.contains(&n.contract.id)) {
            for node in nodes {
                for a in &node.attempts {
                    refs.extend(a.checkpoint.iter().chain(a.previews.iter()).cloned());
                }
            }
        }
    }
    if selection.node_id.is_none() {
        for n in &task.nodes {
            for a in &n.attempts {
                refs.insert(a.base.clone());
            }
        }
    } else {
        for node in task
            .nodes
            .iter()
            .filter(|n| !removed.contains(&n.contract.id))
        {
            for a in &node.attempts {
                refs.remove(&a.base);
                for cp in a.checkpoint.iter().chain(a.previews.iter()) {
                    refs.remove(cp);
                }
            }
        }
        if let Some(base) = &base {
            refs.remove(base);
        }
    }
    let backups = store.directory().join("backups");
    std::fs::create_dir_all(&backups)?;
    ensure!(
        !std::fs::symlink_metadata(&backups)?
            .file_type()
            .is_symlink(),
        "备份目录不允许符号链接"
    );
    let backup = backups.join(format!(
        "discard-{}-{}",
        task.id,
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    std::fs::create_dir_all(&backup)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&backup, std::fs::Permissions::from_mode(0o700))?;
    }
    store.backup_database(&backup.join("workspace.db"))?;
    std::fs::write(backup.join("task.json"), serde_json::to_vec_pretty(&task)?)?;
    let archive = Command::new("tar")
        .arg("-czf")
        .arg(backup.join("candidate.tar.gz"))
        .arg("-C")
        .arg(&worktree)
        .arg(".")
        .output()?;
    ensure!(
        archive.status.success(),
        "候选备份失败，未撤回：{}",
        String::from_utf8_lossy(&archive.stderr)
    );
    source.bundle_candidate(&backup.join("candidate.bundle"), &branch, &refs)?;
    // Archive raw logs BEFORE destructive filesystem/database work. Never follow links.
    let sessions = store.directory().join("sessions");
    let mut raw = Vec::new();
    for path in std::fs::read_dir(&sessions)? {
        let path = path?.path();
        if task
            .sessions
            .iter()
            .any(|s| path.file_name().unwrap().to_string_lossy().contains(s))
        {
            let kind = std::fs::symlink_metadata(&path)?.file_type();
            ensure!(!kind.is_symlink(), "原始会话日志不允许符号链接");
            if kind.is_file() {
                std::fs::copy(&path, backup.join(path.file_name().unwrap()))?;
                raw.push(path);
            }
        }
    }
    if selection.node_id.is_none() {
        source.remove_candidate(&worktree, &branch)?;
        store.remove_records(&task.id, None)?;
        app.tasks.remove(index);
    } else {
        if let Some(base) = &base {
            Repository::open(&worktree)?.restore_candidate(base)?;
        }
        store.remove_records(&task.id, Some(&ids))?;
        let task = &mut app.tasks[index];
        task.nodes.retain(|n| !removed.contains(&n.contract.id));
        if let Some(plan) = &mut task.plan {
            plan.nodes.retain(|n| !removed.contains(&n.id));
        }
        task.plan_revisions
            .retain(|(_, nodes)| !nodes.iter().any(|n| removed.contains(&n.contract.id)));
        task.checkpoint = base
            .or_else(|| {
                task.nodes
                    .iter()
                    .rev()
                    .find_map(|n| n.checkpoint().map(str::to_owned))
            })
            .or(task.baseline.clone());
        task.active_node_id = None;
        task.discussion_pending = true;
        task.state = TaskState::Ready;
        if task.nodes.is_empty() {
            task.plan = None;
            task.state = TaskState::Interrupted;
        }
        store.save_task(task)?;
    }
    // Shared logs may contain deleted prompts/tools; only their private archive remains.
    for path in raw {
        std::fs::remove_file(path)?;
    }
    source.remove_checkpoint_refs(&refs)?;
    store.forget_versions(&refs)?;
    app.notes
        .retain(|note| !refs.iter().any(|cp| note.contains(cp)));
    app.tasks = store.tasks()?;
    app.items.retain(|i| !refs.contains(&i.version));
    app.review_version = None;
    app.latest_checkpoint = None;
    app.task_selection = app
        .tasks
        .iter()
        .find(|t| t.id == selection.task_id)
        .or_else(|| app.tasks.last())
        .map(|t| TaskSelection {
            task_id: t.id.clone(),
            node_id: None,
        });
    app.selected = app.selected.min(app.items.len().saturating_sub(1));
    if let Some((history, prompt)) = app
        .selected_task()
        .map(|t| (t.history.clone(), t.prompt.clone()))
    {
        app.history = history;
        app.task = Some(prompt);
    } else {
        app.history.clear();
        app.task = None;
    }
    app.tasks_revision = app.tasks_revision.wrapping_add(1);
    app.content_revision = app.content_revision.wrapping_add(1);
    if !app.agent_connected {
        app.planning_only = true;
    }
    app.status = "已删除并撤回候选修改 · 主项目未变 · 私有备份保留".into();
    store.save(app)?;
    Ok(())
}
