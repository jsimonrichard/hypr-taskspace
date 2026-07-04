use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::error::{TskError, Result};
use crate::repos::normalize_repo_path;
use crate::task_paths::{linked_checkout_path, scratch_checkout_path};
use crate::vcs::{
    create_linked_checkout, detect_vcs_root, init_scratch_repo, vcs_kind_at, VcsKind,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskRepoSource {
    /// Use the git/jj root from `cwd`, or a scratch repo if none is found.
    Auto,
    /// Always create an isolated repo directory under the task home.
    Scratch,
    /// Use an explicit checkout path (typically a detected VCS root).
    Path(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskRepoSetup {
    Scratch,
    /// Isolated git worktree or jj workspace under the task home.
    Linked {
        source_root: PathBuf,
        kind: VcsKind,
    },
    /// Use the registered checkout directly (no worktree/workspace).
    Direct {
        source_root: PathBuf,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskRepoOptions {
    pub create_worktree: bool,
}

impl Default for TaskRepoOptions {
    fn default() -> Self {
        Self {
            create_worktree: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedTaskRepo {
    pub checkout_path: PathBuf,
    pub setup: TaskRepoSetup,
}

impl TaskRepoSource {
    pub fn resolve(
        &self,
        task_home: &Path,
        cwd: Option<&Path>,
        options: &TaskRepoOptions,
    ) -> Result<ResolvedTaskRepo> {
        let setup = match self {
            Self::Scratch => TaskRepoSetup::Scratch,
            Self::Auto => {
                if let Some(root) = detect_vcs_root(cwd) {
                    if options.create_worktree {
                        let kind = vcs_kind_at(&root).ok_or_else(|| {
                            TskError::Other(format!(
                                "Detected repo has no supported VCS: {}",
                                root.display()
                            ))
                        })?;
                        TaskRepoSetup::Linked {
                            source_root: root,
                            kind,
                        }
                    } else {
                        TaskRepoSetup::Direct { source_root: root }
                    }
                } else {
                    TaskRepoSetup::Scratch
                }
            }
            Self::Path(path) => {
                let path = normalize_repo_path(path);
                let root = detect_vcs_root(Some(&path)).unwrap_or(path.clone());
                if !root.is_dir() {
                    return Err(TskError::Other(format!(
                        "Repo path does not exist: {}",
                        root.display()
                    )));
                }
                if options.create_worktree {
                    let kind = vcs_kind_at(&root).ok_or_else(|| {
                        TskError::Other(format!(
                            "Not a git or jj repo: {}",
                            root.display()
                        ))
                    })?;
                    TaskRepoSetup::Linked {
                        source_root: root,
                        kind,
                    }
                } else {
                    let kind = vcs_kind_at(&root);
                    if kind.is_none() {
                        return Err(TskError::Other(format!(
                            "Not a git or jj repo: {}",
                            root.display()
                        )));
                    }
                    TaskRepoSetup::Direct { source_root: root }
                }
            }
        };
        let checkout_path = match &setup {
            TaskRepoSetup::Scratch => scratch_checkout_path(task_home),
            TaskRepoSetup::Linked { source_root, .. } => {
                linked_checkout_path(task_home, source_root)
            }
            TaskRepoSetup::Direct { source_root } => source_root.clone(),
        };
        Ok(ResolvedTaskRepo {
            checkout_path,
            setup,
        })
    }

    pub fn to_daemon_params(&self, cwd: Option<&Path>) -> Value {
        match self {
            Self::Auto => {
                let mut body = json!({ "repo": "auto" });
                if let Some(cwd) = cwd {
                    body["cwd"] = json!(cwd.display().to_string());
                }
                body
            }
            Self::Scratch => json!({ "repo": "scratch" }),
            Self::Path(path) => json!({
                "repo": "path",
                "repo_path": path.display().to_string(),
            }),
        }
    }

    pub fn from_daemon_params(params: &Value) -> Result<Self> {
        match params.get("repo").and_then(|v| v.as_str()) {
            Some("scratch") => Ok(Self::Scratch),
            Some("path") => {
                let path = params
                    .get("repo_path")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| TskError::Other("repo_path required".into()))?;
                Ok(Self::Path(path.into()))
            }
            _ => Ok(Self::Auto),
        }
    }

    pub fn cwd_from_daemon_params(params: &Value) -> Option<PathBuf> {
        params
            .get("cwd")
            .and_then(|v| v.as_str())
            .map(PathBuf::from)
    }

    pub fn resolve_url(&self) -> Option<String> {
        match self {
            Self::Path(path) => {
                let root = detect_vcs_root(Some(path)).unwrap_or_else(|| path.clone());
                crate::repos::load_repo_config(&root)
                    .ok()
                    .flatten()
                    .and_then(|config| config.url)
            }
            _ => None,
        }
    }
}

/// Create the on-disk checkout for a task (scratch git repo or linked worktree/workspace).
pub fn provision_task_checkout(resolved: &ResolvedTaskRepo, task_id: &str) -> Result<()> {
    match &resolved.setup {
        TaskRepoSetup::Scratch => init_scratch_repo(&resolved.checkout_path),
        TaskRepoSetup::Direct { .. } => Ok(()),
        TaskRepoSetup::Linked {
            source_root,
            kind,
        } => create_linked_checkout(source_root, &resolved.checkout_path, task_id, *kind),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_params_path_roundtrip() {
        let path = PathBuf::from("/home/user/my-project");
        let src = TaskRepoSource::Path(path);
        let params = src.to_daemon_params(None);
        assert_eq!(params["repo"], "path");
        assert_eq!(params["repo_path"], "/home/user/my-project");
        let back = TaskRepoSource::from_daemon_params(&params).unwrap();
        assert_eq!(back, src);
    }

    #[test]
    fn daemon_params_auto_includes_cwd_string() {
        let cwd = PathBuf::from("/tmp/work");
        let params = TaskRepoSource::Auto.to_daemon_params(Some(&cwd));
        assert_eq!(params["repo"], "auto");
        assert_eq!(params["cwd"], "/tmp/work");
    }

    #[test]
    fn resolve_auto_uses_linked_checkout_under_task_home() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("project");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        let task_home = dir.path().join("tasks").join("tabc");
        let resolved = TaskRepoSource::Auto
            .resolve(&task_home, Some(&repo), &TaskRepoOptions::default())
            .unwrap();
        assert_eq!(
            resolved.checkout_path,
            task_home.join("workspace").join("project")
        );
        assert_eq!(
            resolved.setup,
            TaskRepoSetup::Linked {
                source_root: repo,
                kind: VcsKind::Git,
            }
        );
    }

    #[test]
    fn resolve_auto_without_worktree_uses_main_repo() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("project");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        let task_home = dir.path().join("tasks").join("tabc");
        let resolved = TaskRepoSource::Auto
            .resolve(
                &task_home,
                Some(&repo),
                &TaskRepoOptions {
                    create_worktree: false,
                },
            )
            .unwrap();
        assert_eq!(resolved.checkout_path, repo);
        assert_eq!(
            resolved.setup,
            TaskRepoSetup::Direct {
                source_root: repo,
            }
        );
    }
}
