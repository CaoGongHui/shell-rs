//! tmux: the right sidebar's tool for the host's tmux sessions, read over
//! the SSH terminal's own connection.

use shellrs::app::{AttachTmuxSession, DetachTmux};

use crate::support::*;

/// web-01's terminal, on pts/3, attached to `test`; `work` detached.
const IN_TMUX: &str = "\
@@version
tmux 3.2a
@@tty
pts/3
@@clients
/dev/pts/3 $1
@@sessions
$1:1:3:test:/root
$2:0:1:work:/srv/app
@@status 0
";

/// The same sessions, the terminal in neither.
const OUTSIDE: &str = "\
@@version
tmux 3.2a
@@tty
pts/3
@@clients
/dev/pts/7 $1
@@sessions
$1:1:3:test:/root
$2:0:1:work:/srv/app
@@status 0
";

/// What the commands beside the shell print when they went.
const WENT: &str = "@@status 0\n";

/// Show tmux for web-01's terminal: it is on offer from the start.
fn open_tmux(cx: &mut TestAppContext, factory: Arc<FakeTerminalFactory>) -> WindowHandle<Root> {
    let (handle, _) = open_workspace_with_remote_factory(cx, HostStore::seed(), factory);
    in_frame(cx, handle, |window, cx| {
        window.click(("terminal-tab", INITIAL_WEB_TERMINAL), cx)
    });
    in_frame(cx, handle, |window, cx| window.click("tool-tmux", cx));
    handle
}

async fn wait_for_label(
    cx: &mut TestAppContext,
    handle: WindowHandle<Root>,
    id: impl Into<ElementId> + Clone,
    label: &str,
) {
    cx.wait_for(handle.into(), Duration::from_secs(5), |window, _| {
        window
            .try_find(id.clone())
            .is_some_and(|element| element.label() == Some(label))
    })
    .await;
}

/// The fake terminal records a command on its own thread: wait for it to
/// have `count`.
async fn wait_for_execs(
    cx: &mut TestAppContext,
    handle: WindowHandle<Root>,
    factory: &FakeTerminalFactory,
    count: usize,
) {
    cx.wait_for(handle.into(), Duration::from_secs(5), |_, _| {
        factory.exec_count() >= count
    })
    .await;
    assert_eq!(factory.exec_count(), count, "{:?}", factory.exec_commands());
}

/// The fake terminal records what is typed on its own thread too.
async fn wait_for_typed(
    cx: &mut TestAppContext,
    handle: WindowHandle<Root>,
    factory: &FakeTerminalFactory,
    before: usize,
    typed: &str,
) {
    cx.wait_for(handle.into(), Duration::from_secs(5), |_, _| {
        factory.written_text().len() >= before + typed.len()
    })
    .await;
    assert_eq!(&factory.written_text()[before..], typed);
}

fn session(name: &str) -> ElementId {
    ElementId::Name(format!("tmux-session:{name}").into())
}

fn button(verb: &str, name: &str) -> ElementId {
    ElementId::Name(format!("tmux-{verb}:{name}").into())
}

fn terminal_focused(window: &mut gpui_kit::Window) -> bool {
    window.find(("terminal", INITIAL_WEB_TERMINAL)).focused() == Some(true)
}

#[gpui_kit::test]
async fn the_sessions_show_with_the_one_the_terminal_is_in(cx: &mut TestAppContext) {
    let factory = Arc::new(FakeTerminalFactory::answering(&[IN_TMUX]));
    let handle = open_tmux(cx, factory.clone());
    wait_for_label(cx, handle, "tmux-summary", "tmux 3.2a").await;
    in_frame(cx, handle, |window, _| {
        assert_eq!(
            window.find(("tool-sidebar", INITIAL_WEB_TERMINAL)).label(),
            Some("tmux")
        );
        assert_eq!(
            window.find(session("test")).label(),
            Some("test · 已连接 1 · 当前 · /root")
        );
        assert_eq!(
            window.find(session("work")).label(),
            Some("work · 未连接 · /srv/app")
        );
        // The terminal is in `test` already, and can leave it.
        assert!(!enabled(&window.find(button("attach", "test"))));
        assert!(enabled(&window.find(button("attach", "work"))));
        assert!(enabled(&window.find("tmux-new")));
        assert!(enabled(&window.find("tmux-detach")));
    });
    assert_eq!(factory.exec_count(), 1);
    assert!(factory.exec_commands()[0].contains("tmux -u list-sessions"));

    // Read again on 刷新, and only then.
    in_frame(cx, handle, |window, cx| window.click("tmux-refresh", cx));
    wait_for_execs(cx, handle, &factory, 2).await;
}

