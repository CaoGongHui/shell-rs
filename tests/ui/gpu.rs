//! GPU/NPU: the right sidebar's tool for an AI host's accelerators, read
//! over the SSH terminal's own connection; and 侧栏功能, which offers it.

use shellrs::app::{EndProcess, RefreshGpu, ToolKind};

use crate::support::*;

/// Eight H100s, the first four busy with a job of alice's, which `ps`
/// names; a fifth process `ps` could not see.
fn eight_gpus() -> String {
    let mut output = String::from("@@uptime\n5359162.33\n@@nvidia\n");
    for index in 0..8u32 {
        let busy = index < 4;
        output += &format!(
            "{index}, GPU-{index:08}-0000-0000-0000-000000000000, NVIDIA H100 80GB HBM3, \
             00000000:{:02X}:00.0, 550.54.15, {}, {}, {}, 81559, {}, {}, 700.00, [N/A], 0\n",
            0x18 + index,
            if busy { "P0" } else { "P8" },
            if busy { 96 } else { 0 },
            if busy { 61440 } else { 1 },
            if busy { 70 } else { 30 },
            if busy { "600.00" } else { "70.00" },
        );
    }
    output += "@@status 0\n@@nvidia-apps\n";
    for index in 0..4u32 {
        output += &format!(
            "GPU-{index:08}-0000-0000-0000-000000000000, {}, 60000, /usr/bin/python3\n",
            4242 + index
        );
    }
    output += "GPU-00000001-0000-0000-0000-000000000000, 999, 1024, /opt/infer\n";
    output += "@@status 0\n@@ps\n";
    for index in 0..4u32 {
        output += &format!("   {} alice    torchrun train.py\n", 4242 + index);
    }
    output += "@@end\n";
    output
}

/// What the panel says of the eight, up 62 days and some.
const EIGHT_SUMMARY: &str = "共 8 个设备 · 已运行 62 天 00:39:22";

/// One Ascend 910B3 of an Atlas 800T A2, idle.
const ONE_NPU: &str = "\
@@npu
+------------------------------------------------------------------------------------------------+
| npu-smi 24.1.0.3                 Version: 24.1.0.3                                             |
+---------------------------+---------------+----------------------------------------------------+
| NPU   Name                | Health        | Power(W)    Temp(C)           Hugepages-Usage(page)|
| Chip                      | Bus-Id        | AICore(%)   Memory-Usage(MB)  HBM-Usage(MB)        |
+===========================+===============+====================================================+
| 0     910B3               | OK            | 92.9        37                0    / 0             |
| 0                         | 0000:C1:00.0  | 0           0    / 0          3379 / 65536         |
+===========================+===============+====================================================+
+---------------------------+---------------+----------------------------------------------------+
| NPU     Chip              | Process id    | Process name             | Process memory(MB)      |
+===========================+===============+====================================================+
| No running processes found in NPU 0                                                            |
+===========================+===============+====================================================+
@@status 0
@@end
";

/// Turn GPU/NPU on in 侧栏功能, then show it for web-01's terminal.
fn open_gpu(
    cx: &mut TestAppContext,
    factory: Arc<FakeTerminalFactory>,
) -> (WindowHandle<Root>, Entity<Workspace>) {
    let (handle, workspace) = open_workspace_with_remote_factory(cx, HostStore::seed(), factory);
    workspace.update(cx, |workspace, cx| {
        workspace.settings().update(cx, |settings, cx| {
            settings.update(
                |settings| settings.sidebar_tools.set(ToolKind::Gpu, true),
                cx,
            )
        });
    });
    in_frame(cx, handle, |window, cx| {
        window.click(("terminal-tab", INITIAL_WEB_TERMINAL), cx)
    });
    in_frame(cx, handle, |window, cx| window.click("tool-gpu", cx));
    (handle, workspace)
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
    assert_eq!(factory.exec_count(), count);
}

fn label(window: &mut gpui_kit::Window, id: impl Into<ElementId>) -> Option<String> {
    window.find(id).label().map(str::to_owned)
}

fn detail_id(device: usize, name: &str) -> ElementId {
    ElementId::Name(format!("gpu-detail:{device}:{name}").into())
}

/// A field of an unfolded device's details.
fn detail(window: &mut gpui_kit::Window, device: usize, name: &str) -> Option<String> {
    label(window, detail_id(device, name))
}

