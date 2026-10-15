//! Serial hosts: the host form's 串口, their terminals on a serial port of
//! this computer, and what an SSH host has that they have not.

use crate::support::*;
use shellrs::app::SendBreak;
use shellrs::host::{FlowControl, Protocol, SerialSettings};

/// A store with one serial host, `console` on `/dev/ttyUSB0`.
fn serial_store() -> (HostStore, HostId) {
    let mut store = HostStore::empty();
    let id = store.insert_unnotified(HostDraft::serial(
        "console",
        "/dev/ttyUSB0",
        SerialSettings::default(),
        None,
    ));
    (store, id)
}

async fn connect(cx: &mut TestAppContext, handle: WindowHandle<Root>, id: HostId) {
    in_frame(cx, handle, |window, cx| {
        window.dispatch_action(Box::new(ConnectHost(id)), cx)
    });
    cx.wait_for(handle.into(), Duration::from_secs(2), |window, cx| {
        window.render_frame(cx);
        window
            .try_find("status-connection")
            .is_some_and(|status| status.label() == Some("已连接 console"))
    })
    .await;
}

fn replace_text(window: &mut gpui_kit::Window, id: &'static str, text: &str, cx: &mut App) {
    window.click(id, cx);
    window.press("secondary-a", cx);
    window.input(text, cx);
}

#[gpui_kit::test]
async fn a_serial_host_is_made_from_its_device_and_line(cx: &mut TestAppContext) {
    let tester = Arc::new(FakeConnectionTester::default());
    let (handle, workspace) = open_workspace_with_tester(cx, HostStore::empty(), tester.clone());
    in_frame(cx, handle, |window, cx| window.click("new-host-panel", cx));
    in_frame(cx, handle, |window, cx| {
        // A new host is an SSH host.
        assert_eq!(
            window.within("host-protocol").find(0usize).selected(),
            Some(true)
        );
        assert!(window.find("host-address").visible());
        window.within("host-protocol").click(2usize, cx);
    });
    in_frame(cx, handle, |window, cx| {
        // Nothing of an SSH host's: no address, login or route.
        for id in [
            "host-address",
            "host-port",
            "host-user",
            "host-auth-source",
            "host-route",
        ] {
            assert!(window.try_find(id).is_none(), "{id}");
        }
        // The device and its rate share a row; how each character is
        // framed takes the next, four across.
        let [name, device, baud] = ["host-name", "host-serial-device", "host-serial-baud"]
            .map(|id| window.find(id).bounds());
        assert_eq!(device.top(), baud.top());
        assert!(device.size.width > baud.size.width);
        // The rate takes a third of the row, more than the quarter each of
        // the framing gets, so 115200 fits beside its menu.
        assert_eq!((device.left(), baud.right()), (name.left(), name.right()));
        assert!(baud.size.width > name.size.width / 4.);
        let framing = [
            "host-serial-data-bits",
            "host-serial-parity",
            "host-serial-stop-bits",
            "host-serial-flow",
        ]
        .map(|id| window.find(id).bounds());
        assert!(
            framing
                .iter()
                .all(|field| field.top() == framing[0].top() && field.top() > device.bottom())
        );
        assert_eq!(window.find("host-serial-baud").value(), Some("115200"));
        assert_eq!(window.find("host-serial-flow").value(), Some("无"));
        window.click("host-name", cx);
        window.click("commit", cx);
    });
    cx.wait_for(handle.into(), Duration::from_secs(2), |window, _| {
        window
            .try_find("form-error")
            .is_some_and(|error| error.visible())
    })
    .await;
    in_frame(cx, handle, |window, cx| {
        assert_eq!(window.find("form-error").label(), Some("请输入串口设备"));
        window.click("host-serial-device", cx);
        window.input("/dev/ttyUSB0", cx);
        replace_text(window, "host-serial-baud", "9600", cx);
        window.within("host-serial-flow").click("input", cx);
    });
    for key in ["down", "enter"] {
        in_frame(cx, handle, |window, cx| window.press(key, cx));
    }
    in_frame(cx, handle, |window, _| {
        assert_eq!(window.find("host-serial-flow").value(), Some("RTS/CTS"));
    });
    // 测试连接 opens the port with what the form shows, and says how it went
    // beside the form's error from before.
    let shown = in_frame(cx, handle, |window, cx| {
        window.click("test-connection", cx);
        window.notifications(cx).len()
    });
    cx.wait_for(handle.into(), Duration::from_secs(2), |window, cx| {
        window.render_frame(cx);
        window.notifications(cx).len() == shown + 1
    })
    .await;
    let line = SerialSettings {
        baud: 9600,
        flow: FlowControl::RtsCts,
        ..SerialSettings::default()
    };
    assert_eq!(
        tester.serial_lines(),
        [SerialLine {
            device: "/dev/ttyUSB0".into(),
            settings: line,
        }]
    );
    assert!(tester.requests().is_empty());
    in_frame(cx, handle, |window, cx| window.click("commit", cx));
    wait_for_dialog_to_close(cx, handle).await;
    cx.update(|cx| {
        let host = &workspace.read(cx).store().read(cx).hosts()[0];
        assert_eq!(host.name.as_ref(), "/dev/ttyUSB0");
        assert_eq!(host.address.as_ref(), "/dev/ttyUSB0");
        assert_eq!(host.protocol, Protocol::Serial(line));
    });
}

