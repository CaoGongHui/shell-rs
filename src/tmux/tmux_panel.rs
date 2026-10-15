use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    scroll::ScrollableElement as _,
    spinner::Spinner,
    tag::Tag,
    tooltip::Tooltip,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::linux::{self, Parsed};
use super::model::{TmuxSession, TmuxTable};
use crate::app::{
    AttachTmuxSession, CatalogIcon, DetachTmux, KillTmuxSession, NewTmuxSession, RefreshTmux,
    RenameTmuxSession,
};
use crate::i18n::t;
use crate::shared::{soft_tag, tinted};
use crate::terminal::{ExecResult, ExecTarget, RemoteTerminalId, exec_answer};

/// How long a terminal not connected is left before it is asked again.
const RETRY: Duration = Duration::from_secs(2);
/// How long after a command typed into the terminal the sessions are read
/// again: tmux attaches as the shell runs it, a moment later.
const AFTER_TYPING: Duration = Duration::from_secs(1);

/// tmux: the sessions of the host of the SSH terminal in front, read when
/// the panel comes on screen, on 刷新 and after a command, with the one the
/// terminal is in marked. 新建 starts one and 退出当前 detaches the
/// terminal; each session attaches, is renamed or ends.
///
/// Each reading runs `linux::command` beside the shell, on the terminal's
/// own connection. What each terminal's host showed last is kept, so
/// coming back to a terminal shows its sessions at once while they are read
/// again.
pub struct TmuxPanel {
    target: Option<ExecTarget>,
    active: bool,
    hosts: HashMap<RemoteTerminalId, HostState>,
    /// The reading on its way; dropping it stops it.
    reading: Option<Task<()>>,
    /// Whether a reading is on its way.
    loading: bool,
    /// The workspace's focus handle: the panel's buttons dispatch on it, so
    /// they work whatever is focused.
    dispatch: FocusHandle,
}

#[derive(Default)]
struct HostState {
    table: Option<Rc<TmuxTable>>,
    problem: Option<Problem>,
}

#[derive(Clone, Debug, PartialEq)]
enum Problem {
    /// The terminal is not connected, so there is no connection to read on.
    NotConnected,
    /// No `tmux` on the host.
    Missing,
    /// Not a shell that runs `sh` (cmd.exe).
    Unsupported,
    /// tmux would not list its sessions: what it said.
    Unlisted(String),
    /// The command did not run, or did not finish.
    Failed(String),
}

impl TmuxPanel {
    pub fn new(dispatch: FocusHandle) -> Self {
        Self {
            target: None,
            active: false,
            hosts: HashMap::new(),
            reading: None,
            loading: false,
            dispatch,
        }
    }

    /// Read `target`'s host when `active`, the panel showing it, and was
    /// not before.
    pub fn set_target(&mut self, target: Option<ExecTarget>, active: bool, cx: &mut Context<Self>) {
        if self.target == target && self.active == active {
            return;
        }
        self.target = target;
        self.active = active;
        self.read(None, cx);
        cx.notify();
    }

