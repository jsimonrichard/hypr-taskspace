//! Install agent skill packs into a shared data dir, then soft-link Cursor/Claude paths.
//!
//! Layout:
//! ```text
//! $TSK_SHARE_DIR/pack/     # default: ~/.local/share/tsk/pack
//!   → symlink to checkout pack/ or /usr/share/tsk/pack
//! ~/.cursor/skills/<name>  → $share/pack/skills/<name>
//! ~/.claude/skills/<name>  → $share/pack/skills/<name>
//! ```

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{Result, TskError};
use crate::share::SYSTEM_SHARE_DIR;
use crate::xdg::{data_home, expand, tsk_data_dir};

#[derive(Debug, Clone, Default)]
pub struct AgentsInstallOpts {
    pub global: bool,
    /// Also link skills into `<repo>/.agents/skills` (+ vendor roots).
    pub repo_path: Option<PathBuf>,
    pub force: bool,
    pub pack_dir: Option<PathBuf>,
    pub share_dir: Option<PathBuf>,
}

/// Resolve the source pack directory (checkout `pack/` or packaged share).
pub fn pack_dir() -> PathBuf {
    if let Ok(p) = std::env::var("TSK_PACK_DIR") {
        return expand(p);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(root) = exe
            .ancestors()
            .find(|a| a.join("pack/skills").is_dir() && a.join("Cargo.toml").is_file())
        {
            return root.join("pack");
        }
    }
    let packaged = PathBuf::from(SYSTEM_SHARE_DIR).join("pack");
    if packaged.join("skills").is_dir() {
        return packaged;
    }
    let under_data = tsk_data_dir().join("pack");
    if under_data.join("skills").is_dir() {
        return under_data;
    }
    // Dev: crates/tsk-core → workspace root
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("pack")
}

/// Shared data root that Cursor/Claude skill links point through.
pub fn agents_share_dir() -> PathBuf {
    if let Ok(p) = std::env::var("TSK_SHARE_DIR") {
        return expand(p);
    }
    data_home().join("tsk")
}

pub fn install_agents(opts: &AgentsInstallOpts) -> Result<Vec<String>> {
    if !opts.global && opts.repo_path.is_none() {
        return Err(TskError::Other(
            "specify --global and/or --repo-path <abs>".into(),
        ));
    }
    let source_pack = opts.pack_dir.clone().unwrap_or_else(pack_dir);
    if !source_pack.join("skills").is_dir() {
        return Err(TskError::Other(format!(
            "agent pack not found at {} (expected skills/ under pack)",
            source_pack.display()
        )));
    }
    let share = opts.share_dir.clone().unwrap_or_else(agents_share_dir);
    let mut log = Vec::new();
    let shared_pack = ensure_shared_pack(&source_pack, &share, opts.force, &mut log)?;

    if opts.global {
        log.extend(install_global(&shared_pack, opts.force)?);
    }
    if let Some(repo) = &opts.repo_path {
        log.extend(install_repo(&shared_pack, repo, opts.force)?);
    }
    Ok(log)
}

fn ensure_shared_pack(
    source_pack: &Path,
    share: &Path,
    force: bool,
    log: &mut Vec<String>,
) -> Result<PathBuf> {
    let source = fs::canonicalize(source_pack).map_err(|source| TskError::Read {
        path: source_pack.to_path_buf(),
        source,
    })?;
    fs::create_dir_all(share).map_err(|source| TskError::Write {
        path: share.to_path_buf(),
        source,
    })?;
    let shared_pack = share.join("pack");

    if path_exists(&shared_pack) {
        let meta = fs::symlink_metadata(&shared_pack).map_err(|source| TskError::Read {
            path: shared_pack.clone(),
            source,
        })?;
        if meta.file_type().is_symlink() {
            let current = fs::read_link(&shared_pack).map_err(|source| TskError::Read {
                path: shared_pack.clone(),
                source,
            })?;
            let current_canon =
                fs::canonicalize(share.join(&current)).unwrap_or_else(|_| current.clone());
            if current_canon == source {
                log.push(format!(
                    "share pack ok: {} → {}",
                    shared_pack.display(),
                    source.display()
                ));
                return Ok(shared_pack);
            }
            if !force {
                return Err(TskError::Other(format!(
                    "refusing to replace {} (points elsewhere); pass --force",
                    shared_pack.display()
                )));
            }
            fs::remove_file(&shared_pack).map_err(|source| TskError::Write {
                path: shared_pack.clone(),
                source,
            })?;
        } else if meta.is_dir() {
            if !force {
                return Err(TskError::Other(format!(
                    "refusing to replace directory {} with a symlink (pass --force)",
                    shared_pack.display()
                )));
            }
            fs::remove_dir_all(&shared_pack).map_err(|source| TskError::Write {
                path: shared_pack.clone(),
                source,
            })?;
        } else if force {
            fs::remove_file(&shared_pack).map_err(|source| TskError::Write {
                path: shared_pack.clone(),
                source,
            })?;
        } else {
            return Err(TskError::Other(format!(
                "refusing to overwrite {}",
                shared_pack.display()
            )));
        }
    }

    symlink_path(&source, &shared_pack)?;
    log.push(format!(
        "symlink {} → {}",
        shared_pack.display(),
        source.display()
    ));
    Ok(shared_pack)
}

