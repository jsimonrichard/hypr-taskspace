//! Detect local version-control roots (git, Jujutsu).

use std::path::{Path, PathBuf};

use crate::xdg::expand;

/// Walk upward from `start` (or the process cwd when `None`) looking for a git or jj workspace.
pub fn detect_vcs_root(start: Option<&Path>) -> Option<PathBuf> {
    let start = start
        .map(expand)
        .or_else(|| std::env::current_dir().ok().map(|p| expand(&p)))
        .filter(|p| p.is_dir())?;

    let mut dir = start.as_path();
    loop {
        if dir.join(".jj").is_dir() || dir.join(".git").exists() {
            return Some(dir.to_path_buf());
        }
        dir = dir.parent()?;
    }
}

/// Short display name for a repo path (usually the directory name).
pub fn repo_label(path: &Path) -> String {
    expand(path)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
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
    }

    #[test]
    fn detect_none_outside_repo() {
        let dir = tempdir().unwrap();
        assert!(detect_vcs_root(Some(dir.path())).is_none());
    }
}