    /// Read the host again: 刷新, and after a command.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.read(None, cx);
        cx.notify();
    }

    /// Read the host again a moment after a command was typed into the
    /// terminal.
    pub fn refresh_soon(&mut self, cx: &mut Context<Self>) {
        self.read(Some(AFTER_TYPING), cx);
        cx.notify();
    }

    /// The sessions of the host on screen, as the panel last showed them.
    pub fn table(&self) -> Option<Rc<TmuxTable>> {
        let target = self.target.as_ref()?;
        self.hosts.get(&target.terminal)?.table.clone()
    }

    /// Start reading the target's host over after `delay`, or stop when
    /// there is nothing to show. A terminal not connected is asked again
    /// every little while, so its sessions come once it connects.
    fn read(&mut self, delay: Option<Duration>, cx: &mut Context<Self>) {
        self.reading = None;
        self.loading = false;
        let Some(target) = self.target.clone().filter(|_| self.active) else {
            return;
        };
        self.loading = true;
        self.reading = Some(cx.spawn(async move |this, cx| {
            if let Some(delay) = delay {
                cx.background_executor().timer(delay).await;
            }
            loop {
                let reply = this.update(cx, |_, cx| {
                    let view = target.view.upgrade()?;
                    view.read(cx).exec(linux::command(), cx)
                });
                let Ok(reply) = reply else { return };
                let answer = match reply {
                    Some(reply) => exec_answer(reply, cx).await,
                    None => None,
                };
                let connected = answer.is_some();
                if this
                    .update(cx, |this, cx| this.record(target.terminal, answer, cx))
                    .is_err()
                    || connected
                {
                    return;
                }
                cx.background_executor().timer(RETRY).await;
            }
        }));
    }

    fn record(
        &mut self,
        terminal: RemoteTerminalId,
        answer: Option<ExecResult>,
        cx: &mut Context<Self>,
    ) {
        let host = self.hosts.entry(terminal).or_default();
        host.problem = match answer {
            None => Some(Problem::NotConnected),
            Some(Err(error)) => Some(Problem::Failed(error)),
            Some(Ok(output)) => match linux::parse(&output) {
                Parsed::Table(table) => {
                    host.table = Some(Rc::new(table));
                    None
                }
                Parsed::Missing => {
                    host.table = None;
                    Some(Problem::Missing)
                }
                Parsed::Failed(why) => Some(Problem::Unlisted(why)),
                Parsed::Unsupported => Some(Problem::Unsupported),
            },
        };
        self.loading = false;
        cx.notify();
    }

    /// The version, or that there is no tmux, and 刷新; then 新建 and
    /// 退出当前, which work only with tmux there and the terminal in a
    /// session.
    fn render_header(
        &self,
        table: Option<&TmuxTable>,
        problem: Option<&Problem>,
        cx: &App,
    ) -> impl IntoElement {
        let summary: SharedString = match (table, problem) {
            (_, Some(Problem::Missing)) => t!("tmux.panel.missing"),
            (Some(table), _) => table.version.clone().into(),
            _ => SharedString::default(),
        };
        let (refresh, new, detach) = (
            self.dispatch.clone(),
            self.dispatch.clone(),
            self.dispatch.clone(),
        );
        v_flex()
            .flex_shrink_0()
            .px_3()
            .pt_3()
            .pb_3()
            .gap_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        div()
                            .id("tmux-summary")
                            .test_support()
                            .aria_label(summary.clone())
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(summary),
                    )
                    .child(
                        Button::new("tmux-refresh")
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(CatalogIcon::RefreshCw))
                            .loading(self.loading)
                            .tooltip(t!("tools.refresh"))
                            .accessibility_label(t!("tools.refresh"))
                            .on_click(move |_, window, cx| {
                                refresh.dispatch_action(&RefreshTmux, window, cx)
                            }),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("tmux-new")
                            .primary()
                            .small()
                            .flex_1()
                            .icon(Icon::new(IconName::Plus))
                            .label(t!("tmux.panel.new"))
                            .disabled(table.is_none())
                            .on_click(move |_, window, cx| {
                                new.dispatch_action(&NewTmuxSession, window, cx)
                            }),
                    )
                    .child(
                        Button::new("tmux-detach")
                            .outline()
                            .small()
                            .flex_1()
                            .icon(Icon::new(CatalogIcon::LogOut))
                            .label(t!("tmux.panel.detach"))
                            .tooltip(t!("tmux.panel.detach_tooltip"))
                            .disabled(table.and_then(TmuxTable::client).is_none())
                            .on_click(move |_, window, cx| {
                                detach.dispatch_action(&DetachTmux, window, cx)
                            }),
                    ),
            )
    }

    fn render_sessions(&self, table: &TmuxTable, cx: &App) -> AnyElement {
        if table.sessions.is_empty() {
            return render_note("tmux-empty", t!("tmux.panel.no_sessions"), cx).into_any_element();
        }
        v_flex()
            .p_3()
            .gap_2()
            .children(table.sessions.iter().map(|session| {
                let current = table.current.as_deref() == Some(session.id.as_str());
                render_session(session, current, &self.dispatch, cx)
            }))
            .into_any_element()
    }
}

impl Render for TmuxPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let host = self
            .target
            .as_ref()
            .and_then(|target| self.hosts.get(&target.terminal));
        let table = host.and_then(|host| host.table.clone());
        let problem = host.and_then(|host| host.problem.clone());
        let body = match (&table, &problem) {
            // Sessions from before a failed reading still say something;
            // the failure goes above them.
            (Some(table), None) => self.render_sessions(table, cx),
            (Some(table), Some(problem @ (Problem::Failed(_) | Problem::Unlisted(_)))) => v_flex()
                .child(div().px_3().pt_3().child(render_problem(problem, cx)))
                .child(self.render_sessions(table, cx))
                .into_any_element(),
            (_, Some(problem)) => match problem {
                Problem::Failed(_) | Problem::Unlisted(_) => div()
                    .p_3()
                    .child(render_problem(problem, cx))
                    .into_any_element(),
                _ => render_note("tmux-message", problem_text(problem), cx).into_any_element(),
            },
            (None, None) => h_flex()
                .id("tmux-loading")
                .test_support()
                .gap_2()
                .justify_center()
                .py_8()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(Spinner::new().small())
                .child(t!("tools.reading"))
                .into_any_element(),
        };
        v_flex()
            .id("tmux")
            .test_support()
            .size_full()
            .child(self.render_header(table.as_deref(), problem.as_ref(), cx))
            // The scrollbar goes on a layer beside the scrolled content,
            // which `overflow_y_scrollbar` lays out.
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .child(body)
                    .overflow_y_scrollbar(),
            )
    }
}

/// What the panel says in place of the sessions when there are none to
/// show, in the middle of it.
fn render_note(id: &'static str, text: SharedString, cx: &App) -> impl IntoElement {
    v_flex().size_full().p_3().justify_center().child(
        div()
            .id(id)
            .test_support()
            .aria_label(text.clone())
            .text_sm()
            .text_center()
            .text_color(cx.theme().muted_foreground)
            .child(text),
    )
}

