//! Layout of task home directories.

use std::path::{Path, PathBuf};

use crate::repos::paths_match;
use crate::vcs::repo_label;
use crate::xdg::expand;

pub const SCRATCH_DIR_NAME: &str = "scratch";

/// `~/tsk-tasks/<task-id>/workspace/` — parent for all task checkouts.
pub fn task_workspace_dir(task_home: &Path) -> PathBuf {
    task_home.join("workspace")
}

/// Empty workspace directory for scratch tasks (`<task-home>/workspace/`).
pub fn scratch_checkout_path(task_home: &Path) -> PathBuf {
    task_workspace_dir(task_home)
}

pub fn ensure_scratch_workspace(dest: &Path) -> crate::error::Result<()> {
    std::fs::create_dir_all(dest).map_err(|source| crate::error::TskError::Write {
        path: dest.to_path_buf(),
        source,
    })
}

pub fn linked_checkout_path(task_home: &Path, source_root: &Path) -> PathBuf {
    task_workspace_dir(task_home).join(repo_label(source_root))
}

/// Sibling dest: `<task-home>/workspace/<repo-label>-<suffix>`.
pub fn sibling_checkout_path(task_home: &Path, source_root: &Path, suffix: &str) -> PathBuf {
    task_workspace_dir(task_home).join(format!("{}-{suffix}", repo_label(source_root)))
}

/// jj workspace / git worktree name: `<task-id>-<suffix>`.
pub fn sibling_workspace_name(task_id: &str, suffix: &str) -> String {
    format!("{task_id}-{suffix}")
}

/// Suffix for `tsk checkout add`: start with ASCII alphanumeric, then `[A-Za-z0-9_-]*`.
pub fn validate_checkout_suffix(suffix: &str) -> crate::error::Result<()> {
    let valid = suffix
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric())
        && suffix
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if valid {
        Ok(())
    } else {
        Err(crate::error::TskError::InvalidCheckoutSuffix {
            suffix: suffix.to_string(),
        })
    }
}

/// If `path` is under `<tasks_base>/<id>/workspace/…`, return that task id.
pub fn task_id_from_managed_path(path: &Path, tasks_base: &Path) -> Option<String> {
    let path = expand(path);
    let base = expand(tasks_base);
    let rel = path.strip_prefix(&base).ok()?;
    let mut comps = rel.components();
    let id = comps.next()?.as_os_str().to_str()?.to_string();
    let workspace = comps.next()?;
    if workspace.as_os_str() != "workspace" {
        return None;
    }
    Some(id)
}

/// jj/git name for a managed checkout under the task home.
///
/// Primary folder (`<repo-label>`) is `task_id`. A sibling
/// (`<repo-label>-<suffix>`) is `{task_id}-{suffix}`.
pub fn workspace_name_for_owned_checkout(
    task_id: &str,
    source_root: &Path,
    checkout: &Path,
) -> crate::error::Result<String> {
    let label = repo_label(source_root);
    let folder = checkout
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| crate::error::TskError::OwnedCheckoutNameMismatch {
            path: checkout.to_path_buf(),
            label: label.clone(),
        })?;
    if folder == label {
        return Ok(task_id.to_string());
    }
    let prefix = format!("{label}-");
    if let Some(suffix) = folder.strip_prefix(&prefix) {
        validate_checkout_suffix(suffix)?;
        return Ok(sibling_workspace_name(task_id, suffix));
    }
    Err(crate::error::TskError::OwnedCheckoutNameMismatch {
        path: checkout.to_path_buf(),
        label,
    })
}

/// True when `path` is a scratch task workspace (`<tasks_base>/<id>/workspace` or legacy `.../scratch`).
pub fn is_scratch_workspace_path(path: &Path, tasks_base_dir: &Path) -> bool {
    let path = expand(path);
    let tasks_base = expand(tasks_base_dir);
    let Ok(rel) = path.strip_prefix(&tasks_base) else {
        return false;
    };
    let components = rel.components().collect::<Vec<_>>();
    if components.len() < 2 {
        return false;
    }
    if components[1].as_os_str() != "workspace" {
        return false;
    }
    match components.len() {
        2 => true,
        3 if components[2].as_os_str() == SCRATCH_DIR_NAME => true,
        _ => false,
    }
}