#[gpui_kit::test]
async fn eight_gpus_show_their_sums_then_three_lines_each(cx: &mut TestAppContext) {
    let factory = Arc::new(FakeTerminalFactory::answering(&[&eight_gpus()]));
    let (handle, _) = open_gpu(cx, factory.clone());
    wait_for_label(cx, handle, "gpu-summary", EIGHT_SUMMARY).await;
    in_frame(cx, handle, |window, _| {
        assert_eq!(
            window.find(("tool-sidebar", INITIAL_WEB_TERMINAL)).label(),
            Some("GPU/NPU")
        );
        assert_eq!(
            label(window, "gpu-model").as_deref(),
            Some("NVIDIA H100 80GB HBM3 · 驱动 550.54.15")
        );
        // Over all eight: half of them at 96%, 4 × 60 GB and 4 × 1 MB of
        // 8 × 81 559 MiB, the hottest, and the sum of what they draw.
        assert_eq!(label(window, "gpu-utilization").as_deref(), Some("48%"));
        assert_eq!(label(window, "gpu-memory").as_deref(), Some("38%"));
        assert_eq!(
            label(window, "gpu-memory-amount").as_deref(),
            Some("240.0 / 637 GB")
        );
        assert_eq!(label(window, "gpu-temperature").as_deref(), Some("70 °C"));
        assert_eq!(label(window, "gpu-power").as_deref(), Some("2680 W"));

        assert_eq!(
            label(window, ("gpu-device", 0u64)).as_deref(),
            Some("GPU 0 · NVIDIA H100 80GB HBM3 · 利用率 96% · 显存 60.0 / 80 GB · 70 °C · 600 W")
        );
        assert_eq!(
            label(window, ("gpu-device", 7u64)).as_deref(),
            Some("GPU 7 · NVIDIA H100 80GB HBM3 · 利用率 0% · 显存 1 MB / 80 GB · 30 °C · 70 W")
        );
        assert!(window.try_find(("gpu-device", 8u64)).is_none());
        // The one model is the header's, not every row's.
        assert!(window.try_find(("gpu-device-name", 0u64)).is_none());
        // Folded, eight of them.
        assert!(window.try_find(("gpu-device-details", 0u64)).is_none());

        assert_eq!(label(window, "gpu-processes-count").as_deref(), Some("5"));
        assert_eq!(
            label(window, ("gpu-process", 4242u64)).as_deref(),
            Some("torchrun train.py · GPU 0 · PID 4242 · alice · 显存 58.6 GB")
        );
        // Not seen by `ps`: its program, and no owner.
        assert_eq!(
            label(window, ("gpu-process", 999u64)).as_deref(),
            Some("/opt/infer · GPU 1 · PID 999 · — · 显存 1.0 GB")
        );
        // By device, the most memory first.
        let top = |window: &mut gpui_kit::Window, pid: u64| {
            f32::from(window.find(("gpu-process", pid)).bounds().origin.y)
        };
        assert!(top(window, 4243) < top(window, 999));
        assert!(top(window, 999) < top(window, 4244));
    });
    assert_eq!(factory.exec_count(), 1);
    assert!(factory.exec_commands()[0].contains("nvidia-smi --query-gpu="));

    // Every two seconds while it shows; hidden, no more.
    cx.executor().advance_clock(Duration::from_secs(2));
    wait_for_execs(cx, handle, &factory, 2).await;
    in_frame(cx, handle, |window, cx| window.click("tool-gpu", cx));
    cx.executor().advance_clock(Duration::from_secs(10));
    cx.run_until_parked();
    assert_eq!(factory.exec_count(), 2);
}

#[gpui_kit::test]
async fn a_click_on_a_device_unfolds_its_details(cx: &mut TestAppContext) {
    let factory = Arc::new(FakeTerminalFactory::answering(&[&eight_gpus()]));
    let (handle, _) = open_gpu(cx, factory);
    wait_for_label(cx, handle, "gpu-summary", EIGHT_SUMMARY).await;

    in_frame(cx, handle, |window, cx| {
        window.click(("gpu-device", 2u64), cx)
    });
    in_frame(cx, handle, |window, _| {
        assert!(window.find(("gpu-device-details", 2u64)).visible());
        for (name, value) in [
            ("model", "NVIDIA H100 80GB HBM3"),
            ("uuid", "GPU-00000002-0000-0000-0000-000000000000"),
            ("bus", "00000000:1A:00.0"),
            ("pstate", "P0"),
            ("power", "600 / 700 W"),
            ("health", "正常"),
        ] {
            assert_eq!(detail(window, 2, name).as_deref(), Some(value), "{name}");
        }
        // No fan on a data-center card.
        assert!(window.try_find(detail_id(2, "fan")).is_none());
        assert!(window.try_find(("gpu-device-details", 3u64)).is_none());
    });
    in_frame(cx, handle, |window, cx| {
        window.click(("gpu-device", 2u64), cx)
    });
    in_frame(cx, handle, |window, _| {
        assert!(window.try_find(("gpu-device-details", 2u64)).is_none());
    });
}

