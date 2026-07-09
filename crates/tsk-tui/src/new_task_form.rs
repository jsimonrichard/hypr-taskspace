//! Focus model for the new-task form dialog.

use tsk_core::TaskRepoSource;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewTaskFormFocus {
    Worktree,
    Container,
    Name,
    Buttons,
}

pub fn worktree_field_visible(repo: &TaskRepoSource) -> bool {
    !matches!(repo, TaskRepoSource::Scratch)
}

pub fn form_fields(repo: &TaskRepoSource) -> Vec<NewTaskFormFocus> {
    let mut fields = vec![NewTaskFormFocus::Name];
    if worktree_field_visible(repo) {
        fields.push(NewTaskFormFocus::Worktree);
    }
    fields.push(NewTaskFormFocus::Container);
    fields.push(NewTaskFormFocus::Buttons);
    fields
}

pub fn initial_form_focus(_repo: &TaskRepoSource) -> NewTaskFormFocus {
    NewTaskFormFocus::Name
}

pub fn cycle_form_focus(
    current: NewTaskFormFocus,
    repo: &TaskRepoSource,
    delta: i32,
) -> NewTaskFormFocus {
    let fields = form_fields(repo);
    let idx = fields
        .iter()
        .position(|&field| field == current)
        .unwrap_or(0) as i32;
    let next = (idx + delta).rem_euclid(fields.len() as i32);
    fields[next as usize]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn scratch_form_skips_worktree_field() {
        let repo = TaskRepoSource::Scratch;
        assert_eq!(initial_form_focus(&repo), NewTaskFormFocus::Name);
        assert_eq!(
            form_fields(&repo),
            vec![
                NewTaskFormFocus::Name,
                NewTaskFormFocus::Container,
                NewTaskFormFocus::Buttons
            ]
        );
    }

    #[test]
    fn linked_repo_form_includes_worktree_and_container() {
        let repo = TaskRepoSource::Path(PathBuf::from("/tmp/project"));
        assert_eq!(initial_form_focus(&repo), NewTaskFormFocus::Name);
        assert_eq!(
            cycle_form_focus(NewTaskFormFocus::Name, &repo, 1),
            NewTaskFormFocus::Worktree
        );
        assert_eq!(
            cycle_form_focus(NewTaskFormFocus::Worktree, &repo, 1),
            NewTaskFormFocus::Container
        );
        assert_eq!(
            cycle_form_focus(NewTaskFormFocus::Buttons, &repo, 1),
            NewTaskFormFocus::Name
        );
    }
}
