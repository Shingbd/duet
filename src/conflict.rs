//! Human-only conflict resolution in managed worktrees. Source/candidate remain untouched.
use crate::{
    checkpoint::{Repository, validate_path},
    merge::{self, MergePreview},
    nodes::TaskRecord,
    storage::WorkspaceStore,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Draft {
    pub task_id: String,
    pub signature: String,
    pub source_head: String,
    pub source_branch: String,
    pub delivery: String,
    pub base: String,
    pub initial: String,
    pub snapshot: String,
    pub nonce: String,
    pub branch: String,
    pub conflicts: Vec<String>,
    pub resolved: Vec<String>,
    #[serde(default)]
    pub extra_paths: Vec<String>,
    #[serde(default)]
    pub lockfile_pending: bool,
    #[serde(default)]
    pub advice: Option<Advice>,
    #[serde(default)]
    pub advice_error: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Replacement {
    pub old: String,
    pub new: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileAdvice {
    #[serde(default)]
    pub snapshot: Option<String>,
    pub path: String,
    pub description: String,
    pub source_impact: String,
    pub candidate_impact: String,
    pub merge_impact: String,
    pub recommendation: String,
    pub replacements: Vec<Replacement>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Advice {
    pub snapshot: String,
    pub files: Vec<FileAdvice>,
}
pub enum Preparation {
    Preview(MergePreview),
    Conflict(Draft),
}
fn signature(task: &TaskRecord) -> Result<String> {
    Ok(serde_json::to_string(&serde_json::to_value(task)?)?)
}
fn directory(store: &WorkspaceStore, id: &str) -> Result<PathBuf> {
    ensure!(
        !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()),
        "不受管任务 ID"
    );
    Ok(store.directory().join("integrations").join(id))
}
pub fn worktree(store: &WorkspaceStore, draft: &Draft) -> Result<PathBuf> {
    ensure!(
        !draft.nonce.is_empty() && draft.nonce.bytes().all(|b| b.is_ascii_digit()),
        "无效集成编号"
    );
    Ok(directory(store, &draft.task_id)?
        .join(&draft.nonce)
        .join("worktree"))
}
pub fn private_dir(path: &Path) -> Result<()> {
    for ancestor in path.ancestors() {
        if let Ok(meta) = fs::symlink_metadata(ancestor) {
            ensure!(!meta.file_type().is_symlink(), "集成路径不允许符号链接");
        }
    }
    fs::create_dir_all(path)?;
    ensure!(
        !fs::symlink_metadata(path)?.file_type().is_symlink(),
        "集成目录不允许符号链接"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}
pub fn save(store: &WorkspaceStore, draft: &Draft) -> Result<()> {
    let dir = directory(store, &draft.task_id)?;
    private_dir(&dir)?;
    let generation = dir.join(&draft.nonce);
    private_dir(&generation)?;
    let data = serde_json::to_vec_pretty(draft)?;
    for path in [generation.join("record.json"), dir.join("current.json")] {
        let temp = path.with_extension("tmp");
        ensure!(!temp.exists(), "集成记录临时文件已存在");
        use std::io::Write;
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temp)?;
        file.write_all(&data)?;
        file.sync_all()?;
        fs::rename(temp, path)?;
    }
    Ok(())
}
pub fn load(store: &WorkspaceStore, id: &str) -> Result<Option<Draft>> {
    let dir = directory(store, id)?;
    if !dir.exists() {
        return Ok(None);
    }
    ensure!(
        !fs::symlink_metadata(&dir)?.file_type().is_symlink(),
        "集成目录不允许符号链接"
    );
    ensure!(
        !fs::symlink_metadata(dir.parent().unwrap())?
            .file_type()
            .is_symlink(),
        "集成父目录不允许符号链接"
    );
    let file = dir.join("current.json");
    if !file.exists() {
        return Ok(None);
    }
    ensure!(
        !fs::symlink_metadata(&file)?.file_type().is_symlink(),
        "集成记录不允许符号链接"
    );
    let draft: Draft = serde_json::from_slice(&fs::read(file)?)?;
    ensure!(draft.task_id == id, "集成任务不匹配");
    Ok(Some(draft))
}
pub fn prepare(
    repo: &Repository,
    store: &WorkspaceStore,
    task: &TaskRecord,
    rebuild: bool,
) -> Result<Preparation> {
    merge::validate_delivery(repo, task)?;
    store.candidate(&task.id)?;
    repo.merge_guard()?;
    if !rebuild && let Some(mut draft) = load(store, &task.id)? {
        let candidate = identity(repo, store, &draft)?;
        let lock = safe_file(candidate.root(), "Cargo.lock")?;
        let source = repo.resolve_snapshot(&draft.source_head)?;
        let old = if repo.snapshot_has_path(&source, "Cargo.lock")? {
            Some(repo.read(&source, "Cargo.lock")?.into_bytes())
        } else {
            None
        };
        draft.lockfile_pending = if lock.exists() {
            Some(fs::read(lock)?) != old
        } else {
            old.is_some()
        };
        return Ok(Preparation::Conflict(draft));
    }
    let head = repo.baseline()?.commit;
    let delivery = task.checkpoint.clone().context("缺少最终交付")?;
    let (tree, conflicts) = repo.integration_tree(&head, &delivery)?;
    if conflicts.is_empty() {
        return Ok(Preparation::Preview(merge::prepare(repo, store, task)?));
    }
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_nanos()
        .to_string();
    let dir = directory(store, &task.id)?;
    private_dir(dir.parent().unwrap())?;
    private_dir(&dir)?;
    private_dir(&dir.join(&nonce))?;
    let snapshot = repo.integration_commit(&tree, &head, &delivery, &task.id)?;
    let draft = Draft {
        task_id: task.id.clone(),
        signature: signature(task)?,
        source_head: head.clone(),
        source_branch: repo.branch_ref()?,
        delivery: delivery.clone(),
        base: repo.merge_base(&head, &delivery)?.commit,
        initial: snapshot.commit.clone(),
        snapshot: snapshot.commit,
        nonce: nonce.clone(),
        branch: format!("duet/integration-{}-{nonce}", task.id),
        conflicts,
        resolved: Vec::new(),
        extra_paths: Vec::new(),
        lockfile_pending: false,
        advice: None,
        advice_error: None,
    };
    repo.create_worktree_at(&worktree(store, &draft)?, &draft.branch, &draft.initial)?;
    save(store, &draft)?;
    Ok(Preparation::Conflict(draft))
}
pub fn identity(repo: &Repository, store: &WorkspaceStore, draft: &Draft) -> Result<Repository> {
    let path = worktree(store, draft)?;
    let dir = path.parent().unwrap();
    ensure!(
        !fs::symlink_metadata(directory(store, &draft.task_id)?.parent().unwrap())?
            .file_type()
            .is_symlink(),
        "集成目录不允许符号链接"
    );
    for p in [
        directory(store, &draft.task_id)?,
        dir.to_path_buf(),
        path.clone(),
    ] {
        ensure!(
            !fs::symlink_metadata(p)?.file_type().is_symlink(),
            "集成工作区不允许符号链接"
        );
    }
    ensure!(
        draft.branch == format!("duet/integration-{}-{}", draft.task_id, draft.nonce),
        "不受管集成分支"
    );
    let candidate = Repository::open(&path)?;
    ensure!(
        candidate.root() == path.canonicalize()?
            && candidate.candidate_identity()?.0 == repo.candidate_identity()?.0
            && candidate.candidate_identity()?.1 == draft.branch
            && candidate.baseline()?.commit == draft.initial,
        "集成工作区身份/HEAD 已变化，拒绝写入"
    );
    Ok(candidate)
}
pub fn validate(
    repo: &Repository,
    store: &WorkspaceStore,
    task: &TaskRecord,
    draft: &Draft,
) -> Result<Repository> {
    merge::validate_delivery(repo, task)?;
    store.candidate(&task.id)?;
    repo.merge_guard()?;
    ensure!(
        draft.task_id == task.id
            && draft.signature == signature(task)?
            && draft.delivery == task.checkpoint.as_deref().unwrap(),
        "节点/审批变化，r 创建新集成工作区；旧修复保留"
    );
    ensure!(
        repo.baseline()?.commit == draft.source_head && repo.branch_ref()? == draft.source_branch,
        "主项目 HEAD/分支已变化，r 创建新集成工作区；旧修复保留"
    );
    ensure!(
        draft.extra_paths.iter().all(|p| p == "Cargo.lock"),
        "不支持的额外集成路径"
    );
    identity(repo, store, draft)
}
impl Draft {
    pub fn all_resolved(&self) -> bool {
        self.conflicts.iter().all(|p| self.resolved.contains(p))
    }
    pub fn needs_lockfile_permission(&self) -> bool {
        self.lockfile_pending && !self.extra_paths.iter().any(|p| p == "Cargo.lock")
    }
    pub fn next_step(&self) -> &'static str {
        if self.needs_lockfile_permission() {
            "▶ l → Enter：允许更新依赖版本清单 Cargo.lock"
        } else if self.all_resolved() {
            "▶ Enter：查看最终改动（还不会合入）"
        } else {
            "▶ y → Enter：先让 AI 解释当前文件，再由你选方案"
        }
    }
}
pub fn display(repo: &Repository, draft: &Draft, index: usize, side: u8) -> Result<String> {
    let path = draft.conflicts.get(index).context("冲突文件不存在")?;
    if side != 4 {
        return text(repo, draft, index, side);
    }
    let resolved = draft.resolved.contains(path);
    let mut info = if resolved {
        "✓ 这个文件已标记处理，不必重新选主项目或 AI 候选。\n\n3 查看修复后的固定代码；最终以整任务 diff 为准。\n".to_owned()
    } else {
        "⚠ 双方改到了同一处，Git 不知道该保留哪边，需要你决定。\n".to_owned()
    };
    if draft.needs_lockfile_permission() {
        info.push_str("\n还需你确认：Cargo.lock\n这是依赖版本清单，本次草稿改了它，但它不在原节点批准范围。\nl → Enter 只允许这项更新，不代表合入主项目，也不会运行测试。\n");
    } else if !draft.extra_paths.is_empty() {
        info.push_str("\n✓ 已允许更新 Cargo.lock，仍需审阅最终 diff。\n");
    }
    if let Some(error) = &draft.advice_error {
        info.push_str(&format!("\n⚠ 上次 AI 分析未完成：{error}\n"));
    }
    if let Some(advice) = draft
        .advice
        .as_ref()
        .and_then(|a| a.files.iter().find(|f| &f.path == path))
    {
        if resolved {
            info.push_str(&format!(
                "\n修复说明（参考，不代替代码审阅）\n{}\n",
                advice.merge_impact
            ));
        } else {
            info.push_str(&format!("\n为什么不能直接合入？\n{}\n\n选 o：只保留主项目，会怎样？\n{}\n\n选 t：只保留 AI 候选，会怎样？\n{}\n\n结合双方，会怎样？\n{}\n\nAI 的建议（由你决定）\n{}\n",advice.description,advice.source_impact,advice.candidate_impact,advice.merge_impact,advice.recommendation));
            if advice.replacements.is_empty() {
                info.push_str("\nAI 没有提供可直接应用的代码方案。可选 o / t，或 e → Enter 手动编辑；不要按 a。\n");
            } else {
                info.push_str("\na → Enter 才会把下列建议写入独立草稿，不会合入主项目。\n");
                for (i, patch) in advice.replacements.iter().enumerate() {
                    info.push_str(&format!("\n建议代码 {}（尚未应用）\n", i + 1));
                    for line in patch.old.lines() {
                        info.push_str(&format!("- {line}\n"));
                    }
                    for line in patch.new.lines() {
                        info.push_str(&format!("+ {line}\n"));
                    }
                }
            }
        }
    } else if !resolved {
        info.push_str("\ny → Enter 让 AI 只读解释双方修改与选择影响。\n没有方案会被自动应用。\n");
    }
    if resolved && draft.all_resolved() {
        info.push_str("\n接下来：查看最终 diff → 审阅 → 确认风险 → 明确合入。\n现在主项目未变，已处理不等于测试通过。\n");
    }
    Ok(info)
}
pub fn text(repo: &Repository, draft: &Draft, index: usize, side: u8) -> Result<String> {
    let path = draft.conflicts.get(index).context("冲突文件不存在")?;
    let commit = match side {
        0 => &draft.base,
        1 => &draft.source_head,
        2 => &draft.delivery,
        _ => &draft.snapshot,
    };
    let snapshot = repo.resolve_snapshot(commit)?;
    if !repo.snapshot_has_path(&snapshot, path)? {
        return Ok("∅ 此版本没有该文件".into());
    }
    repo.read(&snapshot, path)
}
fn safe_file(root: &Path, path: &str) -> Result<PathBuf> {
    validate_path(path)?;
    let file = root.join(path);
    let mut current = root.to_owned();
    for component in Path::new(path).components() {
        current.push(component);
        if let Ok(meta) = fs::symlink_metadata(&current) {
            ensure!(!meta.file_type().is_symlink(), "不允许符号链接：{path}");
        }
    }
    Ok(file)
}
fn has_markers(text: &str) -> bool {
    text.lines()
        .any(|l| l.starts_with("<<<<<<< ") || l == "=======" || l.starts_with(">>>>>>> "))
}
pub fn refresh(
    repo: &Repository,
    store: &WorkspaceStore,
    task: &TaskRecord,
    draft: &mut Draft,
) -> Result<()> {
    let candidate = validate(repo, store, task, draft)?;
    let snapshot = candidate.capture_worktree_from(&repo.resolve_snapshot(&draft.source_head)?)?;
    let changes = repo.snapshot_changes(&snapshot)?;
    ensure!(
        changes.iter().all(
            |c| task.nodes.iter().any(|n| n.contract.contains_path(&c.path))
                || draft.extra_paths.contains(&c.path)
        ),
        "集成修复出现任务范围外修改；拒绝生成集成版本"
    );
    draft.snapshot = snapshot.commit;
    draft.resolved.retain(|path| {
        let file = candidate.root().join(path);
        !file.exists() || fs::read_to_string(file).is_ok_and(|s| !has_markers(&s))
    });
    save(store, draft)
}
pub fn choose(
    repo: &Repository,
    store: &WorkspaceStore,
    task: &TaskRecord,
    draft: &mut Draft,
    index: usize,
    side: u8,
) -> Result<()> {
    ensure!(matches!(side, 1 | 2), "无效版本选择");
    let candidate = validate(repo, store, task, draft)?;
    let path = draft
        .conflicts
        .get(index)
        .context("冲突文件不存在")?
        .clone();
    let file = safe_file(candidate.root(), &path)?;
    let revision = if side == 1 {
        &draft.source_head
    } else {
        &draft.delivery
    };
    let snapshot = repo.resolve_snapshot(revision)?;
    if repo.snapshot_has_path(&snapshot, &path)? {
        if let Some(parent) = file.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&file, repo.read(&snapshot, &path)?)?;
    } else if file.exists() {
        ensure!(file.is_file(), "不能删除目录");
        fs::remove_file(&file)?;
    }
    mark(repo, store, task, draft, index)
}
pub fn edit_path(
    repo: &Repository,
    store: &WorkspaceStore,
    task: &TaskRecord,
    draft: &Draft,
    index: usize,
) -> Result<PathBuf> {
    let candidate = validate(repo, store, task, draft)?;
    let path = draft.conflicts.get(index).context("冲突文件不存在")?;
    let file = safe_file(candidate.root(), path)?;
    ensure!(file.is_file(), "该文件不存在，请先选主项目/候选版本");
    ensure!(fs::read(&file)?.len() <= 1024 * 1024, "编辑文件超过1MiB");
    std::str::from_utf8(&fs::read(&file)?)?;
    Ok(file)
}
pub fn mark(
    repo: &Repository,
    store: &WorkspaceStore,
    task: &TaskRecord,
    draft: &mut Draft,
    index: usize,
) -> Result<()> {
    let candidate = validate(repo, store, task, draft)?;
    let path = draft
        .conflicts
        .get(index)
        .context("冲突文件不存在")?
        .clone();
    let file = safe_file(candidate.root(), &path)?;
    ensure!(
        !file.exists() || !has_markers(&fs::read_to_string(file)?),
        "冲突标记尚未清除，未标记解决"
    );
    if !draft.resolved.contains(&path) {
        draft.resolved.push(path);
    }
    refresh(repo, store, task, draft)
}
pub fn authorize_lockfile(
    repo: &Repository,
    store: &WorkspaceStore,
    task: &TaskRecord,
    draft: &mut Draft,
) -> Result<()> {
    validate(repo, store, task, draft)?;
    ensure!(
        task.nodes
            .iter()
            .any(|n| n.contract.contains_path("Cargo.toml")),
        "任务未授权 Cargo.toml，不能追加 Cargo.lock"
    );
    if !draft.extra_paths.iter().any(|p| p == "Cargo.lock") {
        draft.extra_paths.push("Cargo.lock".into());
    }
    save(store, draft)
}
pub fn archive_task(
    repo: &Repository,
    store: &WorkspaceStore,
    id: &str,
    backup: &Path,
) -> Result<()> {
    let dir = directory(store, id)?;
    if !dir.exists() {
        return Ok(());
    }
    ensure!(
        !fs::symlink_metadata(&dir)?.file_type().is_symlink(),
        "集成目录不允许符号链接"
    );
    let mut drafts = Vec::new();
    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let record = entry.path().join("record.json");
        ensure!(
            !fs::symlink_metadata(&record)?.file_type().is_symlink(),
            "集成记录不允许符号链接"
        );
        let draft: Draft = serde_json::from_slice(&fs::read(record)?)?;
        ensure!(
            draft.task_id == id && entry.file_name().to_string_lossy() == draft.nonce,
            "集成记录编号不匹配"
        );
        identity(repo, store, &draft)?;
        drafts.push(draft);
    }
    let tar = std::process::Command::new("tar")
        .args(["-czf"])
        .arg(backup.join("integrations.tar.gz"))
        .arg("-C")
        .arg(dir.parent().unwrap())
        .arg(id)
        .output()?;
    ensure!(tar.status.success(), "集成草稿备份失败，未清理");
    repo.archive_integration_refs(id, &backup.join("integration-refs.bundle"))?;
    for draft in drafts {
        let commits =
            std::collections::HashSet::from([draft.initial.clone(), draft.snapshot.clone()]);
        repo.bundle_candidate(
            &backup.join(format!("integration-{}.bundle", draft.nonce)),
            &draft.branch,
            &commits,
        )?;
        repo.remove_candidate(&worktree(store, &draft)?, &draft.branch)?;
        repo.remove_checkpoint_refs(&commits)?;
    }
    fs::remove_dir_all(dir)?;
    Ok(())
}
/// Manual edits from review are integration revisions, never a mutation of approved node snapshots.
pub fn prepare_edit(
    repo: &Repository,
    store: &WorkspaceStore,
    task: &TaskRecord,
    path: &str,
) -> Result<Draft> {
    validate_path(path)?;
    ensure!(
        task.nodes.iter().any(|n| n.contract.contains_path(path)),
        "文件不在任务授权范围"
    );
    let mut draft = match prepare(repo, store, task, false)? {
        Preparation::Conflict(draft) => draft,
        Preparation::Preview(preview) => {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)?
                .as_nanos()
                .to_string();
            let dir = directory(store, &task.id)?;
            private_dir(&dir.join(&nonce))?;
            let draft = Draft {
                task_id: task.id.clone(),
                signature: signature(task)?,
                source_head: preview.source_head.clone(),
                source_branch: preview.branch,
                delivery: preview.delivery.clone(),
                base: repo
                    .merge_base(&preview.source_head, &preview.delivery)?
                    .commit,
                initial: preview.commit.clone(),
                snapshot: preview.commit,
                nonce: nonce.clone(),
                branch: format!("duet/integration-{}-{}", task.id, nonce),
                conflicts: Vec::new(),
                resolved: Vec::new(),
                extra_paths: Vec::new(),
                lockfile_pending: false,
                advice: None,
                advice_error: None,
            };
            repo.create_worktree_at(&worktree(store, &draft)?, &draft.branch, &draft.initial)?;
            draft
        }
    };
    validate(repo, store, task, &draft)?;
    if !draft.conflicts.iter().any(|p| p == path) {
        draft.conflicts.push(path.into());
    }
    draft.resolved.retain(|p| p != path);
    save(store, &draft)?;
    Ok(draft)
}
pub fn finish(
    repo: &Repository,
    store: &WorkspaceStore,
    task: &TaskRecord,
    draft: &mut Draft,
) -> Result<MergePreview> {
    refresh(repo, store, task, draft)?;
    ensure!(
        draft.conflicts.iter().all(|p| draft.resolved.contains(p)),
        "还有未确认解决的冲突"
    );
    let snapshot = repo.resolve_snapshot(&draft.snapshot)?;
    for change in repo.snapshot_changes(&snapshot)? {
        if repo.snapshot_has_path(&snapshot, &change.path)? {
            ensure!(
                !has_markers(&repo.read(&snapshot, &change.path)?),
                "{} 仍有冲突标记",
                change.path
            );
        }
    }
    let tree = repo.tree(&draft.snapshot)?;
    let snapshot = repo.integration_commit(&tree, &draft.source_head, &draft.delivery, &task.id)?;
    // A final immutable commit, independent of subsequent draft edits.
    merge::preview(
        repo,
        task,
        draft.source_head.clone(),
        draft.source_branch.clone(),
        draft.delivery.clone(),
        snapshot.commit,
    )
}
