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

fn path_str(path: &Path) -> Result<&str> {
    path.to_str()
        .ok_or_else(|| TskError::Other(format!("Invalid path: {}", path.display())))
}

/// Read-only jj invocation: do not snapshot working copies (avoids repo-lock races).
fn jj_inspect(repo: &Path) -> Result<Command> {
    let path = path_str(repo)?;
    let mut cmd = Command::new("jj");
    cmd.args(["--ignore-working-copy", "--color=never", "-R", path]);
    Ok(cmd)
}

fn jj_mutate(repo: &Path) -> Result<Command> {
    let path = path_str(repo)?;
    let mut cmd = Command::new("jj");
    cmd.args(["--color=never", "-R", path]);
    Ok(cmd)
}

/// Template: workspace name, tab, root path (may be empty for pre-0.38 workspaces).
const JJ_WORKSPACE_LIST_TEMPLATE: &str = r#"name ++ "\t" ++ root ++ "\n""#;

/// Create a git worktree or jj workspace under `dest` linked to `source_root`.
///
/// `revision` is a git commit-ish or jj revset used as the new checkout's
/// start-point / `-r` parent. When `None`, git uses the source `HEAD` and jj
/// uses `trunk()`/`main`.
pub fn create_linked_checkout(
    source_root: &Path,
    dest: &Path,
    workspace_name: &str,
    kind: VcsKind,
    revision: Option<&str>,
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
        VcsKind::Git => create_git_worktree(source_root, dest, workspace_name, revision),
        VcsKind::Jj => {
            let revision = match revision.filter(|r| !r.is_empty()) {
                Some(rev) => Some(rev.to_string()),
                None => resolve_jj_default_base(source_root),
            };
            create_jj_workspace(source_root, dest, workspace_name, revision.as_deref())
        }
    }
}

