//! Detect local version-control roots (git, Jujutsu) and manage task checkouts.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::error::{TskError, Result};
use crate::xdg::expand;
use crate::models::Task;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VcsKind {
    Git,
    Jj,
}

/// Walk upward from `start` (or the process cwd when `None`) looking for a git or jj workspace.
pub fn detect_vcs_root(start: Option<&Path>) -> Option<PathBuf> {
    let start = start
        .map(expand)
        .or_else(|| std::env::current_dir().ok().map(|p| expand(&p)))
        .filter(|p| p.is_dir())?;

    let mut dir = start.as_path();
    loop {
        if let Some(kind) = vcs_kind_at(dir) {
            let _ = kind;
            return Some(dir.to_path_buf());
        }
        dir = dir.parent()?;
    }
}

/// Which VCS owns `root` (must already be a repo root).
pub fn vcs_kind_at(root: &Path) -> Option<VcsKind> {
    let root = expand(root);
    if root.join(".jj").is_dir() {
        Some(VcsKind::Jj)
    } else if root.join(".git").exists() {
        Some(VcsKind::Git)
    } else {
        None
    }
}

/// Short display name for a repo path (usually the directory name).
pub fn repo_label(path: &Path) -> String {
    expand(path)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// Initialize an empty git repo (test fixtures and local dev checkouts).
pub fn init_scratch_repo(dest: &Path) -> Result<()> {
    std::fs::create_dir_all(dest).map_err(|source| TskError::Write {
        path: dest.to_path_buf(),
        source,
    })?;
    run_checked(
        Command::new("git").args(["init", dest.to_str().unwrap_or("")]),
        "git init",
    )
}

/// Stable jj workspace name for a tsk task checkout.
pub fn jj_workspace_name_for_task(task_id: &str) -> String {
    task_id.to_string()
}

/// Create a git worktree or jj workspace under `dest` linked to `source_root`.
pub fn create_linked_checkout(
    source_root: &Path,
    dest: &Path,
    workspace_name: &str,
    kind: VcsKind,
) -> Result<()> {
    if dest.is_dir() {
        return match linked_checkout_kind(dest) {
            Some(VcsKind::Git) => Ok(()),
            Some(VcsKind::Jj) => reconnect_jj_workspace(dest),
            None => Err(TskError::Other(format!(
                "Checkout path exists but is not a git/jj workspace: {}",
                dest.display()
            ))),
        };
    }
    if dest.exists() {
        return Err(TskError::Other(format!(
            "Checkout path already exists: {}",
            dest.display()
        )));
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|source| TskError::Write {
            path: parent.to_path_buf(),
            source,
        })?;
    }

    match kind {
        VcsKind::Git => create_git_worktree(source_root, dest, workspace_name),
        VcsKind::Jj => create_jj_workspace(source_root, dest, workspace_name),
    }
}

/// Refresh a jj workspace after it became stale (reactivation / reuse).
pub fn reconnect_jj_workspace(checkout: &Path) -> Result<()> {
    let path = checkout.to_str().ok_or_else(|| {
        TskError::Other(format!("Invalid checkout path: {}", checkout.display()))
    })?;
    run_checked(
        Command::new("jj").args([ "-R", path, "workspace", "update-stale"]),
        "jj workspace update-stale",
    )
}

/// Ensure a task's managed checkout is usable before opening a terminal or similar.
pub fn ensure_task_checkout_ready(task: &Task, config: &crate::config::TskConfig) -> Result<()> {
    if !crate::task_paths::is_managed_task_checkout(
        &task.repo_path,
        &config.tasks_base_dir,
        &task.id,
    ) {
        return ensure_checkout_ready(&task.repo_path);
    }
    let source = task.source_repo_path.as_deref();
    let name = jj_workspace_name_for_task(&task.id);
    reattach_linked_checkout(&task.repo_path, source, Some(&name))
}

/// Ensure a managed jj checkout is usable (no-op for git and non-jj paths).
pub fn ensure_checkout_ready(checkout: &Path) -> Result<()> {
    if linked_checkout_kind(checkout) == Some(VcsKind::Jj) {
        reconnect_jj_workspace(checkout)?;
    }
    Ok(())
}

/// Stable git branch name for a tsk task worktree.
pub fn git_branch_for_task(task_id: &str) -> String {
    format!("tsk-{task_id}")
}

