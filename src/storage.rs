use crate::app::{App, Page, Panel, View};
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

pub struct WorkspaceStore {
    connection: Connection,
    directory: PathBuf,
    _lock: fs::File,
}

#[derive(Serialize, Deserialize)]
struct SavedUi {
    light: bool,
    view: View,
    panel: Panel,
    page: Page,
    selected: Option<(String, String)>,
    notes: Vec<String>,
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
        ensure!(version <= 2, "项目数据版本 {version} 高于当前程序支持版本");
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
        // Exclusive project lock ensures these cannot belong to another live instance.
        connection.execute(
            "UPDATE runs SET status='Interrupted' WHERE status='Running'",
            [],
        )?;
        Ok(Self {
            connection,
            directory,
            _lock: lock,
        })
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
        Ok(())
    }

    pub fn finish_run(&mut self, id: &str, status: &str, checkpoint: Option<&str>) -> Result<()> {
        self.connection.execute(
            "UPDATE runs SET status=?2,checkpoint=?3 WHERE id=?1",
            params![id, status, checkpoint],
        )?;
        Ok(())
    }

    pub fn restore(&self, app: &mut App) -> Result<()> {
        let saved: Option<String> = self
            .connection
            .query_row(
                "SELECT value FROM ui_state WHERE key='workspace'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(saved) = saved {
            let saved: SavedUi =
                serde_json::from_str(&saved).context("工作台状态损坏，未覆盖原数据")?;
            app.light = saved.light;
            app.view = saved.view;
            app.panel = saved.panel;
            app.page = saved.page;
            app.notes = saved.notes;
            if let Some((version, title)) = saved.selected
                && let Some(index) = app
                    .items
                    .iter()
                    .position(|i| i.version == version && i.title == title)
            {
                app.selected = index;
            }
        }
        let latest: Option<(String, String, String)> = self
            .connection
            .query_row(
                "SELECT prompt,session,status FROM runs ORDER BY created_at DESC,id DESC LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        if let Some((prompt, session, status)) = latest {
            app.task = Some(prompt);
            app.history = self
                .history(&session)?
                .into_iter()
                .map(|message| format!("{}\n{}", message.role, message.content))
                .collect();
            app.run.status = match status.as_str() {
                "Settled" => crate::rpc::RunStatus::Settled,
                "Failed" => crate::rpc::RunStatus::Failed,
                "Aborted" | "Interrupted" => crate::rpc::RunStatus::Aborted,
                _ => crate::rpc::RunStatus::Idle,
            };
        }
        for item in &mut app.items {
            let state: Option<(bool, u16, u16)> = self.connection.query_row(
                "SELECT is_read, scroll, evidence_scroll FROM reading_progress WHERE version=?1 AND path=?2",
                params![item.version, item.title], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            ).optional()?;
            if let Some((read, scroll, evidence_scroll)) = state {
                item.read = read;
                item.scroll = scroll;
                item.evidence_scroll = evidence_scroll;
            }
        }
        Ok(())
    }

    pub fn save(&mut self, app: &App) -> Result<()> {
        let saved = SavedUi {
            light: app.light,
            view: app.view,
            panel: app.panel,
            page: app.page,
            selected: app
                .current()
                .map(|item| (item.version.clone(), item.title.clone())),
            notes: app.notes.clone(),
        };
        let encoded = serde_json::to_string(&saved)?;
        let tx = self.connection.transaction()?;
        tx.execute(
            "INSERT INTO ui_state(key,value) VALUES('workspace',?1)
            ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            [encoded],
        )?;
        for item in &app.items {
            tx.execute(
                "INSERT INTO reading_progress VALUES(?1,?2,?3,?4,?5)
                ON CONFLICT(version,path) DO UPDATE SET is_read=excluded.is_read,
                    scroll=excluded.scroll, evidence_scroll=excluded.evidence_scroll",
                params![
                    item.version,
                    item.title,
                    item.read,
                    item.scroll,
                    item.evidence_scroll
                ],
            )?;
        }
        tx.commit()?;
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
