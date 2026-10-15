//! The 新建 and 重命名 dialog of tmux: one name field.

use std::rc::Rc;

use gpui_kit::component::{
    Sizable as _, WindowExt as _,
    form::{Field, Form},
    input::{Input, InputState},
};
use gpui_kit::*;

use super::model::valid_name;
use crate::i18n::t;
use crate::shared::{commit_footer, dismiss_form_error, form_error_notification};

/// What the dialog names.
pub enum SessionName {
    /// A session to start; with no name, tmux numbers it.
    New,
    /// The session called this.
    Rename(String),
}

/// What runs with the name once the dialog closed: `None` for a new session
/// tmux numbers.
pub type NameHandler = Rc<dyn Fn(Option<String>, &mut Window, &mut App)>;

struct NameForm {
    renaming: Option<String>,
    /// The host's sessions' names: two sessions cannot share one.
    taken: Vec<String>,
    name: Entity<InputState>,
    on_ok: NameHandler,
}

impl NameForm {
    fn commit(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let name = self.name.read(cx).value().trim().to_string();
        if self.renaming.as_deref() == Some(name.as_str()) {
            return true;
        }
        let error = if name.is_empty() {
            self.renaming.is_some().then(|| t!("tmux.name.missing"))
        } else if !valid_name(&name) {
            Some(t!("tmux.name.invalid"))
        } else if self.taken.contains(&name) {
            Some(t!("tmux.name.taken", name = name))
        } else {
            None
        };
        if let Some(error) = error {
            window.push_notification(form_error_notification(error), cx);
            return false;
        }
        // Once the dialog is gone: the terminal takes the keyboard after.
        let on_ok = self.on_ok.clone();
        let name = (!name.is_empty()).then_some(name);
        window.defer(cx, move |window, cx| on_ok(name, window, cx));
        true
    }
}

impl Render for NameForm {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let field = Field::new()
            .label(t!("tmux.name.label"))
            .child(Input::new(&self.name).id("tmux-session-name").small());
        Form::new().child(if self.renaming.is_some() {
            field.required(true)
        } else {
            field.description(t!("tmux.name.description"))
        })
    }
}

/// Open the dialog that names a new session, or renames one.
pub fn open_name_dialog(
    subject: SessionName,
    taken: Vec<String>,
    on_ok: NameHandler,
    window: &mut Window,
    cx: &mut App,
) {
    let (renaming, title, commit) = match subject {
        SessionName::New => (None, t!("tmux.new.title"), t!("tmux.new.confirm")),
        SessionName::Rename(name) => (Some(name), t!("tmux.rename.title"), t!("common.save")),
    };
    let form = cx.new(|cx| NameForm {
        name: cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("tmux.name.placeholder"))
                .default_value(renaming.clone().unwrap_or_default())
        }),
        renaming,
        taken,
        on_ok,
    });
    window.open_dialog(cx, {
        let form = form.clone();
        move |dialog, _, _| {
            dialog
                .title(title.clone())
                // Closed by its buttons or Escape, not by a click beside it.
                .overlay_closable(false)
                .child(form.clone())
                .footer(commit_footer("commit", commit.clone()))
                .on_ok({
                    let form = form.clone();
                    move |_, window, cx| form.update(cx, |form, cx| form.commit(window, cx))
                })
                .on_close(|_, window, cx| dismiss_form_error(window, cx))
        }
    });
    let name = form.read(cx).name.clone();
    name.update(cx, |name, cx| name.focus(window, cx));
}
