use gpui_kit::component::{
    ActiveTheme as _, Placement, Selectable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    dock::{BasePanel, Panel, PanelControl, PanelEvent},
    h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::app::{ToggleTool, ToolKind};
use crate::docker::{Container, DockerObject, DockerPanel, ObjectSummary};
use crate::explorer::ExplorerPanel;
use crate::gpu::GpuPanel;
use crate::history::HistoryPanel;
use crate::host::{HostId, HostOs, HostStore, SnippetCategoryId};
use crate::kubernetes::{KubernetesCommand, KubernetesPanel};
use crate::monitor::{MonitorDetail, MonitorPanel};
use crate::netstat::NetstatPanel;
use crate::processes::{Process, ProcessDetails, ProcessPanel, ProcessSort};
use crate::services::{Service, ServicePanel};
use crate::snippets::SnippetPanel;
use crate::terminal::{ExecTarget, RemoteTerminalId, TerminalView};

impl ToolKind {
    /// Whether the tool runs commands or SFTP on a channel of its own beside
    /// the terminal, which a terminal a bastion host opened cannot have.
    /// Command snippets only type into the terminal.
    pub fn runs_commands(self) -> bool {
        !matches!(self, ToolKind::Snippets)
    }

    /// Whether the tool works on a host running `os`. A host not identified
    /// yet gets every tool, and is told if one cannot work there.
    pub fn works_on(self, os: Option<HostOs>) -> bool {
        match self {
            ToolKind::Services
            | ToolKind::Processes
            | ToolKind::Connections
            | ToolKind::Monitor
            | ToolKind::Gpu => os.is_none_or(HostOs::is_linux),
            ToolKind::Snippets | ToolKind::Files => true,
            // Wherever `sh` runs `docker`: Linux, and a Mac with Docker
            // Desktop; and bash's history file wherever `sh` reads it.
            ToolKind::Docker | ToolKind::History | ToolKind::Kubernetes => {
                os != Some(HostOs::Windows)
            }
        }
    }

    /// The id of the tool's button in the switch.
    fn button_id(self) -> &'static str {
        match self {
            ToolKind::Snippets => "tool-snippets",
            ToolKind::History => "tool-history",
            ToolKind::Files => "tool-files",
            ToolKind::Docker => "tool-docker",
            ToolKind::Kubernetes => "tool-kubernetes",
            ToolKind::Services => "tool-services",
            ToolKind::Processes => "tool-processes",
            ToolKind::Connections => "tool-connections",
            ToolKind::Monitor => "tool-monitor",
            ToolKind::Gpu => "tool-gpu",
        }
    }
}

/// The SSH terminal the right sidebar works on.
#[derive(Clone, PartialEq)]
pub struct ToolTerminal {
    pub id: RemoteTerminalId,
    pub host: HostId,
    /// Runs the tools' commands on the terminal's own connection.
    pub view: WeakEntity<TerminalView>,
}

/// The right dock's one panel: the tool the switch at the window's edge
/// picked, working on the SSH terminal in front. It is there only while one
/// is; the workspace hides it, switch and all, with any other tab.
///
/// One panel showing either tool, like the left dock's `Sidebar`, rather
/// than a panel per tool swapped in the dock: swapping takes the displaced
/// panel out of the dock and the focus with it, and two panels in one tab
/// group always draw a tab bar.
///
/// Each tool is a feature module's entity held here, alive while hidden so
/// it keeps its state, told which terminal is in front and whether it is on
/// screen.
pub struct ToolSidebar {
    tool: ToolKind,
    /// The terminal in front; `None` while the sidebar is hidden for
    /// another kind of tab.
    terminal: Option<ToolTerminal>,
    /// Whether the right dock is open.
    shown: bool,
    /// For the name of the terminal's host, beside the tool's.
    store: Entity<HostStore>,
    monitor: Entity<MonitorPanel>,
    snippets: Entity<SnippetPanel>,
    history: Entity<HistoryPanel>,
    /// The terminal in front's 文件, which the workspace owns: one per
    /// terminal, made the first time it is shown.
    files: Option<Entity<ExplorerPanel>>,
    docker: Entity<DockerPanel>,
    /// Created only when Kubernetes is first selected; unused terminals
    /// do not allocate its inputs, subscriptions or query state.
    kubernetes: Option<Entity<KubernetesPanel>>,
    services: Entity<ServicePanel>,
    processes: Entity<ProcessPanel>,
    netstat: Entity<NetstatPanel>,
    dispatch: FocusHandle,
    gpu: Entity<GpuPanel>,
    focus_handle: FocusHandle,
}