#[gpui_kit::test]
async fn every_device_unfolds_and_folds_at_once(cx: &mut TestAppContext) {
    let factory = Arc::new(FakeTerminalFactory::answering(&[&eight_gpus()]));
    let (handle, _) = open_gpu(cx, factory);
    wait_for_label(cx, handle, "gpu-summary", EIGHT_SUMMARY).await;
    let unfolded = |window: &mut gpui_kit::Window| {
        (0..8u64)
            .filter(|index| window.try_find(("gpu-device-details", *index)).is_some())
            .count()
    };

    // One unfolded by hand, then all of them.
    in_frame(cx, handle, |window, cx| {
        window.click(("gpu-device", 3u64), cx)
    });
    in_frame(cx, handle, |window, cx| {
        assert_eq!(unfolded(window), 1);
        assert_eq!(window.find("gpu-devices-fold").label(), Some("全部展开"));
        window.click("gpu-devices-fold", cx);
    });
    in_frame(cx, handle, |window, cx| {
        assert_eq!(unfolded(window), 8);
        assert_eq!(window.find("gpu-devices-fold").label(), Some("全部收起"));
        // Folding one by hand leaves the rest; the first, the others now
        // being far below.
        window.click(("gpu-device", 0u64), cx);
    });
    in_frame(cx, handle, |window, cx| {
        assert_eq!(unfolded(window), 7);
        assert_eq!(window.find("gpu-devices-fold").label(), Some("全部展开"));
        window.click("gpu-devices-fold", cx);
    });
    in_frame(cx, handle, |window, cx| {
        assert_eq!(unfolded(window), 8);
        window.click("gpu-devices-fold", cx);
    });
    in_frame(cx, handle, |window, _| {
        assert_eq!(unfolded(window), 0);
    });
}

#[gpui_kit::test]
async fn one_npu_shows_unfolded(cx: &mut TestAppContext) {
    let factory = Arc::new(FakeTerminalFactory::answering(&[ONE_NPU]));
    let (handle, _) = open_gpu(cx, factory);
    wait_for_label(cx, handle, "gpu-summary", "共 1 个设备").await;
    in_frame(cx, handle, |window, _| {
        assert_eq!(
            label(window, "gpu-model").as_deref(),
            Some("910B3 · 驱动 24.1.0.3")
        );
        assert_eq!(
            label(window, ("gpu-device", 0u64)).as_deref(),
            Some("NPU 0 · 910B3 · 利用率 0% · 显存 3.3 / 64 GB · 37 °C · 93 W")
        );
        assert_eq!(detail(window, 0, "bus").as_deref(), Some("0000:C1:00.0"));
        assert_eq!(detail(window, 0, "power").as_deref(), Some("93 W"));
        assert_eq!(detail(window, 0, "health").as_deref(), Some("正常"));
        assert!(window.try_find(detail_id(0, "uuid")).is_none());
        assert_eq!(
            label(window, "gpu-no-processes").as_deref(),
            Some("没有计算进程")
        );
        // Nothing to unfold all at once.
        assert!(window.try_find("gpu-devices-fold").is_none());
    });
}

#[gpui_kit::test]
async fn ending_a_gpu_process_asks_first_then_reads_again(cx: &mut TestAppContext) {
    let eight = eight_gpus();
    let factory = Arc::new(FakeTerminalFactory::answering(&[
        &eight,
        "@@status 0\n",
        &eight,
    ]));
    let (handle, _) = open_gpu(cx, factory.clone());
    wait_for_label(cx, handle, "gpu-summary", EIGHT_SUMMARY).await;

    in_frame(cx, handle, |window, cx| {
        window.dispatch_action(
            Box::new(EndProcess {
                pid: 4242,
                force: false,
            }),
            cx,
        )
    });
    // Nothing is sent before the answer.
    assert_eq!(factory.exec_count(), 1);
    in_frame(cx, handle, |window, cx| window.click("ok", cx));
    wait_for_execs(cx, handle, &factory, 2).await;
    assert_eq!(
        factory.exec_commands()[1],
        "sh -c 'kill -TERM 4242 2>&1; echo @@status $?'"
    );
    // A moment later, GPU/NPU reads again.
    cx.executor().advance_clock(Duration::from_secs(1));
    wait_for_execs(cx, handle, &factory, 3).await;
    assert!(factory.exec_commands()[2].contains("nvidia-smi"));
}

