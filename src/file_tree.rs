//! Project-only navigation. No editor/model is started by scanning a tree.
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
#[derive(Clone, Debug)]
pub struct Entry {
    pub path: String,
    pub directory: bool,
    pub status: char,
}
#[derive(Default, Serialize, Deserialize)]
pub struct Tree {
    pub selected: String,
    pub expanded: BTreeSet<String>,
    pub show_ignored: bool,
    #[serde(skip)]
    pub entries: Vec<Entry>,
    #[serde(skip)]
    pub truncated: bool,
}
impl Tree {
    pub fn refresh(&mut self, root: &Path) -> Result<()> {
        let mut traversal_limited = false;
        let files = if let Ok(repo) = crate::checkpoint::Repository::open(root) {
            repo.project_entries(self.show_ignored)?
        } else {
            fn walk(
                root: &Path,
                dir: &Path,
                out: &mut Vec<(String, char)>,
                depth: usize,
                visited: &mut usize,
            ) -> Result<()> {
                if depth > 32 || out.len() >= 5000 || *visited >= 10000 {
                    return Ok(());
                }
                for item in std::fs::read_dir(dir)? {
                    if *visited >= 10000 {
                        break;
                    }
                    *visited += 1;
                    let item = item?;
                    let name = item.file_name();
                    if name == ".git" || name == ".duet" {
                        continue;
                    }
                    let meta = std::fs::symlink_metadata(item.path())?;
                    if meta.file_type().is_symlink() {
                        continue;
                    }
                    if meta.is_dir() {
                        walk(root, &item.path(), out, depth + 1, visited)?;
                    } else if meta.is_file()
                        && let Some(path) =
                            item.path().strip_prefix(root).ok().and_then(|p| p.to_str())
                    {
                        out.push((path.into(), ' '));
                    }
                    if out.len() >= 5000 {
                        break;
                    }
                }
                Ok(())
            }
            let mut out = Vec::new();
            let mut visited = 0;
            walk(root, root, &mut out, 0, &mut visited)?;
            traversal_limited = visited >= 10000 || out.len() >= 5000;
            out
        };
        self.truncated = traversal_limited || files.len() > 5000;
        let mut map = BTreeMap::new();
        for (path, status) in files.into_iter().take(5000) {
            if crate::checkpoint::validate_path(&path).is_err() {
                continue;
            }
            let mut prefix = String::new();
            let parts = path.split('/').collect::<Vec<_>>();
            for part in &parts[..parts.len() - 1] {
                if !prefix.is_empty() {
                    prefix.push('/');
                }
                prefix.push_str(part);
                map.entry(prefix.clone()).or_insert(Entry {
                    path: prefix.clone(),
                    directory: true,
                    status: ' ',
                });
            }
            map.insert(
                path.clone(),
                Entry {
                    path,
                    directory: false,
                    status,
                },
            );
        }
        self.entries = map.into_values().collect();
        self.expanded
            .retain(|p| self.entries.iter().any(|e| e.directory && &e.path == p));
        if !self.entries.iter().any(|e| e.path == self.selected) {
            self.selected = self
                .visible()
                .first()
                .map(|e| e.path.clone())
                .unwrap_or_default();
        }
        Ok(())
    }
    pub fn visible(&self) -> Vec<&Entry> {
        let mut entries = self
            .entries
            .iter()
            .filter(|e| {
                let mut path = Path::new(&e.path).parent();
                while let Some(p) = path {
                    if p.as_os_str().is_empty() {
                        break;
                    }
                    if !self.expanded.contains(&p.to_string_lossy().to_string()) {
                        return false;
                    }
                    path = p.parent();
                }
                true
            })
            .collect::<Vec<_>>();
        entries.sort_by(|a, b| {
            let aa = a.path.split('/').collect::<Vec<_>>();
            let bb = b.path.split('/').collect::<Vec<_>>();
            for i in 0..aa.len().min(bb.len()) {
                if aa[i] != bb[i] {
                    let ad = i + 1 < aa.len() || a.directory;
                    let bd = i + 1 < bb.len() || b.directory;
                    return bd.cmp(&ad).then_with(|| aa[i].cmp(bb[i]));
                }
            }
            aa.len().cmp(&bb.len())
        });
        entries
    }
    pub fn entry(&self) -> Option<&Entry> {
        self.entries.iter().find(|e| e.path == self.selected)
    }
    pub fn move_by(&mut self, delta: isize) {
        let rows = self.visible();
        let pos = rows
            .iter()
            .position(|e| e.path == self.selected)
            .unwrap_or(0);
        if !rows.is_empty() {
            self.selected = rows[pos.saturating_add_signed(delta).min(rows.len() - 1)]
                .path
                .clone();
        }
    }
    pub fn reveal(&mut self, path: &str) {
        self.selected = path.into();
        let mut parent = Path::new(path).parent();
        while let Some(p) = parent {
            if p.as_os_str().is_empty() {
                break;
            }
            self.expanded.insert(p.to_string_lossy().into());
            parent = p.parent();
        }
    }
    pub fn expand(&mut self) {
        if self.entry().is_some_and(|e| e.directory) {
            self.expanded.insert(self.selected.clone());
        }
    }
    pub fn collapse(&mut self) {
        if !self.expanded.remove(&self.selected)
            && let Some(parent) = Path::new(&self.selected)
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
        {
            self.selected = parent.to_string_lossy().into();
            self.expanded.remove(&self.selected);
        }
    }
}