#[gpui_kit::test]
async fn a_serial_host_keeps_its_protocol_and_reconnects_at_a_new_rate(cx: &mut TestAppContext) {
    let (store, id) = serial_store();
    let serial = fake_serial_terminals();
    let (handle, workspace) = open_workspace_with_terminals(
        cx,
        store,
        Arc::new(FakeTerminalFactory::default()),
        serial.clone(),
    );
    connect(cx, handle, id).await;
    assert_eq!(
        serial.lines(),
        [SerialLine {
            device: "/dev/ttyUSB0".into(),
            settings: SerialSettings::default(),
        }]
    );

    in_frame(cx, handle, |window, cx| {
        window.dispatch_action(Box::new(EditHost(id)), cx)
    });
    in_frame(cx, handle, |window, cx| {
        let mut protocols = window.within("host-protocol");
        assert_eq!(protocols.find(2usize).selected(), Some(true));
        // SSH cannot be chosen any more.
        protocols.click(0usize, cx);
    });
    in_frame(cx, handle, |window, cx| {
        assert_eq!(
            window.within("host-protocol").find(2usize).selected(),
            Some(true)
        );
        assert_eq!(
            window.find("host-serial-device").value(),
            Some("/dev/ttyUSB0")
        );
        replace_text(window, "host-serial-baud", "57600", cx);
        window.click("commit", cx);
    });
    wait_for_dialog_to_close(cx, handle).await;
    cx.wait_for(handle.into(), Duration::from_secs(2), |_, _| {
        serial.lines().len() == 2
    })
    .await;
    assert_eq!(serial.lines()[1].settings.baud, 57600);

    // A new name alone does not reconnect.
    let store = cx.update(|cx| workspace.read(cx).store().clone());
    cx.update(|cx| {
        store.update(cx, |store, cx| {
            let mut draft = store.host(id).unwrap().draft();
            draft.name = "控制台".into();
            assert!(store.update(id, draft, cx));
        })
    });
    cx.run_until_parked();
    assert_eq!(serial.lines().len(), 2);
}

#[gpui_kit::test]
async fn a_serial_terminal_has_no_sftp_and_only_snippets_beside_it(cx: &mut TestAppContext) {
    let (store, id) = serial_store();
    let line = Arc::new(FakeTerminalFactory::default());
    let (handle, _) = open_workspace_with_terminals(
        cx,
        store,
        Arc::new(FakeTerminalFactory::default()),
        Arc::new(FixedSerialTerminalTransportProvider::new(line.clone())),
    );
    connect(cx, handle, id).await;
    in_frame(cx, handle, |window, _| {
        assert!(window.try_find(("sftp", 1_u64)).is_none());
        assert!(window.find(("reconnect", 1_u64)).visible());
        assert_eq!(
            window.find(("terminal-tab-os", 1_u64)).label(),
            Some("串口")
        );
        // Snippets type into the terminal, which a serial line takes; the
        // rest run commands beside it, which it has no room for.
        assert!(window.find("tool-snippets").visible());
        for tool in [
            "tool-history",
            "tool-files",
            "tool-tmux",
            "tool-processes",
            "tool-monitor",
        ] {
            assert!(window.try_find(tool).is_none(), "{tool}");
        }
    });

    in_frame(cx, handle, |window, cx| {
        window.dispatch_action(Box::new(SendBreak(RemoteTerminalId(1))), cx)
    });
    cx.wait_for(handle.into(), Duration::from_secs(2), |_, _| {
        line.breaks() == 1
    })
    .await;
}

#[gpui_kit::test]
async fn a_serial_port_is_connected_to_without_saving_it(cx: &mut TestAppContext) {
    let serial = fake_serial_terminals();
    let (handle, workspace) = open_workspace_with_terminals(
        cx,
        HostStore::empty(),
        Arc::new(FakeTerminalFactory::default()),
        serial.clone(),
    );
    in_frame(cx, handle, |window, cx| {
        window.click("temporary-connection", cx)
    });
    in_frame(cx, handle, |window, cx| {
        window.within("host-protocol").click(2usize, cx)
    });
    in_frame(cx, handle, |window, cx| {
        window.click("host-serial-device", cx);
        window.input("COM3", cx);
        window.click("commit", cx);
    });
    cx.wait_for(handle.into(), Duration::from_secs(2), |_, _| {
        serial.lines().len() == 1
    })
    .await;
    assert_eq!(serial.lines()[0].device, "COM3");
    cx.update(|cx| {
        let store = workspace.read(cx).store().read(cx);
        assert!(store.hosts().is_empty());
        assert_eq!(store.temporary_hosts().len(), 1);
    });
}