fn create_git_worktree(source_root: &Path, dest: &Path, branch: &str) -> Result<()> {
    let branch = format!("tsk-{branch}");
    let source = source_root.to_str().ok_or_else(|| {
        TskError::Other(format!(
            "Invalid source repo path: {}",
            source_root.display()
        ))
    })?;
    let dest_str = dest.to_str().ok_or_else(|| {
        TskError::Other(format!("Invalid checkout path: {}", dest.display()))
    })?;

    let add_new_branch = Command::new("git")
        .args([
            "-C",
            source,
            "worktree",
            "add",
            "-b",
            branch.as_str(),
            dest_str,
        ])
        .output();
    match add_new_branch {
        Ok(out) if out.status.success() => return Ok(()),
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            if !stderr.contains("already exists") {
                return Err(TskError::Other(format!(
                    "git worktree add failed: {}",
                    stderr.trim()
                )));
            }
        }
        Err(e) => {
            return Err(TskError::Other(format!("failed to run git worktree add: {e}")));
        }
    }

    run_checked(
        Command::new("git").args([
            "-C",
            source,
            "worktree",
            "add",
            dest_str,
            branch.as_str(),
        ]),
        "git worktree add",
    )
}

fn create_jj_workspace(source_root: &Path, dest: &Path, name: &str) -> Result<()> {
    let source = source_root.to_str().ok_or_else(|| {
        TskError::Other(format!(
            "Invalid source repo path: {}",
            source_root.display()
        ))
    })?;
    let dest_str = dest.to_str().ok_or_else(|| {
        TskError::Other(format!("Invalid checkout path: {}", dest.display()))
    })?;

    run_checked(
        Command::new("jj").args([
            "-R",
            source,
            "workspace",
            "add",
            "--name",
            name,
            dest_str,
        ]),
        "jj workspace add",
    )
}

/// Stop tracking a jj workspace without deleting files (e.g. archive).
pub fn detach_jj_workspace(source_root: &Path, workspace_name: &str) -> Result<()> {
    forget_jj_workspace(source_root, workspace_name)
}

/// Re-link a detached checkout to its source repo (e.g. restore from archive).
pub fn reattach_linked_checkout(
    checkout: &Path,
    source_root: Option<&Path>,
    workspace_name: Option<&str>,
) -> Result<()> {
    if !checkout.exists() {
        return Ok(());
    }
    match linked_checkout_kind(checkout) {
        Some(VcsKind::Jj) => {
            let name = workspace_name
                .map(str::to_string)
                .or_else(|| jj_workspace_name_at(checkout).ok())
                .unwrap_or_default();
            if name.is_empty() {
                return Ok(());
            }
            let source = source_root
                .map(|p| p.to_path_buf())
                .or_else(|| jj_repo_root_from_checkout(checkout))
                .ok_or_else(|| {
                    TskError::Other(format!(
                        "Could not find jj repository for {}",
                        checkout.display()
                    ))
                })?;
            if jj_workspace_registered_at_source(&source, &name) {
                reconnect_jj_workspace(checkout)
            } else {
                relink_forgotten_jj_workspace(&source, checkout, &name)
            }
        }
        Some(VcsKind::Git) => reattach_git_worktree(source_root, checkout, workspace_name),
        None => {
            if let (Some(source), Some(task_id)) = (source_root, workspace_name) {
                if checkout.exists() && !checkout.join(".jj").is_dir() {
                    return reattach_git_worktree(Some(source), checkout, Some(task_id));
                }
            }
            Ok(())
        }
    }
}

fn reattach_git_worktree(
    source_root: Option<&Path>,
    checkout: &Path,
    task_id: Option<&str>,
) -> Result<()> {
    let task_id = task_id
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| {
            TskError::Other(format!(
                "Could not determine git worktree branch for {}",
                checkout.display()
            ))
        })?;
    let source = source_root
        .map(|p| p.to_path_buf())
        .ok_or_else(|| {
            TskError::Other(format!(
                "Could not find git repository for {}",
                checkout.display()
            ))
        })?;
    if git_worktree_listed_at_source(&source, checkout) && is_git_worktree(checkout) {
        return Ok(());
    }
    relink_detached_git_worktree(&source, checkout, task_id)
}

