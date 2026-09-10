use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::error::{Result, TskError};
use crate::models::Task;
use crate::repos::{is_scratch_task, normalize_repo_path, task_source_repo_path};
use crate::task_paths::{
    ensure_scratch_workspace, linked_checkout_path, scratch_checkout_path, sibling_checkout_path,
    sibling_workspace_name, validate_checkout_suffix,
};
use crate::vcs::{
    checkout_belongs_to_repo, create_linked_checkout, current_checkout_revision, detect_vcs_root,
    jj_workspace_checkout, resolve_revision_id, vcs_kind_at, VcsKind,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskRepoSource {
    /// Use the git/jj root from `cwd`, or an empty scratch workspace if none is found.
    Auto,
    /// Always create an empty workspace directory under the task home.
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

/// Where a new linked checkout should start from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForkFrom {
    /// jj: `trunk()`/`main`. git: source `HEAD`.
    Default,
    /// Git commit-ish or jj revset, evaluated in the source repo.
    Revision(String),
    /// Live `@` / `HEAD` of the current checkout (`cwd`, else `fallback_task_id`).
    Current { fallback_task_id: Option<String> },
    /// Named jj workspace, or a tsk task checkout of the same repo.
    Workspace(String),
}

impl ForkFrom {
    pub fn is_default(&self) -> bool {
        matches!(self, Self::Default)
    }

    pub fn from_daemon_params(params: &Value) -> Result<Self> {
        match params.get("fork_from").and_then(|v| v.as_str()) {
            None | Some("default") | Some("") => Ok(Self::Default),
            Some("revision") => {
                let rev = params
                    .get("fork_revision")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| TskError::Other("fork_revision required".into()))?;
                Ok(Self::Revision(rev.to_string()))
            }
            Some("current") => {
                let fallback_task_id = params
                    .get("current_task_id")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(ToString::to_string);
                Ok(Self::Current { fallback_task_id })
            }
            Some("workspace") => {
                let name = params
                    .get("fork_workspace")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| TskError::Other("fork_workspace required".into()))?;
                Ok(Self::Workspace(name.to_string()))
            }
            Some(other) => Err(TskError::Other(format!("unknown fork_from '{other}'"))),
        }
    }

    pub fn write_daemon_params(&self, body: &mut Value) {
        let Some(obj) = body.as_object_mut() else {
            return;
        };
        match self {
            Self::Default => {
                obj.insert("fork_from".into(), json!("default"));
            }
            Self::Revision(rev) => {
                obj.insert("fork_from".into(), json!("revision"));
                obj.insert("fork_revision".into(), json!(rev));
            }
            Self::Current { fallback_task_id } => {
                obj.insert("fork_from".into(), json!("current"));
                if let Some(id) = fallback_task_id {
                    obj.insert("current_task_id".into(), json!(id));
                }
            }
            Self::Workspace(name) => {
                obj.insert("fork_from".into(), json!("workspace"));
                obj.insert("fork_workspace".into(), json!(name));
            }
        }
    }

    /// Resolve to a commit id to pass to `create_linked_checkout`, or `None` for the VCS default.
    pub fn resolve_revision(
        &self,
        source_root: &Path,
        kind: VcsKind,
        current_checkout: Option<&Path>,
        named_checkout: Option<&Path>,
    ) -> Result<Option<String>> {
        match self {
            Self::Default => Ok(None),
            Self::Revision(rev) => Ok(Some(resolve_revision_id(source_root, kind, rev)?)),
            Self::Current { .. } => {
                let checkout = current_checkout.ok_or_else(|| TskError::NoCheckoutToFork {
                    path: current_checkout_hint(current_checkout),
                })?;
                ensure_fork_checkout_in_repo(source_root, checkout, kind)?;
                Ok(Some(current_checkout_revision(checkout, kind)?))
            }
            Self::Workspace(name) => {
                let checkout = match named_checkout {
                    Some(path) => path.to_path_buf(),
                    None if kind == VcsKind::Jj => jj_workspace_checkout(source_root, name)?,
                    None => {
                        return Err(TskError::UnknownForkWorkspace {
                            name: name.clone(),
                            path: source_root.to_path_buf(),
                        });
                    }
                };
                ensure_fork_checkout_in_repo(source_root, &checkout, kind)?;
                Ok(Some(current_checkout_revision(&checkout, kind)?))
            }
        }
    }
}