fn problem_text(problem: &Problem) -> SharedString {
    match problem {
        Problem::NotConnected => t!("tmux.panel.not_connected"),
        Problem::Missing => t!("tmux.panel.missing"),
        Problem::Unsupported => t!("tmux.panel.unsupported"),
        Problem::Unlisted(why) => t!("tmux.panel.unlisted", error = why),
        Problem::Failed(error) => t!("tools.read_failed", error = error),
    }
}

fn render_problem(problem: &Problem, cx: &App) -> impl IntoElement {
    let text = problem_text(problem);
    div()
        .id("tmux-message")
        .test_support()
        .aria_label(text.clone())
        .text_sm()
        .text_color(cx.theme().danger)
        .child(text)
}

/// A session's card: its name, marked when the terminal is in it, how many
/// terminals are attached and where it started; then 连接, 重命名 and
/// 结束会话.
fn render_session(
    session: &TmuxSession,
    current: bool,
    dispatch: &FocusHandle,
    cx: &App,
) -> impl IntoElement {
    let state = if session.attached > 0 {
        t!("tmux.session.attached", count = session.attached)
    } else {
        t!("tmux.session.detached")
    };
    let mut label = vec![session.name.clone(), state.to_string()];
    if current {
        label.push(t!("tmux.session.current").to_string());
    }
    if !session.path.is_empty() {
        label.push(session.path.clone());
    }
    let badge = if session.attached > 0 {
        soft_tag(state, cx.theme().success)
    } else {
        soft_tag(state, cx.theme().muted_foreground)
    };
    let name = SharedString::from(session.name.clone());
    let path = SharedString::from(session.path.clone());
    let button = |verb: &str| SharedString::from(format!("tmux-{verb}:{}", session.name));
    let (attach, rename, kill) = (dispatch.clone(), dispatch.clone(), dispatch.clone());
    let (attach_id, rename_id, kill_id) =
        (session.id.clone(), session.id.clone(), session.id.clone());
    h_flex()
        .id(SharedString::from(format!("tmux-session:{}", session.name)))
        .test_support()
        .aria_label(label.join(" · "))
        .gap_2()
        .p_3()
        .rounded(cx.theme().radius)
        .bg(cx.theme().tokens.group_box)
        .border_1()
        .border_color(if current {
            cx.theme().border
        } else {
            cx.theme().tokens.group_box.into()
        })
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_1()
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            // Cut short if it must, whole when pointed at.
                            div()
                                .id(button("name"))
                                .min_w_0()
                                .truncate()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .child(name.clone())
                                .tooltip(move |window, cx| {
                                    Tooltip::new(name.clone()).build(window, cx)
                                }),
                        )
                        .when(current, |line| {
                            line.child(
                                Tag::secondary()
                                    .small()
                                    .rounded_full()
                                    .flex_shrink_0()
                                    .child(t!("tmux.session.current")),
                            )
                        }),
                )
                .child(h_flex().gap_2().child(badge.flex_shrink_0()).when(
                    !path.is_empty(),
                    |line| {
                        line.child(
                            div()
                                .id(button("path"))
                                .min_w_0()
                                .truncate()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(path.clone())
                                .tooltip(move |window, cx| {
                                    Tooltip::new(path.clone()).build(window, cx)
                                }),
                        )
                    },
                )),
        )
        .child(
            h_flex()
                .flex_shrink_0()
                .gap_1()
                .child(
                    Button::new(button("attach"))
                        .ghost()
                        .small()
                        .icon(Icon::new(CatalogIcon::SquareTerminal))
                        .tooltip(if current {
                            t!("tmux.session.attached_here")
                        } else {
                            t!("tmux.session.attach")
                        })
                        .accessibility_label(t!("tmux.session.attach"))
                        .disabled(current)
                        .on_click(move |_, window, cx| {
                            attach.dispatch_action(
                                &AttachTmuxSession(attach_id.clone()),
                                window,
                                cx,
                            )
                        }),
                )
                .child(
                    Button::new(button("rename"))
                        .ghost()
                        .small()
                        .icon(Icon::new(CatalogIcon::Pencil))
                        .tooltip(t!("tmux.session.rename"))
                        .accessibility_label(t!("tmux.session.rename"))
                        .on_click(move |_, window, cx| {
                            rename.dispatch_action(
                                &RenameTmuxSession(rename_id.clone()),
                                window,
                                cx,
                            )
                        }),
                )
                .child(
                    Button::new(button("kill"))
                        .small()
                        .custom(tinted(cx.theme().danger, cx))
                        .icon(Icon::new(CatalogIcon::Trash))
                        .tooltip(t!("tmux.session.kill"))
                        .accessibility_label(t!("tmux.session.kill"))
                        .on_click(move |_, window, cx| {
                            kill.dispatch_action(&KillTmuxSession(kill_id.clone()), window, cx)
                        }),
                ),
        )
}