/// Detach a linked checkout from its source repo without deleting files (archive).
pub fn detach_linked_checkout(
    checkout: &Path,
    source_root: Option<&Path>,
    workspace_name: Option<&str>,
) -> Result<()> {
    if !checkout.exists() {
        return Ok(());
    }
    match linked_checkout_kind(checkout) {
        Some(VcsKind::Git) => {
            let source = source_root
                .map(|p| p.to_path_buf())
                .ok_or_else(|| {
                    TskError::Other(format!(
                        "Could not find git repository for {}",
                        checkout.display()
                    ))
                })?;
            detach_git_worktree(&source, checkout)
        }
        Some(VcsKind::Jj) => {
            let name = workspace_name
                .map(str::to_string)
                .or_else(|| jj_workspace_name_at(checkout).ok())
                .unwrap_or_default();
            if name.is_empty() {
                return Ok(());
            }
            let source = source_root
                .map(|p| p.to_path_buf())
                .or_else(|| jj_repo_root_from_checkout(checkout))
                .ok_or_else(|| {
                    TskError::Other(format!(
                        "Could not find jj repository for {}",
                        checkout.display()
                    ))
                })?;
            forget_jj_workspace(&source, &name)
        }
        None => Ok(()),
    }
}

/// Remove a task-linked checkout (git worktree or jj workspace).
pub fn remove_linked_checkout(
    checkout: &Path,
    source_root: Option<&Path>,
    workspace_name: Option<&str>,
) -> Result<()> {
    if !checkout.exists() && source_root.is_none() {
        return Ok(());
    }

    match linked_checkout_kind(checkout) {
        Some(VcsKind::Git) if checkout.exists() => remove_git_worktree(checkout),
        Some(VcsKind::Jj) => {
            let name = workspace_name
                .map(str::to_string)
                .or_else(|| jj_workspace_name_at(checkout).ok())
                .unwrap_or_default();
            let source = source_root
                .map(|p| p.to_path_buf())
                .or_else(|| jj_repo_root_from_checkout(checkout));
            if let Some(source) = source {
                if !name.is_empty() {
                    let _ = forget_jj_workspace(&source, &name);
                }
            }
            if checkout.exists() {
                std::fs::remove_dir_all(checkout).map_err(|source| TskError::Write {
                    path: checkout.to_path_buf(),
                    source,
                })?;
            }
            Ok(())
        }
        None if checkout.exists() => {
            std::fs::remove_dir_all(checkout).map_err(|source| TskError::Write {
                path: checkout.to_path_buf(),
                source,
            })
        }
        _ => Ok(()),
    }
}

fn linked_checkout_kind(checkout: &Path) -> Option<VcsKind> {
    let checkout = expand(checkout);
    if is_git_worktree(&checkout) {
        Some(VcsKind::Git)
    } else if checkout.join(".jj").is_dir() {
        Some(VcsKind::Jj)
    } else {
        None
    }
}

fn is_git_worktree(path: &Path) -> bool {
    let git = path.join(".git");
    git.is_file()
}

fn remove_git_worktree(checkout: &Path) -> Result<()> {
    let path = checkout.to_str().ok_or_else(|| {
        TskError::Other(format!("Invalid checkout path: {}", checkout.display()))
    })?;
    let out = Command::new("git")
        .args(["-C", path, "worktree", "remove", "--force", path])
        .output()
        .map_err(|e| TskError::Other(format!("failed to run git worktree remove: {e}")))?;
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    Err(TskError::Other(format!(
        "git worktree remove failed: {}",
        stderr.trim()
    )))
}

/// Stop tracking a git worktree without deleting files (archive).
fn detach_git_worktree(source_root: &Path, checkout: &Path) -> Result<()> {
    let checkout = expand(checkout);
    if is_git_worktree(&checkout) {
        let git_file = checkout.join(".git");
        std::fs::remove_file(&git_file).map_err(|source| TskError::Write {
            path: git_file,
            source,
        })?;
    }
    prune_git_worktrees(source_root)
}

fn prune_git_worktrees(source_root: &Path) -> Result<()> {
    let source = source_root.to_str().ok_or_else(|| {
        TskError::Other(format!(
            "Invalid git repository path: {}",
            source_root.display()
        ))
    })?;
    let out = Command::new("git")
        .args(["-C", source, "worktree", "prune"])
        .output()
        .map_err(|e| TskError::Other(format!("failed to run git worktree prune: {e}")))?;
    if out.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&out.stderr);
        Err(TskError::Other(format!(
            "git worktree prune failed: {}",
            stderr.trim()
        )))
    }
}

