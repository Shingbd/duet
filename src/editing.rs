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
    Workspace {
        directory: PathBuf,
    },
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
    pub workspace: crate::helix_workspace::Workspace,
    pub cwd: PathBuf,
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
    let mut workspace = crate::helix_workspace::Workspace::new(root, store, app.light)?;
    let (target, cwd, path) = match selection {
        Selection::ReviewBuffer => {
            let b = app.current_buffer().context("无审核缓冲区")?;
            let file = workspace.review(b, project_repo.as_ref().context("审核需要 Git")?)?;
            (
                Target::Workspace {
                    directory: workspace.directory.clone(),
                },
                workspace.directory.clone(),
                file,
            )
        }
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
    if matches!(target, Target::Project { .. }) {
        let rel = path.strip_prefix(root)?.to_str().context("路径不是UTF-8")?;
        workspace.project(rel)?;
    }
    workspace.active = Some(path.clone());
    let terminal = crate::embedded::PtyEditor::spawn_managed(
        &executable,
        &cwd,
        &path,
        rows,
        cols,
        Some(&workspace.config_home()),
    )?;
    app.mode = Mode::Browse;
    app.panel = crate::app::Panel::Reader;
    app.editor_focused = true;
    app.editor_project = matches!(target, Target::Project { .. });
    app.editor_conflict = matches!(target, Target::Integration { .. });
    app.status = "Helix 在代码框内运行 · Ctrl+] 切回 Duet · :wq 保存退出 · 不自动合入".into();
    Ok(Session {
        terminal,
        target,
        workspace,
        cwd,
    })
}
pub fn switch_project(session: &mut Session, store: &WorkspaceStore, path: &str) -> Result<()> {
    ensure!(
        !matches!(session.target, Target::Integration { .. }),
        "请先退出冲突编辑器"
    );
    let file = session.workspace.project(path)?;
    if session.workspace.active.as_ref() == Some(&file) {
        return Ok(());
    }
    let command = crate::helix_workspace::open_command(&file)?;
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let backup = store
        .directory()
        .join("backups")
        .join(format!("helix-switch-{nonce}"));
    conflict::private_dir(&backup)?;
    fs::copy(&file, backup.join("before-file"))?;
    fs::write(backup.join("target.txt"), file.to_string_lossy().as_bytes())?;
    session.terminal.send(&command)?;
    session.workspace.active = Some(file);
    session.target = Target::Workspace {
        directory: session.workspace.directory.clone(),
    };
    Ok(())
}
pub fn switch_review(session: &mut Session, app: &mut App, repo: &Repository) -> Result<bool> {
    ensure!(
        !matches!(session.target, Target::Integration { .. }),
        "请先退出冲突编辑器"
    );
    let b = app.current_buffer().context("没有审核缓冲区")?;
    let file = session.workspace.review(b, repo)?;
    if session.workspace.active.as_ref() == Some(&file) {
        return Ok(false);
    }
    session
        .terminal
        .send(&crate::helix_workspace::open_command(&file)?)?;
    session.workspace.active = Some(file);
    session.target = Target::Workspace {
        directory: session.workspace.directory.clone(),
    };
    app.editor_project = false;
    app.editor_focused = false;
    app.panel = crate::app::Panel::Queue;
    Ok(true)
}
pub fn poll_bridge(session: &mut Session, app: &mut App) -> Result<bool> {
    if session.workspace.light != app.light {
        session.terminal.send(
            format!(
                "\x1b\x1b:theme duet-{}\r",
                if app.light { "light" } else { "dark" }
            )
            .as_bytes(),
        )?;
        session.workspace.light = app.light;
    }
    let Some(frame) = session.workspace.take_frame()? else {
        return Ok(false);
    };
    let binding = session
        .workspace
        .binding(&frame.meta.path, &session.cwd)?
        .clone();
    if binding.identity.is_none() {
        app.view = View::Project;
        app.editor_project = true;
        app.review_compare = false;
        ensure!(
            matches!(frame.meta.action.as_str(), "leave" | "sync" | "publish"),
            "项目代码讨论尚未开放；未发送代码"
        );
        let path = binding
            .file
            .strip_prefix(&session.workspace.manifest.root)?
            .to_str()
            .context("非 UTF-8 路径")?;
        app.project_tree.reveal(path);
        session.workspace.active = Some(binding.file);
        app.editor_focused = false;
        app.panel = crate::app::Panel::Queue;
        return Ok(true);
    }
    let context = session.workspace.absorb(&frame, app, &session.cwd)?;
    app.view = View::Review;
    app.editor_project = false;
    let id = binding.identity.as_ref().unwrap();
    if app.view == View::Review {
        let selection = crate::app::TaskSelection {
            task_id: id.task.clone(),
            node_id: id.node.clone(),
        };
        if app.task_selection.as_ref() != Some(&selection) {
            app.select_task_entry(selection);
        }
        if let Some(i) = app
            .items
            .iter()
            .position(|i| i.title == id.path && i.version == id.version)
        {
            app.selected = i;
        }
    }
    match frame.meta.action.as_str() {
        "ask" => {
            app.review_chat_context = Some(context);
            app.review_chat_open = true;
            app.mode = Mode::ReviewChat {
                text: String::new(),
            };
            app.panel = crate::app::Panel::Evidence;
            app.editor_focused = false;
        }
        "publish" => {
            app.requested_review_publish = true;
            app.editor_focused = false;
            app.panel = crate::app::Panel::Queue;
        }
        "apply" => {
            if let Some(proposal) = session.workspace.confirm.take() {
                session
                    .workspace
                    .stage_apply(&proposal, &frame, app, &session.cwd)?;
                session.terminal.send(b"\x1bp\x19")?;
                session.workspace.pending = Some(std::time::Instant::now());
            } else if let Some(proposal) = app.review_proposal.clone() {
                ensure!(
                    context.buffer == proposal.context.buffer
                        && context.revision == proposal.context.revision,
                    "讨论后 Helix 文本已变化；请重新提问"
                );
                app.mode = Mode::ConfirmReviewPatch {
                    proposal: Box::new(proposal),
                };
                app.editor_focused = false;
            } else {
                app.status = "尚无可应用建议；Ctrl+g 先提问".into();
            }
        }
        "leave" => {
            app.editor_focused = false;
            app.panel = crate::app::Panel::Queue;
        }
        "diff" => {
            app.review_compare = true;
            app.editor_focused = false;
            app.panel = crate::app::Panel::Reader;
        }
        "sync" => {
            if session.workspace.applying {
                session.workspace.applying = false;
                app.status = "建议已写入 Helix 未保存草稿；未发布/批准/合入".into();
            }
            app.editor_focused = false;
            app.panel = crate::app::Panel::Queue;
        }
        _ => anyhow::bail!("未知审核上下文请求"),
    }
    Ok(true)
}
pub fn finish(target: Target, success: bool, app: &mut App, store: &WorkspaceStore) -> Result<()> {
    app.editor_screen = None;
    app.editor_focused = false;
    app.editor_project = false;
    app.editor_conflict = false;
    app.review_compare = false;
    match target {
        Target::Workspace { directory } => {
            crate::helix_workspace::Workspace::sync_saved(&directory, app)?;
            app.helix_unknown = false;
            app.panel = crate::app::Panel::Queue;
            app.status = "Helix 已退出；已保存的独立审核草稿保留，未自动发布/批准/合入".into();
        }
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