#[gpui_kit::test]
async fn a_host_without_the_tools_says_so_and_reads_no_more(cx: &mut TestAppContext) {
    let factory = Arc::new(FakeTerminalFactory::answering(&["@@end\n"]));
    let (handle, _) = open_gpu(cx, factory.clone());
    wait_for_label(
        cx,
        handle,
        "gpu-message",
        "这台主机上没有 nvidia-smi 或 npu-smi。目前支持 NVIDIA GPU 和华为昇腾 NPU。",
    )
    .await;
    cx.executor().advance_clock(Duration::from_secs(10));
    cx.run_until_parked();
    assert_eq!(factory.exec_count(), 1);
}

#[gpui_kit::test]
async fn a_tool_stopped_for_hanging_stops_the_readings_until_refresh(cx: &mut TestAppContext) {
    let factory = Arc::new(FakeTerminalFactory::answering(&[
        "@@nvidia\n\n@@status 137\n@@end\n",
    ]));
    let (handle, _) = open_gpu(cx, factory.clone());
    wait_for_label(cx, handle, "gpu-paused", "已停止自动读取，点刷新按钮重试。").await;
    in_frame(cx, handle, |window, _| {
        assert_eq!(
            label(window, "gpu-errors").as_deref(),
            Some("nvidia-smi 在 3 秒内没有回答，已被停止。")
        );
        assert!(window.try_find("gpu-no-devices").is_none());
    });
    cx.executor().advance_clock(Duration::from_secs(10));
    cx.run_until_parked();
    assert_eq!(factory.exec_count(), 1);

    in_frame(cx, handle, |window, cx| {
        window.dispatch_action(Box::new(RefreshGpu), cx)
    });
    wait_for_execs(cx, handle, &factory, 2).await;
}

#[gpui_kit::test]
fn the_gpu_tool_is_not_offered_on_a_host_known_not_to_run_linux(cx: &mut TestAppContext) {
    let mut store = HostStore::seed();
    store.set_host_os_unnotified(HostId(STAGING_API), Some(HostOs::MacOs));
    let (handle, workspace) = open_workspace_with_store(cx, store);
    workspace.update(cx, |workspace, cx| {
        workspace.settings().update(cx, |settings, cx| {
            settings.update(
                |settings| settings.sidebar_tools.set(ToolKind::Gpu, true),
                cx,
            )
        });
    });
    in_frame(cx, handle, |window, cx| {
        window.click(("terminal-tab", INITIAL_WEB_TERMINAL), cx)
    });
    in_frame(cx, handle, |window, _| {
        assert!(window.find("tool-gpu").visible());
    });
    in_frame(cx, handle, |window, cx| {
        window.click(("terminal-tab", INITIAL_STAGING_TERMINAL), cx)
    });
    in_frame(cx, handle, |window, _| {
        assert!(window.try_find("tool-gpu").is_none());
    });
}

fn sidebar_tools(cx: &mut TestAppContext, workspace: &Entity<Workspace>) -> serde_json::Value {
    workspace.read_with(cx, |workspace, cx| {
        serde_json::to_value(workspace.settings().read(cx).settings().sidebar_tools).unwrap()
    })
}

