//! Task HANDOFF.md — structured agent/human contract at the workspace root.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{Result, TskError};
use crate::task_paths::task_workspace_dir;

/// Required `##` headings. Goal and Success criteria must also be non-empty.
pub const REQUIRED_HEADINGS: &[&str] = &[
    "Goal",
    "Scope",
    "Out of scope",
    "Success criteria",
    "Constraints",
];

/// Optional headings — may be omitted entirely.
pub const OPTIONAL_HEADINGS: &[&str] = &["Principles", "Handoff notes"];

const HANDOFF_FILE_NAME: &str = "HANDOFF.md";

/// Canonical path: `<task-home>/workspace/HANDOFF.md`.
pub fn handoff_path(task_home: &Path) -> PathBuf {
    task_workspace_dir(task_home).join(HANDOFF_FILE_NAME)
}

/// Metadata stamped at instruct time.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HandoffMeta {
    pub task_id: String,
    pub task_name: String,
    pub created: String,
    pub repo: String,
    pub source_repo: String,
    /// Orch may set these; not required by tsk.
    pub parent_plan: String,
    pub concern_index: String,
}

/// A validated handoff document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handoff {
    pub meta: HandoffMeta,
    /// Full markdown (title + meta + sections).
    pub markdown: String,
}

impl Handoff {
    /// Parse and validate markdown (no path in errors — use [`validate_at`]).
    pub fn parse(markdown: &str) -> Result<Self> {
        validate_markdown(markdown, None)?;
        Ok(Self {
            meta: parse_meta(markdown),
            markdown: markdown.to_string(),
        })
    }

    /// Read and validate from disk.
    pub fn read(path: &Path) -> Result<Self> {
        let markdown = fs::read_to_string(path).map_err(|source| {
            if source.kind() == std::io::ErrorKind::NotFound {
                TskError::HandoffNotFound {
                    path: path.to_path_buf(),
                }
            } else {
                TskError::Read {
                    path: path.to_path_buf(),
                    source,
                }
            }
        })?;
        validate_markdown(&markdown, Some(path))?;
        Ok(Self {
            meta: parse_meta(&markdown),
            markdown,
        })
    }

    /// Validate markdown, associating failures with `path`.
    pub fn validate_at(markdown: &str, path: &Path) -> Result<()> {
        validate_markdown(markdown, Some(path))
    }

    /// Write markdown, creating parent dirs.
    pub fn write(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| TskError::Write {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        fs::write(path, &self.markdown).map_err(|source| TskError::Write {
            path: path.to_path_buf(),
            source,
        })?;
        Ok(())
    }

    /// Stamp instruct-time meta into the header bullet list.
    pub fn with_meta(mut self, meta: HandoffMeta) -> Self {
        self.markdown = rewrite_meta(&self.markdown, &meta);
        self.meta = meta;
        self
    }

    /// One-line Goal summary for listings.
    pub fn goal_oneline(&self) -> String {
        section_body(&self.markdown, "Goal")
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .unwrap_or("(no goal)")
            .chars()
            .take(80)
            .collect()
    }
}

/// Empty template with required headings (and optional ones present but blank).
/// Does **not** pass validation until Goal / Success criteria are filled.
pub fn handoff_template() -> String {
    r#"# HANDOFF

- **task_id:** …
- **task_name:** …
- **created:** …
- **repo:** …
- **source_repo:** …

## Goal


## Principles


## Scope


## Out of scope


## Success criteria


## Constraints


## Handoff notes

"#
    .to_string()
}

fn validate_markdown(markdown: &str, path: Option<&Path>) -> Result<()> {
    let path_buf = path.map(Path::to_path_buf);
    for h in REQUIRED_HEADINGS {
        let needle = format!("## {h}");
        if !markdown.lines().any(|l| l.trim() == needle) {
            return Err(invalid(
                path_buf.clone(),
                format!("missing heading `## {h}`"),
            ));
        }
    }
    if section_body(markdown, "Goal").trim().is_empty() {
        return Err(invalid(
            path_buf.clone(),
            "Goal section is empty — add one paragraph".into(),
        ));
    }
    if section_body(markdown, "Success criteria").trim().is_empty() {
        return Err(invalid(
            path_buf,
            "Success criteria section is empty — add numbered observable criteria".into(),
        ));
    }
    Ok(())
}

fn invalid(path: Option<PathBuf>, reason: String) -> TskError {
    TskError::InvalidHandoff {
        path: path.unwrap_or_else(|| PathBuf::from("HANDOFF.md")),
        reason,
    }
}

fn section_body(markdown: &str, heading: &str) -> String {
    let needle = format!("## {heading}");
    let mut lines = markdown.lines();
    while let Some(line) = lines.next() {
        if line.trim() == needle {
            let mut body = String::new();
            for l in lines.by_ref() {
                if l.starts_with("## ") {
                    break;
                }
                body.push_str(l);
                body.push('\n');
            }
            return body;
        }
    }
    String::new()
}

fn parse_meta(markdown: &str) -> HandoffMeta {
    let mut meta = HandoffMeta::default();
    for line in markdown.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("- **task_id:**") {
            meta.task_id = rest.trim().to_string();
        } else if let Some(rest) = t.strip_prefix("- **task_name:**") {
            meta.task_name = rest.trim().to_string();
        } else if let Some(rest) = t.strip_prefix("- **parent_plan:**") {
            meta.parent_plan = rest.trim().to_string();
        } else if let Some(rest) = t.strip_prefix("- **concern_index:**") {
            meta.concern_index = rest.trim().to_string();
        } else if let Some(rest) = t.strip_prefix("- **created:**") {
            meta.created = rest.trim().to_string();
        } else if let Some(rest) = t.strip_prefix("- **repo:**") {
            meta.repo = rest.trim().to_string();
        } else if let Some(rest) = t.strip_prefix("- **source_repo:**") {
            meta.source_repo = rest.trim().to_string();
        }
        if t.starts_with("## ") {
            break;
        }
    }
    meta
}

