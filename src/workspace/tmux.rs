//! The right sidebar's tmux tool's part of `Workspace`: starting, attaching,
//! detaching, renaming and ending the sessions of the host of the SSH
//! terminal in front.
//!
//! A terminal not in tmux is attached by typing the command into it, as
//! 历史命令 does; a terminal in tmux has its client told beside the shell,
//! since what is typed into it goes to the program in its pane.

use std::rc::Rc;

use gpui_kit::component::{WindowExt as _, notification::Notification};
use gpui_kit::*;

use crate::app::{
    AttachTmuxSession, DetachTmux, KillTmuxSession, NewTmuxSession, RefreshTmux, RenameTmuxSession,
};
use crate::i18n::t;
use crate::shared::confirm_danger;
use crate::tmux::{
    SessionName, TmuxTable, attach_typed, detach_command, done, kill_command, new_command,
    new_typed, open_name_dialog, rename_command, switch_command,
};

use super::{
    Workspace,
    tool_sidebar::{ToolSidebar, ToolTerminal},
    tools::{HostCommand, run_on_host},
};

impl Workspace {
    /// The SSH terminal in front and its host's sessions, once read.
    fn tmux(&self, cx: &App) -> Option<(ToolTerminal, Rc<TmuxTable>)> {
        let terminal = self.tool_terminal(cx)?;
        let table = self.tools.read(cx).tmux_table(cx)?;
        Some((terminal, table))
    }

    /// tmux's 刷新: read the host's sessions again.
    pub(super) fn on_refresh_tmux(
        &mut self,
        _: &RefreshTmux,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.tools.update(cx, |tools, cx| tools.refresh_tmux(cx));
    }

    /// tmux's 新建: ask for a name, then start the session in the terminal.
    pub(super) fn on_new_tmux_session(
        &mut self,
        _: &NewTmuxSession,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((_, table)) = self.tmux(cx) else {
            return;
        };
        let workspace = cx.entity().downgrade();
        open_name_dialog(
            SessionName::New,
            table.names(),
            Rc::new(move |name, window, cx| {
                workspace
                    .update(cx, |this, cx| this.new_tmux_session(name, window, cx))
                    .ok();
            }),
            window,
            cx,
        );
    }

    fn new_tmux_session(
        &mut self,
        name: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((terminal, table)) = self.tmux(cx) else {
            return;
        };
        let failed = |why: &str| t!("tmux.new.failed", error = why);
        match table.client() {
            Some(client) => self.tell_tmux(
                &terminal,
                new_command(name.as_deref(), client),
                Box::new(failed),
                window,
                cx,
            ),
            None => self.type_tmux(&terminal, &new_typed(name.as_deref()), failed, window, cx),
        }
    }

    /// A session's 连接: the terminal into it.
    pub(super) fn on_attach_tmux_session(
        &mut self,
        action: &AttachTmuxSession,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((terminal, table)) = self.tmux(cx) else {
            return;
        };
        let Some(session) = table.session(&action.0).cloned() else {
            return;
        };
        if table.current.as_deref() == Some(session.id.as_str()) {
            return;
        }
        let name = session.name.clone();
        let failed = move |why: &str| t!("tmux.attach.failed", name = name, error = why);
        match table.client() {
            Some(client) => self.tell_tmux(
                &terminal,
                switch_command(client, &session.id),
                Box::new(failed),
                window,
                cx,
            ),
            None => self.type_tmux(&terminal, &attach_typed(&session), failed, window, cx),
        }
    }

    /// tmux's 退出当前: the terminal out of its session, which runs on.
    pub(super) fn on_detach_tmux(
        &mut self,
        _: &DetachTmux,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((terminal, table)) = self.tmux(cx) else {
            return;
        };
        let Some(client) = table.client() else {
            return;
        };
        self.tell_tmux(
            &terminal,
            detach_command(client),
            Box::new(|why| t!("tmux.detach.failed", error = why)),
            window,
            cx,
        );
    }

