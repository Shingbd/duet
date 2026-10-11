use crate::{
    app::{App, Page, Panel, View},
    nodes::{TaskRecord, TaskState},
};
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

pub struct WorkspaceStore {
    connection: Connection,
    directory: PathBuf,
    _lock: fs::File,
    last_ui: Option<String>,
    last_tasks: HashMap<String, String>,
    saved_tasks_revision: Option<(usize, u64)>,
    last_progress: HashMap<(String, String), (bool, u16, u16)>,
}

impl Drop for WorkspaceStore {
    fn drop(&mut self) {
        // Explicit unlock releases ownership even during another thread's fork/exec,
        // whose short-lived child may temporarily inherit the open file description.
        let _ = self._lock.unlock();
    }
}

#[derive(Serialize, Deserialize)]
struct SavedUi {
    light: bool,
    view: View,
    panel: Panel,
    page: Page,
    selected: Option<(String, String)>,
    notes: Vec<String>,
    #[serde(default)]
    task_scroll: u16,
    #[serde(default)]
    task_evidence_scroll: u16,
    #[serde(default)]
    task_selection: Option<crate::app::TaskSelection>,
    #[serde(default)]
    review_version: Option<String>,
    #[serde(default)]
    follow_live: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub session: String,
    pub role: String,
    pub content: String,
}