/// True when `repo_path` lives under `<tasks_base>/<task_id>/workspace/`.
pub fn is_managed_task_checkout(repo_path: &Path, tasks_base: &Path, task_id: &str) -> bool {
    let workspace_root = expand(tasks_base.join(task_id).join("workspace"));
    let repo_path = expand(repo_path);
    repo_path.starts_with(&workspace_root) || paths_match(&repo_path, &workspace_root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_checkout_detects_workspace_paths() {
        let base = PathBuf::from("/tmp/tsk-tasks");
        let path = base.join("t1").join("workspace").join("my-app");
        assert!(is_managed_task_checkout(&path, &base, "t1"));
        assert!(!is_managed_task_checkout(
            &PathBuf::from("/home/user/my-app"),
            &base,
            "t1"
        ));
    }

    #[test]
    fn linked_checkout_uses_repo_folder_name() {
        let home = PathBuf::from("/tmp/tsk-tasks/t1");
        let source = PathBuf::from("/home/user/my-app");
        assert_eq!(
            linked_checkout_path(&home, &source),
            PathBuf::from("/tmp/tsk-tasks/t1/workspace/my-app")
        );
    }

    #[test]
    fn sibling_checkout_appends_suffix_to_repo_and_task_id() {
        let home = PathBuf::from("/tmp/tsk-tasks/t74c8e14d");
        let source = PathBuf::from("/home/user/hypr-taskspace");
        assert_eq!(
            sibling_checkout_path(&home, &source, "review"),
            PathBuf::from("/tmp/tsk-tasks/t74c8e14d/workspace/hypr-taskspace-review")
        );
        assert_eq!(
            sibling_workspace_name("t74c8e14d", "review"),
            "t74c8e14d-review"
        );
        assert_eq!(
            workspace_name_for_owned_checkout(
                "t74c8e14d",
                &source,
                &PathBuf::from("/tmp/tsk-tasks/t74c8e14d/workspace/hypr-taskspace")
            )
            .unwrap(),
            "t74c8e14d"
        );
        assert_eq!(
            workspace_name_for_owned_checkout(
                "t74c8e14d",
                &source,
                &PathBuf::from("/tmp/tsk-tasks/t74c8e14d/workspace/hypr-taskspace-review")
            )
            .unwrap(),
            "t74c8e14d-review"
        );
    }

    #[test]
    fn checkout_suffix_rejects_empty_and_path_chars() {
        assert!(validate_checkout_suffix("review").is_ok());
        assert!(validate_checkout_suffix("r").is_ok());
        assert!(validate_checkout_suffix("pr_2").is_ok());
        assert!(validate_checkout_suffix("").is_err());
        assert!(validate_checkout_suffix("-review").is_err());
        assert!(validate_checkout_suffix("re/view").is_err());
        assert!(validate_checkout_suffix("t74c8e14d@").is_err());
    }

    #[test]
    fn task_id_from_managed_path_reads_workspace_child() {
        let base = PathBuf::from("/tmp/tsk-tasks");
        assert_eq!(
            task_id_from_managed_path(
                &base
                    .join("t74c8e14d")
                    .join("workspace")
                    .join("hypr-taskspace"),
                &base
            )
            .as_deref(),
            Some("t74c8e14d")
        );
        assert_eq!(
            task_id_from_managed_path(&base.join("t74c8e14d").join(".tsk"), &base),
            None
        );
        assert_eq!(
            task_id_from_managed_path(&PathBuf::from("/home/user/hypr-taskspace"), &base),
            None
        );
    }

    #[test]
    fn scratch_workspace_path_detection() {
        let base = PathBuf::from("/tmp/tsk-tasks");
        assert!(is_scratch_workspace_path(
            &base.join("t1").join("workspace"),
            &base
        ));
        assert!(is_scratch_workspace_path(
            &base.join("t1").join("workspace").join("scratch"),
            &base
        ));
        assert!(!is_scratch_workspace_path(
            &base.join("t1").join("workspace").join("my-app"),
            &base
        ));
        assert!(!is_scratch_workspace_path(
            &PathBuf::from("/home/user/my-app"),
            &base
        ));
    }
}
