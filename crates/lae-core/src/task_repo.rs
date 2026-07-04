use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::error::{LaeError, Result};
use crate::repos::normalize_repo_path;
use crate::vcs::detect_vcs_root;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskRepoSource {
    /// Use the git/jj root from `cwd`, or a scratch repo if none is found.
    Auto,
    /// Always create an isolated repo directory under the task home.
    Scratch,
    /// Use an explicit checkout path (typically a detected VCS root).
    Path(PathBuf),
}

impl TaskRepoSource {
    pub fn resolve(
        &self,
        task_home: &Path,
        cwd: Option<&Path>,
    ) -> Result<(PathBuf, bool)> {
        let (path, create_dir) = match self {
            Self::Scratch => (task_home.join("repo"), true),
            Self::Auto => {
                if let Some(root) = detect_vcs_root(cwd) {
                    (root, false)
                } else {
                    (task_home.join("repo"), true)
                }
            }
            Self::Path(path) => {
                let path = normalize_repo_path(path);
                let root = detect_vcs_root(Some(&path)).unwrap_or(path);
                if !root.is_dir() {
                    return Err(LaeError::Other(format!(
                        "Repo path does not exist: {}",
                        root.display()
                    )));
                }
                (root, false)
            }
        };
        Ok((path, create_dir))
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
                    .ok_or_else(|| LaeError::Other("repo_path required".into()))?;
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
                    .and_then(|repo| repo.url)
            }
            _ => None,
        }
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
}