    /// A session's 重命名: ask for the new name, then give it.
    pub(super) fn on_rename_tmux_session(
        &mut self,
        action: &RenameTmuxSession,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((terminal, table)) = self.tmux(cx) else {
            return;
        };
        let Some(session) = table.session(&action.0).cloned() else {
            return;
        };
        let taken = table
            .names()
            .into_iter()
            .filter(|name| *name != session.name)
            .collect();
        let workspace = cx.entity().downgrade();
        open_name_dialog(
            SessionName::Rename(session.name.clone()),
            taken,
            Rc::new(move |name, window, cx| {
                let Some(name) = name else { return };
                let old = session.name.clone();
                run_on_host(
                    HostCommand {
                        script: rename_command(&session.id, &name),
                        outcome: done,
                        done: None,
                        failed: Box::new(move |why| {
                            t!("tmux.rename.failed", name = old, error = why)
                        }),
                        then: ToolSidebar::refresh_tmux,
                    },
                    &terminal,
                    workspace.clone(),
                    window,
                    cx,
                );
            }),
            window,
            cx,
        );
    }

    /// A session's 结束会话: ask, then end it with what runs in it.
    pub(super) fn on_kill_tmux_session(
        &mut self,
        action: &KillTmuxSession,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((terminal, table)) = self.tmux(cx) else {
            return;
        };
        let Some(session) = table.session(&action.0).cloned() else {
            return;
        };
        let description = if table.current.as_deref() == Some(session.id.as_str()) {
            t!("tmux.kill.description_current")
        } else {
            t!("tmux.kill.description")
        };
        let workspace = cx.entity().downgrade();
        confirm_danger(
            t!("tmux.kill.title", name = session.name),
            Some(description),
            t!("tmux.kill.confirm"),
            Rc::new(move |window, cx| {
                let name = session.name.clone();
                run_on_host(
                    HostCommand {
                        script: kill_command(&session.id),
                        outcome: done,
                        done: Some(t!("tmux.kill.done", name = session.name)),
                        failed: Box::new(move |why| {
                            t!("tmux.kill.failed", name = name, error = why)
                        }),
                        then: ToolSidebar::refresh_tmux,
                    },
                    &terminal,
                    workspace.clone(),
                    window,
                    cx,
                )
            }),
            window,
            cx,
        );
    }

    /// Tell the terminal's tmux client what to do, beside the shell; the
    /// terminal takes the keyboard, to carry on in the session.
    fn tell_tmux(
        &mut self,
        terminal: &ToolTerminal,
        script: Option<String>,
        failed: Box<dyn FnOnce(&str) -> SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = terminal.view.upgrade() {
            let focus = view.read(cx).focus_handle();
            window.focus(&focus, cx);
        }
        run_on_host(
            HostCommand {
                script,
                outcome: done,
                done: None,
                failed,
                then: ToolSidebar::refresh_tmux,
            },
            terminal,
            cx.entity().downgrade(),
            window,
            cx,
        );
    }

    /// Type `command` into the terminal and run it, in place of what is
    /// typed there, and read the sessions again once tmux has had a moment;
    /// the terminal takes the keyboard. Why not, when it cannot (a
    /// full-screen program has the keys), is a notification.
    fn type_tmux(
        &mut self,
        terminal: &ToolTerminal,
        command: &str,
        failed: impl FnOnce(&str) -> SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = terminal.view.upgrade() else {
            return;
        };
        match view.update(cx, |view, cx| view.enter_command(command, true, true, cx)) {
            Ok(()) => {
                let focus = view.read(cx).focus_handle();
                window.focus(&focus, cx);
                self.tools
                    .update(cx, |tools, cx| tools.refresh_tmux_soon(cx));
            }
            Err(why) => window.push_notification(Notification::error(failed(&why)), cx),
        }
    }
}