/// 设置 › 终端 › 侧栏功能, the page's third group: the tools shown in a
/// row, the rest in another; a click or a drag moves one across.
#[gpui_kit::test]
fn the_tools_move_between_the_rows_in_the_settings(cx: &mut TestAppContext) {
    let (handle, workspace) = open_workspace(cx);
    // Tall enough for the whole group.
    cx.simulate_window_resize(handle.into(), size(px(1280.), px(1400.)));
    in_frame(cx, handle, |window, cx| {
        window.click(("terminal-tab", INITIAL_WEB_TERMINAL), cx)
    });
    in_frame(cx, handle, |window, _| {
        assert!(window.find("tool-monitor").visible());
        assert!(window.try_find("tool-gpu").is_none(), "off to begin with");
    });

    let open = |cx: &mut TestAppContext| {
        in_frame(cx, handle, |window, cx| {
            window.dispatch_action(Box::new(OpenSettings), cx)
        });
        in_frame(cx, handle, |window, cx| {
            window.within("settings").click("0-1", cx)
        });
        // The group's entry under 终端 in the left column, which scrolls to
        // it: the page draws only the groups near the view.
        in_frame(cx, handle, |window, cx| {
            window.within("settings").click("0-1-2", cx)
        });
    };
    let row_of = |window: &mut gpui_kit::Window, tool: &str| {
        let id = format!("sidebar-tool-{tool}");
        if window
            .within("sidebar-tools-shown")
            .try_find(id.clone())
            .is_some()
        {
            "shown"
        } else if window.within("sidebar-tools-hidden").try_find(id).is_some() {
            "hidden"
        } else {
            "nowhere"
        }
    };
    open(cx);
    in_frame(cx, handle, |window, cx| {
        assert_eq!(row_of(window, "monitor"), "shown");
        assert_eq!(row_of(window, "gpu"), "hidden");
        assert_eq!(window.find("sidebar-tool-gpu").label(), Some("GPU/NPU"));
        assert!(!enabled(&window.find("sidebar-tools-reset")));
        // A click moves it up.
        window.click("sidebar-tool-gpu", cx);
    });
    assert_eq!(
        sidebar_tools(cx, &workspace),
        serde_json::json!({"gpu": true})
    );
    in_frame(cx, handle, |window, cx| {
        assert_eq!(row_of(window, "gpu"), "shown");
        assert!(enabled(&window.find("sidebar-tools-reset")));
        // A drag takes it to the other row.
        window.drag_to("sidebar-tool-monitor", "sidebar-tools-hidden", cx);
    });
    assert_eq!(
        sidebar_tools(cx, &workspace),
        serde_json::json!({"gpu": true, "monitor": false})
    );
    in_frame(cx, handle, |window, cx| {
        assert_eq!(row_of(window, "monitor"), "hidden");
        // Dropped on its own row, it stays.
        window.drag_to("sidebar-tool-gpu", "sidebar-tools-shown", cx);
    });
    assert_eq!(
        sidebar_tools(cx, &workspace),
        serde_json::json!({"gpu": true, "monitor": false})
    );

    in_frame(cx, handle, |window, cx| {
        window.click(("terminal-tab", INITIAL_WEB_TERMINAL), cx)
    });
    in_frame(cx, handle, |window, _| {
        assert!(window.find("tool-gpu").visible());
        assert!(window.try_find("tool-monitor").is_none());
    });

    // 恢复默认, for these rows alone.
    open(cx);
    in_frame(cx, handle, |window, cx| {
        window.click("sidebar-tools-reset", cx)
    });
    assert_eq!(sidebar_tools(cx, &workspace), serde_json::json!({}));
    in_frame(cx, handle, |window, cx| {
        assert_eq!(row_of(window, "monitor"), "shown");
        assert_eq!(row_of(window, "gpu"), "hidden");
        window.click("sidebar-tool-gpu", cx);
    });
    assert_eq!(
        sidebar_tools(cx, &workspace),
        serde_json::json!({"gpu": true})
    );

    // The page's 重置 takes them back with the rest of the page.
    in_frame(cx, handle, |window, cx| {
        window.within("settings").click("reset", cx)
    });
    assert_eq!(sidebar_tools(cx, &workspace), serde_json::json!({}));
}

/// What the settings leave shown is what the switch offers, at once; the
/// tool showing taken out hides the sidebar until it is back.
#[gpui_kit::test]
fn the_switch_offers_what_the_settings_show(cx: &mut TestAppContext) {
    let (handle, workspace) = open_workspace(cx);
    let set = |cx: &mut TestAppContext, change: fn(&mut shellrs::settings::SidebarTools)| {
        workspace.update(cx, |workspace, cx| {
            workspace.settings().update(cx, |settings, cx| {
                settings.update(|settings| change(&mut settings.sidebar_tools), cx)
            });
        });
        cx.run_until_parked();
    };
    in_frame(cx, handle, |window, cx| {
        window.click(("terminal-tab", INITIAL_WEB_TERMINAL), cx)
    });
    in_frame(cx, handle, |window, cx| window.click("tool-monitor", cx));
    in_frame(cx, handle, |window, _| {
        assert!(
            window
                .try_find(("tool-sidebar", INITIAL_WEB_TERMINAL))
                .is_some()
        );
    });

    set(cx, |tools| {
        tools.set(ToolKind::Monitor, false);
        tools.set(ToolKind::Gpu, true);
    });
    in_frame(cx, handle, |window, _| {
        assert!(window.try_find("tool-monitor").is_none());
        assert!(
            window
                .try_find(("tool-sidebar", INITIAL_WEB_TERMINAL))
                .is_none()
        );
        assert!(window.find("tool-gpu").visible());
    });

    set(cx, |tools| tools.reset());
    in_frame(cx, handle, |window, _| {
        assert!(window.try_find("tool-gpu").is_none());
        // Back as it was: the monitor, showing.
        assert_eq!(window.find("tool-monitor").checked(), Some(true));
        assert_eq!(
            window.find(("tool-sidebar", INITIAL_WEB_TERMINAL)).label(),
            Some("系统监控")
        );
    });
}
