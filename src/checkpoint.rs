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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    pub path: String,
    pub deleted: bool,
    pub kind: ChangeKind,
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

    fn git_output(&self, args: &[&str], index: Option<&Path>) -> Result<std::process::Output> {
        let mut command = Command::new("git");
        command.arg("-C").arg(&self.root).args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "commit.gpgSign=false",
        ]);
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
        command.args(args).output().context("Git 执行失败")
    }
    fn git(&self, args: &[&str], index: Option<&Path>) -> Result<String> {
        let output = self.git_output(args, index)?;
        ensure!(
            output.status.success(),
            "Git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).context("Git 输出不是 UTF-8")
    }

    pub fn tree(&self, commit: &str) -> Result<String> {
        let commit = self.resolve_snapshot(commit)?.commit;
        Ok(self
            .git(&["rev-parse", &format!("{commit}^{{tree}}")], None)?
            .trim()
            .into())
    }
    pub fn branch_ref(&self) -> Result<String> {
        let reference = self
            .git(&["symbolic-ref", "HEAD"], None)
            .context("主项目为 detached HEAD，拒绝自动合并")?;
        ensure!(
            reference.trim().starts_with("refs/heads/"),
            "主项目必须在普通分支"
        );
        Ok(reference.trim().into())
    }
    pub fn merge_guard(&self) -> Result<()> {
        ensure!(
            !self.is_dirty()?,
            "主项目存在未提交/暂存/未跟踪文件，请先自行处理；不会自动 stash"
        );
        for name in [
            "MERGE_HEAD",
            "CHERRY_PICK_HEAD",
            "REVERT_HEAD",
            "REBASE_HEAD",
            "rebase-apply",
            "rebase-merge",
            "sequencer",
        ] {
            let path = self.git(&["rev-parse", "--git-path", name], None)?;
            ensure!(
                !self.root.join(path.trim()).exists(),
                "主项目正在 merge/rebase/cherry-pick/revert，拒绝合并"
            );
        }
        let flags = self.git(&["ls-files", "-v"], None)?;
        ensure!(
            !flags
                .lines()
                .any(|l| l.starts_with("S ")
                    || l.as_bytes().first().is_some_and(u8::is_ascii_lowercase)),
            "主项目含 skip-worktree/assume-unchanged，不能可靠检查人工文件"
        );
        let custom = self.git_output(
            &[
                "config",
                "--get-regexp",
                r"^(merge\..*\.driver|filter\..*\.(smudge|process))$",
            ],
            None,
        )?;
        ensure!(
            custom.status.code() == Some(1),
            "自定义 merge driver/checkout filter 暂不支持受控合并；避免自动执行仓库程序"
        );
        Ok(())
    }
    pub fn is_ancestor(&self, older: &str, newer: &str) -> Result<bool> {
        let output = self.git_output(&["merge-base", "--is-ancestor", older, newer], None)?;
        ensure!(
            matches!(output.status.code(), Some(0 | 1)),
            "无法检查提交祖先"
        );
        Ok(output.status.success())
    }
    pub fn prepare_integration(
        &self,
        source: &str,
        delivery: &str,
        task: &str,
    ) -> Result<Snapshot> {
        let (tree, conflicts) = self.integration_tree(source, delivery)?;
        ensure!(
            conflicts.is_empty(),
            "集成冲突，主项目未变：{}",
            conflicts.join("、")
        );
        ensure!(
            tree != self.tree(source)?,
            "候选已包含在主项目或集成无差异，无需重复合并"
        );
        self.integration_commit(&tree, source, delivery, task)
    }
    pub fn integration_commit(
        &self,
        tree: &str,
        source: &str,
        delivery: &str,
        task: &str,
    ) -> Result<Snapshot> {
        let commit = self
            .git(
                &[
                    "commit-tree",
                    tree,
                    "-p",
                    source,
                    "-p",
                    delivery,
                    "-m",
                    &format!("Merge approved Duet task {task} (verification separately recorded)"),
                ],
                None,
            )?
            .trim()
            .to_owned();
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let reference = format!("refs/duet/integrations/{task}/{nonce}");
        self.git(&["update-ref", &reference, &commit, ""], None)?;
        Ok(Snapshot {
            commit,
            parent: Some(source.into()),
            reference: Some(reference),
        })
    }
    pub fn integration_tree(&self, source: &str, delivery: &str) -> Result<(String, Vec<String>)> {
        let source = self.resolve_snapshot(source)?.commit;
        let delivery = self.resolve_snapshot(delivery)?.commit;
        let out = self.git_output(
            &[
                "merge-tree",
                "--write-tree",
                "--name-only",
                "-z",
                &source,
                &delivery,
            ],
            None,
        )?;
        ensure!(
            matches!(out.status.code(), Some(0 | 1)),
            "无法构造集成树：{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let stdout = String::from_utf8(out.stdout)?;
        let mut fields = stdout.split('\0');
        let tree = fields.next().context("没有集成树")?.to_owned();
        ensure!(
            tree.bytes().all(|b| b.is_ascii_hexdigit()) && matches!(tree.len(), 40 | 64),
            "无效集成树"
        );
        let mut paths = Vec::new();
        if !out.status.success() {
            for path in fields.take_while(|p| !p.is_empty()) {
                validate_path(path)?;
                paths.push(path.into());
            }
        }
        ensure!(
            out.status.success() || !paths.is_empty(),
            "Git 返回失败但未列出冲突"
        );
        Ok((tree, paths))
    }
    pub fn create_worktree_at(&self, path: &Path, branch: &str, commit: &str) -> Result<()> {
        let commit = self.resolve_snapshot(commit)?.commit;
        self.git(
            &[
                "worktree",
                "add",
                "-b",
                branch,
                path.to_str().context("工作区路径不是 UTF-8")?,
                &commit,
            ],
            None,
        )?;
        Ok(())
    }
    pub fn merge_base(&self, left: &str, right: &str) -> Result<Snapshot> {
        self.resolve_snapshot(self.git(&["merge-base", left, right], None)?.trim())
    }
    pub fn snapshot_has_path(&self, snapshot: &Snapshot, path: &str) -> Result<bool> {
        validate_path(path)?;
        Ok(self.files(snapshot)?.iter().any(|p| p == path))
    }
    pub fn integration_diff(&self, source: &str, commit: &str) -> Result<String> {
        self.git(
            &[
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--no-color",
                source,
                commit,
                "--",
            ],
            None,
        )
    }
    pub fn fast_forward_integration(&self, source: &str, branch: &str, commit: &str) -> Result<()> {
        self.merge_guard()?;
        ensure!(
            self.baseline()?.commit == source && self.branch_ref()? == branch,
            "主项目 HEAD/分支在预览后改变，请重新预览"
        );
        self.git(
            &[
                "merge",
                "--ff-only",
                "--no-edit",
                "--no-autostash",
                "--no-overwrite-ignore",
                commit,
            ],
            None,
        )
        .context("合并未完成，请检查主项目 Git 状态；不会 reset/覆盖恢复")?;
        ensure!(
            self.baseline()?.commit == commit,
            "合并结果与确认版本不一致"
        );
        Ok(())
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

    /// Only call after WorkspaceStore::candidate verifies this is an owned worktree.
    pub fn restore_candidate(&self, revision: &str) -> Result<()> {
        let snapshot = self.resolve_snapshot(revision)?;
        self.git(
            &[
                "restore",
                "--source",
                &snapshot.commit,
                "--staged",
                "--worktree",
                "--",
                ".",
            ],
            None,
        )?;
        self.git(&["clean", "-fdx", "--", "."], None)?;
        Ok(())
    }
    pub fn candidate_identity(&self) -> Result<(PathBuf, String)> {
        let common = self.git(&["rev-parse", "--git-common-dir"], None)?;
        let common = PathBuf::from(common.trim());
        let common = if common.is_absolute() {
            common
        } else {
            self.root.join(common)
        };
        Ok((
            common.canonicalize()?,
            self.git(&["rev-parse", "--abbrev-ref", "HEAD"], None)?
                .trim()
                .into(),
        ))
    }
    pub fn remove_candidate(&self, path: &Path, branch: &str) -> Result<()> {
        self.git(
            &[
                "worktree",
                "remove",
                "--force",
                path.to_str().context("候选路径不是 UTF-8")?,
            ],
            None,
        )?;
        self.git(&["branch", "-D", branch], None)?;
        Ok(())
    }
    pub fn bundle_candidate(
        &self,
        path: &Path,
        branch: &str,
        commits: &std::collections::HashSet<String>,
    ) -> Result<()> {
        let refs = self.git(
            &[
                "for-each-ref",
                "--format=%(refname) %(objectname)",
                "refs/duet/checkpoints/",
            ],
            None,
        )?;
        let mut args = vec![
            "bundle",
            "create",
            path.to_str().context("备份路径不是 UTF-8")?,
            branch,
        ];
        for line in refs.lines() {
            if let Some((reference, commit)) = line.split_once(' ')
                && commits.contains(commit)
            {
                args.push(reference);
            }
        }
        self.git(&args, None)?;
        Ok(())
    }
    pub fn archive_integration_refs(&self, id: &str, backup: &Path) -> Result<()> {
        ensure!(
            !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()),
            "不受管任务 ID"
        );
        let prefix = format!("refs/duet/integrations/{id}/");
        let list = self.git(
            &["for-each-ref", "--format=%(refname) %(objectname)", &prefix],
            None,
        )?;
        let refs = list
            .lines()
            .filter_map(|l| l.split_once(' '))
            .collect::<Vec<_>>();
        if refs.is_empty() {
            return Ok(());
        }
        let mut args = vec![
            "bundle",
            "create",
            backup.to_str().context("备份路径不是 UTF-8")?,
        ];
        args.extend(refs.iter().map(|(r, _)| *r));
        self.git(&args, None)?;
        for (reference, commit) in refs {
            self.git(&["update-ref", "-d", reference, commit], None)?;
        }
        Ok(())
    }
    pub fn remove_checkpoint_refs(
        &self,
        commits: &std::collections::HashSet<String>,
    ) -> Result<()> {
        let refs = self.git(
            &[
                "for-each-ref",
                "--format=%(refname) %(objectname)",
                "refs/duet/checkpoints/",
            ],
            None,
        )?;
        for line in refs.lines() {
            if let Some((reference, commit)) = line.split_once(' ')
                && commits.contains(commit)
            {
                self.git(&["update-ref", "-d", reference, commit], None)?;
            }
        }
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

    pub fn project_entries(&self, ignored: bool) -> Result<Vec<(String, char)>> {
        let mut files = self
            .git(
                &[
                    "ls-files",
                    "-z",
                    "--cached",
                    "--others",
                    "--exclude-standard",
                ],
                None,
            )?
            .split('\0')
            .filter(|p| !p.is_empty())
            .map(|p| (p.to_string(), ' '))
            .collect::<std::collections::BTreeMap<_, _>>();
        if ignored {
            for path in self
                .git(
                    &[
                        "ls-files",
                        "-z",
                        "--others",
                        "--ignored",
                        "--exclude-standard",
                    ],
                    None,
                )?
                .split('\0')
                .filter(|p| !p.is_empty())
            {
                files.entry(path.into()).or_insert('!');
            }
        }
        let status = self.git(
            &[
                "-c",
                "core.fsmonitor=false",
                "--no-optional-locks",
                "status",
                "--porcelain=v1",
                "-z",
                "--untracked-files=all",
                "--no-renames",
            ],
            None,
        )?;
        for entry in status.split('\0').filter(|p| p.len() >= 4) {
            let code = if entry.starts_with("??") {
                '?'
            } else if entry[..2].contains('D') {
                'D'
            } else if entry[..2].contains('A') {
                'A'
            } else {
                'M'
            };
            files.insert(entry[3..].into(), code);
        }
        files.retain(|path, _| !path.split('/').any(|part| matches!(part, ".git" | ".duet")));
        Ok(files.into_iter().collect())
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
                kind: match status {
                    "A" => ChangeKind::Added,
                    "D" => ChangeKind::Deleted,
                    _ => ChangeKind::Modified,
                },
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

    pub fn diff_file(&self, snapshot: &Snapshot, path: &str) -> Result<String> {
        validate_path(path)?;
        let parent = snapshot.parent.as_ref().context("检查点没有差异基线")?;
        let literal = format!(":(literal){path}");
        self.git(
            &[
                "--no-pager",
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--no-renames",
                parent,
                &snapshot.commit,
                "--",
                &literal,
            ],
            None,
        )
    }

    /// Capture only explicitly selected paths. Never stage into the user's index.
    /// Caller must ensure no concurrent writer changes selected files during capture.
    pub fn capture(&self, paths: &[String]) -> Result<Snapshot> {
        ensure!(!paths.is_empty(), "必须明确选择需要快照的文件");
        self.capture_from(paths, &self.baseline()?)
    }

    /// Capture the working tree relative to a previous node, including deletion of
    /// files added by earlier nodes. A separate index keeps user HEAD/index untouched.
    pub fn capture_worktree_from(&self, base: &Snapshot) -> Result<Snapshot> {
        let mut paths = self.changed_paths()?;
        let previous = self.git(
            &[
                "diff",
                "--name-only",
                "--no-renames",
                "-z",
                "HEAD",
                &base.commit,
                "--",
            ],
            None,
        )?;
        paths.extend(
            previous
                .split('\0')
                .filter(|p| !p.is_empty())
                .map(str::to_owned),
        );
        paths.sort();
        paths.dedup();
        self.capture_from(&paths, base)
    }

    pub fn capture_from(&self, paths: &[String], base: &Snapshot) -> Result<Snapshot> {
        let parent = self.resolve_snapshot(&base.commit)?.commit;
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
                &format!("Duet immutable checkpoint {nonce}"),
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
