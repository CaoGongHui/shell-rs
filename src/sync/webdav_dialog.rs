//! The WebDAV dialog of 同步: the server, the username and the password,
//! to set sync up there, or to sign in again once the server refused them.
//!
//! Setting up, it looks whether the server has sync already and goes on to
//! the sync password's dialog (create or join), handing it the login: the
//! keychain keeps the password only once sync is set up.

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    dialog::{DialogAction, DialogClose, DialogFooter},
    form::{Field, Form},
    h_flex,
    input::{Input, InputContentType, InputState},
    notification::Notification,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use zeroize::Zeroizing;

use super::sync_dialog::{JoinReason, PasswordDialog, open_password_dialog};
use super::{JobError, SyncService};
use crate::i18n::t;
use crate::settings::SyncLocation;
use crate::shared::{dismiss_form_error, form_error_notification};

/// What the WebDAV dialog does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WebDavDialog {
    /// Set sync up on a server: create it there, or join it.
    SetUp,
    /// Sign in to the server sync goes through anew.
    Login { url: String, username: String },
}

pub struct WebDavForm {
    service: Entity<SyncService>,
    dialog: WebDavDialog,
    url: Entity<InputState>,
    username: Entity<InputState>,
    password: Entity<InputState>,
    working: bool,
    testing: bool,
}

impl WebDavForm {
    fn new(
        service: Entity<SyncService>,
        dialog: WebDavDialog,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (url, username) = match &dialog {
            WebDavDialog::SetUp => (String::new(), String::new()),
            WebDavDialog::Login { url, username } => (url.clone(), username.clone()),
        };
        let url = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("https://dav.jianguoyun.com/dav/ShellRS")
                .default_value(url)
        });
        let username = cx.new(|cx| InputState::new(window, cx).default_value(username));
        let password = cx.new(|cx| InputState::new(window, cx).masked(true));
        Self {
            service,
            dialog,
            url,
            username,
            password,
            working: false,
            testing: false,
        }
    }

    fn focus(&self, window: &mut Window, cx: &mut App) {
        let first = match self.dialog {
            WebDavDialog::SetUp => &self.url,
            WebDavDialog::Login { .. } => &self.password,
        };
        first.update(cx, |state, cx| state.focus(window, cx));
    }

    /// The server and login the fields name, or what is wrong with them.
    fn login(&self, cx: &App) -> Result<(SyncLocation, Zeroizing<String>), SharedString> {
        let url = self.url.read(cx).value().trim().to_string();
        let username = self.username.read(cx).value().trim().to_string();
        let password = Zeroizing::new(self.password.read(cx).value().to_string());
        let address = url::Url::parse(&url).ok().filter(|address| {
            matches!(address.scheme(), "http" | "https") && address.host_str().is_some()
        });
        if address.is_none() {
            return Err(t!("sync.webdav.url_invalid"));
        }
        if username.is_empty() {
            return Err(t!("sync.webdav.username_missing"));
        }
        if password.is_empty() {
            return Err(t!("sync.webdav.password_missing"));
        }
        Ok((SyncLocation::WebDav { url, username }, password))
    }

    fn test(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.testing {
            return;
        }
        let (location, password) = match self.login(cx) {
            Ok(login) => login,
            Err(error) => {
                window.push_notification(form_error_notification(error), cx);
                return;
            }
        };
        let test = self.service.update(cx, |service, cx| {
            service.test_connection(location, Some(password), cx)
        });
        self.testing = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = test.await;
            this.update_in(cx, |this, window, cx| {
                this.testing = false;
                cx.notify();
                window.push_notification(tested(result), cx);
            })
            .ok();
        })
        .detach();
    }

    /// Check the fields and start. Returns whether the dialog may close:
    /// never at once, the job closes it when it went through.
    fn commit(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.working {
            return false;
        }
        let (location, password) = match self.login(cx) {
            Ok(login) => login,
            Err(error) => {
                window.push_notification(form_error_notification(error), cx);
                return false;
            }
        };
        self.working = true;
        cx.notify();
        let service = self.service.clone();
        match &self.dialog {
            WebDavDialog::SetUp => {
                let inspect = service.update(cx, |service, cx| {
                    service.inspect(location.clone(), Some(password.clone()), cx)
                });
                cx.spawn_in(window, async move |this, cx| {
                    let found = inspect.await;
                    this.update_in(cx, |this, window, cx| {
                        this.working = false;
                        cx.notify();
                        match found {
                            Ok(present) => {
                                dismiss_form_error(window, cx);
                                window.close_dialog(cx);
                                let dialog = if present {
                                    PasswordDialog::Join {
                                        location,
                                        reason: JoinReason::First,
                                    }
                                } else {
                                    PasswordDialog::Create { location }
                                };
                                open_password_dialog(service, dialog, Some(password), window, cx);
                            }
                            Err(error) => window
                                .push_notification(form_error_notification(error.describe()), cx),
                        }
                    })
                    .ok();
                })
                .detach();
            }
            WebDavDialog::Login { .. } => {
                let SyncLocation::WebDav { username, .. } = location else {
                    return false;
                };
                let change = service.update(cx, |service, cx| {
                    service.change_login(username, password, cx)
                });
                cx.spawn_in(window, async move |this, cx| {
                    let result = change.await;
                    this.update_in(cx, |this, window, cx| {
                        this.working = false;
                        cx.notify();
                        match result {
                            Ok(()) => {
                                dismiss_form_error(window, cx);
                                window.close_dialog(cx);
                            }
                            Err(error) => window
                                .push_notification(form_error_notification(error.describe()), cx),
                        }
                    })
                    .ok();
                })
                .detach();
            }
        }
        false
    }
}

