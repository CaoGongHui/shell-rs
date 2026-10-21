use gpui_kit::component::{
    Sizable as _, WindowExt as _,
    form::{Field, Form},
    input::{Input, InputState},
    v_flex,
};
use gpui_kit::*;

use crate::host::{HostStore, SavedTerminalId, TerminalGroupId};
use crate::i18n::{t, tn};
use crate::shared::{
    DeleteHandler, commit_footer, confirm_delete, dismiss_form_error, form_error_notification,
};

/// What the one-field dialog of 项目 names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameDialog {
    NewGroup,
    RenameGroup(TerminalGroupId),
    RenameTerminal(SavedTerminalId),
}

/// The dialog's body: the name, validated on commit; the store is only
/// touched when it passes. A terminal's may be left empty, for it to go by
/// its tab's titles again.
pub struct NameForm {
    store: Entity<HostStore>,
    dialog: NameDialog,
    name: Entity<InputState>,
}

impl NameForm {
    fn new(
        dialog: NameDialog,
        store: Entity<HostStore>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let name = {
            let store = store.read(cx);
            match dialog {
                NameDialog::NewGroup => None,
                NameDialog::RenameGroup(id) => store.terminal_group(id).map(|g| g.name.clone()),
                // Only a name given by hand: the one it was given goes.
                NameDialog::RenameTerminal(id) => store
                    .saved_terminal(id)
                    .filter(|terminal| terminal.renamed)
                    .map(|terminal| terminal.name.clone()),
            }
        };
        let placeholder = match dialog {
            NameDialog::NewGroup | NameDialog::RenameGroup(_) => {
                t!("project.dialog.group_placeholder")
            }
            NameDialog::RenameTerminal(_) => t!("project.dialog.terminal_placeholder"),
        };
        let name = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(placeholder)
                .default_value(name.unwrap_or_default())
        });
        Self {
            store,
            dialog,
            name,
        }
    }

    /// Validate and write to the store. Returns whether the dialog may close.
    fn commit(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let name: SharedString = self.name.read(cx).value().trim().to_string().into();
        let renaming = match self.dialog {
            NameDialog::RenameGroup(id) => Some(id),
            _ => None,
        };
        let taken = matches!(
            self.dialog,
            NameDialog::NewGroup | NameDialog::RenameGroup(_)
        ) && self
            .store
            .read(cx)
            .terminal_groups()
            .iter()
            .any(|group| group.name == name && Some(group.id) != renaming);
        let optional = matches!(self.dialog, NameDialog::RenameTerminal(_));
        let error = if name.is_empty() && !optional {
            Some(t!("project.dialog.name_missing"))
        } else if taken {
            Some(t!("project.dialog.group_taken"))
        } else {
            None
        };
        if let Some(error) = error {
            window.push_notification(form_error_notification(error), cx);
            return false;
        }
        let dialog = self.dialog;
        self.store.update(cx, |store, cx| match dialog {
            NameDialog::NewGroup => {
                store.insert_terminal_group(name, cx);
            }
            NameDialog::RenameGroup(id) => {
                store.rename_terminal_group(id, name, cx);
            }
            NameDialog::RenameTerminal(id) => {
                store.rename_saved_terminal(id, (!name.is_empty()).then_some(name), cx);
            }
        });
        true
    }
}

impl Render for NameForm {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let required = !matches!(self.dialog, NameDialog::RenameTerminal(_));
        v_flex().gap_3().w_full().child(
            Form::new().child(
                Field::new()
                    .label(t!("project.dialog.name"))
                    .required(required)
                    .child(Input::new(&self.name).id("project-name").small()),
            ),
        )
    }
}

/// Open the new-group dialog, or a rename dialog, of 项目.
pub fn open_name_dialog(
    dialog: NameDialog,
    store: Entity<HostStore>,
    window: &mut Window,
    cx: &mut App,
) {
    let form = cx.new(|cx| NameForm::new(dialog, store, window, cx));
    let (title, commit_label) = match dialog {
        NameDialog::NewGroup => (t!("project.dialog.new_group"), t!("project.dialog.create")),
        NameDialog::RenameGroup(_) => (t!("project.dialog.rename_group"), t!("common.save")),
        NameDialog::RenameTerminal(_) => (t!("project.dialog.rename_terminal"), t!("common.save")),
    };
    window.open_dialog(cx, {
        let form = form.clone();
        move |dialog, _, _| {
            dialog
                .title(title.clone())
                // Closed by its buttons or Escape, not by a click beside it.
                .overlay_closable(false)
                .child(form.clone())
                .footer(commit_footer("commit", commit_label.clone()))
                .on_ok({
                    let form = form.clone();
                    move |_, window, cx| form.update(cx, |form, cx| form.commit(window, cx))
                })
                .on_close(|_, window, cx| dismiss_form_error(window, cx))
        }
    });
}

/// Ask before deleting a group, which takes its terminals with it.
pub fn confirm_delete_terminal_group(
    name: &str,
    terminals: usize,
    on_delete: DeleteHandler,
    window: &mut Window,
    cx: &mut App,
) {
    let description = (terminals > 0).then(|| tn!("project.delete_group.terminals", terminals));
    confirm_delete(name, description, on_delete, window, cx);
}
