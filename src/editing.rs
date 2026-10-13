//! Host-owned editing targets and snapshots. A terminal editor has no automatic approval powers.
use crate::{
    app::{App, Mode, View},
    checkpoint::{Repository, validate_path},
    conflict,
    embedded::Selection,
    storage::WorkspaceStore,
};
use anyhow::{Context, Result, ensure};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
pub enum Target {
    Project {
        root: PathBuf,
        path: String,
    },
    Integration {
        draft: Box<conflict::Draft>,
        index: usize,
    },
}
pub struct Session {
    pub terminal: crate::embedded::PtyEditor,
    pub target: Target,
}
/// Only executable-relative bundled tools are considered; never search the opened repository.
pub fn helix_candidates(
    configured: &Path,
    path: Option<&std::ffi::OsStr>,
    host: &Path,
) -> Vec<PathBuf> {
    if configured.is_absolute() || configured.components().count() > 1 {
        return vec![configured.to_path_buf()];
    }
    let mut candidates = path
        .map(|p| {
            std::env::split_paths(p)
                .map(|d| d.join(configured))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if configured == Path::new("hx")
        && let Some(bin) = host.parent()
    {
        candidates.push(bin.join("helix/hx"));
        if let Some(target) = bin.parent() {
            candidates.push(target.join("tools/helix-25.07.1-x86_64-linux/hx"));
        }
    }
    candidates
}
pub fn safe_file(root: &Path, path: &str) -> Result<PathBuf> {
    validate_path(path)?;
    let mut file = root.to_path_buf();
    for part in Path::new(path).components() {
        file.push(part);
        if let Ok(meta) = fs::symlink_metadata(&file) {
            ensure!(!meta.file_type().is_symlink(), "编辑路径不允许符号链接");
        }
    }
    let meta = fs::symlink_metadata(&file).context("文件不存在；删除文件暂不支持编辑")?;
    ensure!(
        meta.is_file() && meta.len() <= 1024 * 1024,
        "仅支持最多 1 MiB 的普通文件"
    );
    fs::read_to_string(&file).context("文件不是 UTF-8")?;
    Ok(file)
}
pub fn start(
    selection: Selection,
    app: &mut App,
    store: &WorkspaceStore,
    rows: u16,
    cols: u16,
) -> Result<Session> {
    ensure!(cfg!(unix), "内嵌 Helix 当前仅支持 Unix PTY");
    let configured = &app.config.helix_path;
    let candidates = helix_candidates(
        configured,
        std::env::var_os("PATH").as_deref(),
        &std::env::current_exe()?,
    );
    let executable = candidates
        .into_iter()
        .find(|p| {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                p.metadata()
                    .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            }
            #[cfg(not(unix))]
            {
                p.is_file()
            }
        })
        .with_context(|| {
            format!(
                "找不到 Helix：{}；请安装或配置 helix_path / --helix",
                configured.display()
            )
        })?
        .canonicalize()?;
    let root = store.directory().parent().context("缺少项目根目录")?;
    let project_repo = Repository::open(root).ok();
    let (target, cwd, path) = match selection {
        Selection::ProjectFile { path } => {
            ensure!(
                app.view == View::Project && app.project_tree.selected == path,
                "文件树选择已改变"
            );
            let file = safe_file(root, &path)?;
            (
                Target::Project {
                    root: root.into(),
                    path,
                },
                root.to_path_buf(),
                file,
            )
        }
        Selection::Project { path, version } => {
            let repo = project_repo.as_ref().context("固定项目版本需要 Git 仓库")?;
            ensure!(
                app.view == View::Project
                    && app
                        .current()
                        .is_some_and(|f| f.title == path && f.version == version),
                "选择已改变"
            );
            ensure!(
                app.project_version.as_deref() == Some(version.as_str()),
                "不是所选项目固定版本"
            );
            let file = safe_file(repo.root(), &path)?;
            (
                Target::Project {
                    root: repo.root().into(),
                    path,
                },
                repo.root().to_path_buf(),
                file,
            )
        }
        Selection::Review {
            task_id,
            path,
            version,
        } => {
            let repo = project_repo.as_ref().context("审核编辑需要 Git 仓库")?;
            ensure!(
                app.view == View::Review
                    && app
                        .current()
                        .is_some_and(|f| f.title == path && f.version == version),
                "审核选择已改变"
            );
            let task = app
                .tasks
                .iter()
                .find(|t| t.id == task_id)
                .context("任务不存在")?;
            task.merge_ready()
                .context("待审节点先完成审核；内嵌编辑审核代码仅用于已批准任务的最终集成修复")?;
            let draft = conflict::prepare_edit(repo, store, task, &path)?;
            let index = draft.conflicts.iter().position(|p| p == &path).unwrap();
            let cwd = conflict::worktree(store, &draft)?;
            let file = safe_file(&cwd, &path)?;
            (
                Target::Integration {
                    draft: Box::new(draft),
                    index,
                },
                cwd,
                file,
            )
        }
        Selection::Conflict { draft, index } => {
            let repo = project_repo.as_ref().context("集成编辑需要 Git 仓库")?;
            let task = app
                .tasks
                .iter()
                .find(|t| t.id == draft.task_id)
                .context("任务不存在")?;
            let cwd = conflict::validate(repo, store, task, &draft)?
                .root()
                .to_path_buf();
            let path = draft.conflicts.get(index).context("文件不存在")?;
            let file = safe_file(&cwd, path)?;
            (Target::Integration { draft, index }, cwd, file)
        }
    };
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let backup = store
        .directory()
        .join("backups")
        .join(format!("helix-{nonce}"));
    conflict::private_dir(&backup)?;
    fs::copy(&path, backup.join("before-file"))?;
    fs::write(
        backup.join("target.txt"),
        format!(
            "path={}\nmain_head={}\n",
            path.display(),
            project_repo
                .as_ref()
                .and_then(|r| r.baseline().ok())
                .map(|s| s.commit)
                .unwrap_or_else(|| "no-committed-baseline".into())
        ),
    )?;
    let terminal = crate::embedded::PtyEditor::spawn(&executable, &cwd, &path, rows, cols)?;
    app.mode = Mode::Browse;
    app.panel = crate::app::Panel::Reader;
    app.editor_focused = true;
    app.editor_project = matches!(target, Target::Project { .. });
    app.status = "Helix 在代码框内运行 · Ctrl+] 切回 Duet · :wq 保存退出 · 不自动合入".into();
    Ok(Session { terminal, target })
}
pub fn switch_project(session: &mut Session, store: &WorkspaceStore, path: &str) -> Result<()> {
    let Target::Project {
        root,
        path: current,
    } = &mut session.target
    else {
        anyhow::bail!("请先保存退出当前审核/冲突编辑器");
    };
    if current == path {
        return Ok(());
    }
    ensure!(
        !path.bytes().any(|b| matches!(b, 37 | 34 | 39 | 92)),
        "特殊文件名不通过 Helix 命令行切换；请在 Helix 文件选择器中手动打开，原缓冲区保留"
    );
    let file = safe_file(root, path)?;
    ensure!(
        !file
            .to_string_lossy()
            .bytes()
            .any(|b| matches!(b, 37 | 34 | 39 | 92)),
        "项目目录含特殊插值字符；请在 Helix 文件选择器中手动切换，原缓冲区保留"
    );
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let backup = store
        .directory()
        .join("backups")
        .join(format!("helix-switch-{nonce}"));
    conflict::private_dir(&backup)?;
    fs::copy(&file, backup.join("before-file"))?;
    fs::write(backup.join("target.txt"), file.to_string_lossy().as_bytes())?;
    let quoted = file
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    session
        .terminal
        .send(format!("\u{1b}\u{1b}:open \"{quoted}\"\r").as_bytes())?;
    *current = path.into();
    Ok(())
}
pub fn finish(target: Target, success: bool, app: &mut App, store: &WorkspaceStore) -> Result<()> {
    app.editor_screen = None;
    app.editor_focused = false;
    app.editor_project = false;
    match target {
        Target::Project { root, path } => {
            safe_file(&root, &path)?;
            app.status = if success {
                format!(
                    "Helix 已退出：{path} 的保存写入实际主项目；视图仍为固定 HEAD，提交后重开刷新。不自动提交/合入。"
                )
            } else {
                "Helix 非正常结束，已保存的主项目改动和备份保留；不会宣称保存成功".into()
            };
        }
        Target::Integration { mut draft, index } => {
            let repo = Repository::open(store.directory().parent().unwrap())?;
            let task = app
                .tasks
                .iter()
                .find(|t| t.id == draft.task_id)
                .context("任务不存在")?;
            if success {
                conflict::mark(&repo, store, task, &mut draft, index)?;
            } else {
                conflict::refresh(&repo, store, task, &mut draft)?;
            }
            let body = conflict::display(&repo, &draft, index, 3)?;
            app.mode = Mode::Conflict {
                draft: *draft,
                index,
                side: 3,
                scroll: 0,
                body,
            };
            app.status =
                "Helix 草稿已捕获为新固定版本 · 未改原候选/节点批准，仍须最终 diff 和合入确认"
                    .into();
        }
    }
    Ok(())
}