fn install_global(shared_pack: &Path, force: bool) -> Result<Vec<String>> {
    let mut log = Vec::new();
    let home = expand("~");
    let skills_root = shared_pack.join("skills");
    if !skills_root.is_dir() {
        return Err(TskError::Other(format!(
            "missing skills dir under {}",
            shared_pack.display()
        )));
    }
    for entry in fs::read_dir(&skills_root).map_err(|source| TskError::Read {
        path: skills_root.clone(),
        source,
    })? {
        let entry = entry.map_err(|source| TskError::Read {
            path: skills_root.clone(),
            source,
        })?;
        if !entry
            .file_type()
            .map_err(|source| TskError::Read {
                path: entry.path(),
                source,
            })?
            .is_dir()
        {
            continue;
        }
        let name = entry.file_name();
        let src = entry.path();
        for base in [home.join(".cursor/skills"), home.join(".claude/skills")] {
            if let Some(msg) = link_dir(&src, &base.join(&name), force)? {
                log.push(msg);
            }
        }
    }
    Ok(log)
}

fn install_repo(shared_pack: &Path, repo: &Path, force: bool) -> Result<Vec<String>> {
    if !repo.is_dir() {
        return Err(TskError::Other(format!(
            "repo path not found: {}",
            repo.display()
        )));
    }
    let mut log = Vec::new();
    let skills_src = shared_pack.join("skills");
    let dest_root = repo.join(".agents/skills");
    fs::create_dir_all(&dest_root).map_err(|source| TskError::Write {
        path: dest_root.clone(),
        source,
    })?;
    for entry in fs::read_dir(&skills_src).map_err(|source| TskError::Read {
        path: skills_src.clone(),
        source,
    })? {
        let entry = entry.map_err(|source| TskError::Read {
            path: skills_src.clone(),
            source,
        })?;
        if !entry
            .file_type()
            .map_err(|source| TskError::Read {
                path: entry.path(),
                source,
            })?
            .is_dir()
        {
            continue;
        }
        let name = entry.file_name();
        if let Some(msg) = link_dir(&entry.path(), &dest_root.join(&name), force)? {
            log.push(msg);
        }
    }
    link_vendor_skill_roots(repo, &mut log)?;
    Ok(log)
}

