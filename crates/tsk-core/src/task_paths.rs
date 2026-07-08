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
    repo_path.starts_with(&workspace_root)
        || paths_match(&repo_path, &workspace_root)
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