/// Refresh a jj workspace after it became stale (reactivation / reuse).
pub fn reconnect_jj_workspace(checkout: &Path) -> Result<()> {
    run_checked(
        jj_mutate(checkout)?.args(["workspace", "update-stale"]),
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

fn create_git_worktree(
    source_root: &Path,
    dest: &Path,
    branch: &str,
    start_point: Option<&str>,
) -> Result<()> {
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

    let mut add_args = vec![
        "-C",
        source,
        "worktree",
        "add",
        "-b",
        branch.as_str(),
        dest_str,
    ];
    if let Some(rev) = start_point.filter(|r| !r.is_empty()) {
        add_args.push(rev);
    }
    let add_new_branch = Command::new("git").args(&add_args).output();
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
    let dest_str = path_str(dest)?;
    let mut cmd = jj_mutate(source_root)?;
    cmd.args(["workspace", "add", "--name", name]);
    if let Some(rev) = revision.filter(|r| !r.is_empty()) {
        cmd.args(["-r", rev]);
    }
    cmd.arg(dest_str);
    run_checked(&mut cmd, "jj workspace add")
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

/// Resolve a git commit-ish or jj revset to a commit id in `repo`.
///
/// Snapshots a jj working copy so `@` / `workspace@` reflect live files.
pub fn resolve_revision_id(repo: &Path, kind: VcsKind, rev: &str) -> Result<String> {
    let rev = rev.trim();
    if rev.is_empty() {
        return Err(TskError::UnknownRevision {
            rev: rev.to_string(),
            path: repo.to_path_buf(),
        });
    }
    match kind {
        VcsKind::Git => git_rev_parse(repo, rev),
        VcsKind::Jj => {
            let id = jj_template_live(repo, rev, "commit_id")?;
            if id.is_empty() {
                return Err(TskError::UnknownRevision {
                    rev: rev.to_string(),
                    path: repo.to_path_buf(),
                });
            }
            Ok(id)
        }
    }
}

/// Live working-copy / HEAD commit of `checkout`.
pub fn current_checkout_revision(checkout: &Path, kind: VcsKind) -> Result<String> {
    match kind {
        VcsKind::Git => git_rev_parse(checkout, "HEAD"),
        VcsKind::Jj => resolve_revision_id(checkout, VcsKind::Jj, "@"),
    }
}

/// Whether `checkout` is a workspace/worktree of `source_root`.
pub fn checkout_belongs_to_repo(
    source_root: &Path,
    checkout: &Path,
    kind: VcsKind,
) -> Result<bool> {
    let source = expand(source_root);
    let checkout = expand(checkout);
    if !checkout.is_dir() {
        return Ok(false);
    }
    match kind {
        VcsKind::Git => {
            let a = git_common_dir(&source)?;
            let b = git_common_dir(&checkout)?;
            Ok(same_path(&a, &b))
        }
        VcsKind::Jj => {
            let checkout_canon = std::fs::canonicalize(&checkout).unwrap_or(checkout);
            Ok(jj_list_workspaces(&source)?.iter().any(|(_, root)| {
                root.as_ref()
                    .is_some_and(|path| same_path(path, &checkout_canon))
            }))
        }
    }
}

/// Working-copy root for a named jj workspace in `source_root`.
pub fn jj_workspace_checkout(source_root: &Path, workspace_name: &str) -> Result<PathBuf> {
    let name = workspace_name.trim();
    if name.is_empty() {
        return Err(TskError::UnknownForkWorkspace {
            name: workspace_name.to_string(),
            path: source_root.to_path_buf(),
        });
    }
    for (ws_name, root) in jj_list_workspaces(source_root)? {
        if ws_name == name {
            return root.ok_or_else(|| TskError::UnknownForkWorkspace {
                name: name.to_string(),
                path: source_root.to_path_buf(),
            });
        }
    }
    Err(TskError::UnknownForkWorkspace {
        name: name.to_string(),
        path: source_root.to_path_buf(),
    })
}

fn same_path(a: &Path, b: &Path) -> bool {
    let a = std::fs::canonicalize(a).unwrap_or_else(|_| a.to_path_buf());
    let b = std::fs::canonicalize(b).unwrap_or_else(|_| b.to_path_buf());
    a == b
}

fn git_rev_parse(repo: &Path, rev: &str) -> Result<String> {
    let path = path_str(repo)?;
    let out = Command::new("git")
        .args([
            "-C",
            path,
            "rev-parse",
            "--verify",
            &format!("{rev}^{{commit}}"),
        ])
        .output()
        .map_err(|e| TskError::Other(format!("failed to run git rev-parse: {e}")))?;
    if !out.status.success() {
        return Err(TskError::UnknownRevision {
            rev: rev.to_string(),
            path: repo.to_path_buf(),
        });
    }
    let id = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if id.is_empty() {
        return Err(TskError::UnknownRevision {
            rev: rev.to_string(),
            path: repo.to_path_buf(),
        });
    }
    Ok(id)
}

fn git_common_dir(repo: &Path) -> Result<PathBuf> {
    let path = path_str(repo)?;
    let out = Command::new("git")
        .args(["-C", path, "rev-parse", "--git-common-dir"])
        .output()
        .map_err(|e| TskError::Other(format!("failed to run git rev-parse: {e}")))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(TskError::Other(format!(
            "git rev-parse --git-common-dir failed in {}: {}",
            repo.display(),
            stderr.trim()
        )));
    }
    let raw = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if raw.is_empty() {
        return Err(TskError::Other(format!(
            "git rev-parse --git-common-dir returned empty for {}",
            repo.display()
        )));
    }
    let dir = PathBuf::from(&raw);
    let absolute = if dir.is_absolute() {
        dir
    } else {
        expand(repo).join(dir)
    };
    Ok(std::fs::canonicalize(&absolute).unwrap_or(absolute))
}

/// Snapshot the working copy, then evaluate a jj template (for live `@`).
fn jj_template_live(checkout: &Path, revset: &str, template: &str) -> Result<String> {
    let out = jj_mutate(checkout)?
        .args(["log", "-r", revset, "-T", template, "--no-graph"])
        .output()
        .map_err(|e| TskError::Other(format!("failed to run jj log: {e}")))?;
    if !out.status.success() {
        return Err(TskError::UnknownRevision {
            rev: revset.to_string(),
            path: checkout.to_path_buf(),
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Stop tracking a jj workspace without deleting files (e.g. archive).
pub fn detach_jj_workspace(source_root: &Path, workspace_name: &str) -> Result<()> {
    forget_jj_workspace(source_root, workspace_name)
}

/// Re-link a detached checkout to its source repo (e.g. restore from archive).
///
/// VCS kind comes from the source repo when we have one. A jj source must not
/// fall through to `git worktree add` just because the checkout has a leftover
/// `.git` file or no `.jj` after `workspace forget`.
pub fn reattach_linked_checkout(
    checkout: &Path,
    source_root: Option<&Path>,
    workspace_name: Option<&str>,
) -> Result<()> {
    if let Some(name) = workspace_name {
        recover_relink_backups(checkout, name)?;
    }
    let source = source_root.map(expand);
    let kind = source
        .as_deref()
        .and_then(vcs_kind_at)
        .or_else(|| linked_checkout_kind(checkout));
    match kind {
        Some(VcsKind::Jj) => {
            let name = workspace_name
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(str::to_string)
                .or_else(|| jj_workspace_name_at(checkout).ok())
                .unwrap_or_default();
            if name.is_empty() {
                return if checkout.join(".jj").is_dir() {
                    reconnect_jj_workspace(checkout)
                } else {
                    Ok(())
                };
            }
            let source = source
                .or_else(|| jj_repo_root_from_checkout(checkout))
                .ok_or_else(|| {
                    TskError::Other(format!(
                        "Could not find jj repository for {}",
                        checkout.display()
                    ))
                })?;
            reattach_jj_workspace(&source, checkout, &name)
        }
        Some(VcsKind::Git) => reattach_git_worktree(source.as_deref(), checkout, workspace_name),
        None => Ok(()),
    }
}

fn reattach_jj_workspace(source: &Path, checkout: &Path, name: &str) -> Result<()> {
    match jj_workspace_usable_at_checkout(source, checkout, name) {
        Ok(true) => reconnect_jj_workspace(checkout),
        Ok(false) => {
            match jj_workspace_registered_at_source(source, name) {
                Ok(true) => forget_jj_workspace(source, name)?,
                Ok(false) => {}
                Err(err) => {
                    // A failed `jj workspace list` must not be treated as "forgotten":
                    // relink moves the live tree aside, then `workspace add` fails if
                    // the name still exists, leaving the checkout in `.{name}-relink-tmp`.
                    eprintln!(
                        "tsk: could not list jj workspaces for {}; skipping relink: {err}",
                        source.display()
                    );
                    return if checkout.join(".jj").is_dir() {
                        reconnect_jj_workspace(checkout).or(Err(err))
                    } else {
                        Err(err)
                    };
                }
            }
            relink_forgotten_jj_workspace(source, checkout, name)
        }
        Err(err) => {
            eprintln!(
                "tsk: could not list jj workspaces for {}; skipping relink: {err}",
                source.display()
            );
            if checkout.join(".jj").is_dir() {
                reconnect_jj_workspace(checkout).or(Err(err))
            } else {
                Err(err)
            }
        }
    }
}

/// Whether `name` is a live jj workspace whose working copy is `checkout`.
///
/// A registered name with a missing or other root is a ghost (dest deleted
/// after `workspace add`, or leftover after a failed relink). An unrecorded
/// root (pre-0.38) is live only when `checkout` still has `.jj`.
fn jj_workspace_usable_at_checkout(source: &Path, checkout: &Path, name: &str) -> Result<bool> {
    if !checkout.join(".jj").is_dir() {
        return Ok(false);
    }
    let checkout_canon = std::fs::canonicalize(checkout).unwrap_or_else(|_| expand(checkout));
    for (ws_name, root) in jj_list_workspaces(source)? {
        if ws_name != name {
            continue;
        }
        return Ok(match root {
            Some(path) => same_path(&path, &checkout_canon),
            None => true,
        });
    }
    Ok(false)
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
    match git_worktree_listed_at_source(&source, checkout) {
        Ok(true) if is_git_worktree(checkout) => Ok(()),
        Ok(_) => relink_detached_git_worktree(&source, checkout, task_id),
        Err(err) => {
            eprintln!(
                "tsk: git worktree list failed for {}; skipping relink: {err}",
                source.display()
            );
            if is_git_worktree(checkout) {
                Ok(())
            } else {
                Err(err)
            }
        }
    }
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
    // Prefer jj when both exist (colocated repo / jj workspace that also has a
    // `.git` file). Matching `vcs_kind_at` keeps archive/restore on the jj path.
    if checkout.join(".jj").is_dir() {
        Some(VcsKind::Jj)
    } else if is_git_worktree(&checkout) {
        Some(VcsKind::Git)
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

fn git_worktree_listed_at_source(source_root: &Path, checkout: &Path) -> Result<bool> {
    let source = path_str(source_root)?;
    let out = Command::new("git")
        .args(["-C", source, "worktree", "list"])
        .output()
        .map_err(|e| TskError::Other(format!("failed to run git worktree list: {e}")))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(TskError::Other(format!(
            "git worktree list failed: {}",
            stderr.trim()
        )));
    }
    let checkout_canon = std::fs::canonicalize(checkout).unwrap_or_else(|_| expand(checkout));
    Ok(String::from_utf8_lossy(&out.stdout).lines().any(|line| {
        let Some(path) = line.split_whitespace().next() else {
            return false;
        };
        let path = expand(Path::new(path));
        std::fs::canonicalize(&path).unwrap_or(path) == checkout_canon
    }))
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
    recover_relink_backups(&checkout, task_id)?;
    let backup = git_relink_backup_path(&checkout, task_id)?;
    prepare_relink_backup(&checkout, &backup)?;

    if checkout.exists() {
        move_dir_contents(&checkout, &backup)?;
        std::fs::remove_dir(&checkout).map_err(|source| TskError::Write {
            path: checkout.clone(),
            source,
        })?;
    }

    if let Err(err) = add_git_worktree_existing_branch(source_root, &checkout, task_id) {
        restore_relink_backup(&checkout, &backup);
        return Err(err);
    }

    overlay_backup_onto_checkout(&backup, &checkout, ".git")?;
    let _ = std::fs::remove_dir_all(&backup);
    Ok(())
}

fn relink_forgotten_jj_workspace(source_root: &Path, checkout: &Path, name: &str) -> Result<()> {
    let checkout = expand(checkout);
    recover_relink_backups(&checkout, name)?;

    let mut target = read_jj_restore_target(&checkout).unwrap_or_default();
    // Live @ is usually unreadable after forget; try only as a last-chance edit id.
    if target.edit_change_id.is_none() {
        if let Ok(id) = jj_working_copy_change_id(&checkout) {
            target.edit_change_id = Some(id);
        }
    }

    let backup = jj_relink_backup_path(&checkout, name)?;
    prepare_relink_backup(&checkout, &backup)?;
    if checkout.exists() {
        move_dir_contents(&checkout, &backup)?;
    }

    let revision = target
        .base_commit_id
        .clone()
        .or_else(|| resolve_jj_default_base(source_root));
    if let Err(err) = create_jj_workspace(source_root, &checkout, name, revision.as_deref()) {
        restore_relink_backup(&checkout, &backup);
        return Err(err);
    }

    if let Some(change_id) = target.edit_change_id.as_deref() {
        if jj_revision_exists(&checkout, change_id) {
            if let Err(err) =
                run_checked(jj_mutate(&checkout)?.args(["edit", change_id]), "jj edit")
            {
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

    overlay_backup_onto_checkout(&backup, &checkout, ".jj")?;
    let _ = std::fs::remove_dir_all(&backup);

    if let Err(err) = save_jj_restore_target_before_forget(&checkout) {
        eprintln!(
            "tsk: failed to refresh jj restore target for {}: {err}",
            checkout.display()
        );
    }

    Ok(())
}

fn jj_relink_backup_path(checkout: &Path, name: &str) -> Result<PathBuf> {
    relink_backup_dir(checkout, &format!(".{name}-relink-tmp"))
}

fn git_relink_backup_path(checkout: &Path, name: &str) -> Result<PathBuf> {
    relink_backup_dir(checkout, &format!(".{name}-git-relink-tmp"))
}

fn relink_backup_dir(checkout: &Path, dir_name: &str) -> Result<PathBuf> {
    let parent = checkout
        .parent()
        .ok_or_else(|| TskError::Other(format!("Invalid checkout path: {}", checkout.display())))?;
    Ok(parent.join(dir_name))
}

fn relink_backup_candidates(checkout: &Path, name: &str) -> Vec<PathBuf> {
    let Ok(jj) = jj_relink_backup_path(checkout, name) else {
        return Vec::new();
    };
    let Ok(git) = git_relink_backup_path(checkout, name) else {
        return vec![jj];
    };
    vec![jj, git]
}

fn dir_has_entries(path: &Path) -> bool {
    std::fs::read_dir(path)
        .ok()
        .map(|entries| entries.filter_map(std::result::Result::ok).next().is_some())
        .unwrap_or(false)
}

/// Move files back from a leftover `.{id}-relink-tmp` if the checkout was emptied
/// by an interrupted relink. Never deletes a non-empty leftover while the
/// checkout still has files.
fn recover_relink_backups(checkout: &Path, name: &str) -> Result<()> {
    let checkout = expand(checkout);
    for backup in relink_backup_candidates(&checkout, name) {
        if !backup.exists() {
            continue;
        }
        if !dir_has_entries(&backup) {
            let _ = std::fs::remove_dir_all(&backup);
            continue;
        }
        let checkout_empty = !checkout.exists() || !dir_has_entries(&checkout);
        if checkout_empty {
            std::fs::create_dir_all(&checkout).map_err(|source| TskError::Write {
                path: checkout.clone(),
                source,
            })?;
            move_dir_contents(&backup, &checkout)?;
            let _ = std::fs::remove_dir_all(&backup);
        } else {
            eprintln!(
                "tsk: leaving leftover relink backup at {} (checkout is not empty)",
                backup.display()
            );
        }
    }
    Ok(())
}

fn prepare_relink_backup(checkout: &Path, backup: &Path) -> Result<()> {
    if backup.exists() && dir_has_entries(backup) && dir_has_entries(checkout) {
        return Err(TskError::Other(format!(
            "Refusing to overwrite leftover relink backup at {}",
            backup.display()
        )));
    }
    if backup.exists() {
        std::fs::remove_dir_all(backup).map_err(|source| TskError::Write {
            path: backup.to_path_buf(),
            source,
        })?;
    }
    std::fs::create_dir_all(backup).map_err(|source| TskError::Write {
        path: backup.to_path_buf(),
        source,
    })
}

fn move_dir_contents(src: &Path, dest: &Path) -> Result<()> {
    std::fs::create_dir_all(dest).map_err(|source| TskError::Write {
        path: dest.to_path_buf(),
        source,
    })?;
    let entries = std::fs::read_dir(src).map_err(|source| TskError::Read {
        path: src.to_path_buf(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| TskError::Read {
            path: src.to_path_buf(),
            source,
        })?;
        let dest_path = dest.join(entry.file_name());
        std::fs::rename(entry.path(), &dest_path).map_err(|source| TskError::Write {
            path: entry.path(),
            source,
        })?;
    }
    Ok(())
}

fn restore_relink_backup(checkout: &Path, backup: &Path) {
    if !backup.exists() {
        return;
    }
    if let Err(err) = std::fs::create_dir_all(checkout) {
        eprintln!(
            "tsk: failed to recreate {} while restoring relink backup: {err}",
            checkout.display()
        );
        return;
    }
    if let Err(err) = move_dir_contents(backup, checkout) {
        eprintln!(
            "tsk: failed to restore checkout from {}: {err}; files left in backup",
            backup.display()
        );
        return;
    }
    let _ = std::fs::remove_dir_all(backup);
}

fn overlay_backup_onto_checkout(backup: &Path, checkout: &Path, skip_name: &str) -> Result<()> {
    let entries = std::fs::read_dir(backup).map_err(|source| TskError::Read {
        path: backup.to_path_buf(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| TskError::Read {
            path: backup.to_path_buf(),
            source,
        })?;
        if entry.file_name() == skip_name {
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
    Ok(())
}

fn jj_workspace_registered_at_source(source_root: &Path, workspace_name: &str) -> Result<bool> {
    Ok(jj_list_workspaces(source_root)?
        .iter()
        .any(|(name, _)| name == workspace_name))
}

fn jj_list_workspaces(source_root: &Path) -> Result<Vec<(String, Option<PathBuf>)>> {
    let out = jj_inspect(source_root)?
        .args(["workspace", "list", "-T", JJ_WORKSPACE_LIST_TEMPLATE])
        .output()
        .map_err(|e| TskError::Other(format!("failed to run jj workspace list: {e}")))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(TskError::Other(format!(
            "jj workspace list failed: {}",
            stderr.trim()
        )));
    }
    Ok(parse_jj_workspace_list(&String::from_utf8_lossy(
        &out.stdout,
    )))
}

fn parse_jj_workspace_list(stdout: &str) -> Vec<(String, Option<PathBuf>)> {
    stdout
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            let (name, rest) = match line.split_once('\t') {
                Some(pair) => pair,
                None => (line, ""),
            };
            let name = name.trim();
            if name.is_empty() {
                return None;
            }
            let root = rest.trim();
            let path = if root.is_empty() {
                None
            } else {
                Some(expand(Path::new(root)))
            };
            Some((name.to_string(), path))
        })
        .collect()
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
    let out = jj_inspect(checkout)?
        .args(["log", "-r", revset, "-T", template, "--no-graph"])
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
    let out = jj_inspect(source_root)?
        .args(["workspace", "forget", workspace_name])
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
    let out = jj_inspect(checkout)
        .ok()?
        .args(["workspace", "root"])
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
    let canonical = std::fs::canonicalize(checkout).unwrap_or_else(|_| expand(checkout));
    for (name, root) in jj_list_workspaces(checkout)? {
        let Some(ws_path) = root else {
            continue;
        };
        let ws_canonical = std::fs::canonicalize(&ws_path).unwrap_or(ws_path);
        if ws_canonical == canonical {
            return Ok(name);
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

/// Clone URL for `origin` (or the first remote if `origin` is missing).
pub fn read_origin_remote_url(repo: &Path) -> Option<String> {
    let repo = expand(repo);
    match vcs_kind_at(&repo)? {
        VcsKind::Git => git_origin_url(&repo),
        VcsKind::Jj => jj_origin_url(&repo),
    }
}

fn git_origin_url(repo: &Path) -> Option<String> {
    let path = repo.to_str()?;
    if let Some(url) = git_remote_get_url(path, "origin") {
        return Some(url);
    }
    let list = Command::new("git")
        .args(["-C", path, "remote"])
        .output()
        .ok()?;
    if !list.status.success() {
        return None;
    }
    let first = String::from_utf8_lossy(&list.stdout)
        .lines()
        .map(str::trim)
        .find(|name| !name.is_empty())?
        .to_string();
    git_remote_get_url(path, &first)
}

fn git_remote_get_url(repo: &str, name: &str) -> Option<String> {
    let out = Command::new("git")
        .args(["-C", repo, "remote", "get-url", name])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let url = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if url.is_empty() {
        None
    } else {
        Some(url)
    }
}

fn jj_origin_url(repo: &Path) -> Option<String> {
    let mut cmd = jj_inspect(repo).ok()?;
    cmd.args(["git", "remote", "list"]);
    let out = cmd.output().ok()?;
    if !out.status.success() {
        return None;
    }
    parse_jj_remote_list(&String::from_utf8_lossy(&out.stdout))
}

fn parse_jj_remote_list(stdout: &str) -> Option<String> {
    let mut first = None;
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((name, url)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        let url = url.trim();
        if url.is_empty() {
            continue;
        }
        if name == "origin" {
            return Some(url.to_string());
        }
        if first.is_none() {
            first = Some(url.to_string());
        }
    }
    first
}

/// Turn a git/jj clone URL into an https browse page, or `None` if it is not web-reachable.
pub fn remote_to_browse_url(remote: &str) -> Option<String> {
    let remote = remote.trim();
    if remote.is_empty() {
        return None;
    }
    if let Some((user_host, path)) = scp_style_remote(remote) {
        let host = user_host.rsplit('@').next()?;
        return https_browse(host, path);
    }
    if let Some(rest) = remote.strip_prefix("ssh://") {
        return ssh_style_browse(rest);
    }
    if let Some(rest) = remote.strip_prefix("git://") {
        let (host, path) = rest.split_once('/')?;
        return https_browse(host, path);
    }
    if remote.starts_with("http://") || remote.starts_with("https://") {
        return Some(strip_git_suffix(remote));
    }
    None
}

fn scp_style_remote(remote: &str) -> Option<(&str, &str)> {
    if remote.contains("://") {
        return None;
    }
    let (user_host, path) = remote.split_once(':')?;
    if user_host.contains('/') || path.is_empty() {
        return None;
    }
    Some((user_host, path))
}

fn ssh_style_browse(rest: &str) -> Option<String> {
    let rest = rest
        .split_once('@')
        .map(|(_, hostpath)| hostpath)
        .unwrap_or(rest);
    let (hostport, path) = rest.split_once('/')?;
    let host = hostport.split(':').next()?;
    https_browse(host, path)
}

fn https_browse(host: &str, path: &str) -> Option<String> {
    let host = host.trim();
    let path = path.trim().trim_start_matches('/');
    if host.is_empty() || path.is_empty() {
        return None;
    }
    Some(format!("https://{host}/{}", strip_git_suffix(path)))
}

fn strip_git_suffix(url: &str) -> String {
    url.trim()
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .to_string()
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
    fn remote_to_browse_url_converts_common_clone_urls() {
        assert_eq!(
            remote_to_browse_url("git@github.com:jsimonrichard/hypr-taskspace.git"),
            Some("https://github.com/jsimonrichard/hypr-taskspace".into())
        );
        assert_eq!(
            remote_to_browse_url("https://github.com/org/app.git"),
            Some("https://github.com/org/app".into())
        );
        assert_eq!(
            remote_to_browse_url("ssh://git@gitlab.com/group/sub/repo.git"),
            Some("https://gitlab.com/group/sub/repo".into())
        );
        assert_eq!(
            remote_to_browse_url("git://codeberg.org/foo/bar.git"),
            Some("https://codeberg.org/foo/bar".into())
        );
        assert_eq!(remote_to_browse_url("file:///tmp/repo.git"), None);
        assert_eq!(remote_to_browse_url(""), None);
    }

    #[test]
    fn parse_jj_remote_list_prefers_origin() {
        let stdout = "upstream git@example.com:other/repo.git\norigin git@github.com:org/app.git\n";
        assert_eq!(
            parse_jj_remote_list(stdout).as_deref(),
            Some("git@github.com:org/app.git")
        );
    }

    #[test]
    fn read_origin_remote_url_from_git() {
        let dir = tempdir().unwrap();
        let repo = dir.path().join("app");
        init_scratch_repo(&repo).unwrap();
        run_checked(
            Command::new("git").args([
                "-C",
                repo.to_str().unwrap(),
                "remote",
                "add",
                "origin",
                "git@github.com:org/app.git",
            ]),
            "git remote add",
        )
        .unwrap();
        assert_eq!(
            read_origin_remote_url(&repo).as_deref(),
            Some("git@github.com:org/app.git")
        );
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
        create_linked_checkout(&source, &dest, "t1", VcsKind::Git, None).unwrap();
        assert!(dest.is_dir());
        assert!(is_git_worktree(&dest));
        remove_linked_checkout(&dest, Some(&source), Some("t1")).unwrap();
        assert!(!dest.exists());
    }

    fn git_commit(repo: &Path, message: &str) -> String {
        let repo_str = repo.to_str().unwrap();
        for args in [
            &["config", "user.email", "tsk@test"][..],
            &["config", "user.name", "tsk"][..],
        ] {
            let mut cmd = Command::new("git");
            cmd.arg("-C").arg(repo_str);
            cmd.args(args);
            run_checked(&mut cmd, "git config").unwrap();
        }
        run_checked(
            Command::new("git").args(["-C", repo_str, "add", "-A"]),
            "git add",
        )
        .unwrap();
        run_checked(
            Command::new("git").args(["-C", repo_str, "commit", "-m", message, "--allow-empty"]),
            "git commit",
        )
        .unwrap();
        git_rev_parse(repo, "HEAD").unwrap()
    }

    #[test]
    fn git_worktree_from_explicit_revision() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("main");
        init_scratch_repo(&source).unwrap();
        let first = git_commit(&source, "first");
        fs::write(source.join("later.txt"), "on main").unwrap();
        let later = git_commit(&source, "later");
        assert_ne!(first, later);

        let dest = dir
            .path()
            .join("tasks")
            .join("told")
            .join("workspace")
            .join("main");
        create_linked_checkout(&source, &dest, "told", VcsKind::Git, Some(&first)).unwrap();
        assert_eq!(git_rev_parse(&dest, "HEAD").unwrap(), first);
        assert!(!dest.join("later.txt").exists());
    }

    #[test]
    fn git_unknown_revision_errors() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("main");
        init_scratch_repo(&source).unwrap();
        git_commit(&source, "init");
        let err = resolve_revision_id(&source, VcsKind::Git, "definitely-missing").unwrap_err();
        assert!(matches!(err, TskError::UnknownRevision { .. }));
    }

    fn jj_available() -> bool {
        Command::new("jj")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn init_jj_repo(source: &Path) {
        fs::create_dir_all(source).unwrap();
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
    }

    #[test]
    fn jj_workspace_from_explicit_revision_not_trunk() {
        if !jj_available() {
            eprintln!("skipping jj_workspace_from_explicit_revision_not_trunk: jj not available");
            return;
        }

        let dir = tempdir().unwrap();
        let source = dir.path().join("main");
        init_jj_repo(&source);
        fs::write(source.join("main.txt"), "on main").unwrap();
        run_checked(
            Command::new("jj").args(["-R", source.to_str().unwrap(), "describe", "-m", "main tip"]),
            "jj describe main",
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
        run_checked(
            Command::new("jj").args(["-R", source.to_str().unwrap(), "new"]),
            "jj new feature",
        )
        .unwrap();
        fs::write(source.join("feature.txt"), "feature work").unwrap();
        run_checked(
            Command::new("jj").args([
                "-R",
                source.to_str().unwrap(),
                "describe",
                "-m",
                "feature tip",
            ]),
            "jj describe feature",
        )
        .unwrap();
        let feature_commit = jj_template_live(&source, "@", "commit_id").unwrap();
        assert_ne!(main_commit, feature_commit);

        let dest = dir
            .path()
            .join("tasks")
            .join("tfrom")
            .join("workspace")
            .join("main");
        create_linked_checkout(&source, &dest, "tfrom", VcsKind::Jj, Some(&feature_commit))
            .unwrap();
        let parent = jj_template(&dest, "@-", "commit_id").unwrap();
        assert_eq!(parent, feature_commit);
        assert_ne!(parent, main_commit);
    }

    #[test]
    fn jj_from_current_uses_workspace_working_copy() {
        if !jj_available() {
            eprintln!("skipping jj_from_current_uses_workspace_working_copy: jj not available");
            return;
        }

        let dir = tempdir().unwrap();
        let source = dir.path().join("main");
        init_jj_repo(&source);
        fs::write(source.join("main.txt"), "on main").unwrap();
        run_checked(
            Command::new("jj").args(["-R", source.to_str().unwrap(), "describe", "-m", "main tip"]),
            "jj describe main",
        )
        .unwrap();
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

        let mid = dir
            .path()
            .join("tasks")
            .join("tmid")
            .join("workspace")
            .join("main");
        create_linked_checkout(&source, &mid, "tmid", VcsKind::Jj, None).unwrap();
        fs::write(mid.join("side.txt"), "side work").unwrap();
        run_checked(
            Command::new("jj").args(["-R", mid.to_str().unwrap(), "describe", "-m", "side tip"]),
            "jj describe side",
        )
        .unwrap();
        let side = current_checkout_revision(&mid, VcsKind::Jj).unwrap();

        let dest = dir
            .path()
            .join("tasks")
            .join("tcur")
            .join("workspace")
            .join("main");
        let resolved = crate::task_repo::ForkFrom::Current {
            fallback_task_id: None,
        }
        .resolve_revision(&source, VcsKind::Jj, Some(&mid), None)
        .unwrap()
        .expect("current revision");
        assert_eq!(resolved, side);

        create_linked_checkout(&source, &dest, "tcur", VcsKind::Jj, Some(&resolved)).unwrap();
        assert_eq!(jj_template(&dest, "@-", "commit_id").unwrap(), side);
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
        create_linked_checkout(&source, &dest, "tabc123", VcsKind::Git, None).unwrap();
        fs::write(dest.join("local.txt"), "local only").unwrap();

        detach_linked_checkout(&dest, Some(&source), Some("tabc123")).unwrap();
        assert!(!is_git_worktree(&dest));
        assert!(dest.join("local.txt").is_file());
        assert!(!git_worktree_listed_at_source(&source, &dest).unwrap());

        reattach_linked_checkout(&dest, Some(&source), Some("tabc123")).unwrap();
        assert!(is_git_worktree(&dest));
        assert!(git_worktree_listed_at_source(&source, &dest).unwrap());
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
        if !Command::new("jj")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
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
        create_linked_checkout(&source, &dest, "tjj123", VcsKind::Jj, None).unwrap();

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
        assert!(!jj_workspace_registered_at_source(&source, "tjj123").unwrap());

        reattach_linked_checkout(&dest, Some(&source), Some("tjj123")).unwrap();
        assert!(jj_workspace_registered_at_source(&source, "tjj123").unwrap());
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
        if !Command::new("jj")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
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
        create_linked_checkout(&source, &dest, "tjjempty", VcsKind::Jj, None).unwrap();

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
        assert!(!jj_workspace_registered_at_source(&source, "tjjempty").unwrap());

        reattach_linked_checkout(&dest, Some(&source), Some("tjjempty")).unwrap();
        assert!(jj_workspace_registered_at_source(&source, "tjjempty").unwrap());

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

    #[test]
    fn parse_jj_workspace_list_reads_name_and_root() {
        let parsed = parse_jj_workspace_list(
            "default\t/tmp/main\nt6bf28161\t/tmp/tasks/t6bf28161/workspace/app\n",
        );
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].0, "default");
        assert_eq!(
            parsed[1],
            (
                "t6bf28161".into(),
                Some(PathBuf::from("/tmp/tasks/t6bf28161/workspace/app"))
            )
        );
    }

    #[test]
    fn linked_checkout_kind_prefers_jj_over_git_worktree_file() {
        let dir = tempdir().unwrap();
        let checkout = dir.path().join("ws");
        fs::create_dir_all(checkout.join(".jj")).unwrap();
        fs::write(
            checkout.join(".git"),
            "gitdir: /tmp/main/.git/worktrees/ws\n",
        )
        .unwrap();
        assert_eq!(linked_checkout_kind(&checkout), Some(VcsKind::Jj));
    }

    fn seed_jj_main_bookmark(source: &Path) {
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
    }

    #[test]
    fn reattach_jj_source_ignores_leftover_git_worktree_file() {
        if !jj_available() {
            eprintln!(
                "skipping reattach_jj_source_ignores_leftover_git_worktree_file: jj not available"
            );
            return;
        }

        let dir = tempdir().unwrap();
        let source = dir.path().join("main");
        init_jj_repo(&source);
        seed_jj_main_bookmark(&source);

        let dest = dir
            .path()
            .join("tasks")
            .join("tjjgit")
            .join("workspace")
            .join("main");
        create_linked_checkout(&source, &dest, "tjjgit", VcsKind::Jj, None).unwrap();
        fs::write(dest.join("local.txt"), "jj not git").unwrap();

        detach_linked_checkout(&dest, Some(&source), Some("tjjgit")).unwrap();
        let jj_dir = dest.join(".jj");
        if jj_dir.exists() {
            fs::remove_dir_all(&jj_dir).unwrap();
        }
        fs::write(
            dest.join(".git"),
            format!("gitdir: {}/.git/worktrees/tjjgit\n", source.display()),
        )
        .unwrap();
        assert_eq!(linked_checkout_kind(&dest), Some(VcsKind::Git));
        assert_eq!(vcs_kind_at(&source), Some(VcsKind::Jj));

        reattach_linked_checkout(&dest, Some(&source), Some("tjjgit")).unwrap();
        assert!(
            dest.join(".jj").is_dir(),
            "jj source must relink as a workspace, not git worktree add"
        );
        assert!(jj_workspace_registered_at_source(&source, "tjjgit").unwrap());
        assert_eq!(
            fs::read_to_string(dest.join("local.txt")).unwrap(),
            "jj not git"
        );
    }

    #[test]
    fn reattach_jj_recovers_missing_checkout_from_relink_backup() {
        if !jj_available() {
            eprintln!(
                "skipping reattach_jj_recovers_missing_checkout_from_relink_backup: jj not available"
            );
            return;
        }

        let dir = tempdir().unwrap();
        let source = dir.path().join("main");
        init_jj_repo(&source);
        seed_jj_main_bookmark(&source);

        let dest = dir
            .path()
            .join("tasks")
            .join("tjjmiss")
            .join("workspace")
            .join("main");
        create_linked_checkout(&source, &dest, "tjjmiss", VcsKind::Jj, None).unwrap();
        fs::write(dest.join("local.txt"), "from backup").unwrap();
        detach_linked_checkout(&dest, Some(&source), Some("tjjmiss")).unwrap();

        let backup = dest.parent().unwrap().join(".tjjmiss-relink-tmp");
        fs::rename(&dest, &backup).unwrap();
        assert!(!dest.exists());

        reattach_linked_checkout(&dest, Some(&source), Some("tjjmiss")).unwrap();
        assert!(dest.join(".jj").is_dir());
        assert!(jj_workspace_registered_at_source(&source, "tjjmiss").unwrap());
        assert_eq!(
            fs::read_to_string(dest.join("local.txt")).unwrap(),
            "from backup"
        );
        assert!(!backup.exists());
    }

    #[test]
    fn reattach_jj_forgets_ghost_workspace_then_relinks() {
        if !jj_available() {
            eprintln!(
                "skipping reattach_jj_forgets_ghost_workspace_then_relinks: jj not available"
            );
            return;
        }

        let dir = tempdir().unwrap();
        let source = dir.path().join("main");
        init_jj_repo(&source);
        seed_jj_main_bookmark(&source);

        let dest = dir
            .path()
            .join("tasks")
            .join("tjjghost")
            .join("workspace")
            .join("main");
        create_linked_checkout(&source, &dest, "tjjghost", VcsKind::Jj, None).unwrap();
        fs::write(dest.join("local.txt"), "ghost dest").unwrap();

        let backup = dest.parent().unwrap().join(".tjjghost-relink-tmp");
        fs::rename(&dest, &backup).unwrap();
        assert!(jj_workspace_registered_at_source(&source, "tjjghost").unwrap());
        assert!(!dest.exists());

        reattach_linked_checkout(&dest, Some(&source), Some("tjjghost")).unwrap();
        assert!(dest.join(".jj").is_dir());
        assert!(jj_workspace_registered_at_source(&source, "tjjghost").unwrap());
        assert_eq!(
            fs::read_to_string(dest.join("local.txt")).unwrap(),
            "ghost dest"
        );
        assert!(!backup.exists());
    }

    #[test]
    fn recover_relink_backups_restores_interrupted_jj_relink() {
        let dir = tempdir().unwrap();
        let workspace = dir.path().join("workspace");
        let checkout = workspace.join("app");
        let backup = workspace.join(".tid-relink-tmp");
        fs::create_dir_all(&checkout).unwrap();
        fs::create_dir_all(backup.join(".jj")).unwrap();
        fs::write(backup.join("local.txt"), "keep me").unwrap();

        recover_relink_backups(&checkout, "tid").unwrap();

        assert_eq!(
            fs::read_to_string(checkout.join("local.txt")).unwrap(),
            "keep me"
        );
        assert!(checkout.join(".jj").is_dir());
        assert!(!backup.exists());
    }

    #[test]
    fn recover_relink_backups_leaves_nonempty_backup_when_checkout_has_files() {
        let dir = tempdir().unwrap();
        let workspace = dir.path().join("workspace");
        let checkout = workspace.join("app");
        let backup = workspace.join(".tid-relink-tmp");
        fs::create_dir_all(&checkout).unwrap();
        fs::create_dir_all(&backup).unwrap();
        fs::write(checkout.join("live.txt"), "live").unwrap();
        fs::write(backup.join("saved.txt"), "saved").unwrap();

        recover_relink_backups(&checkout, "tid").unwrap();

        assert!(backup.join("saved.txt").is_file());
        assert!(checkout.join("live.txt").is_file());
        assert!(!checkout.join("saved.txt").exists());
    }

    #[test]
    fn relink_forgotten_jj_workspace_rolls_back_if_name_still_registered() {
        if !jj_available() {
            eprintln!(
                "skipping relink_forgotten_jj_workspace_rolls_back_if_name_still_registered: jj not available"
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
            .join("tjjlive")
            .join("workspace")
            .join("main");
        create_linked_checkout(&source, &dest, "tjjlive", VcsKind::Jj, None).unwrap();
        fs::write(dest.join("local.txt"), "must survive false-negative relink").unwrap();

        let err = relink_forgotten_jj_workspace(&source, &dest, "tjjlive").unwrap_err();
        assert!(
            err.to_string().contains("already exists"),
            "expected duplicate workspace name error, got: {err}"
        );
        assert_eq!(
            fs::read_to_string(dest.join("local.txt")).unwrap(),
            "must survive false-negative relink"
        );
        assert!(dest.join(".jj").is_dir());
        assert!(jj_workspace_registered_at_source(&source, "tjjlive").unwrap());
        assert!(!dir
            .path()
            .join("tasks")
            .join("tjjlive")
            .join("workspace")
            .join(".tjjlive-relink-tmp")
            .exists());
    }
}
