//! On macOS the window's close button hides ShellRS instead of closing it.
//!
//! ShellRS has one main window, and the workspace in it owns everything that
//! runs: connections, transfers, port forwards, the external CLI. Closing
//! it would stop all of that yet leave the app in the Dock with no window,
//! and the Dock icon could not bring one back. So, like Mac terminals,
//! closing hides the app: the Dock icon, ⌘Tab or opening ShellRS again
//! shows the window as it was, and ⌘Q quits. Windows and Linux quit when
//! the window closes, as GPUI does there by default, but like ⌘Q only once
//! the quit guard lets them (unsaved changes in an editor are asked about).

use gpui_kit::{AnyWindowHandle, App, Global, Window};

/// The workspace's window: the one the Dock icon brings back. Others, such
/// as an SFTP tab's 查找文件, come and go beside it.
#[derive(Clone, Copy)]
pub struct MainWindow(pub AnyWindowHandle);

impl Global for MainWindow {}

/// Make the window's close button hide ShellRS on macOS. Elsewhere closing
/// quits, after asking the quit guard.
#[cfg(target_os = "macos")]
pub fn hide_when_closed(window: &Window, cx: &App) {
    window.on_window_should_close(cx, |_, cx| {
        cx.hide();
        false
    });
}

#[cfg(not(target_os = "macos"))]
pub fn hide_when_closed(window: &Window, cx: &App) {
    window.on_window_should_close(cx, |window, cx| !super::quit::quit_held_back(window, cx));
}

/// Show the window again, in front of other apps: when the Dock icon is
/// clicked, or ShellRS is opened again while it runs.
pub fn bring_forward(window: &mut Window, cx: &mut App) {
    // Activating alone may leave a hidden app's windows hidden.
    #[cfg(target_os = "macos")]
    if let Some(main_thread) = objc2::MainThreadMarker::new() {
        objc2_app_kit::NSApplication::sharedApplication(main_thread).unhide(None);
    }
    cx.activate(true);
    window.activate_window();
}

/// Put ShellRS away, when 显示或隐藏 ShellRS is pressed with it in front:
/// hidden on macOS, as ⌘H does, minimized elsewhere.
pub fn send_back(window: &mut Window, cx: &mut App) {
    #[cfg(target_os = "macos")]
    {
        let _ = window;
        cx.hide();
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = cx;
        window.minimize_window();
    }
}

/// Put a maximized window back to its size before. Zooming again does on
/// macOS and Linux; on Windows GPUI's zoom only maximizes, so the system
/// is asked.
pub fn restore_window(window: &Window) {
    #[cfg(windows)]
    {
        use raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
        use windows_sys::Win32::UI::WindowsAndMessaging::{SW_RESTORE, ShowWindow};
        if let Ok(handle) = window.window_handle()
            && let RawWindowHandle::Win32(handle) = handle.as_raw()
        {
            // SAFETY: the handle is this open window's.
            unsafe { ShowWindow(handle.hwnd.get() as *mut core::ffi::c_void, SW_RESTORE) };
        }
    }
    #[cfg(not(windows))]
    window.zoom_window();
}