impl ToolSidebar {
    /// `dispatch` is the workspace's focus handle, which the tools' buttons
    /// dispatch on.
    pub fn new(
        store: Entity<HostStore>,
        dispatch: FocusHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            tool: ToolKind::default(),
            terminal: None,
            shown: false,
            store: store.clone(),
            monitor: cx.new(|_| MonitorPanel::new(dispatch.clone())),
            snippets: cx.new(|cx| SnippetPanel::new(store, dispatch.clone(), window, cx)),
            history: cx.new(|cx| HistoryPanel::new(dispatch.clone(), window, cx)),
            files: None,
            docker: cx.new(|_| DockerPanel::new(dispatch.clone())),
            kubernetes: None,
            services: cx.new(|cx| ServicePanel::new(dispatch.clone(), window, cx)),
            processes: cx.new(|cx| ProcessPanel::new(dispatch.clone(), window, cx)),
            netstat: cx.new(|cx| NetstatPanel::new(dispatch.clone(), window, cx)),
            gpu: cx.new(|cx| GpuPanel::new(dispatch.clone(), cx)),
            dispatch,
            focus_handle: cx.focus_handle(),
        }
    }

    pub fn tool(&self) -> ToolKind {
        self.tool
    }

    pub fn disable_kubernetes(&mut self, cx: &mut Context<Self>) {
        if let Some(panel) = self.kubernetes.take() {
            panel.update(cx, |panel, cx| panel.set_target(None, false, cx));
            cx.notify();
        }
    }

    pub fn kubernetes_command(
        &mut self,
        command: &KubernetesCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(panel) = &self.kubernetes {
            panel.update(cx, |panel, cx| panel.command(command, window, cx));
        }
    }

    /// Work on the SSH terminal now in front, and say whether the dock is
    /// open: a tool reads the host only while it is on screen.
    pub fn set_terminal(
        &mut self,
        terminal: Option<ToolTerminal>,
        shown: bool,
        cx: &mut Context<Self>,
    ) {
        if self.terminal != terminal || self.shown != shown {
            self.terminal = terminal;
            self.shown = shown;
            self.sync_tools(cx);
            cx.notify();
        }
    }

    pub fn toggle_monitor_detail(&mut self, detail: MonitorDetail, cx: &mut Context<Self>) {
        self.monitor
            .update(cx, |monitor, cx| monitor.toggle(detail, cx));
    }

    /// 网络连接's 刷新.
    pub fn refresh_connections(&mut self, cx: &mut Context<Self>) {
        self.netstat.update(cx, |netstat, cx| netstat.refresh(cx));
    }

    /// Fold a category of 命令片段 away, or unfold it.
    pub fn toggle_snippet_category(
        &mut self,
        id: Option<SnippetCategoryId>,
        cx: &mut Context<Self>,
    ) {
        self.snippets
            .update(cx, |snippets, cx| snippets.toggle_category(id, cx));
    }

    /// 历史命令's 刷新.
    pub fn refresh_history(&mut self, cx: &mut Context<Self>) {
        self.history.update(cx, |history, cx| history.refresh(cx));
    }

    /// Docker's 刷新, and after a command.
    pub fn refresh_docker(&mut self, cx: &mut Context<Self>) {
        self.docker.update(cx, |docker, cx| docker.refresh(cx));
    }

    /// Fold a compose project away, or unfold it.
    pub fn toggle_docker_project(&mut self, name: &str, cx: &mut Context<Self>) {
        self.docker
            .update(cx, |docker, cx| docker.toggle_project(name, cx));
    }

    /// A container of the host in front, as Docker last showed it.
    pub fn container(&self, id: &str, cx: &App) -> Option<Container> {
        self.docker.read(cx).container(id)
    }

    /// A volume, an image or a network of the host in front, as Docker
    /// last showed it.
    pub fn docker_summary(
        &self,
        object: DockerObject,
        id: &str,
        cx: &App,
    ) -> Option<ObjectSummary> {
        self.docker.read(cx).summary_of(object, id)
    }

    /// 系统服务's 刷新, and after a command.
    pub fn refresh_services(&mut self, cx: &mut Context<Self>) {
        self.services
            .update(cx, |services, cx| services.refresh(cx));
    }

    /// A service of the host in front, as 系统服务 last showed it.
    pub fn service(&self, name: &str, cx: &App) -> Option<Service> {
        self.services.read(cx).service(name)
    }

    /// 进程管理's 刷新.
    pub fn refresh_processes(&mut self, cx: &mut Context<Self>) {
        self.processes
            .update(cx, |processes, cx| processes.refresh(cx));
    }

    /// Read the processes again in a moment, once one was told to end:
    /// those of 进程管理, or of GPU/NPU when it is the one showing.
    pub fn refresh_processes_soon(&mut self, cx: &mut Context<Self>) {
        if self.tool == ToolKind::Gpu {
            self.gpu.update(cx, |gpu, cx| gpu.refresh_soon(cx));
        } else {
            self.processes
                .update(cx, |processes, cx| processes.refresh_soon(cx));
        }
    }

    /// GPU/NPU's 刷新.
    pub fn refresh_gpu(&mut self, cx: &mut Context<Self>) {
        self.gpu.update(cx, |gpu, cx| gpu.refresh(cx));
    }

    /// Unfold a device of GPU/NPU, or fold it away.
    pub fn toggle_gpu_device(&mut self, key: &str, cx: &mut Context<Self>) {
        self.gpu.update(cx, |gpu, cx| gpu.toggle(key, cx));
    }

    /// GPU/NPU's 全部展开 and 全部收起.
    pub fn toggle_all_gpu_devices(&mut self, cx: &mut Context<Self>) {
        self.gpu.update(cx, |gpu, cx| gpu.toggle_all(cx));
    }

    /// 进程管理's sort buttons.
    pub fn sort_processes(&mut self, by: ProcessSort, cx: &mut Context<Self>) {
        self.processes
            .update(cx, |processes, cx| processes.sort(by, cx));
    }

    /// A process of the host in front, as 进程管理 last showed it.
    pub fn process(&self, pid: u32, cx: &App) -> Option<Process> {
        self.processes.read(cx).process(pid)
    }

    /// The name and owner of a process of the host in front, for the
    /// question before ending it: as the tool showing listed it, 进程管理
    /// or GPU/NPU.
    pub fn process_label(&self, pid: u32, cx: &App) -> Option<(String, Option<String>)> {
        if self.tool == ToolKind::Gpu {
            return self.gpu.read(cx).process(pid);
        }
        let process = self.process(pid, cx)?;
        Some((process.name, process.user))
    }

    /// The same with its family, for its details.
    pub fn process_details(&self, pid: u32, cx: &App) -> Option<ProcessDetails> {
        self.processes.read(cx).details(pid)
    }

    /// Point each tool at the terminal, running only the one on screen.
    fn sync_tools(&mut self, cx: &mut Context<Self>) {
        let target = self.terminal.as_ref().map(|terminal| ExecTarget {
            terminal: terminal.id,
            view: terminal.view.clone(),
        });
        let showing = |tool| self.shown && self.tool == tool;
        let kubernetes = showing(ToolKind::Kubernetes);
        if let Some(panel) = &self.kubernetes {
            panel.update(cx, |panel, cx| {
                panel.set_target(target.clone(), kubernetes, cx)
            });
        }
        let (monitor, netstat, processes, services, docker, history, gpu) = (
            showing(ToolKind::Monitor),
            showing(ToolKind::Connections),
            showing(ToolKind::Processes),
            showing(ToolKind::Services),
            showing(ToolKind::Docker),
            showing(ToolKind::History),
            showing(ToolKind::Gpu),
        );
        self.history.update(cx, |panel, cx| {
            panel.set_target(target.clone(), history, cx)
        });
        self.docker
            .update(cx, |panel, cx| panel.set_target(target.clone(), docker, cx));
        self.services.update(cx, |panel, cx| {
            panel.set_target(target.clone(), services, cx)
        });
        self.monitor.update(cx, |panel, cx| {
            panel.set_target(target.clone(), monitor, cx)
        });
        self.processes.update(cx, |panel, cx| {
            panel.set_target(target.clone(), processes, cx)
        });
        self.gpu
            .update(cx, |panel, cx| panel.set_target(target.clone(), gpu, cx));
        self.netstat
            .update(cx, |panel, cx| panel.set_target(target, netstat, cx));
    }

    /// Show another terminal's 文件, or none. A focus inside the one going
    /// away moves to the sidebar itself, as on `show`.
    pub fn set_files(
        &mut self,
        files: Option<Entity<ExplorerPanel>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.files == files {
            return;
        }
        if self.focus_handle.contains_focused(window, cx) {
            window.focus(&self.focus_handle, cx);
        }
        self.files = files;
        cx.notify();
    }

    /// Show another tool. A focus inside the one going away moves to the
    /// sidebar itself: left on an element no longer drawn, it would take
    /// every window-level shortcut's dispatch path with it.
    pub fn show(&mut self, tool: ToolKind, window: &mut Window, cx: &mut Context<Self>) {
        if self.tool == tool && (tool != ToolKind::Kubernetes || self.kubernetes.is_some()) {
            return;
        }
        if self.focus_handle.contains_focused(window, cx) {
            window.focus(&self.focus_handle, cx);
        }
        self.tool = tool;
        if tool == ToolKind::Kubernetes && self.kubernetes.is_none() {
            self.kubernetes =
                Some(cx.new(|cx| KubernetesPanel::new(self.dispatch.clone(), window, cx)));
        }
        self.sync_tools(cx);
        cx.notify();
    }
}