fn current_checkout_hint(current_checkout: Option<&Path>) -> PathBuf {
    current_checkout
        .map(Path::to_path_buf)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

fn ensure_fork_checkout_in_repo(source_root: &Path, checkout: &Path, kind: VcsKind) -> Result<()> {
    if checkout_belongs_to_repo(source_root, checkout, kind)? {
        return Ok(());
    }
    Err(TskError::ForkCheckoutNotInRepo {
        checkout: checkout.to_path_buf(),
        source_root: source_root.to_path_buf(),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskRepoOptions {
    pub create_worktree: bool,
    /// Create a Distrobox container and launch apps via `distrobox enter`.
    pub container_isolation: bool,
    /// When true with `container_isolation`, skip Distrobox create in `create_task`
    /// so the caller (e.g. TUI) can stream setup progress itself.
    pub defer_container_create: bool,
    pub fork_from: ForkFrom,
}

impl Default for TaskRepoOptions {
    fn default() -> Self {
        Self {
            create_worktree: true,
            container_isolation: false,
            defer_container_create: false,
            fork_from: ForkFrom::Default,
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
                        TskError::Other(format!("Not a git or jj repo: {}", root.display()))
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

/// Create the on-disk checkout for a task (scratch workspace or linked worktree/workspace).
pub fn provision_task_checkout(
    resolved: &ResolvedTaskRepo,
    task_id: &str,
    revision: Option<&str>,
) -> Result<()> {
    match &resolved.setup {
        TaskRepoSetup::Scratch => {
            if revision.is_some() {
                return Err(TskError::ForkRequiresLinkedCheckout);
            }
            ensure_scratch_workspace(&resolved.checkout_path)
        }
        TaskRepoSetup::Direct { .. } => {
            if revision.is_some() {
                return Err(TskError::ForkRequiresLinkedCheckout);
            }
            Ok(())
        }
        TaskRepoSetup::Linked { source_root, kind } => create_linked_checkout(
            source_root,
            &resolved.checkout_path,
            task_id,
            *kind,
            revision,
        ),
    }
}

/// Create a sibling git worktree / jj workspace under the task home.
///
/// `current_checkout` is the live `@` / `HEAD` source when `fork_from` is
/// [`ForkFrom::Current`]. Dest path is returned for `cd "$(tsk checkout add …)"`.
pub fn add_sibling_checkout(
    task: &Task,
    tasks_base: &Path,
    suffix: &str,
    fork_from: &ForkFrom,
    current_checkout: Option<&Path>,
) -> Result<PathBuf> {
    validate_checkout_suffix(suffix)?;
    if is_scratch_task(task) {
        return Err(TskError::ScratchHasNoLinkedRepo {
            id: task.id.clone(),
        });
    }

    let source_root = task_source_repo_path(task);
    let kind = vcs_kind_at(source_root).ok_or_else(|| TskError::NotARepo {
        path: source_root.to_path_buf(),
    })?;

    let task_home = tasks_base.join(&task.id);
    let dest = sibling_checkout_path(&task_home, source_root, suffix);
    let name = sibling_workspace_name(&task.id, suffix);
    let revision = fork_from.resolve_revision(source_root, kind, current_checkout, None)?;
    create_linked_checkout(source_root, &dest, &name, kind, revision.as_deref())?;
    Ok(dest)
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
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(resolved.checkout_path, repo);
        assert_eq!(resolved.setup, TaskRepoSetup::Direct { source_root: repo });
    }

    #[test]
    fn resolve_scratch_uses_task_workspace_dir() {
        let dir = tempfile::tempdir().unwrap();
        let task_home = dir.path().join("tasks").join("tabc");
        let resolved = TaskRepoSource::Scratch
            .resolve(&task_home, None, &TaskRepoOptions::default())
            .unwrap();
        assert_eq!(resolved.checkout_path, task_home.join("workspace"));
        assert_eq!(resolved.setup, TaskRepoSetup::Scratch);
    }

    #[test]
    fn daemon_params_fork_from_roundtrip() {
        let cases = [
            ForkFrom::Default,
            ForkFrom::Revision("abc123".into()),
            ForkFrom::Current {
                fallback_task_id: Some("t231590d8".into()),
            },
            ForkFrom::Workspace("default".into()),
        ];
        for fork in cases {
            let mut body = json!({});
            fork.write_daemon_params(&mut body);
            assert_eq!(ForkFrom::from_daemon_params(&body).unwrap(), fork);
        }
    }

    #[test]
    fn fork_from_default_resolves_to_none() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("project");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        assert_eq!(
            ForkFrom::Default
                .resolve_revision(&repo, VcsKind::Git, None, None)
                .unwrap(),
            None
        );
    }

    #[test]
    fn provision_scratch_rejects_a_revision() {
        let dir = tempfile::tempdir().unwrap();
        let task_home = dir.path().join("tasks").join("tabc");
        let resolved = TaskRepoSource::Scratch
            .resolve(&task_home, None, &TaskRepoOptions::default())
            .unwrap();
        let err = provision_task_checkout(&resolved, "tabc", Some("main")).unwrap_err();
        assert!(matches!(err, TskError::ForkRequiresLinkedCheckout));
    }

    #[test]
    fn fork_from_current_requires_a_checkout() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("project");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        let err = ForkFrom::Current {
            fallback_task_id: None,
        }
        .resolve_revision(&repo, VcsKind::Git, None, None)
        .unwrap_err();
        assert!(matches!(err, TskError::NoCheckoutToFork { .. }));
    }

    #[test]
    fn provision_scratch_workspace_is_empty_dir_without_git() {
        let dir = tempfile::tempdir().unwrap();
        let task_home = dir.path().join("tasks").join("tabc");
        let resolved = TaskRepoSource::Scratch
            .resolve(&task_home, None, &TaskRepoOptions::default())
            .unwrap();
        provision_task_checkout(&resolved, "tabc", None).unwrap();
        assert!(resolved.checkout_path.is_dir());
        assert!(!resolved.checkout_path.join(".git").exists());
    }

    fn linked_task(id: &str, repo_path: PathBuf, source: PathBuf) -> Task {
        let now = chrono::Utc::now();
        Task {
            id: id.into(),
            name: id.into(),
            status: crate::models::TaskStatus::Active,
            repo_url: None,
            repo_path,
            source_repo_path: Some(source),
            branch: None,
            container_name: format!("tsk-{id}"),
            container_isolation: false,
            workspace_count: 10,
            browser_profile: None,
            created_at: now,
            last_active_at: now,
            listed_at: now,
            agent_notes_path: None,
            ports: vec![],
        }
    }

    fn git_commit(repo: &Path, message: &str) {
        let repo_str = repo.to_str().unwrap();
        for args in [
            &["config", "user.email", "tsk@test"][..],
            &["config", "user.name", "tsk"][..],
        ] {
            std::process::Command::new("git")
                .arg("-C")
                .arg(repo_str)
                .args(args)
                .status()
                .unwrap();
        }
        std::process::Command::new("git")
            .args(["-C", repo_str, "add", "-A"])
            .status()
            .unwrap();
        std::process::Command::new("git")
            .args(["-C", repo_str, "commit", "--allow-empty", "-m", message])
            .status()
            .unwrap();
    }

    #[test]
    fn add_sibling_rejects_scratch_and_bad_suffix() {
        let dir = tempfile::tempdir().unwrap();
        let tasks_base = dir.path().join("tasks");
        let home = tasks_base.join("tabc");
        let scratch = home.join("workspace");
        std::fs::create_dir_all(&scratch).unwrap();
        let now = chrono::Utc::now();
        let task = Task {
            id: "tabc".into(),
            name: "notes".into(),
            status: crate::models::TaskStatus::Active,
            repo_url: None,
            repo_path: scratch,
            source_repo_path: None,
            branch: None,
            container_name: "tsk-tabc".into(),
            container_isolation: false,
            workspace_count: 10,
            browser_profile: None,
            created_at: now,
            last_active_at: now,
            listed_at: now,
            agent_notes_path: None,
            ports: vec![],
        };
        let err = add_sibling_checkout(
            &task,
            &tasks_base,
            "review",
            &ForkFrom::Current {
                fallback_task_id: Some("tabc".into()),
            },
            None,
        )
        .unwrap_err();
        assert!(matches!(err, TskError::ScratchHasNoLinkedRepo { .. }));

        let source = dir.path().join("app");
        crate::vcs::init_scratch_repo(&source).unwrap();
        git_commit(&source, "init");
        let primary = home.join("workspace").join("app");
        let linked = linked_task("tabc", primary, source);
        let err = add_sibling_checkout(
            &linked,
            &tasks_base,
            "re/view",
            &ForkFrom::Current {
                fallback_task_id: Some("tabc".into()),
            },
            None,
        )
        .unwrap_err();
        assert!(matches!(err, TskError::InvalidCheckoutSuffix { .. }));
    }

    #[test]
    fn add_sibling_git_worktree_from_current() {
        let dir = tempfile::tempdir().unwrap();
        let tasks_base = dir.path().join("tasks");
        let source = dir.path().join("app");
        crate::vcs::init_scratch_repo(&source).unwrap();
        git_commit(&source, "init");
        let primary = tasks_base.join("tabc").join("workspace").join("app");
        crate::vcs::create_linked_checkout(&source, &primary, "tabc", VcsKind::Git, None).unwrap();
        let task = linked_task("tabc", primary.clone(), source);
        let dest = add_sibling_checkout(
            &task,
            &tasks_base,
            "review",
            &ForkFrom::Current {
                fallback_task_id: Some("tabc".into()),
            },
            Some(&primary),
        )
        .unwrap();
        assert_eq!(
            dest,
            tasks_base.join("tabc").join("workspace").join("app-review")
        );
        assert!(dest.join(".git").exists());
        assert_eq!(
            crate::vcs::current_branch(&dest).as_deref(),
            Some("tsk-tabc-review")
        );
        let again = add_sibling_checkout(
            &task,
            &tasks_base,
            "review",
            &ForkFrom::Current {
                fallback_task_id: Some("tabc".into()),
            },
            Some(&primary),
        )
        .unwrap();
        assert_eq!(again, dest);
    }

    #[test]
    fn add_sibling_git_worktree_from_explicit_revision() {
        let dir = tempfile::tempdir().unwrap();
        let tasks_base = dir.path().join("tasks");
        let source = dir.path().join("app");
        crate::vcs::init_scratch_repo(&source).unwrap();
        git_commit(&source, "first");
        let first = crate::vcs::resolve_revision_id(&source, VcsKind::Git, "HEAD").unwrap();
        std::fs::write(source.join("later.txt"), "later").unwrap();
        git_commit(&source, "later");
        let primary = tasks_base.join("tabc").join("workspace").join("app");
        crate::vcs::create_linked_checkout(&source, &primary, "tabc", VcsKind::Git, None).unwrap();
        let task = linked_task("tabc", primary, source.clone());
        let dest = add_sibling_checkout(
            &task,
            &tasks_base,
            "old",
            &ForkFrom::Revision(first.clone()),
            None,
        )
        .unwrap();
        assert_eq!(
            crate::vcs::resolve_revision_id(&dest, VcsKind::Git, "HEAD").unwrap(),
            first
        );
        assert!(!dest.join("later.txt").exists());
    }

    #[test]
    fn add_sibling_jj_workspace_from_current() {
        if std::process::Command::new("jj")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            let dir = tempfile::tempdir().unwrap();
            let tasks_base = dir.path().join("tasks");
            let source = dir.path().join("app");
            std::fs::create_dir_all(&source).unwrap();
            let src = source.to_str().unwrap();
            let init = std::process::Command::new("jj")
                .args(["git", "init", "--colocate", src])
                .status()
                .unwrap();
            if !init.success() {
                std::process::Command::new("jj")
                    .args(["git", "init", src])
                    .status()
                    .unwrap();
            }
            std::process::Command::new("jj")
                .args(["-R", src, "describe", "-m", "init"])
                .status()
                .unwrap();
            let primary = tasks_base.join("tabc").join("workspace").join("app");
            crate::vcs::create_linked_checkout(&source, &primary, "tabc", VcsKind::Jj, None)
                .unwrap();
            let task = linked_task("tabc", primary.clone(), source);
            let dest = add_sibling_checkout(
                &task,
                &tasks_base,
                "review",
                &ForkFrom::Current {
                    fallback_task_id: Some("tabc".into()),
                },
                Some(&primary),
            )
            .unwrap();
            assert_eq!(
                dest,
                tasks_base.join("tabc").join("workspace").join("app-review")
            );
            assert!(dest.join(".jj").is_dir());
        }
    }
}