fn link_vendor_skill_roots(repo: &Path, log: &mut Vec<String>) -> Result<()> {
    for rel in [".claude/skills", ".cursor/skills"] {
        let link = repo.join(rel);
        if let Some(parent) = link.parent() {
            fs::create_dir_all(parent).map_err(|source| TskError::Write {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        if path_exists(&link) {
            if is_symlink_to(&link, Path::new("../.agents/skills"))? {
                log.push(format!("symlink ok: {}", link.display()));
                continue;
            }
            log.push(format!("skip (exists): {}", link.display()));
            continue;
        }
        let target = PathBuf::from("../.agents/skills");
        symlink_path(&target, &link)?;
        log.push(format!("symlink {} → {}", link.display(), target.display()));
    }
    Ok(())
}

fn link_dir(target: &Path, link: &Path, force: bool) -> Result<Option<String>> {
    let abs_target = make_absolute(target)?;
    if !abs_target.is_dir() {
        return Err(TskError::Other(format!(
            "skill dir not found: {}",
            abs_target.display()
        )));
    }
    if path_exists(link) && !force {
        if is_same_link(link, &abs_target)? {
            return Ok(Some(format!("symlink ok: {}", link.display())));
        }
        return Ok(Some(format!(
            "skip (exists): {} — use --force to overwrite",
            link.display()
        )));
    }
    prepare_link_dest(link, force)?;
    if let Some(parent) = link.parent() {
        fs::create_dir_all(parent).map_err(|source| TskError::Write {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    symlink_path(&abs_target, link)?;
    Ok(Some(format!(
        "symlink {} → {}",
        link.display(),
        abs_target.display()
    )))
}

fn make_absolute(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()
            .map_err(|source| TskError::Other(format!("cwd: {source}")))?
            .join(path))
    }
}

fn is_same_link(link: &Path, abs_target: &Path) -> Result<bool> {
    let meta = match fs::symlink_metadata(link) {
        Ok(m) => m,
        Err(_) => return Ok(false),
    };
    if !meta.file_type().is_symlink() {
        return Ok(false);
    }
    let current = fs::read_link(link).map_err(|source| TskError::Read {
        path: link.to_path_buf(),
        source,
    })?;
    let resolved = if current.is_absolute() {
        current
    } else {
        link.parent()
            .unwrap_or_else(|| Path::new("."))
            .join(current)
    };
    if resolved == *abs_target {
        return Ok(true);
    }
    match (fs::canonicalize(&resolved), fs::canonicalize(abs_target)) {
        (Ok(a), Ok(b)) => Ok(a == b),
        _ => Ok(false),
    }
}

fn prepare_link_dest(link: &Path, force: bool) -> Result<()> {
    if !path_exists(link) {
        return Ok(());
    }
    if !force {
        return Err(TskError::Other(format!(
            "refusing to overwrite {} (pass --force)",
            link.display()
        )));
    }
    let meta = fs::symlink_metadata(link).map_err(|source| TskError::Read {
        path: link.to_path_buf(),
        source,
    })?;
    if meta.file_type().is_symlink() || meta.is_file() {
        fs::remove_file(link).map_err(|source| TskError::Write {
            path: link.to_path_buf(),
            source,
        })?;
    } else if meta.is_dir() {
        fs::remove_dir_all(link).map_err(|source| TskError::Write {
            path: link.to_path_buf(),
            source,
        })?;
    } else {
        fs::remove_file(link).map_err(|source| TskError::Write {
            path: link.to_path_buf(),
            source,
        })?;
    }
    Ok(())
}

fn path_exists(path: &Path) -> bool {
    path.exists() || fs::symlink_metadata(path).is_ok()
}

fn is_symlink_to(link: &Path, want: &Path) -> Result<bool> {
    let meta = match fs::symlink_metadata(link) {
        Ok(m) => m,
        Err(_) => return Ok(false),
    };
    if !meta.file_type().is_symlink() {
        return Ok(false);
    }
    let current = fs::read_link(link).map_err(|source| TskError::Read {
        path: link.to_path_buf(),
        source,
    })?;
    Ok(current == want)
}

fn symlink_path(target: &Path, link: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link).map_err(|source| TskError::Write {
            path: link.to_path_buf(),
            source,
        })
    }
    #[cfg(not(unix))]
    {
        let _ = (target, link);
        Err(TskError::Other(
            "agent pack install requires unix symlinks".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn install_global_links_skills() {
        let dir = tempdir().unwrap();
        let pack = dir.path().join("pack");
        let skill = pack.join("skills/read-handoff");
        fs::create_dir_all(&skill).unwrap();
        fs::write(skill.join("SKILL.md"), "# test\n").unwrap();

        let share = dir.path().join("share");
        let home = dir.path().join("home");
        fs::create_dir_all(home.join(".cursor")).unwrap();
        // expand("~") uses HOME
        std::env::set_var("HOME", &home);

        let log = install_agents(&AgentsInstallOpts {
            global: true,
            repo_path: None,
            force: false,
            pack_dir: Some(pack.clone()),
            share_dir: Some(share.clone()),
        })
        .unwrap();
        assert!(log
            .iter()
            .any(|l| l.contains("share pack") || l.contains("symlink")));
        let linked = home.join(".cursor/skills/read-handoff");
        assert!(linked.symlink_metadata().unwrap().file_type().is_symlink());
        assert!(linked.join("SKILL.md").is_file());
        assert!(share
            .join("pack")
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink());
    }

    #[test]
    fn pack_dir_finds_workspace_pack() {
        let p = pack_dir();
        assert!(
            p.join("skills").is_dir() || p.ends_with("pack"),
            "unexpected pack_dir {}",
            p.display()
        );
    }
}