impl EventEmitter<PanelEvent> for ToolSidebar {}

impl Focusable for ToolSidebar {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl BasePanel for ToolSidebar {
    fn panel_name(&self) -> &'static str {
        "ToolSidebar"
    }

    fn closable(&self, _: &App) -> bool {
        false
    }

    fn zoomable(&self, _: &App) -> bool {
        false
    }
}

impl Panel for ToolSidebar {
    /// The tool, and the host it works on: with the center split, more than
    /// one SSH terminal may show.
    fn title(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let host = self.terminal.as_ref().and_then(|terminal| {
            self.store
                .read(cx)
                .host(terminal.host)
                .map(|host| host.name.clone())
        });
        let muted = cx.theme().muted_foreground;
        h_flex()
            .id("tool-sidebar-title")
            .test_support()
            .min_w_0()
            .gap_1()
            .child(self.tool.icon().small())
            .child(div().flex_shrink_0().child(self.tool.label()))
            .when_some(host, |title, host| {
                title
                    .aria_label(format!("{} · {host}", self.tool.label()))
                    .child(div().flex_shrink_0().text_color(muted).child("·"))
                    .child(
                        div()
                            .id("tool-sidebar-host")
                            .test_support()
                            .aria_label(host.clone())
                            .min_w_0()
                            .truncate()
                            .text_color(muted)
                            .child(host),
                    )
            })
    }

