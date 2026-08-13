//! Detect local version-control roots (git, Jujutsu) and manage task checkouts.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::error::{Result, TskError};
use crate::models::Task;
use crate::xdg::expand;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VcsKind {
    Git,
    Jj,
}

/// Sidecar (relative to checkout) storing the jj working-copy change id across workspace forget.
/// Per-checkout restore metadata under the task home (not inside any repo tree):
/// `~/tsk-tasks/<id>/.tsk/jj-restore/<checkout-name>`.
const JJ_RESTORE_STATE_DIR: &str = ".tsk/jj-restore";
/// Brief-lived flat file beside checkouts (`…/workspace/.jj-working-change`).
const JJ_WORKING_CHANGE_SIDECAR_WORKSPACE: &str = ".jj-working-change";
/// Oldest path: inside the checkout repo tree.
const JJ_WORKING_CHANGE_SIDECAR_IN_REPO: &str = ".tsk/jj-working-change";

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
        VcsKind::Jj => {
            let revision = resolve_jj_default_base(source_root);
            create_jj_workspace(source_root, dest, workspace_name, revision.as_deref())
        }
    }
}

/// Refresh a jj workspace after it became stale (reactivation / reuse).
pub fn reconnect_jj_workspace(checkout: &Path) -> Result<()> {
    let path = checkout
        .to_str()
        .ok_or_else(|| TskError::Other(format!("Invalid checkout path: {}", checkout.display())))?;
    run_checked(
        Command::new("jj").args(["-R", path, "workspace", "update-stale"]),
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
    let dest_str = dest
        .to_str()
        .ok_or_else(|| TskError::Other(format!("Invalid checkout path: {}", dest.display())))?;

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
            return Err(TskError::Other(format!(
                "failed to run git worktree add: {e}"
            )));
        }
    }

    run_checked(
        Command::new("git").args(["-C", source, "worktree", "add", dest_str, branch.as_str()]),
        "git worktree add",
    )
}

fn create_jj_workspace(
    source_root: &Path,
    dest: &Path,
    name: &str,
    revision: Option<&str>,
) -> Result<()> {
    let source = source_root.to_str().ok_or_else(|| {
        TskError::Other(format!(
            "Invalid source repo path: {}",
            source_root.display()
        ))
    })?;
    let dest_str = dest
        .to_str()
        .ok_or_else(|| TskError::Other(format!("Invalid checkout path: {}", dest.display())))?;

    let mut args = vec![
        "-R".to_string(),
        source.to_string(),
        "workspace".to_string(),
        "add".to_string(),
        "--name".to_string(),
        name.to_string(),
    ];
    if let Some(rev) = revision.filter(|r| !r.is_empty()) {
        args.push("-r".to_string());
        args.push(rev.to_string());
    }
    args.push(dest_str.to_string());

    run_checked(Command::new("jj").args(&args), "jj workspace add")
}

