use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Component, Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_FILE_BYTES: u64 = 1_048_576;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub commit: String,
    pub parent: Option<String>,
    pub reference: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    pub path: String,
    pub deleted: bool,
}

#[derive(Debug, Clone)]
pub struct Repository {
    root: PathBuf,
}

impl Repository {
    pub fn open(path: &Path) -> Result<Self> {
        let output = Command::new("git")
            .arg("-C")
            .arg(path)
            .args(["rev-parse", "--show-toplevel"])
            .output()
            .context("无法启动 Git")?;
        ensure!(
            output.status.success(),
            "不是有效 Git 工作区：{}",
            path.display()
        );
        let root = PathBuf::from(String::from_utf8(output.stdout)?.trim());
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn git(&self, args: &[&str], index: Option<&Path>) -> Result<String> {
        let mut command = Command::new("git");
        command.arg("-C").arg(&self.root);
        command
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE");
        command
            .env("GIT_AUTHOR_NAME", "Duet Workbench")
            .env("GIT_AUTHOR_EMAIL", "duet@localhost")
            .env("GIT_COMMITTER_NAME", "Duet Workbench")
            .env("GIT_COMMITTER_EMAIL", "duet@localhost");
        if let Some(index) = index {
            command.env("GIT_INDEX_FILE", index);
        }
        let output = command.args(args).output().context("Git 执行失败")?;
        ensure!(
            output.status.success(),
            "Git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).context("Git 输出不是 UTF-8")
    }

    pub fn create_worktree(&self, path: &Path, branch: &str) -> Result<()> {
        let path = path.to_str().context("worktree 路径不是 UTF-8")?;
        let baseline = self.baseline()?;
        self.git(
            &["worktree", "add", "-b", branch, path, &baseline.commit],
            None,
        )?;
        Ok(())
    }

    pub fn changed_paths(&self) -> Result<Vec<String>> {
        let tracked = self.git(&["diff", "--name-only", "-z", "HEAD", "--"], None)?;
        let untracked = self.git(&["ls-files", "--others", "--exclude-standard", "-z"], None)?;
        let mut paths: Vec<String> = tracked
            .split('\0')
            .chain(untracked.split('\0'))
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect();
        paths.sort();
        paths.dedup();
        Ok(paths)
    }

    pub fn baseline(&self) -> Result<Snapshot> {
        let commit = self.git(&["rev-parse", "--verify", "HEAD^{commit}"], None)?;
        Ok(Snapshot {
            commit: commit.trim().into(),
            parent: None,
            reference: None,
        })
    }

    pub fn resolve_snapshot(&self, revision: &str) -> Result<Snapshot> {
        let spec = format!("{revision}^{{commit}}");
        let commit = self.git(&["rev-parse", "--verify", "--end-of-options", &spec], None)?;
        let commit = commit.trim().to_owned();
        let line = self.git(&["rev-list", "--parents", "-n", "1", &commit], None)?;
        let parent = line.split_whitespace().nth(1).map(str::to_owned);
        Ok(Snapshot {
            commit,
            parent,
            reference: Some(revision.into()),
        })
    }

    pub fn is_dirty(&self) -> Result<bool> {
        Ok(!self.git(&["status", "--porcelain"], None)?.is_empty())
    }

    pub fn files(&self, snapshot: &Snapshot) -> Result<Vec<String>> {
        let output = self.git(
            &["ls-tree", "-r", "-z", "--name-only", &snapshot.commit],
            None,
        )?;
        Ok(output
            .split('\0')
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect())
    }

    /// Read change membership from immutable commits, never the current worktree.
    pub fn snapshot_changes(&self, snapshot: &Snapshot) -> Result<Vec<FileChange>> {
        let Some(parent) = &snapshot.parent else {
            return Ok(Vec::new());
        };
        let output = self.git(
            &[
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--no-renames",
                "--name-status",
                "-z",
                parent,
                &snapshot.commit,
                "--",
            ],
            None,
        )?;
        let mut fields = output.split('\0').filter(|field| !field.is_empty());
        let mut changes = Vec::new();
        while let Some(status) = fields.next() {
            let path = fields.next().context("Git 变更列表不完整")?;
            changes.push(FileChange {
                path: path.into(),
                deleted: status == "D",
            });
        }
        Ok(changes)
    }

    pub fn read(&self, snapshot: &Snapshot, path: &str) -> Result<String> {
        validate_path(path)?;
        let spec = format!("{}:{}", snapshot.commit, path);
        let size: u64 = self.git(&["cat-file", "-s", &spec], None)?.trim().parse()?;
        ensure!(size <= MAX_FILE_BYTES, "文件超过 1 MiB，原型暂不打开");
        self.git(&["show", &spec], None)
    }

    pub fn diff(&self, snapshot: &Snapshot) -> Result<String> {
        let Some(parent) = &snapshot.parent else {
            return Ok("这是基线快照，没有阶段 diff。工作区的未提交改动不会混入此视图。".into());
        };
        self.git(
            &[
                "--no-pager",
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                parent,
                &snapshot.commit,
                "--",
            ],
            None,
        )
    }

    /// Capture only explicitly selected paths. Never stage into the user's index.
    /// Caller must ensure no concurrent writer changes selected files during capture.
    pub fn capture(&self, paths: &[String]) -> Result<Snapshot> {
        ensure!(!paths.is_empty(), "必须明确选择需要快照的文件");
        for path in paths {
            validate_path(path)?;
            let mut current = self.root.clone();
            for part in Path::new(path).components() {
                current.push(part);
                if let Ok(meta) = fs::symlink_metadata(&current) {
                    ensure!(!meta.file_type().is_symlink(), "原型拒绝符号链接：{path}");
                }
            }
            let ignored = Command::new("git")
                .arg("-C")
                .arg(&self.root)
                .args(["check-ignore", "--", path])
                .output()?;
            ensure!(
                ignored.status.code() != Some(0),
                "拒绝捕获 Git 忽略文件：{path}"
            );
            if self.root.join(path).exists() {
                let meta = fs::metadata(self.root.join(path))?;
                ensure!(meta.is_file(), "只能选择文件：{path}");
                ensure!(meta.len() <= MAX_FILE_BYTES, "文件超过 1 MiB：{path}");
            }
        }
        let parent = self.baseline()?.commit;
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        // Git requires an absent path for a new index; own directory prevents collisions.
        let dir = std::env::temp_dir().join(format!("duet-index-{}-{nonce}", std::process::id()));
        fs::create_dir(&dir)?;
        let _cleanup = Cleanup(dir.clone());
        let index = dir.join("index");
        self.git(&["read-tree", &parent], Some(&index))?;
        for path in paths {
            let full = self.root.join(path);
            if !full.exists() {
                // A missing untracked path must not silently become a successful checkpoint.
                let spec = format!("{parent}:{path}");
                self.git(&["cat-file", "-e", &spec], None)
                    .with_context(|| format!("文件既不存在也未被跟踪：{path}"))?;
                self.git(
                    &["update-index", "--force-remove", "--", path],
                    Some(&index),
                )?;
                continue;
            }
            let oid = self.git(&["hash-object", "--no-filters", "-w", "--", path], None)?;
            #[cfg(unix)]
            let mode = {
                use std::os::unix::fs::PermissionsExt;
                if fs::metadata(&full)?.permissions().mode() & 0o111 != 0 {
                    "100755"
                } else {
                    "100644"
                }
            };
            #[cfg(not(unix))]
            let mode = "100644";
            self.git(
                &[
                    "update-index",
                    "--add",
                    "--cacheinfo",
                    mode,
                    oid.trim(),
                    path,
                ],
                Some(&index),
            )?;
        }
        let tree = self.git(&["write-tree"], Some(&index))?;
        let commit = self.git(
            &[
                "commit-tree",
                tree.trim(),
                "-p",
                &parent,
                "-m",
                "Duet immutable checkpoint",
            ],
            None,
        )?;
        let commit = commit.trim().to_owned();
        let reference = format!("refs/duet/checkpoints/{nonce}");
        // Empty old value enforces create-only reference.
        self.git(&["update-ref", &reference, &commit, ""], None)?;
        Ok(Snapshot {
            commit,
            parent: Some(parent),
            reference: Some(reference),
        })
    }
}

pub fn validate_path(path: &str) -> Result<()> {
    if path.is_empty() || path.contains('\0') {
        bail!("无效文件路径");
    }
    ensure!(
        Path::new(path)
            .components()
            .all(|p| matches!(p, Component::Normal(_))),
        "路径必须位于仓库内：{path}"
    );
    Ok(())
}

struct Cleanup(PathBuf);
impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