fn git_worktree_listed_at_source(source_root: &Path, checkout: &Path) -> bool {
    let Some(source) = source_root.to_str() else {
        return false;
    };
    let Ok(out) = Command::new("git")
        .args(["-C", source, "worktree", "list"])
        .output()
    else {
        return false;
    };
    if !out.status.success() {
        return false;
    }
    let checkout_canon =
        std::fs::canonicalize(checkout).unwrap_or_else(|_| expand(checkout));
    String::from_utf8_lossy(&out.stdout).lines().any(|line| {
        let Some(path) = line.split_whitespace().next() else {
            return false;
        };
        let path = expand(Path::new(path));
        std::fs::canonicalize(&path).unwrap_or(path) == checkout_canon
    })
}

fn add_git_worktree_existing_branch(
    source_root: &Path,
    dest: &Path,
    task_id: &str,
) -> Result<()> {
    let branch = git_branch_for_task(task_id);
    let source = source_root.to_str().ok_or_else(|| {
        TskError::Other(format!(
            "Invalid source repo path: {}",
            source_root.display()
        ))
    })?;
    let dest_str = dest.to_str().ok_or_else(|| {
        TskError::Other(format!("Invalid checkout path: {}", dest.display()))
    })?;
    run_checked(
        Command::new("git").args([
            "-C",
            source,
            "worktree",
            "add",
            dest_str,
            branch.as_str(),
        ]),
        "git worktree add",
    )
}

/// Re-register a detached git worktree directory (files kept on disk).
fn relink_detached_git_worktree(
    source_root: &Path,
    checkout: &Path,
    task_id: &str,
) -> Result<()> {
    let checkout = expand(checkout);
    let parent = checkout.parent().ok_or_else(|| {
        TskError::Other(format!("Invalid checkout path: {}", checkout.display()))
    })?;
    let backup = parent.join(format!(".{task_id}-git-relink-tmp"));
    if backup.exists() {
        std::fs::remove_dir_all(&backup).map_err(|source| TskError::Write {
            path: backup.clone(),
            source,
        })?;
    }
    std::fs::create_dir_all(&backup).map_err(|source| TskError::Write {
        path: backup.clone(),
        source,
    })?;

    if checkout.exists() {
        for entry in std::fs::read_dir(&checkout).map_err(|source| TskError::Read {
            path: checkout.clone(),
            source,
        })? {
            let entry = entry.map_err(|source| TskError::Read {
                path: checkout.clone(),
                source,
            })?;
            let dest = backup.join(entry.file_name());
            std::fs::rename(entry.path(), dest).map_err(|source| TskError::Write {
                path: entry.path(),
                source,
            })?;
        }
        std::fs::remove_dir(&checkout).map_err(|source| TskError::Write {
            path: checkout.clone(),
            source,
        })?;
    }

    add_git_worktree_existing_branch(source_root, &checkout, task_id)?;

    for entry in std::fs::read_dir(&backup).map_err(|source| TskError::Read {
        path: backup.clone(),
        source,
    })? {
        let entry = entry.map_err(|source| TskError::Read {
            path: backup.clone(),
            source,
        })?;
        if entry.file_name() == ".git" {
            continue;
        }
        let dest = checkout.join(entry.file_name());
        if dest.exists() {
            if dest.is_dir() {
                std::fs::remove_dir_all(&dest).map_err(|source| TskError::Write {
                    path: dest.clone(),
                    source,
                })?;
            } else {
                std::fs::remove_file(&dest).map_err(|source| TskError::Write {
                    path: dest.clone(),
                    source,
                })?;
            }
        }
        std::fs::rename(entry.path(), dest).map_err(|source| TskError::Write {
            path: entry.path(),
            source,
        })?;
    }

    let _ = std::fs::remove_dir_all(&backup);
    Ok(())
}