#[gpui_kit::test]
async fn a_host_without_tmux_says_so(cx: &mut TestAppContext) {
    let factory = Arc::new(FakeTerminalFactory::answering(&["@@missing\n"]));
    let handle = open_tmux(cx, factory.clone());
    wait_for_label(cx, handle, "tmux-message", "远端未安装 tmux").await;
    in_frame(cx, handle, |window, _| {
        assert_eq!(window.find("tmux-summary").label(), Some("远端未安装 tmux"));
        assert!(!enabled(&window.find("tmux-new")));
        assert!(!enabled(&window.find("tmux-detach")));
        // In the middle of the panel, under the buttons.
        let panel = window.find("tmux").bounds();
        let buttons = window.find("tmux-new").bounds();
        let message = window.find("tmux-message").bounds();
        assert!(message.top() > buttons.bottom(), "{message:?} {buttons:?}");
        let middle = (panel.top() + panel.bottom()) / 2.;
        assert!(
            message.top() > panel.top() + (middle - panel.top()) / 2.
                && message.bottom() < middle + (panel.bottom() - middle) / 2.,
            "{message:?} in {panel:?}"
        );
    });
}

#[gpui_kit::test]
async fn a_terminal_not_in_tmux_is_attached_by_typing_into_it(cx: &mut TestAppContext) {
    let factory = Arc::new(FakeTerminalFactory::answering(&[OUTSIDE]));
    let handle = open_tmux(cx, factory.clone());
    wait_for_label(cx, handle, "tmux-summary", "tmux 3.2a").await;
    in_frame(cx, handle, |window, _| {
        assert!(enabled(&window.find(button("attach", "test"))));
        // Not in a session: nothing to detach from.
        assert!(!enabled(&window.find("tmux-detach")));
    });
    let before = factory.written_text().len();

    in_frame(cx, handle, |window, cx| {
        window.click(button("attach", "work"), cx)
    });
    wait_for_typed(
        cx,
        handle,
        &factory,
        before,
        "\x05\x15tmux attach -t work\r",
    )
    .await;
    in_frame(cx, handle, |window, _| assert!(terminal_focused(window)));
    // Nothing beside the shell but the reading; another a moment later,
    // once tmux has attached.
    assert_eq!(factory.exec_count(), 1);
    cx.executor().advance_clock(Duration::from_secs(1));
    wait_for_execs(cx, handle, &factory, 2).await;
}

#[gpui_kit::test]
async fn a_terminal_in_tmux_is_switched_and_detached_beside_the_shell(cx: &mut TestAppContext) {
    let factory = Arc::new(FakeTerminalFactory::answering(&[
        IN_TMUX, WENT, IN_TMUX, WENT, IN_TMUX,
    ]));
    let handle = open_tmux(cx, factory.clone());
    wait_for_label(cx, handle, "tmux-summary", "tmux 3.2a").await;
    let typed = factory.written_text();

    in_frame(cx, handle, |window, cx| {
        window.dispatch_action(Box::new(AttachTmuxSession("$2".into())), cx)
    });
    wait_for_execs(cx, handle, &factory, 3).await;
    let commands = factory.exec_commands();
    assert_eq!(
        commands[1],
        "sh -c 'export PATH=$PATH:/usr/local/bin:/opt/homebrew/bin; \
         tmux switch-client -c /dev/pts/3 -t \\$2 2>&1; echo @@status $?'"
    );
    assert!(commands[2].contains("tmux -u list-sessions"));
    in_frame(cx, handle, |window, _| assert!(terminal_focused(window)));

    in_frame(cx, handle, |window, cx| {
        window.dispatch_action(Box::new(DetachTmux), cx)
    });
    wait_for_execs(cx, handle, &factory, 5).await;
    assert!(
        factory.exec_commands()[3].contains("tmux detach-client -t /dev/pts/3 2>&1"),
        "{:?}",
        factory.exec_commands()
    );
    // Nothing typed: the keys would have gone to the program in the pane.
    assert_eq!(factory.written_text(), typed);
}