/// What 测试连接 found: it reads and writes there, or why not.
fn tested(result: Result<(), JobError>) -> Notification {
    match result {
        Ok(()) => Notification::success(t!("sync.webdav.test_ok")),
        Err(error) => Notification::error(error.describe()).title(t!("sync.webdav.test_failed")),
    }
}

impl Render for WebDavForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let login = matches!(self.dialog, WebDavDialog::Login { .. });
        let busy = self.working;
        v_flex()
            .gap_3()
            .w_full()
            .child(
                Form::new()
                    .child(
                        Field::new()
                            .label(t!("sync.webdav.url"))
                            .required(true)
                            .child(
                                Input::new(&self.url)
                                    .id("webdav-url")
                                    .small()
                                    // A server signed in to again is the same
                                    // server: another one is another sync.
                                    .disabled(login || busy),
                            ),
                    )
                    .child(
                        Field::new()
                            .label(t!("sync.webdav.username"))
                            .required(true)
                            .child(
                                Input::new(&self.username)
                                    .id("webdav-username")
                                    .small()
                                    .disabled(busy),
                            ),
                    )
                    .child(
                        Field::new()
                            .label(t!("sync.webdav.password"))
                            .required(true)
                            .child(
                                Input::new(&self.password)
                                    .id("webdav-password")
                                    .small()
                                    .mask_toggle()
                                    .content_type(InputContentType::Password)
                                    .disabled(busy),
                            ),
                    ),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(muted)
                    .child(t!("sync.webdav.app_password")),
            )
            .when(busy, |view| {
                view.child(
                    div()
                        .id("webdav-working")
                        .test_support()
                        .text_sm()
                        .text_color(muted)
                        .child(t!("sync.webdav.connecting")),
                )
            })
    }
}

/// The footer: 测试连接 on the left, 取消 and the commit on the right, as
/// the host dialog has them.
fn footer(form: &Entity<WebDavForm>, commit_label: SharedString, cx: &App) -> DialogFooter {
    let testing = form.read(cx).testing;
    DialogFooter::new()
        .w_full()
        .justify_between()
        .child(
            Button::new("webdav-test")
                .label(t!("sync.webdav.test"))
                .icon(crate::app::CatalogIcon::Plug)
                .small()
                .loading(testing)
                .disabled(testing)
                .on_click({
                    let form = form.clone();
                    move |_, window, cx| form.update(cx, |form, cx| form.test(window, cx))
                }),
        )
        .child(
            h_flex()
                .gap_2()
                .child(DialogClose::new().trigger(|button| button.label(t!("common.cancel"))))
                .child(
                    DialogAction::new()
                        .child(Button::new("webdav-commit").primary().label(commit_label)),
                ),
        )
}

/// Open the WebDAV dialog.
pub fn open_webdav_dialog(
    service: Entity<SyncService>,
    dialog: WebDavDialog,
    window: &mut Window,
    cx: &mut App,
) {
    let (title, commit_label) = match dialog {
        WebDavDialog::SetUp => (t!("sync.webdav.set_up_title"), t!("sync.webdav.next")),
        WebDavDialog::Login { .. } => (t!("sync.webdav.login_title"), t!("common.save")),
    };
    let form = cx.new(|cx| WebDavForm::new(service, dialog, window, cx));
    window.open_dialog(cx, {
        let form = form.clone();
        move |dialog, _, cx| {
            dialog
                .title(title.clone())
                .overlay_closable(false)
                .child(form.clone())
                .footer(footer(&form, commit_label.clone(), cx))
                .on_ok({
                    let form = form.clone();
                    move |_, window, cx| form.update(cx, |form, cx| form.commit(window, cx))
                })
                .on_close(|_, window, cx| dismiss_form_error(window, cx))
        }
    });
    form.update(cx, |form, cx| form.focus(window, cx));
}
