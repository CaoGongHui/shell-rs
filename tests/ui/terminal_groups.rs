//! Real Dock drags and broadcast controls, with an independent transport log
//! for every participant. No server, PTY or keychain is touched.

use std::collections::HashMap;

use crate::support::*;
use gpui_kit::component::dock::ToggleZoom;
use shellrs::app::{CloseSettings, PasteTerminal};

struct PerHostTerminals(HashMap<String, Arc<FakeTerminalFactory>>);

impl RemoteTerminalTransportProvider for PerHostTerminals {
    fn factory_for(&self, login: &HostLogin) -> SharedTerminalTransportFactory {
        self.0[&login.host].clone()
    }
}

struct Fixture {
    handle: WindowHandle<Root>,
    workspace: Entity<Workspace>,
    web: Arc<FakeTerminalFactory>,
    staging: Arc<FakeTerminalFactory>,
    local: Arc<FakeTerminalFactory>,
}

async fn open(cx: &mut TestAppContext) -> Fixture {
    init_app(cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    // Only web uses application cursor keys and bracketed paste.
    let web = Arc::new(FakeTerminalFactory::printing(
        "\x1b[?1h\x1b[?2004hready-web",
    ));
    let staging = Arc::new(FakeTerminalFactory::printing("ready-staging"));
    let local = Arc::new(FakeTerminalFactory::printing("ready-local"));
    let mut workspace = None;
    let handle = cx.open_window(size(px(1440.), px(900.)), |window, cx| {
        let seed = HostStore::seed();
        let remote = Arc::new(PerHostTerminals(HashMap::from([
            (
                seed.host(HostId(WEB_01)).unwrap().address.to_string(),
                web.clone(),
            ),
            (
                seed.host(HostId(STAGING_API)).unwrap().address.to_string(),
                staging.clone(),
            ),
        ])));
        let view = cx.new(|cx| {
            Workspace::new_with_services(
                cx.new(|_| seed),
                cx.new(|_| settings_store()),
                remote,
                fake_telnet_terminals(),
                fake_serial_terminals(),
                Arc::new(FixedLocalTerminalTransportProvider::new(local.clone())),
                Arc::new(FakeSftpProvider::default()),
                Arc::new(FakeLocalDirectory::default()),
                Arc::new(FakeConnectionTester::default()),
                Arc::new(FakeForwardProvider::default()),
                window,
                cx,
            )
        });
        workspace = Some(view.clone());
        Root::new(view, window, cx)
    });
    let workspace = workspace.unwrap();
    cx.wait_for(handle.into(), Duration::from_secs(2), |_, cx| {
        [INITIAL_WEB_TERMINAL, INITIAL_STAGING_TERMINAL]
            .into_iter()
            .all(|id| {
                let workspace = workspace.read(cx);
                let terminal = workspace
                    .remote_terminal(RemoteTerminalId(id))
                    .unwrap()
                    .read(cx);
                let view = terminal.terminal().read(cx);
                view.lifecycle(cx).accepts_input() && view.screen_text(cx).contains("ready-")
            })
    })
    .await;
    Fixture {
        handle,
        workspace,
        web,
        staging,
        local,
    }
}

fn split_pair(cx: &mut TestAppContext, handle: WindowHandle<Root>) {
    in_frame(cx, handle, |window, cx| {
        window.click(("terminal-tab", INITIAL_STAGING_TERMINAL), cx);
    });
    in_frame(cx, handle, |window, cx| {
        let from = window
            .find(("terminal-tab", INITIAL_STAGING_TERMINAL))
            .bounds()
            .center();
        let content = window.find(("terminal", INITIAL_STAGING_TERMINAL)).bounds();
        window.drag(
            from,
            point(content.right() - px(12.), content.center().y),
            cx,
        );
    });
    in_frame(cx, handle, |window, _| {
        let left = window.find(("terminal", INITIAL_WEB_TERMINAL));
        let right = window.find(("terminal", INITIAL_STAGING_TERMINAL));
        assert!(left.visible() && right.visible());
        assert!(left.bounds().right() <= right.bounds().left());
    });
}

fn select_pair(cx: &mut TestAppContext, handle: WindowHandle<Root>) {
    in_frame(cx, handle, |window, cx| {
        window.click("toggle-terminal-broadcast", cx)
    });
    in_frame(cx, handle, |window, cx| {
        for id in [INITIAL_WEB_TERMINAL, INITIAL_STAGING_TERMINAL] {
            assert_eq!(window.find(("broadcast-remote", id)).checked(), Some(false));
            window.click(("broadcast-remote", id), cx);
        }
    });
    in_frame(cx, handle, |window, _| {
        assert_eq!(
            window.find("toggle-terminal-broadcast").label(),
            Some("广播中 · 2 个终端")
        );
    });
}

#[gpui_kit::test]
async fn permanent_toggle_replaces_logos_without_adding_a_toolbar(cx: &mut TestAppContext) {
    let fixture = open(cx).await;
    let handle = fixture.handle;
    in_frame(cx, handle, |window, cx| {
        assert!(window.try_find("terminal-controls").is_none());
        assert!(window.try_find("split-terminal-right").is_none());
        assert!(window.try_find("start-terminal-broadcast").is_none());
        let hide = window.find("toggle-dock:Left").bounds();
        let broadcast = window.find("toggle-terminal-broadcast").bounds();
        let tab = window.find(("terminal-tab", INITIAL_WEB_TERMINAL)).bounds();
        assert!(hide.right() <= broadcast.left());
        assert!(broadcast.right() <= tab.left());
        assert!((hide.center().y - broadcast.center().y).abs() <= px(1.));
        assert!(
            window
                .find(("terminal-tab-os", INITIAL_WEB_TERMINAL))
                .visible()
        );
        window.click("toggle-terminal-broadcast", cx);
    });
    let active = cx.update(|cx| fixture.workspace.read(cx).active_tab());
    in_frame(cx, handle, |window, cx| {
        assert!(
            window
                .try_find(("terminal-tab-os", INITIAL_WEB_TERMINAL))
                .is_none()
        );
        assert_eq!(
            window
                .find(("broadcast-remote", INITIAL_WEB_TERMINAL))
                .checked(),
            Some(false)
        );
        window.click(("broadcast-remote", INITIAL_WEB_TERMINAL), cx);
    });
    assert_eq!(
        cx.update(|cx| fixture.workspace.read(cx).active_tab()),
        active
    );
    in_frame(cx, handle, |window, cx| {
        window.click("toggle-terminal-broadcast", cx)
    });
    in_frame(cx, handle, |window, _| {
        assert!(
            window
                .find(("terminal-tab-os", INITIAL_WEB_TERMINAL))
                .visible()
        );
        assert!(
            window
                .try_find(("broadcast-remote", INITIAL_WEB_TERMINAL))
                .is_none()
        );
    });
    // Still present with one tab, and on the empty start page.
    for id in [INITIAL_WEB_TERMINAL, INITIAL_STAGING_TERMINAL] {
        in_frame(cx, handle, |window, cx| {
            window.click(("close-terminal", id), cx)
        });
        in_frame(cx, handle, |window, _| {
            assert!(window.find("toggle-terminal-broadcast").visible())
        });
    }
}

#[gpui_kit::test]
async fn broadcast_reaches_background_tabs_and_encodes_each_terminal_mode(cx: &mut TestAppContext) {
    let fixture = open(cx).await;
    let handle = fixture.handle;
    in_frame(cx, handle, |window, cx| {
        window.click(("terminal-tab", INITIAL_WEB_TERMINAL), cx);
    });
    in_frame(cx, handle, |window, cx| window.input("solo", cx));
    cx.wait_for(handle.into(), Duration::from_secs(2), |_, _| {
        fixture.web.written_text() == "solo"
    })
    .await;
    assert_eq!(fixture.staging.written_text(), "");
    select_pair(cx, handle);
    in_frame(cx, handle, |window, cx| {
        window.dispatch_action(Box::new(FindInTerminal), cx)
    });
    in_frame(cx, handle, |window, cx| {
        window.input("search-only", cx);
        window.press("escape", cx);
    });
    assert_eq!(fixture.web.written_text(), "solo");
    assert_eq!(fixture.staging.written_text(), "");
    in_frame(cx, handle, |window, cx| {
        window.click(("terminal", INITIAL_WEB_TERMINAL), cx);
        window.input("中文", cx);
        window.press("up", cx);
        window.press("enter", cx);
        window.press("ctrl-c", cx);
        cx.write_to_clipboard(ClipboardItem::new_string("one\ntwo".into()));
        window.dispatch_action(Box::new(PasteTerminal), cx);
    });
    cx.wait_for(handle.into(), Duration::from_secs(2), |_, _| {
        fixture.staging.written_text().ends_with("one\ntwo")
            && fixture.web.written_text().ends_with("\x1b[201~")
    })
    .await;
    assert_eq!(
        fixture.web.written_text(),
        "solo中文\x1bOA\r\x03\x1b[200~one\ntwo\x1b[201~"
    );
    assert_eq!(fixture.staging.written_text(), "中文\x1b[A\r\x03one\ntwo");
    in_frame(cx, handle, |window, cx| {
        window.click(("terminal-tab", INITIAL_STAGING_TERMINAL), cx)
    });
    in_frame(cx, handle, |window, cx| window.input("from-b", cx));
    cx.wait_for(handle.into(), Duration::from_secs(2), |_, _| {
        fixture.web.written_text().ends_with("from-b")
    })
    .await;
    in_frame(cx, handle, |window, cx| {
        window.click("toggle-terminal-broadcast", cx)
    });
    in_frame(cx, handle, |window, cx| window.input("alone", cx));
    cx.wait_for(handle.into(), Duration::from_secs(2), |_, _| {
        fixture.staging.written_text().ends_with("alone")
    })
    .await;
    assert!(fixture.web.written_text().ends_with("from-b"));
}

#[gpui_kit::test]
async fn checking_local_tabs_is_immediate_and_unchecking_stops_broadcast(cx: &mut TestAppContext) {
    let fixture = open(cx).await;
    let handle = fixture.handle;
    in_frame(cx, handle, |window, cx| {
        window.click("new-local-terminal", cx)
    });
    cx.wait_for(handle.into(), Duration::from_secs(2), |_, cx| {
        fixture
            .workspace
            .read(cx)
            .local_terminal(LocalTerminalId(1))
            .is_some_and(|panel| {
                panel
                    .read(cx)
                    .terminal()
                    .read(cx)
                    .lifecycle(cx)
                    .accepts_input()
            })
    })
    .await;
    in_frame(cx, handle, |window, cx| {
        window.click("toggle-terminal-broadcast", cx)
    });
    in_frame(cx, handle, |window, cx| {
        window.click(("broadcast-local", 1_u64), cx)
    });
    in_frame(cx, handle, |window, cx| window.input("only-local", cx));
    cx.wait_for(handle.into(), Duration::from_secs(2), |_, _| {
        fixture.local.written_text() == "only-local"
    })
    .await;
    assert_eq!(fixture.web.written_text(), "");
    in_frame(cx, handle, |window, cx| {
        window.click(("broadcast-remote", INITIAL_WEB_TERMINAL), cx)
    });
    in_frame(cx, handle, |window, cx| window.input("shared", cx));
    cx.wait_for(handle.into(), Duration::from_secs(2), |_, _| {
        fixture.web.written_text() == "shared"
    })
    .await;
    assert_eq!(fixture.staging.written_text(), "");
    in_frame(cx, handle, |window, cx| {
        window.click(("broadcast-remote", INITIAL_WEB_TERMINAL), cx)
    });
    in_frame(cx, handle, |window, cx| window.input("unshared", cx));
    cx.wait_for(handle.into(), Duration::from_secs(2), |_, _| {
        fixture.local.written_text().ends_with("unshared")
    })
    .await;
    assert_eq!(fixture.web.written_text(), "shared");
    // Opening a terminal while enabled does not select it automatically.
    in_frame(cx, handle, |window, cx| {
        window.click("new-local-terminal", cx)
    });
    cx.wait_for(handle.into(), Duration::from_secs(2), |window, _| {
        window.try_find(("broadcast-local", 2_u64)).is_some()
    })
    .await;
    in_frame(cx, handle, |window, _| {
        assert_eq!(
            window.find(("broadcast-local", 2_u64)).checked(),
            Some(false)
        )
    });
    in_frame(cx, handle, |window, cx| {
        window.click(("close-local-terminal", 1_u64), cx)
    });
    in_frame(cx, handle, |window, _| {
        assert_eq!(
            window.find("toggle-terminal-broadcast").label(),
            Some("命令广播 · 勾选至少两个终端")
        )
    });
}

#[gpui_kit::test]
async fn hidden_tabs_stay_selected_but_reconnected_tabs_leave_broadcast(cx: &mut TestAppContext) {
    let fixture = open(cx).await;
    let handle = fixture.handle;
    select_pair(cx, handle);
    in_frame(cx, handle, |window, cx| {
        window.dispatch_action(Box::new(OpenSettings), cx)
    });
    in_frame(cx, handle, |window, _| {
        assert_eq!(
            window.find("toggle-terminal-broadcast").label(),
            Some("广播中 · 2 个终端")
        )
    });
    in_frame(cx, handle, |window, cx| {
        window.dispatch_action(Box::new(CloseSettings), cx)
    });
    in_frame(cx, handle, |window, cx| {
        window.click(("terminal-tab", INITIAL_WEB_TERMINAL), cx);
    });
    in_frame(cx, handle, |window, cx| {
        window.dispatch_action(
            Box::new(ReconnectTerminal(RemoteTerminalId(
                INITIAL_STAGING_TERMINAL,
            ))),
            cx,
        );
    });
    cx.wait_for(handle.into(), Duration::from_secs(2), |_, cx| {
        fixture.staging.starts() == 2
            && fixture
                .workspace
                .read(cx)
                .remote_terminal(RemoteTerminalId(INITIAL_STAGING_TERMINAL))
                .unwrap()
                .read(cx)
                .terminal()
                .read(cx)
                .lifecycle(cx)
                .accepts_input()
    })
    .await;
    in_frame(cx, handle, |window, cx| {
        assert_eq!(
            window
                .find(("broadcast-remote", INITIAL_STAGING_TERMINAL))
                .checked(),
            Some(false)
        );
        window.click(("terminal-tab", INITIAL_WEB_TERMINAL), cx);
    });
    in_frame(cx, handle, |window, cx| {
        window.click(("terminal", INITIAL_WEB_TERMINAL), cx);
        window.input("after-reconnect", cx);
    });
    cx.wait_for(handle.into(), Duration::from_secs(2), |_, _| {
        fixture.web.written_text() == "after-reconnect"
    })
    .await;
    assert_eq!(fixture.staging.written_text(), "");
}

#[gpui_kit::test]
async fn dragging_and_zooming_preserve_broadcast_membership_and_connections(
    cx: &mut TestAppContext,
) {
    let fixture = open(cx).await;
    let handle = fixture.handle;
    select_pair(cx, handle);
    split_pair(cx, handle);
    in_frame(cx, handle, |window, cx| {
        window.click(("terminal", INITIAL_WEB_TERMINAL), cx);
        window.dispatch_action(Box::new(ToggleZoom), cx);
    });
    in_frame(cx, handle, |window, cx| {
        assert_eq!(
            window.find("toggle-terminal-broadcast").label(),
            Some("广播中 · 2 个终端")
        );
        window.input("zoomed", cx);
        window.dispatch_action(Box::new(ToggleZoom), cx);
    });
    cx.wait_for(handle.into(), Duration::from_secs(2), |_, _| {
        fixture.staging.written_text() == "zoomed"
    })
    .await;
    in_frame(cx, handle, |window, cx| {
        let from = window
            .find(("terminal-tab", INITIAL_STAGING_TERMINAL))
            .bounds()
            .center();
        let to = window
            .find(("terminal", INITIAL_WEB_TERMINAL))
            .bounds()
            .center();
        window.drag(from, to, cx);
    });
    in_frame(cx, handle, |window, _| {
        let shown = [INITIAL_WEB_TERMINAL, INITIAL_STAGING_TERMINAL]
            .into_iter()
            .filter(|id| {
                window
                    .try_find(("terminal", *id))
                    .is_some_and(|element| element.visible())
            })
            .count();
        assert_eq!(shown, 1);
        assert_eq!(
            window.find("toggle-terminal-broadcast").label(),
            Some("广播中 · 2 个终端")
        );
        assert_eq!(
            window
                .find(("broadcast-remote", INITIAL_STAGING_TERMINAL))
                .checked(),
            Some(true)
        );
    });
    assert_eq!(fixture.web.starts(), 1);
    assert_eq!(fixture.staging.starts(), 1);
}