impl WorkspaceStore {
    pub fn open(project: &Path) -> Result<Self> {
        let project = project.canonicalize().context("项目文件夹不存在")?;
        ensure!(project.is_dir(), "项目路径必须是文件夹");
        let directory = project.join(".duet");
        match fs::create_dir(&directory) {
            Ok(()) => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
        ensure_regular(&directory, true)?;
        for name in [
            "workspace.db",
            "workspace.db-wal",
            "workspace.db-shm",
            "workspace.db-journal",
            ".gitignore",
            "workspace.lock",
        ] {
            let path = directory.join(name);
            if fs::symlink_metadata(&path).is_ok() {
                ensure_regular(&path, false)?;
            }
        }
        // Self-contained exclusion; do not modify the user's root .gitignore.
        let ignore = directory.join(".gitignore");
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(ignore)
        {
            Ok(mut file) => file.write_all(b"*\n")?,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(directory.join("workspace.lock"))?;
        lock.try_lock().context("该项目已被另一个工作台打开")?;
        let database = directory.join("workspace.db");
        let mut connection = Connection::open(&database)?;
        connection.busy_timeout(Duration::from_secs(3))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&database, fs::Permissions::from_mode(0o600))?;
        }
        let version: u32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        ensure!(version <= 5, "项目数据版本 {version} 高于当前程序支持版本");
        if version == 0 {
            let tx = connection.transaction()?;
            tx.execute_batch(
                "
                CREATE TABLE ui_state (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                CREATE TABLE reading_progress (
                    version TEXT NOT NULL, path TEXT NOT NULL,
                    is_read INTEGER NOT NULL, scroll INTEGER NOT NULL,
                    evidence_scroll INTEGER NOT NULL, PRIMARY KEY (version, path)
                );
                CREATE TABLE messages (
                    id INTEGER PRIMARY KEY, session TEXT NOT NULL,
                    role TEXT NOT NULL CHECK(role IN ('user','assistant','system')),
                    content TEXT NOT NULL, created_at INTEGER NOT NULL DEFAULT (unixepoch())
                );
                CREATE INDEX messages_session ON messages(session, id);
                PRAGMA user_version = 1;
            ",
            )?;
            tx.commit()?;
        }
        if version < 2 {
            let tx = connection.transaction()?;
            tx.execute_batch(
                "CREATE TABLE runs (
                id TEXT PRIMARY KEY, session TEXT NOT NULL, worktree TEXT NOT NULL,
                branch TEXT NOT NULL, prompt TEXT NOT NULL, status TEXT NOT NULL,
                checkpoint TEXT, created_at INTEGER NOT NULL DEFAULT (unixepoch())
            ); PRAGMA user_version = 2;",
            )?;
            tx.commit()?;
        }
        if version < 3 {
            let tx = connection.transaction()?;
            tx.execute_batch("CREATE TABLE tasks (id TEXT PRIMARY KEY, record TEXT NOT NULL);
                CREATE TABLE nodes (task_id TEXT NOT NULL REFERENCES tasks(id), node_id TEXT NOT NULL,
                    position INTEGER NOT NULL, record TEXT NOT NULL, PRIMARY KEY(task_id,node_id));
                PRAGMA user_version=3;")?;
            tx.commit()?;
        }
        if version < 4 {
            connection.pragma_update(None, "user_version", 4)?;
        }
        if version < 5 {
            connection.execute_batch("CREATE TABLE IF NOT EXISTS discussion_messages(task_id TEXT NOT NULL,node_id TEXT,message_id INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE); PRAGMA user_version=5;")?;
        }
        connection.pragma_update(None, "foreign_keys", true)?;
        // Exclusive project lock ensures these cannot belong to another live instance.
        connection.execute(
            "UPDATE runs SET status='Interrupted' WHERE status IN ('Running','Pausing','Paused','Cancelling')",
            [],
        )?;
        let mut store = Self {
            connection,
            directory,
            _lock: lock,
            last_ui: None,
            last_tasks: HashMap::new(),
            saved_tasks_revision: None,
            last_progress: HashMap::new(),
        };
        // Import real legacy runs, never manufacture structured contracts for them.
        let legacy = {
            let mut query = store.connection.prepare("SELECT r.id,r.prompt,r.session,r.status,r.checkpoint FROM runs r LEFT JOIN tasks t ON r.id=t.id WHERE t.id IS NULL ORDER BY r.created_at,r.id")?;
            query
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, Option<String>>(4)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        for (id, prompt, session, status, checkpoint) in legacy {
            let mut task = TaskRecord::legacy(&id, &prompt, &session);
            task.state = TaskState::from_run(&status);
            task.checkpoint = checkpoint;
            store.save_task(&task)?;
        }
        let repository = crate::checkpoint::Repository::open(&project).ok();
        for mut task in store.tasks()? {
            // Recover legacy turn ownership only when exact final summaries prove it.
            let mut ends = Vec::new();
            for node in &task.nodes {
                for attempt in &node.attempts {
                    if !attempt.summary.is_empty() {
                        let mut q=store.connection.prepare("SELECT id,session FROM messages WHERE role='assistant' AND content=?1 ORDER BY id")?;
                        for (id, session) in q
                            .query_map([&attempt.summary], |r| {
                                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
                            })?
                            .collect::<rusqlite::Result<Vec<_>>>()?
                        {
                            if task.sessions.contains(&session) {
                                ends.push((id, session, node.contract.id.clone()));
                            }
                        }
                    }
                }
            }
            ends.sort_by_key(|e| e.0);
            let mut cursors = HashMap::new();
            for (end, session, node) in ends {
                let start = cursors.get(&session).copied().unwrap_or(0_i64);
                store.connection.execute("INSERT OR IGNORE INTO discussion_messages SELECT ?1,?2,id FROM messages WHERE session=?3 AND id>?4 AND id<=?5",params![task.id,node,session,start,end])?;
                cursors.insert(session, end);
            }
            // Repair false completion using immutable diffs, not prose or mutable files.
            for node in &mut task.nodes {
                for attempt in &mut node.attempts {
                    if attempt.execution == crate::nodes::Execution::Implemented
                        && attempt.changed_files.is_none()
                        && let Some(cp) = &attempt.checkpoint
                        && let Some(repo) = &repository
                        && let Ok(snapshot) = repo.resolve_snapshot(cp)
                        && snapshot.parent.is_some()
                        && let Ok(changes) = repo.snapshot_changes(&snapshot)
                    {
                        attempt.changed_files = Some(changes.len());
                    }
                    if attempt.execution == crate::nodes::Execution::Implemented
                        && attempt.changed_files == Some(0)
                    {
                        attempt.execution = crate::nodes::Execution::NoChanges;
                    }
                }
                if node.execution == crate::nodes::Execution::Implemented
                    && node
                        .attempts
                        .last()
                        .is_some_and(|a| a.execution == crate::nodes::Execution::NoChanges)
                {
                    node.execution = crate::nodes::Execution::NoChanges;
                }
            }
            if task.state == TaskState::Implemented
                && task
                    .nodes
                    .iter()
                    .any(|n| n.execution == crate::nodes::Execution::NoChanges)
            {
                task.state = TaskState::Interrupted;
                store.connection.execute(
                    "UPDATE runs SET status='Interrupted' WHERE id=?1",
                    [&task.id],
                )?;
            }
            task.interrupt();
            store.save_task(&task)?;
        }
        Ok(store)
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    pub fn begin_run(
        &mut self,
        id: &str,
        session: &str,
        worktree: &Path,
        branch: &str,
        prompt: &str,
    ) -> Result<()> {
        self.connection.execute(
            "INSERT INTO runs(id,session,worktree,branch,prompt,status)
            VALUES(?1,?2,?3,?4,?5,'Running')",
            params![id, session, worktree.to_string_lossy(), branch, prompt],
        )?;
        self.save_task(&TaskRecord::legacy(id, prompt, session))?;
        Ok(())
    }

    pub fn finish_run(&mut self, id: &str, status: &str, checkpoint: Option<&str>) -> Result<()> {
        self.connection.execute(
            "UPDATE runs SET status=?2,checkpoint=?3 WHERE id=?1",
            params![id, status, checkpoint],
        )?;
        if let Some(mut task) = self.tasks()?.into_iter().find(|t| t.id == id) {
            task.state = TaskState::from_run(status);
            task.checkpoint = checkpoint.map(str::to_owned);
            self.save_task(&task)?;
        }
        Ok(())
    }

    pub fn candidate(&self, id: &str) -> Result<(PathBuf, String)> {
        let (path, branch): (String, String) = self.connection.query_row(
            "SELECT worktree,branch FROM runs WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        ensure!(
            !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()),
            "非受管任务 ID，拒绝文件操作"
        );
        let expected = self.directory.join("worktrees").join(id);
        ensure!(
            PathBuf::from(&path) == expected && branch == format!("duet/task-{id}"),
            "候选工作区与任务记录不匹配"
        );
        let parent = expected.parent().unwrap();
        ensure!(
            !fs::symlink_metadata(parent)?.file_type().is_symlink()
                && !fs::symlink_metadata(&expected)?.file_type().is_symlink(),
            "候选工作区不允许符号链接"
        );
        let repo = crate::checkpoint::Repository::open(&expected)?;
        ensure!(
            repo.root() == expected.canonicalize()?,
            "不是受管候选仓库根"
        );
        let source = crate::checkpoint::Repository::open(self.directory.parent().unwrap())?;
        let (common, actual_branch) = repo.candidate_identity()?;
        ensure!(
            common == source.candidate_identity()?.0 && actual_branch == branch,
            "候选不属于当前主项目或分支不匹配"
        );
        Ok((expected, branch))
    }
    pub fn link_message(&self, task: &str, node: Option<&str>, message: i64) -> Result<()> {
        self.connection.execute(
            "INSERT INTO discussion_messages VALUES(?1,?2,?3)",
            params![task, node, message],
        )?;
        Ok(())
    }
    pub fn remove_records(&mut self, task: &str, nodes: Option<&[String]>) -> Result<()> {
        if let Some(nodes) = nodes {
            let record = self
                .tasks()?
                .into_iter()
                .find(|t| t.id == task)
                .context("任务不存在")?;
            for session in &record.sessions {
                let messages = {
                    let mut q = self
                        .connection
                        .prepare("SELECT id,content FROM messages WHERE session=?1")?;
                    q.query_map([session], |r| {
                        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?
                };
                for (id, content) in messages {
                    if content
                        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-')
                        .any(|token| nodes.iter().any(|n| n == token))
                    {
                        self.connection
                            .execute("DELETE FROM messages WHERE id=?1", [id])?;
                    }
                }
            }
            for node in nodes {
                self.connection.execute("DELETE FROM messages WHERE id IN (SELECT message_id FROM discussion_messages WHERE task_id=?1 AND node_id=?2)",params![task,node])?;
                self.connection.execute(
                    "DELETE FROM nodes WHERE task_id=?1 AND node_id=?2",
                    params![task, node],
                )?;
            }
        } else {
            let record = self
                .tasks()?
                .into_iter()
                .find(|t| t.id == task)
                .context("任务不存在")?;
            for session in &record.sessions {
                self.connection
                    .execute("DELETE FROM messages WHERE session=?1", [session])?;
            }
            self.connection.execute("DELETE FROM messages WHERE id IN (SELECT message_id FROM discussion_messages WHERE task_id=?1)",[task])?;
            self.connection
                .execute("DELETE FROM nodes WHERE task_id=?1", [task])?;
            self.connection
                .execute("DELETE FROM tasks WHERE id=?1", [task])?;
            self.connection
                .execute("DELETE FROM runs WHERE id=?1", [task])?;
        }
        self.connection.execute(
            "DELETE FROM discussion_messages WHERE message_id NOT IN (SELECT id FROM messages)",
            [],
        )?;
        self.last_tasks.remove(task);
        self.saved_tasks_revision = None;
        Ok(())
    }
    pub fn forget_versions(&mut self, versions: &std::collections::HashSet<String>) -> Result<()> {
        for version in versions {
            self.connection
                .execute("DELETE FROM reading_progress WHERE version=?1", [version])?;
        }
        self.last_progress
            .retain(|(version, _), _| !versions.contains(version));
        Ok(())
    }
    pub fn backup_database(&self, path: &Path) -> Result<()> {
        let path = path.to_str().context("备份路径不是 UTF-8")?;
        self.connection.execute("VACUUM INTO ?1", [path])?;
        Ok(())
    }
    pub fn set_session(&mut self, id: &str, session: &str) -> Result<()> {
        ensure!(
            self.connection.execute(
                "UPDATE runs SET session=?2 WHERE id=?1",
                params![id, session]
            )? == 1,
            "任务记录不存在"
        );
        Ok(())
    }
    pub fn save_task(&mut self, task: &TaskRecord) -> Result<()> {
        let encoded = serde_json::to_string(task)?;
        if self.last_tasks.get(&task.id) == Some(&encoded) {
            return Ok(());
        }
        let mut header = serde_json::to_value(task)?;
        header.as_object_mut().unwrap().remove("nodes");
        let tx = self.connection.transaction()?;
        let obsolete: Vec<String> = {
            let mut q = tx.prepare("SELECT node_id FROM nodes WHERE task_id=?1")?;
            q.query_map([&task.id], |r| r.get(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        for id in obsolete {
            if !task.nodes.iter().any(|n| n.contract.id == id) {
                tx.execute(
                    "DELETE FROM nodes WHERE task_id=?1 AND node_id=?2",
                    params![task.id, id],
                )?;
            }
        }
        tx.execute(
            "INSERT INTO tasks VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET record=excluded.record",
            params![task.id, header.to_string()],
        )?;
        for (position, node) in task.nodes.iter().enumerate() {
            tx.execute("INSERT INTO nodes VALUES(?1,?2,?3,?4) ON CONFLICT(task_id,node_id) DO UPDATE SET position=excluded.position,record=excluded.record",
                params![task.id,node.contract.id,position,serde_json::to_string(node)?])?;
        }
        tx.commit()?;
        self.last_tasks.insert(task.id.clone(), encoded);
        Ok(())
    }
    pub fn tasks(&self) -> Result<Vec<TaskRecord>> {
        let mut query = self
            .connection
            .prepare("SELECT record FROM tasks ORDER BY rowid")?;
        let headers = query
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut tasks = Vec::new();
        for header in headers {
            let mut task: TaskRecord =
                serde_json::from_str(&header).context("任务记录损坏，未覆盖原数据")?;
            let mut query = self
                .connection
                .prepare("SELECT record FROM nodes WHERE task_id=?1 ORDER BY position")?;
            for node in query.query_map([&task.id], |row| row.get::<_, String>(0))? {
                task.nodes
                    .push(serde_json::from_str(&node?).context("节点记录损坏，未覆盖原数据")?);
            }
            for session in &task.sessions {
                task.history.extend(self.history(session)?);
            }
            tasks.push(task);
        }
        Ok(tasks)
    }

    pub fn restore(&self, app: &mut App) -> Result<()> {
        app.tasks = self.tasks()?;
        let saved: Option<String> = self
            .connection
            .query_row(
                "SELECT value FROM ui_state WHERE key='workspace'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        let mut saved_selection = None;
        if let Some(saved) = saved {
            let saved: SavedUi =
                serde_json::from_str(&saved).context("工作台状态损坏，未覆盖原数据")?;
            app.light = saved.light;
            app.view = saved.view;
            app.panel = saved.panel;
            app.page = saved.page;
            app.notes = saved.notes;
            app.task_scroll = saved.task_scroll;
            app.task_evidence_scroll = saved.task_evidence_scroll;
            app.task_selection = saved.task_selection;
            app.review_version = saved.review_version;
            app.follow_live = saved.follow_live;
            saved_selection = saved.selected;
        }
        let latest: Option<(String, String, String, String, Option<String>)> = self
            .connection
            .query_row(
                "SELECT id,prompt,session,status,checkpoint FROM runs ORDER BY created_at DESC,id DESC LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?,row.get(4)?)),
            )
            .optional()?;
        let mut latest_checkpoint = None;
        if let Some((id, prompt, session, status, checkpoint)) = latest {
            if app.task_selection.is_none() {
                app.task_selection = Some(crate::app::TaskSelection {
                    task_id: id.clone(),
                    node_id: None,
                });
            }
            app.task = Some(prompt);
            app.history = app
                .tasks
                .iter()
                .find(|t| t.id == id)
                .map(|t| t.history.clone())
                .unwrap_or(self.history(&session)?)
                .into_iter()
                .filter(|message| !message.content.trim().is_empty())
                .collect();
            if status == "Interrupted" {
                app.status = "会话已中断 · 不自动续跑；未发送的纠正需重新提交".into();
            }
            app.run.status = match status.as_str() {
                "Settled" => crate::rpc::RunStatus::Settled,
                "Failed" => crate::rpc::RunStatus::Failed,
                "Aborted" | "Interrupted" => crate::rpc::RunStatus::Aborted,
                _ => crate::rpc::RunStatus::Idle,
            };
            latest_checkpoint = checkpoint;
        }
        // Restore all real task/node attempts so history navigation never reads mutable files.
        for task in app.tasks.clone() {
            let snapshots: Vec<_> = if task.nodes.is_empty() {
                task.checkpoint
                    .iter()
                    .map(|cp| (cp.clone(), None))
                    .collect()
            } else {
                task.nodes
                    .iter()
                    .enumerate()
                    .flat_map(|(index, n)| {
                        n.attempts.iter().flat_map(move |a| {
                            a.checkpoint
                                .iter()
                                .chain(a.previews.iter())
                                .map(move |cp| (cp.clone(), Some(index)))
                        })
                    })
                    .collect()
            };
            let archived = task
                .plan_revisions
                .iter()
                .flat_map(|(_, nodes)| nodes.iter())
                .flat_map(|n| n.attempts.iter())
                .flat_map(|a| a.checkpoint.iter().chain(a.previews.iter()))
                .map(|cp| (cp.clone(), None));
            for (checkpoint, index) in snapshots.into_iter().chain(archived) {
                let restored = (|| -> Result<()> {
                    let repo = crate::checkpoint::Repository::open(
                        self.directory.parent().context("项目目录无效")?,
                    )?;
                    let snapshot = repo.resolve_snapshot(&checkpoint)?;
                    if let Some(index) = index {
                        app.append_node_checkpoint(&repo, &snapshot, &task.id, index)
                    } else {
                        app.append_checkpoint(&repo, &snapshot)?;
                        let summary = task
                            .history
                            .iter()
                            .rev()
                            .find(|m| m.role == "assistant")
                            .map(|m| m.content.as_str())
                            .unwrap_or("");
                        for item in app
                            .items
                            .iter_mut()
                            .filter(|i| i.version == snapshot.commit)
                        {
                            item.intent = task.prompt.clone();
                            item.explanation = summary.into();
                        }
                        Ok(())
                    }
                })();
                if let Err(error) = restored {
                    app.status = format!("对话已恢复，但审核快照无法恢复：{error}");
                }
            }
        }
        app.latest_checkpoint = latest_checkpoint;
        // Delivery items must exist before restoring their selection/reading positions.
        if let Some((version, title)) = saved_selection {
            let selected = app.items.iter().position(|item| {
                item.version == version
                    && (item.title == title
                        || title
                            == format!(
                                "待审 · {}{}",
                                item.title,
                                if item.change == Some(crate::checkpoint::ChangeKind::Deleted) {
                                    "（已删除）"
                                } else {
                                    ""
                                }
                            ))
            });
            if let Some(index) = selected {
                app.selected = index;
            } else if title == "变更"
                && app.latest_checkpoint.as_deref() == Some(&version)
                && let Some(index) = app.changed_indices().first()
            {
                app.selected = *index;
            }
        }
        for item in &mut app.items {
            let mut state: Option<(bool, u16, u16)> = self.connection.query_row(
                "SELECT is_read, scroll, evidence_scroll FROM reading_progress WHERE version=?1 AND path=?2",
                params![item.version, item.title], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            ).optional()?;
            if state.is_none() && item.change.is_some() {
                let legacy = format!(
                    "待审 · {}{}",
                    item.title,
                    if item.change == Some(crate::checkpoint::ChangeKind::Deleted) {
                        "（已删除）"
                    } else {
                        ""
                    }
                );
                state = self.connection.query_row(
                    "SELECT is_read,scroll,evidence_scroll FROM reading_progress WHERE version=?1 AND path=?2",
                    params![item.version,legacy], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional()?;
            }
            if let Some((read, scroll, evidence_scroll)) = state {
                item.read = read;
                item.scroll = scroll;
                item.evidence_scroll = evidence_scroll;
            }
        }
        Ok(())
    }

    pub fn save(&mut self, app: &App) -> Result<()> {
        let revision = (app.tasks.len(), app.tasks_revision);
        if self.saved_tasks_revision != Some(revision) {
            for task in &app.tasks {
                self.save_task(task)?;
            }
            self.saved_tasks_revision = Some(revision);
        }
        let saved = SavedUi {
            light: app.light,
            view: app.view,
            panel: app.panel,
            page: app.page,
            selected: app
                .current()
                .map(|item| (item.version.clone(), item.title.clone())),
            notes: app.notes.clone(),
            task_scroll: app.task_scroll,
            task_evidence_scroll: app.task_evidence_scroll,
            task_selection: app.task_selection.clone(),
            review_version: app.review_version.clone(),
            follow_live: app.follow_live,
        };
        let encoded = serde_json::to_string(&saved)?;
        let changed_ui = self.last_ui.as_ref() != Some(&encoded);
        let changed: Vec<_> = app
            .items
            .iter()
            .filter(|item| {
                self.last_progress
                    .get(&(item.version.clone(), item.title.clone()))
                    != Some(&(item.read, item.scroll, item.evidence_scroll))
            })
            .collect();
        if !changed_ui && changed.is_empty() {
            return Ok(());
        }
        let tx = self.connection.transaction()?;
        if changed_ui {
            tx.execute(
                "INSERT INTO ui_state(key,value) VALUES('workspace',?1)
                ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                [&encoded],
            )?;
        }
        {
            let mut statement = tx.prepare_cached(
                "INSERT INTO reading_progress VALUES(?1,?2,?3,?4,?5)
                ON CONFLICT(version,path) DO UPDATE SET is_read=excluded.is_read,
                    scroll=excluded.scroll, evidence_scroll=excluded.evidence_scroll",
            )?;
            for item in &changed {
                statement.execute(params![
                    item.version,
                    item.title,
                    item.read,
                    item.scroll,
                    item.evidence_scroll
                ])?;
            }
        }
        tx.commit()?;
        self.last_ui = Some(encoded);
        for item in changed {
            self.last_progress.insert(
                (item.version.clone(), item.title.clone()),
                (item.read, item.scroll, item.evidence_scroll),
            );
        }
        Ok(())
    }

    // Reserved for real user/agent conversation events; UI commands are not fake chat.
    pub fn append_message(&mut self, session: &str, role: &str, content: &str) -> Result<i64> {
        ensure!(!session.is_empty(), "会话 ID 不能为空");
        ensure!(
            matches!(role, "user" | "assistant" | "system"),
            "无效消息角色"
        );
        self.connection.execute(
            "INSERT INTO messages(session,role,content) VALUES(?1,?2,?3)",
            params![session, role, content],
        )?;
        Ok(self.connection.last_insert_rowid())
    }

    pub fn history(&self, session: &str) -> Result<Vec<Message>> {
        let mut statement = self
            .connection
            .prepare("SELECT session,role,content FROM messages WHERE session=?1 ORDER BY id")?;
        Ok(statement
            .query_map([session], |row| {
                Ok(Message {
                    session: row.get(0)?,
                    role: row.get(1)?,
                    content: row.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }
}

fn ensure_regular(path: &Path, directory: bool) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        !metadata.file_type().is_symlink(),
        "拒绝通过符号链接读写项目数据：{}",
        path.display()
    );
    ensure!(
        if directory {
            metadata.is_dir()
        } else {
            metadata.is_file()
        },
        "无效项目数据路径：{}",
        path.display()
    );
    Ok(())
}