fn relink_forgotten_jj_workspace(source_root: &Path, checkout: &Path, name: &str) -> Result<()> {
    let checkout = expand(checkout);
    let parent = checkout.parent().ok_or_else(|| {
        TskError::Other(format!("Invalid checkout path: {}", checkout.display()))
    })?;
    let backup = parent.join(format!(".{name}-relink-tmp"));
    if backup.exists() {
        std::fs::remove_dir_all(&backup).map_err(|source| TskError::Write {
            path: backup.clone(),
            source,
        })?;
    }
    std::fs::create_dir_all(&backup).map_err(|source| TskError::Write {
        path: backup.clone(),
        source,
    })?;

    for entry in std::fs::read_dir(&checkout).map_err(|source| TskError::Read {
        path: checkout.clone(),
        source,
    })? {
        let entry = entry.map_err(|source| TskError::Read {
            path: checkout.clone(),
            source,
        })?;
        let dest = backup.join(entry.file_name());
        std::fs::rename(entry.path(), dest).map_err(|source| TskError::Write {
            path: entry.path(),
            source,
        })?;
    }

    create_jj_workspace(source_root, &checkout, name)?;

    for entry in std::fs::read_dir(&backup).map_err(|source| TskError::Read {
        path: backup.clone(),
        source,
    })? {
        let entry = entry.map_err(|source| TskError::Read {
            path: backup.clone(),
            source,
        })?;
        if entry.file_name() == ".jj" {
            continue;
        }
        let dest = checkout.join(entry.file_name());
        if dest.exists() {
            if dest.is_dir() {
                std::fs::remove_dir_all(&dest).map_err(|source| TskError::Write {
                    path: dest.clone(),
                    source,
                })?;
            } else {
                std::fs::remove_file(&dest).map_err(|source| TskError::Write {
                    path: dest.clone(),
                    source,
                })?;
            }
        }
        std::fs::rename(entry.path(), dest).map_err(|source| TskError::Write {
            path: entry.path(),
            source,
        })?;
    }

    let _ = std::fs::remove_dir_all(&backup);
    Ok(())
}

fn jj_workspace_registered_at_source(source_root: &Path, workspace_name: &str) -> bool {
    let Some(source) = source_root.to_str() else {
        return false;
    };
    let Ok(out) = Command::new("jj")
        .args(["-R", source, "workspace", "list"])
        .output()
    else {
        return false;
    };
    if !out.status.success() {
        return false;
    }
    String::from_utf8_lossy(&out.stdout).lines().any(|line| {
        line.split(':').next().map(|n| n.trim()) == Some(workspace_name)
    })
}

fn forget_jj_workspace(source_root: &Path, workspace_name: &str) -> Result<()> {
    if workspace_name.is_empty() {
        return Ok(());
    }
    let source = source_root.to_str().ok_or_else(|| {
        TskError::Other(format!(
            "Invalid jj repository path: {}",
            source_root.display()
        ))
    })?;
    let out = Command::new("jj")
        .args(["-R", source, "workspace", "forget", workspace_name])
        .output()
        .map_err(|e| TskError::Other(format!("failed to run jj workspace forget: {e}")))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        if !stderr.contains("unknown workspace") && !stderr.contains("No such workspace") {
            eprintln!(
                "tsk: jj workspace forget {}: {}",
                workspace_name,
                stderr.trim()
            );
        }
    }
    Ok(())
}

fn jj_repo_root_from_checkout(checkout: &Path) -> Option<PathBuf> {
    let path = checkout.to_str()?;
    let out = Command::new("jj").args(["-R", path, "workspace", "root"]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let root = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if root.is_empty() {
        None
    } else {
        Some(PathBuf::from(root))
    }
}

fn jj_workspace_name_at(checkout: &Path) -> Result<String> {
    jj_workspace_name(checkout)
}

fn jj_workspace_name(checkout: &Path) -> Result<String> {
    let path = checkout.to_str().ok_or_else(|| {
        TskError::Other(format!("Invalid checkout path: {}", checkout.display()))
    })?;
    let out = Command::new("jj")
        .args(["-R", path, "workspace", "list"])
        .output()
        .map_err(|e| TskError::Other(format!("failed to run jj workspace list: {e}")))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(TskError::Other(format!(
            "jj workspace list failed: {}",
            stderr.trim()
        )));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let canonical = std::fs::canonicalize(checkout).unwrap_or_else(|_| expand(checkout));
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let (name, rest) = line
            .split_once(':')
            .map(|(n, r)| (n.trim(), r.trim()))
            .unwrap_or((line, ""));
        let ws_path = expand(Path::new(rest.split_whitespace().next().unwrap_or(rest)));
        let ws_canonical = std::fs::canonicalize(&ws_path).unwrap_or(ws_path);
        if ws_canonical == canonical {
            return Ok(name.to_string());
        }
    }
    checkout
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .ok_or_else(|| {
            TskError::Other(format!(
                "Could not determine jj workspace name for {}",
                checkout.display()
            ))
        })
}

