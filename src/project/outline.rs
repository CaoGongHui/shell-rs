//! Pure functions that lay the store's groups and saved terminals out as
//! the rows of 项目.

use crate::host::{SavedTerminal, SavedTerminalId, TerminalGroup, TerminalGroupId};

/// A row of 项目 as the list shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectRow {
    /// A group's heading, `folded` when its terminals are hidden.
    Group { id: TerminalGroupId, folded: bool },
    /// A terminal, `in_group` when it is under a heading.
    Terminal { id: SavedTerminalId, in_group: bool },
}

/// The groups in their order, each with its terminals below it in theirs
/// when it is expanded, then the terminals of no group, as the host tree
/// lists its groups before its hosts.
pub fn project_rows(groups: &[TerminalGroup], terminals: &[SavedTerminal]) -> Vec<ProjectRow> {
    let mut groups: Vec<_> = groups.iter().collect();
    groups.sort_by_key(|group| (group.sort_order, group.id));
    let mut terminals: Vec<_> = terminals.iter().collect();
    terminals.sort_by_key(|terminal| (terminal.sort_order, terminal.id));
    let mut rows = Vec::new();
    for group in groups {
        let folded = !group.expanded;
        rows.push(ProjectRow::Group {
            id: group.id,
            folded,
        });
        if !folded {
            rows.extend(
                terminals
                    .iter()
                    .filter(|terminal| terminal.group == Some(group.id))
                    .map(|terminal| ProjectRow::Terminal {
                        id: terminal.id,
                        in_group: true,
                    }),
            );
        }
    }
    rows.extend(
        terminals
            .iter()
            .filter(|terminal| terminal.group.is_none())
            .map(|terminal| ProjectRow::Terminal {
                id: terminal.id,
                in_group: false,
            }),
    );
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::{HostStore, TerminalDrop, TerminalNode};

    #[test]
    fn groups_come_first_with_their_terminals_then_the_rest() {
        let mut store = HostStore::empty();
        let shellrs = store.insert_terminal_group_unnotified("shellrs".into());
        let blog = store.insert_terminal_group_unnotified("blog".into());
        let top = store.insert_saved_terminal_unnotified(None, None).unwrap();
        let blog_1 = store
            .insert_saved_terminal_unnotified(Some(blog), None)
            .unwrap();
        let first = store
            .insert_saved_terminal_unnotified(Some(shellrs), None)
            .unwrap();
        let second = store
            .insert_saved_terminal_unnotified(Some(shellrs), None)
            .unwrap();
        store.move_terminal_node_unnotified(
            TerminalNode::Group(blog),
            TerminalDrop::Before(TerminalNode::Group(shellrs)),
        );
        store.move_terminal_node_unnotified(
            TerminalNode::Terminal(second),
            TerminalDrop::Before(TerminalNode::Terminal(first)),
        );
        let rows = |groups: &[TerminalGroup]| project_rows(groups, store.saved_terminals());
        let group = |id, folded| ProjectRow::Group { id, folded };
        let terminal = |id, in_group| ProjectRow::Terminal { id, in_group };
        let mut groups = store.terminal_groups().to_vec();
        assert_eq!(
            rows(&groups),
            [
                group(blog, false),
                terminal(blog_1, true),
                group(shellrs, false),
                terminal(second, true),
                terminal(first, true),
                terminal(top, false),
            ]
        );
        for group in &mut groups {
            group.expanded = group.id != shellrs;
        }
        assert_eq!(
            rows(&groups),
            [
                group(blog, false),
                terminal(blog_1, true),
                group(shellrs, true),
                terminal(top, false),
            ]
        );
    }
}
