//! Which package of a release fits this machine.
//!
//! The keys are part of the manifest format, so they never change. macOS
//! packages match the running binary's architecture. Existing universal
//! builds select their native half's package; an Intel build under Rosetta
//! keeps receiving Intel packages until the user installs the ARM build. A
//! portable copy on Windows has a key of its own: its package is a zip, not
//! the setup program.

use gpui_kit::SharedString;

use crate::i18n::t;

/// The manifest's key for the Windows portable zip.
pub const WINDOWS_PORTABLE: &str = "windows-x86_64-portable";

/// The manifest's key for this build's platform.
pub fn platform_key() -> &'static str {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "macos-aarch64"
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        "macos-x86_64"
    } else if cfg!(all(windows, target_arch = "x86_64")) {
        "windows-x86_64"
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        "linux-x86_64"
    } else {
        "unsupported"
    }
}

/// The manifest's key for the package this copy updates with: this
/// build's, or the portable zip's for a portable copy on Windows.
pub fn package_key(portable: bool) -> &'static str {
    match platform_key() {
        "windows-x86_64" if portable => WINDOWS_PORTABLE,
        key => key,
    }
}

/// The platform as 设置 › 关于 names it.
pub fn platform_label(portable: bool) -> SharedString {
    match package_key(portable) {
        "macos-aarch64" => "macOS · Apple Silicon".into(),
        "macos-x86_64" => "macOS · Intel".into(),
        "windows-x86_64" => "Windows · x64".into(),
        WINDOWS_PORTABLE => t!("update.platform.windows_portable"),
        "linux-x86_64" => "Linux · x64".into(),
        _ => std::env::consts::OS.into(),
    }
}

/// `ShellRS/0.2.0 (macos; aarch64)`: all the update server learns about
/// the machine asking.
pub fn user_agent(version: &str) -> String {
    format!(
        "ShellRS/{version} ({}; {})",
        std::env::consts::OS,
        std::env::consts::ARCH
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_user_agent_names_the_version_system_and_architecture() {
        let agent = user_agent("1.2.3");
        assert!(agent.starts_with("ShellRS/1.2.3 ("), "{agent}");
        assert!(agent.contains(std::env::consts::OS), "{agent}");
        assert!(
            agent.ends_with(&format!("{})", std::env::consts::ARCH)),
            "{agent}"
        );
    }

    #[test]
    fn this_machine_has_a_package() {
        assert_ne!(platform_key(), "unsupported");
        assert!(!platform_label(false).is_empty());
    }

    #[test]
    fn only_windows_has_a_portable_package() {
        assert_eq!(package_key(false), platform_key());
        if platform_key() == "windows-x86_64" {
            assert_eq!(package_key(true), WINDOWS_PORTABLE);
            assert_eq!(platform_label(true), "Windows · x64 · 便携版");
        } else {
            assert_eq!(package_key(true), platform_key());
        }
    }
}