fn rewrite_meta(markdown: &str, meta: &HandoffMeta) -> String {
    let mut out = String::new();
    let mut in_header = true;
    let mut wrote_meta = false;
    for line in markdown.lines() {
        let t = line.trim();
        if in_header && t.starts_with("## ") {
            if !wrote_meta {
                out.push_str(&format_meta_block(meta));
                out.push('\n');
                wrote_meta = true;
            }
            in_header = false;
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if in_header {
            if t.starts_with("# ") {
                out.push_str(line);
                out.push('\n');
                out.push('\n');
                out.push_str(&format_meta_block(meta));
                wrote_meta = true;
                continue;
            }
            if t.starts_with("- **") {
                continue;
            }
            if t.is_empty() && wrote_meta {
                continue;
            }
            if !t.is_empty() && !t.starts_with("- **") && wrote_meta {
                out.push_str(line);
                out.push('\n');
            }
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    if !wrote_meta {
        let mut prepended = format_meta_block(meta);
        prepended.push('\n');
        prepended.push_str(markdown);
        return prepended;
    }
    out
}

fn format_meta_block(meta: &HandoffMeta) -> String {
    let mut lines = vec![
        format!("- **task_id:** {}", empty_as_placeholder(&meta.task_id)),
        format!("- **task_name:** {}", empty_as_placeholder(&meta.task_name)),
    ];
    if !meta.parent_plan.is_empty() {
        lines.push(format!("- **parent_plan:** {}", meta.parent_plan));
    }
    if !meta.concern_index.is_empty() {
        lines.push(format!("- **concern_index:** {}", meta.concern_index));
    }
    lines.push(format!(
        "- **created:** {}",
        empty_as_placeholder(&meta.created)
    ));
    lines.push(format!("- **repo:** {}", empty_as_placeholder(&meta.repo)));
    lines.push(format!(
        "- **source_repo:** {}",
        empty_as_placeholder(&meta.source_repo)
    ));
    let mut block = lines.join("\n");
    block.push('\n');
    block
}

fn empty_as_placeholder(s: &str) -> &str {
    if s.is_empty() {
        "…"
    } else {
        s
    }
}

/// Minimal valid handoff for tests (optional sections omitted).
pub fn sample_handoff_markdown() -> &'static str {
    r#"# HANDOFF

- **task_id:** …
- **task_name:** …
- **created:** …
- **repo:** …
- **source_repo:** …

## Goal
Ship the handoff API.

## Scope
1. Core validate/write
2. CLI instruct

## Out of scope
- Orch AGENT_BRIEF migration

## Success criteria
1. validate accepts this document
2. Path is workspace/HANDOFF.md

## Constraints
- Fail closed on empty Goal
"#
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn handoff_path_is_under_workspace() {
        let home = PathBuf::from("/tmp/tsk-tasks/tabc");
        assert_eq!(
            handoff_path(&home),
            PathBuf::from("/tmp/tsk-tasks/tabc/workspace/HANDOFF.md")
        );
    }

    #[test]
    fn sample_validates_without_optional_headings() {
        let h = Handoff::parse(sample_handoff_markdown()).unwrap();
        assert!(h.goal_oneline().contains("Ship the handoff"));
        assert!(!h.markdown.contains("## Principles"));
        assert!(!h.markdown.contains("## Handoff notes"));
    }

    #[test]
    fn empty_goal_fails() {
        let md =
            sample_handoff_markdown().replace("## Goal\nShip the handoff API.\n", "## Goal\n\n");
        let err = Handoff::parse(&md).unwrap_err();
        match err {
            TskError::InvalidHandoff { reason, .. } => {
                assert!(reason.contains("Goal section is empty"));
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn missing_required_heading_fails() {
        let md =
            sample_handoff_markdown().replace("## Constraints\n- Fail closed on empty Goal\n", "");
        let err = Handoff::parse(&md).unwrap_err();
        match err {
            TskError::InvalidHandoff { reason, .. } => {
                assert!(reason.contains("## Constraints"));
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn with_meta_rewrites_header() {
        let h = Handoff::parse(sample_handoff_markdown())
            .unwrap()
            .with_meta(HandoffMeta {
                task_id: "t123".into(),
                task_name: "demo".into(),
                created: "2026-09-21T00:00:00Z".into(),
                repo: "/tmp/repo".into(),
                source_repo: "/tmp/src".into(),
                parent_plan: String::new(),
                concern_index: String::new(),
            });
        assert!(h.markdown.contains("- **task_id:** t123"));
        assert!(h.markdown.contains("- **task_name:** demo"));
        assert!(!h.markdown.contains("parent_plan"));
    }

    #[test]
    fn write_and_read_roundtrip() {
        let dir = tempdir().unwrap();
        let home = dir.path().join("tabc");
        let path = handoff_path(&home);
        let h = Handoff::parse(sample_handoff_markdown()).unwrap();
        h.write(&path).unwrap();
        let loaded = Handoff::read(&path).unwrap();
        assert_eq!(loaded.goal_oneline(), h.goal_oneline());
    }

    #[test]
    fn template_does_not_validate() {
        let err = Handoff::parse(&handoff_template()).unwrap_err();
        assert!(matches!(err, TskError::InvalidHandoff { .. }));
    }

    #[test]
    fn optional_headings_constant_covers_plan() {
        assert!(OPTIONAL_HEADINGS.contains(&"Principles"));
        assert!(OPTIONAL_HEADINGS.contains(&"Handoff notes"));
    }
}