#[gpui_kit::test]
async fn new_asks_for_a_name_then_starts_the_session(cx: &mut TestAppContext) {
    let factory = Arc::new(FakeTerminalFactory::answering(&[OUTSIDE]));
    let handle = open_tmux(cx, factory.clone());
    wait_for_label(cx, handle, "tmux-summary", "tmux 3.2a").await;

    // A name the shell would need quoted, then one taken: said, and the
    // dialog stays.
    in_frame(cx, handle, |window, cx| window.click("tmux-new", cx));
    for (name, error) in [
        ("my job", "名称只能用字母、数字、- 和 _，且不能以 - 开头"),
        ("work", "已有名为“work”的会话"),
    ] {
        in_frame(cx, handle, |window, cx| {
            window.click("tmux-session-name", cx);
            window.press("secondary-a", cx);
            window.input(name, cx);
            window.click("commit", cx);
        });
        cx.wait_for(handle.into(), Duration::from_secs(2), |window, _| {
            window
                .try_find("form-error")
                .is_some_and(|shown| shown.label() == Some(error))
        })
        .await;
    }

    let before = factory.written_text().len();
    in_frame(cx, handle, |window, cx| {
        window.click("tmux-session-name", cx);
        window.press("secondary-a", cx);
        window.input("build", cx);
        window.click("commit", cx);
    });
    wait_for_typed(cx, handle, &factory, before, "\x05\x15tmux new -s build\r").await;
    in_frame(cx, handle, |window, _| {
        assert!(window.try_find("tmux-session-name").is_none());
        assert!(terminal_focused(window));
    });

    // No name: tmux numbers it.
    let before = factory.written_text().len();
    in_frame(cx, handle, |window, cx| window.click("tmux-new", cx));
    in_frame(cx, handle, |window, cx| window.click("commit", cx));
    wait_for_typed(cx, handle, &factory, before, "\x05\x15tmux new\r").await;
}

#[gpui_kit::test]
async fn a_full_screen_program_keeps_the_keys(cx: &mut TestAppContext) {
    let factory = Arc::new(FakeTerminalFactory {
        behavior: FakeBehavior::Prints("\x1b[?1049h"),
        ..FakeTerminalFactory::answering(&[OUTSIDE])
    });
    let handle = open_tmux(cx, factory.clone());
    wait_for_label(cx, handle, "tmux-summary", "tmux 3.2a").await;
    let typed = factory.written_text();
    in_frame(cx, handle, |window, cx| {
        window.click(button("attach", "work"), cx)
    });
    in_frame(cx, handle, |window, cx| {
        assert_eq!(window.notifications(cx).len(), 1);
    });
    assert_eq!(factory.written_text(), typed);
}

#[gpui_kit::test]
async fn a_session_is_renamed_and_ended_beside_the_shell(cx: &mut TestAppContext) {
    let factory = Arc::new(FakeTerminalFactory::answering(&[
        IN_TMUX, WENT, IN_TMUX, WENT, IN_TMUX,
    ]));
    let handle = open_tmux(cx, factory.clone());
    wait_for_label(cx, handle, "tmux-summary", "tmux 3.2a").await;

    // 重命名, from its name.
    in_frame(cx, handle, |window, cx| {
        window.click(button("rename", "work"), cx)
    });
    in_frame(cx, handle, |window, cx| {
        window.press("secondary-a", cx);
        window.input("train", cx);
        window.click("commit", cx);
    });
    wait_for_execs(cx, handle, &factory, 3).await;
    assert!(
        factory.exec_commands()[1].contains("tmux rename-session -t \\$2 train 2>&1"),
        "{:?}",
        factory.exec_commands()
    );

    // 结束会话 asks first.
    in_frame(cx, handle, |window, cx| {
        window.click(button("kill", "test"), cx)
    });
    assert_eq!(factory.exec_count(), 3);
    in_frame(cx, handle, |window, cx| window.click("ok", cx));
    wait_for_execs(cx, handle, &factory, 5).await;
    assert!(
        factory.exec_commands()[3].contains("tmux kill-session -t \\$1 2>&1"),
        "{:?}",
        factory.exec_commands()
    );
    in_frame(cx, handle, |window, cx| {
        assert_eq!(window.notifications(cx).len(), 1);
    });
}