    fn zoom_control(&self, _: &App) -> Option<PanelControl> {
        None
    }

    fn inner_padding(&self, _: &App) -> bool {
        false
    }
}

impl Render for ToolSidebar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Named after the terminal it works on.
        let id = self.terminal.as_ref().map_or(0, |terminal| terminal.id.0);
        v_flex()
            .id(("tool-sidebar", id))
            .test_support()
            .aria_label(self.tool.label())
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(cx.theme().sidebar)
            .text_color(cx.theme().sidebar_foreground)
            .when(self.terminal.is_some(), |sidebar| {
                let panel = match self.tool {
                    ToolKind::Snippets => self.snippets.clone().into_any_element(),
                    ToolKind::History => self.history.clone().into_any_element(),
                    ToolKind::Files => match &self.files {
                        Some(files) => files.clone().into_any_element(),
                        None => Empty.into_any_element(),
                    },
                    ToolKind::Docker => self.docker.clone().into_any_element(),
                    ToolKind::Kubernetes => self.kubernetes.as_ref().map_or_else(
                        || div().into_any_element(),
                        |panel| panel.clone().into_any_element(),
                    ),
                    ToolKind::Services => self.services.clone().into_any_element(),
                    ToolKind::Processes => self.processes.clone().into_any_element(),
                    ToolKind::Connections => self.netstat.clone().into_any_element(),
                    ToolKind::Monitor => self.monitor.clone().into_any_element(),
                    ToolKind::Gpu => self.gpu.clone().into_any_element(),
                };
                sidebar.child(div().flex_1().min_h_0().child(panel))
            })
    }
}

/// The switch at the window's right edge: a button per tool `offered` for
/// the host in front, the one the sidebar is showing marked, none while it
/// is hidden.
///
/// The workspace draws it beside the dock area rather than in the right
/// dock, which goes off screen whole when it is hidden. The buttons do not
/// take the focus, so the terminal keeps the keyboard, and dispatch on
/// `target`, the workspace's focus handle, so they work whatever is focused.
pub fn render_tool_switch(
    showing: Option<ToolKind>,
    offered: &[ToolKind],
    target: &FocusHandle,
    cx: &App,
) -> impl IntoElement {
    v_flex()
        .id("tool-switch")
        .test_support()
        .flex_shrink_0()
        .h_full()
        .p_1()
        .gap_1()
        .bg(cx.theme().sidebar)
        .border_l_1()
        .border_color(cx.theme().sidebar_border)
        .children(offered.iter().map(|&tool| {
            let target = target.clone();
            Button::new(tool.button_id())
                .ghost()
                .icon(tool.icon())
                .selected(showing == Some(tool))
                // A switch: assistive technology hears it as pressed.
                .toggled(showing == Some(tool))
                .tooltip(tool.label())
                // Toward the window, not off its edge.
                .tooltip_placement(Placement::Left)
                .accessibility_label(tool.label())
                .on_click(move |_, window, cx| {
                    target.dispatch_action(&ToggleTool(tool), window, cx)
                })
        }))
}