/// Prefer trunk()/main so new workspaces never inherit stale default@ parents.
fn resolve_jj_default_base(source_root: &Path) -> Option<String> {
    for revset in ["trunk()", "main"] {
        if let Ok(id) = jj_template(source_root, revset, "commit_id") {
            if !id.is_empty() {
                return Some(id);
            }
        }
    }
    None
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
    let source = source_root.map(|p| p.to_path_buf()).ok_or_else(|| {
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
            let source = source_root.map(|p| p.to_path_buf()).ok_or_else(|| {
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
            if let Err(err) = save_jj_restore_target_before_forget(checkout) {
                eprintln!(
                    "tsk: failed to save jj restore target for {}: {err}",
                    checkout.display()
                );
            }
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
    let path = checkout
        .to_str()
        .ok_or_else(|| TskError::Other(format!("Invalid checkout path: {}", checkout.display())))?;
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
    let checkout_canon = std::fs::canonicalize(checkout).unwrap_or_else(|_| expand(checkout));
    String::from_utf8_lossy(&out.stdout).lines().any(|line| {
        let Some(path) = line.split_whitespace().next() else {
            return false;
        };
        let path = expand(Path::new(path));
        std::fs::canonicalize(&path).unwrap_or(path) == checkout_canon
    })
}

fn add_git_worktree_existing_branch(source_root: &Path, dest: &Path, task_id: &str) -> Result<()> {
    let branch = git_branch_for_task(task_id);
    let source = source_root.to_str().ok_or_else(|| {
        TskError::Other(format!(
            "Invalid source repo path: {}",
            source_root.display()
        ))
    })?;
    let dest_str = dest
        .to_str()
        .ok_or_else(|| TskError::Other(format!("Invalid checkout path: {}", dest.display())))?;
    run_checked(
        Command::new("git").args(["-C", source, "worktree", "add", dest_str, branch.as_str()]),
        "git worktree add",
    )
}

/// Re-register a detached git worktree directory (files kept on disk).
fn relink_detached_git_worktree(source_root: &Path, checkout: &Path, task_id: &str) -> Result<()> {
    let checkout = expand(checkout);
    let parent = checkout
        .parent()
        .ok_or_else(|| TskError::Other(format!("Invalid checkout path: {}", checkout.display())))?;
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
    let mut target = read_jj_restore_target(&checkout).unwrap_or_default();
    // Live @ is usually unreadable after forget; try only as a last-chance edit id.
    if target.edit_change_id.is_none() {
        if let Ok(id) = jj_working_copy_change_id(&checkout) {
            target.edit_change_id = Some(id);
        }
    }

    let parent = checkout
        .parent()
        .ok_or_else(|| TskError::Other(format!("Invalid checkout path: {}", checkout.display())))?;
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

    let revision = target
        .base_commit_id
        .clone()
        .or_else(|| resolve_jj_default_base(source_root));
    create_jj_workspace(source_root, &checkout, name, revision.as_deref())?;

    if let Some(change_id) = target.edit_change_id.as_deref() {
        if jj_revision_exists(&checkout, change_id) {
            let path = checkout.to_str().ok_or_else(|| {
                TskError::Other(format!("Invalid checkout path: {}", checkout.display()))
            })?;
            if let Err(err) = run_checked(
                Command::new("jj").args(["-R", path, "edit", change_id]),
                "jj edit",
            ) {
                eprintln!(
                    "tsk: jj edit {change_id} after workspace relink failed (continuing): {err}"
                );
            }
        } else {
            eprintln!(
                "tsk: saved jj edit change {change_id} no longer exists after workspace forget; keeping workspace on base"
            );
        }
    }

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

    if let Err(err) = save_jj_restore_target_before_forget(&checkout) {
        eprintln!(
            "tsk: failed to refresh jj restore target for {}: {err}",
            checkout.display()
        );
    }

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
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .any(|line| line.split(':').next().map(|n| n.trim()) == Some(workspace_name))
}

/// Task home for a managed checkout: `…/<id>/workspace/<repo>` → `…/<id>`,
/// or scratch `…/<id>/workspace` → `…/<id>`.
fn task_home_for_checkout(checkout: &Path) -> Option<PathBuf> {
    let checkout = expand(checkout);
    let name = checkout.file_name()?.to_string_lossy();
    if name == "workspace" {
        return checkout.parent().map(Path::to_path_buf);
    }
    let parent = checkout.parent()?;
    if parent.file_name()?.to_string_lossy() == "workspace" {
        return parent.parent().map(Path::to_path_buf);
    }
    None
}

/// Key for this checkout under `.tsk/jj-restore/` (repo folder name, or `_` for scratch).
fn jj_restore_checkout_key(checkout: &Path) -> String {
    let checkout = expand(checkout);
    let name = checkout
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "checkout".into());
    if name == "workspace" {
        "_".into()
    } else {
        name
    }
}

/// Canonical sidecar: `~/tsk-tasks/<id>/.tsk/jj-restore/<checkout-key>`.
fn jj_working_change_sidecar(checkout: &Path) -> PathBuf {
    if let Some(home) = task_home_for_checkout(checkout) {
        return home
            .join(JJ_RESTORE_STATE_DIR)
            .join(jj_restore_checkout_key(checkout));
    }
    // Unmanaged path fallback: keep state beside the checkout parent.
    let checkout = expand(checkout);
    match checkout.parent() {
        Some(parent) => parent
            .join(JJ_RESTORE_STATE_DIR)
            .join(jj_restore_checkout_key(&checkout)),
        None => checkout.join(JJ_WORKING_CHANGE_SIDECAR_IN_REPO),
    }
}

fn jj_working_change_sidecar_workspace_flat(checkout: &Path) -> Option<PathBuf> {
    let checkout = expand(checkout);
    let parent = checkout.parent()?;
    if parent.file_name()?.to_string_lossy() == "workspace" {
        return Some(parent.join(JJ_WORKING_CHANGE_SIDECAR_WORKSPACE));
    }
    if checkout.file_name()?.to_string_lossy() == "workspace" {
        return Some(checkout.join(JJ_WORKING_CHANGE_SIDECAR_WORKSPACE));
    }
    None
}

fn jj_working_change_sidecar_in_repo(checkout: &Path) -> PathBuf {
    expand(checkout).join(JJ_WORKING_CHANGE_SIDECAR_IN_REPO)
}

/// Target used to recreate a forgotten jj workspace at a sensible revision.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct JjRestoreTarget {
    /// Change id to `jj edit` after workspace add; absent when `@` was empty (abandoned on forget).
    edit_change_id: Option<String>,
    /// Commit id for `jj workspace add -r` (prefer `@-` when `@` empty).
    base_commit_id: Option<String>,
}

fn jj_template(checkout: &Path, revset: &str, template: &str) -> Result<String> {
    let path = checkout
        .to_str()
        .ok_or_else(|| TskError::Other(format!("Invalid checkout path: {}", checkout.display())))?;
    let out = Command::new("jj")
        .args([
            "--ignore-working-copy",
            "-R",
            path,
            "log",
            "-r",
            revset,
            "-T",
            template,
            "--no-graph",
        ])
        .output()
        .map_err(|e| TskError::Other(format!("failed to run jj log: {e}")))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(TskError::Other(format!(
            "jj log -r {revset} failed: {}",
            stderr.trim()
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn jj_working_copy_is_empty(checkout: &Path) -> Result<bool> {
    let value = jj_template(checkout, "@", "empty")?;
    Ok(value == "true")
}

fn jj_revision_exists(checkout: &Path, rev: &str) -> bool {
    jj_template(checkout, rev, "change_id").is_ok()
}

/// Working-copy change id for a jj checkout (`jj log -r @`), ignoring the working copy snapshot.
fn jj_working_copy_change_id(checkout: &Path) -> Result<String> {
    let id = jj_template(checkout, "@", "change_id")?;
    if id.is_empty() {
        return Err(TskError::Other(format!(
            "jj log -r @ returned empty change id for {}",
            checkout.display()
        )));
    }
    Ok(id)
}

/// Snapshot restore metadata from the live working copy (also used to refresh after relink).
fn save_jj_restore_target_before_forget(checkout: &Path) -> Result<()> {
    let empty = jj_working_copy_is_empty(checkout).unwrap_or(false);
    let edit_change_id = if empty {
        None
    } else {
        jj_working_copy_change_id(checkout).ok()
    };
    // Prefer parent of @ as the stable base; fall back to @ commit id when @- is unavailable.
    let base_commit_id = jj_template(checkout, "@-", "commit_id")
        .ok()
        .filter(|id| !id.is_empty())
        .or_else(|| {
            jj_template(checkout, "@", "commit_id")
                .ok()
                .filter(|id| !id.is_empty())
        });

    write_jj_restore_target(
        checkout,
        &JjRestoreTarget {
            edit_change_id,
            base_commit_id,
        },
    )
}

fn write_jj_restore_target(checkout: &Path, target: &JjRestoreTarget) -> Result<()> {
    let path = jj_working_change_sidecar(checkout);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| TskError::Write {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let mut lines = vec!["v1".to_string()];
    if let Some(id) = target.edit_change_id.as_deref() {
        lines.push(format!("edit:{id}"));
    }
    if let Some(id) = target.base_commit_id.as_deref() {
        lines.push(format!("base:{id}"));
    }
    lines.push(String::new());
    std::fs::write(&path, lines.join("\n")).map_err(|source| TskError::Write { path, source })?;
    // Drop older locations so state is not tracked as source / not ambiguous.
    let in_repo = jj_working_change_sidecar_in_repo(checkout);
    if in_repo.exists() {
        let _ = std::fs::remove_file(in_repo);
    }
    if let Some(flat) = jj_working_change_sidecar_workspace_flat(checkout) {
        if flat.exists() {
            let _ = std::fs::remove_file(flat);
        }
    }
    Ok(())
}

fn parse_jj_restore_target_contents(contents: &str) -> Option<JjRestoreTarget> {
    let trimmed = contents.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut lines = trimmed.lines();
    let first = lines.next()?.trim();
    if first == "v1" {
        let mut target = JjRestoreTarget::default();
        for line in lines {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Some(id) = line.strip_prefix("edit:") {
                let id = id.trim();
                if !id.is_empty() {
                    target.edit_change_id = Some(id.to_string());
                }
            } else if let Some(id) = line.strip_prefix("base:") {
                let id = id.trim();
                if !id.is_empty() {
                    target.base_commit_id = Some(id.to_string());
                }
            }
        }
        return Some(target);
    }

    // Legacy: bare change-id line → edit only, no base.
    Some(JjRestoreTarget {
        edit_change_id: Some(first.to_string()),
        base_commit_id: None,
    })
}

fn read_jj_restore_target(checkout: &Path) -> Option<JjRestoreTarget> {
    let candidates = [
        Some(jj_working_change_sidecar(checkout)),
        jj_working_change_sidecar_workspace_flat(checkout),
        Some(jj_working_change_sidecar_in_repo(checkout)),
    ];
    for path in candidates.into_iter().flatten() {
        if let Ok(contents) = std::fs::read_to_string(path) {
            if let Some(target) = parse_jj_restore_target_contents(&contents) {
                return Some(target);
            }
        }
    }
    None
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
    let out = Command::new("jj")
        .args(["-R", path, "workspace", "root"])
        .output()
        .ok()?;
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
    let path = checkout
        .to_str()
        .ok_or_else(|| TskError::Other(format!("Invalid checkout path: {}", checkout.display())))?;
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
        let dest = dir
            .path()
            .join("tasks")
            .join("t1")
            .join("workspace")
            .join("main");
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
        let dest = dir
            .path()
            .join("tasks")
            .join("tabc123")
            .join("workspace")
            .join("main");
        create_linked_checkout(&source, &dest, "tabc123", VcsKind::Git).unwrap();
        fs::write(dest.join("local.txt"), "local only").unwrap();

        detach_linked_checkout(&dest, Some(&source), Some("tabc123")).unwrap();
        assert!(!is_git_worktree(&dest));
        assert!(dest.join("local.txt").is_file());
        assert!(!git_worktree_listed_at_source(&source, &dest));

        reattach_linked_checkout(&dest, Some(&source), Some("tabc123")).unwrap();
        assert!(is_git_worktree(&dest));
        assert!(git_worktree_listed_at_source(&source, &dest));
        assert_eq!(
            fs::read_to_string(dest.join("local.txt")).unwrap(),
            "local only"
        );
        assert_eq!(
            current_branch(&dest).as_deref(),
            Some(git_branch_for_task("tabc123").as_str())
        );
    }

    #[test]
    fn jj_workspace_detach_reattach_preserves_change_id() {
        if Command::new("jj")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
            == false
        {
            eprintln!(
                "skipping jj_workspace_detach_reattach_preserves_change_id: jj not available"
            );
            return;
        }

        let dir = tempdir().unwrap();
        let source = dir.path().join("main");
        fs::create_dir_all(&source).unwrap();
        run_checked(
            Command::new("jj").args(["git", "init", "--colocate", source.to_str().unwrap()]),
            "jj git init",
        )
        .unwrap_or_else(|_| {
            run_checked(
                Command::new("jj").args(["git", "init", source.to_str().unwrap()]),
                "jj git init",
            )
            .unwrap();
        });

        // Give trunk()/main something to resolve so create_jj_workspace uses -r.
        run_checked(
            Command::new("jj").args([
                "-R",
                source.to_str().unwrap(),
                "bookmark",
                "set",
                "main",
                "-r",
                "@",
            ]),
            "jj bookmark set main",
        )
        .unwrap();

        let dest = dir
            .path()
            .join("tasks")
            .join("tjj123")
            .join("workspace")
            .join("main");
        create_linked_checkout(&source, &dest, "tjj123", VcsKind::Jj).unwrap();

        fs::write(dest.join("local.txt"), "jj local only").unwrap();
        run_checked(
            Command::new("jj").args(["-R", dest.to_str().unwrap(), "describe", "-m", "task work"]),
            "jj describe",
        )
        .unwrap();

        let original_id = jj_working_copy_change_id(&dest).unwrap();
        let parent_commit = jj_template(&dest, "@-", "commit_id").unwrap();

        detach_linked_checkout(&dest, Some(&source), Some("tjj123")).unwrap();
        assert!(
            jj_working_change_sidecar(&dest).is_file(),
            "sidecar should live under task .tsk/jj-restore/"
        );
        assert!(
            !dest.join(JJ_WORKING_CHANGE_SIDECAR_IN_REPO).exists(),
            "sidecar must not live inside the repo tree"
        );
        let expected = dest
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join(".tsk/jj-restore/main");
        assert_eq!(jj_working_change_sidecar(&dest), expected);
        let target = read_jj_restore_target(&dest).expect("sidecar should parse");
        assert_eq!(target.edit_change_id.as_deref(), Some(original_id.as_str()));
        assert_eq!(
            target.base_commit_id.as_deref(),
            Some(parent_commit.as_str())
        );
        assert!(dest.join("local.txt").is_file());
        assert!(!jj_workspace_registered_at_source(&source, "tjj123"));

        reattach_linked_checkout(&dest, Some(&source), Some("tjj123")).unwrap();
        assert!(jj_workspace_registered_at_source(&source, "tjj123"));
        assert_eq!(
            jj_working_copy_change_id(&dest).unwrap(),
            original_id,
            "working-copy change id should be preserved across detach/reattach"
        );
        assert_eq!(
            fs::read_to_string(dest.join("local.txt")).unwrap(),
            "jj local only"
        );
    }

    #[test]
    fn jj_workspace_detach_reattach_empty_wc_uses_parent_base() {
        if Command::new("jj")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
            == false
        {
            eprintln!(
                "skipping jj_workspace_detach_reattach_empty_wc_uses_parent_base: jj not available"
            );
            return;
        }

        let dir = tempdir().unwrap();
        let source = dir.path().join("main");
        fs::create_dir_all(&source).unwrap();
        run_checked(
            Command::new("jj").args(["git", "init", "--colocate", source.to_str().unwrap()]),
            "jj git init",
        )
        .unwrap_or_else(|_| {
            run_checked(
                Command::new("jj").args(["git", "init", source.to_str().unwrap()]),
                "jj git init",
            )
            .unwrap();
        });

        // Seed an "ancient" default@ parent so a missing -r would restore onto the wrong base.
        fs::write(source.join("ancient.txt"), "ancient").unwrap();
        run_checked(
            Command::new("jj").args([
                "-R",
                source.to_str().unwrap(),
                "describe",
                "-m",
                "ancient root",
            ]),
            "jj describe ancient",
        )
        .unwrap();
        run_checked(
            Command::new("jj").args(["-R", source.to_str().unwrap(), "new"]),
            "jj new after ancient",
        )
        .unwrap();

        // Advance main / trunk away from the empty default@ lineage.
        fs::write(source.join("main.txt"), "on main").unwrap();
        run_checked(
            Command::new("jj").args(["-R", source.to_str().unwrap(), "describe", "-m", "main tip"]),
            "jj describe main tip",
        )
        .unwrap();
        let main_commit = jj_template(&source, "@", "commit_id").unwrap();
        run_checked(
            Command::new("jj").args([
                "-R",
                source.to_str().unwrap(),
                "bookmark",
                "set",
                "main",
                "-r",
                "@",
            ]),
            "jj bookmark set main",
        )
        .unwrap();
        // Leave default@ on a fresh empty commit (stale parents relative to main).
        run_checked(
            Command::new("jj").args(["-R", source.to_str().unwrap(), "new", "-r", "root()"]),
            "jj new root for default@",
        )
        .ok();

        assert!(
            resolve_jj_default_base(&source).is_some(),
            "create_jj_workspace must resolve trunk()/main for -r"
        );

        let dest = dir
            .path()
            .join("tasks")
            .join("tjjempty")
            .join("workspace")
            .join("main");
        create_linked_checkout(&source, &dest, "tjjempty", VcsKind::Jj).unwrap();

        // Non-empty described commit, then empty child via jj new.
        fs::write(dest.join("local.txt"), "task file").unwrap();
        run_checked(
            Command::new("jj").args(["-R", dest.to_str().unwrap(), "describe", "-m", "task work"]),
            "jj describe task work",
        )
        .unwrap();
        let parent_commit = jj_template(&dest, "@", "commit_id").unwrap();
        let parent_change = jj_working_copy_change_id(&dest).unwrap();
        run_checked(
            Command::new("jj").args(["-R", dest.to_str().unwrap(), "new"]),
            "jj new empty child",
        )
        .unwrap();
        assert!(jj_working_copy_is_empty(&dest).unwrap());

        detach_linked_checkout(&dest, Some(&source), Some("tjjempty")).unwrap();
        let target = read_jj_restore_target(&dest).expect("sidecar should parse");
        assert!(
            target.edit_change_id.is_none(),
            "empty @ must not save edit change id (abandoned on forget)"
        );
        assert_eq!(
            target.base_commit_id.as_deref(),
            Some(parent_commit.as_str()),
            "base should be parent commit of empty @"
        );
        assert!(!jj_workspace_registered_at_source(&source, "tjjempty"));

        reattach_linked_checkout(&dest, Some(&source), Some("tjjempty")).unwrap();
        assert!(jj_workspace_registered_at_source(&source, "tjjempty"));

        let parent_after = jj_template(&dest, "@-", "commit_id").unwrap();
        assert_eq!(
            parent_after, parent_commit,
            "@- must be the saved parent, not an unrelated ancient/default@ commit"
        );
        assert_ne!(
            parent_after,
            jj_template(&source, "root()", "commit_id").unwrap_or_default()
        );
        // Sanity: we did not land on a random ancient commit unrelated to the task parent.
        let _ = parent_change;
        let _ = main_commit;
        assert_eq!(
            fs::read_to_string(dest.join("local.txt")).unwrap(),
            "task file"
        );
    }

    #[test]
    fn jj_restore_target_legacy_sidecar_parses_as_edit_only() {
        let dir = tempdir().unwrap();
        let checkout = dir.path().join("workspace").join("repo");
        fs::create_dir_all(checkout.join(".tsk")).unwrap();
        fs::write(
            checkout.join(JJ_WORKING_CHANGE_SIDECAR_IN_REPO),
            "abcdef123\n",
        )
        .unwrap();
        let target = read_jj_restore_target(&checkout).unwrap();
        assert_eq!(target.edit_change_id.as_deref(), Some("abcdef123"));
        assert!(target.base_commit_id.is_none());
    }

    #[test]
    fn jj_restore_target_writes_per_checkout_under_task_tsk() {
        let dir = tempdir().unwrap();
        let task_home = dir.path().join("tid");
        let checkout_a = task_home.join("workspace").join("repo-a");
        let checkout_b = task_home.join("workspace").join("repo-b");
        fs::create_dir_all(checkout_a.join(".tsk")).unwrap();
        fs::create_dir_all(&checkout_b).unwrap();
        fs::write(checkout_a.join(JJ_WORKING_CHANGE_SIDECAR_IN_REPO), "old\n").unwrap();
        // Flat workspace file from the intermediate layout.
        fs::write(
            task_home
                .join("workspace")
                .join(JJ_WORKING_CHANGE_SIDECAR_WORKSPACE),
            "flat-old\n",
        )
        .unwrap();

        write_jj_restore_target(
            &checkout_a,
            &JjRestoreTarget {
                edit_change_id: Some("aaa".into()),
                base_commit_id: Some("base-a".into()),
            },
        )
        .unwrap();
        write_jj_restore_target(
            &checkout_b,
            &JjRestoreTarget {
                edit_change_id: Some("bbb".into()),
                base_commit_id: Some("base-b".into()),
            },
        )
        .unwrap();

        assert_eq!(
            jj_working_change_sidecar(&checkout_a),
            task_home.join(".tsk/jj-restore/repo-a")
        );
        assert_eq!(
            jj_working_change_sidecar(&checkout_b),
            task_home.join(".tsk/jj-restore/repo-b")
        );
        assert!(!checkout_a.join(JJ_WORKING_CHANGE_SIDECAR_IN_REPO).exists());
        assert!(!task_home
            .join("workspace")
            .join(JJ_WORKING_CHANGE_SIDECAR_WORKSPACE)
            .exists());

        let a = read_jj_restore_target(&checkout_a).unwrap();
        let b = read_jj_restore_target(&checkout_b).unwrap();
        assert_eq!(a.edit_change_id.as_deref(), Some("aaa"));
        assert_eq!(b.edit_change_id.as_deref(), Some("bbb"));
        assert_ne!(a.base_commit_id, b.base_commit_id);
    }
}