/// Current branch/bookmark name when available.
pub fn current_branch(checkout: &Path) -> Option<String> {
    let checkout = expand(checkout);
    match vcs_kind_at(&checkout)? {
        VcsKind::Git => {
            let path = checkout.to_str()?;
            let out = Command::new("git")
                .args(["-C", path, "branch", "--show-current"])
                .output()
                .ok()?;
            if !out.status.success() {
                return None;
            }
            let branch = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if branch.is_empty() {
                None
            } else {
                Some(branch)
            }
        }
        VcsKind::Jj => None,
    }
}

fn run_checked(cmd: &mut Command, label: &str) -> Result<()> {
    let out = cmd
        .output()
        .map_err(|e| TskError::Other(format!("failed to run {label}: {e}")))?;
    if out.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&out.stderr);
        Err(TskError::Other(format!(
            "{label} failed: {}",
            stderr.trim()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn detect_git_root() {
        let dir = tempdir().unwrap();
        let repo = dir.path().join("my-project");
        fs::create_dir_all(repo.join("src")).unwrap();
        fs::create_dir(repo.join(".git")).unwrap();

        assert_eq!(
            detect_vcs_root(Some(&repo.join("src"))).as_deref(),
            Some(repo.as_path())
        );
        assert_eq!(vcs_kind_at(&repo), Some(VcsKind::Git));
    }

    #[test]
    fn detect_jj_root() {
        let dir = tempdir().unwrap();
        let repo = dir.path().join("jj-app");
        fs::create_dir_all(repo.join("src")).unwrap();
        fs::create_dir(repo.join(".jj")).unwrap();

        assert_eq!(
            detect_vcs_root(Some(&repo.join("src"))).as_deref(),
            Some(repo.as_path())
        );
        assert_eq!(vcs_kind_at(&repo), Some(VcsKind::Jj));
    }

    #[test]
    fn detect_none_outside_repo() {
        let dir = tempdir().unwrap();
        assert!(detect_vcs_root(Some(dir.path())).is_none());
    }

    #[test]
    fn git_worktree_roundtrip() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("main");
        init_scratch_repo(&source).unwrap();
        let source_str = source.to_str().unwrap();
        for args in [
            &["config", "user.email", "tsk@test"][..],
            &["config", "user.name", "tsk"][..],
            &["commit", "--allow-empty", "-m", "init"][..],
        ] {
            let mut cmd = Command::new("git");
            cmd.arg("-C").arg(source_str);
            cmd.args(args);
            run_checked(&mut cmd, "git").unwrap();
        }
        let dest = dir.path().join("tasks").join("t1").join("workspace").join("main");
        create_linked_checkout(&source, &dest, "t1", VcsKind::Git).unwrap();
        assert!(dest.is_dir());
        assert!(is_git_worktree(&dest));
        remove_linked_checkout(&dest, Some(&source), Some("t1")).unwrap();
        assert!(!dest.exists());
    }

    #[test]
    fn git_worktree_detach_reattach_preserves_files() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("main");
        init_scratch_repo(&source).unwrap();
        let source_str = source.to_str().unwrap();
        for args in [
            &["config", "user.email", "tsk@test"][..],
            &["config", "user.name", "tsk"][..],
            &["commit", "--allow-empty", "-m", "init"][..],
        ] {
            let mut cmd = Command::new("git");
            cmd.arg("-C").arg(source_str);
            cmd.args(args);
            run_checked(&mut cmd, "git").unwrap();
        }
        let dest = dir.path().join("tasks").join("tabc123").join("workspace").join("main");
        create_linked_checkout(&source, &dest, "tabc123", VcsKind::Git).unwrap();
        fs::write(dest.join("local.txt"), "local only").unwrap();

        detach_linked_checkout(&dest, Some(&source), Some("tabc123")).unwrap();
        assert!(!is_git_worktree(&dest));
        assert!(dest.join("local.txt").is_file());
        assert!(!git_worktree_listed_at_source(&source, &dest));

        reattach_linked_checkout(&dest, Some(&source), Some("tabc123")).unwrap();
        assert!(is_git_worktree(&dest));
        assert!(git_worktree_listed_at_source(&source, &dest));
        assert_eq!(fs::read_to_string(dest.join("local.txt")).unwrap(), "local only");
        assert_eq!(
            current_branch(&dest).as_deref(),
            Some(git_branch_for_task("tabc123").as_str())
        );
    }
}
