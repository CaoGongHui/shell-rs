//! Application password lock. Independent of settings and workspace, with
//! one process-wide entity controlling both the UI gate and CLI access.
mod activity;
mod password;
mod password_form;

pub use activity::activity_tracker;
use gpui_kit::*;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

use crate::i18n::t;
use password::Verifier;
pub use password_form::{UnlockScreen, open_password_dialog};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PasswordOperation {
    Enable,
    Change,
    Disable,
    Unlock,
}

enum PasswordFailure {
    Incorrect,
    Verify(String),
    Save(String),
}

impl PasswordFailure {
    fn message(self) -> SharedString {
        match self {
            Self::Incorrect => t!("app_lock.wrong_password"),
            Self::Verify(error) => t!("app_lock.failed", error = error),
            Self::Save(error) => t!("app_lock.save_failed", error = error),
        }
    }
}

pub struct AppLock {
    path: Option<PathBuf>,
    verifier: Option<Verifier>,
    load_error: Option<String>,
    locked: bool,
    busy: bool,
    last_activity: Option<Instant>,
    _keyboard_activity: Option<Subscription>,
}

#[derive(Clone)]
struct LockGlobal(Entity<AppLock>);
impl Global for LockGlobal {}

impl AppLock {
    /// Load only the verifier. This does not read hosts, secrets or settings.
    pub fn load(path: PathBuf) -> Self {
        let loaded = password::load(&path);
        let locked = !matches!(loaded, Ok(None));
        let (verifier, load_error) = match loaded {
            Ok(verifier) => (verifier, None),
            Err(error) => (None, Some(error.to_string())),
        };
        Self {
            path: Some(path),
            verifier,
            load_error,
            locked,
            busy: false,
            last_activity: None,
            _keyboard_activity: None,
        }
    }

    /// Isolated state for UI integration tests; never reads the user's files.
    pub fn in_memory() -> Self {
        Self {
            path: None,
            verifier: None,
            load_error: None,
            locked: false,
            busy: false,
            last_activity: None,
            _keyboard_activity: None,
        }
    }

    pub fn install(lock: Entity<Self>, cx: &mut App) {
        let weak = lock.downgrade();
        let subscription = cx.intercept_keystrokes(move |_, _, cx| {
            let _ = weak.update(cx, |lock, cx| lock.record_activity(cx));
        });
        lock.update(cx, |lock, cx| {
            lock._keyboard_activity = Some(subscription);
            lock.record_activity(cx);
        });
        cx.set_global(LockGlobal(lock));
    }
    pub fn global(cx: &App) -> Option<Entity<Self>> {
        cx.try_global::<LockGlobal>().map(|g| g.0.clone())
    }
    pub fn is_locked(&self) -> bool {
        self.locked
    }
    pub fn is_enabled(&self) -> bool {
        self.verifier.is_some() || self.load_error.is_some()
    }
    pub fn is_busy(&self) -> bool {
        self.busy
    }
    pub fn load_error(&self) -> Option<&str> {
        self.load_error.as_deref()
    }

    /// Input does not notify observers: activity must not redraw the app.
    pub fn record_activity(&mut self, cx: &App) {
        if !self.locked {
            self.last_activity = Some(cx.background_executor().now());
        }
    }

    pub fn is_idle(&self, timeout: Duration, cx: &App) -> bool {
        !timeout.is_zero()
            && self.is_enabled()
            && !self.locked
            && !self.busy
            && self.last_activity.is_some_and(|last| {
                cx.background_executor()
                    .now()
                    .saturating_duration_since(last)
                    >= timeout
            })
    }

    pub fn lock(&mut self, cx: &mut Context<Self>) {
        if self.is_enabled() && !self.busy {
            self.locked = true;
            cx.notify();
        }
    }

    fn submit(
        &mut self,
        operation: PasswordOperation,
        current: Zeroizing<String>,
        new: Zeroizing<String>,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), SharedString>> {
        if self.busy
            || self.load_error.is_some()
            || (operation == PasswordOperation::Enable) == self.verifier.is_some()
        {
            return Task::ready(Err(t!("app_lock.unavailable")));
        }
        if operation != PasswordOperation::Unlock && self.locked {
            return Task::ready(Err(t!("app_lock.unavailable")));
        }
        if matches!(
            operation,
            PasswordOperation::Enable | PasswordOperation::Change
        ) && new.chars().count() < 8
        {
            return Task::ready(Err(t!("app_lock.too_short")));
        }
        self.busy = true;
        cx.notify();
        let verifier = self.verifier.clone();
        let path = self.path.clone();
        // Keep the service alive even if its dialog is dismissed during work.
        let service = cx.entity();
        cx.spawn(async move |_, cx| {
            let result = cx
                .background_spawn(async move {
                    if let Some(old) = &verifier {
                        match old.verify(&current) {
                            Ok(true) => {}
                            Ok(false) => return Err(PasswordFailure::Incorrect),
                            Err(error) => return Err(PasswordFailure::Verify(error.to_string())),
                        }
                    }
                    let next = match operation {
                        PasswordOperation::Enable | PasswordOperation::Change => Some(
                            Verifier::new(&new)
                                .map_err(|error| PasswordFailure::Verify(error.to_string()))?,
                        ),
                        PasswordOperation::Disable => None,
                        PasswordOperation::Unlock => verifier,
                    };
                    if operation != PasswordOperation::Unlock
                        && let Some(path) = path
                    {
                        password::save(&path, next.as_ref())
                            .map_err(|error| PasswordFailure::Save(error.to_string()))?;
                    }
                    Ok(next)
                })
                .await;
            service.update(cx, |this, cx| {
                this.busy = false;
                let outcome = result.map_err(PasswordFailure::message).map(|verifier| {
                    this.verifier = verifier;
                    this.locked = false;
                    this.record_activity(cx);
                });
                cx.notify();
                outcome
            })
        })
    }
}

/// CLI and shell use the same gate; tests without an installed gate are unlocked.
pub fn is_locked(cx: &App) -> bool {
    AppLock::global(cx).is_some_and(|lock| lock.read(cx).is_locked())
}

#[cfg(test)]
mod tests {
    use super::{AppLock, PasswordOperation};
    use gpui_kit::{AppContext as _, Entity, TestAppContext};
    use zeroize::Zeroizing;

    fn submit(
        cx: &mut TestAppContext,
        lock: &Entity<AppLock>,
        operation: PasswordOperation,
        current: &str,
        new: &str,
    ) -> bool {
        let task = cx.update(|cx| {
            lock.update(cx, |lock, cx| {
                lock.submit(
                    operation,
                    Zeroizing::new(current.into()),
                    Zeroizing::new(new.into()),
                    cx,
                )
            })
        });
        cx.foreground_executor.block_test(task).is_ok()
    }

    #[gpui_kit::test]
    fn password_lifecycle_survives_restart_and_rejects_wrong_passwords(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app-lock.json");
        let lock = cx.update(|cx| cx.new(|_| AppLock::load(path.clone())));
        assert!(!submit(cx, &lock, PasswordOperation::Enable, "", "short"));
        assert!(submit(
            cx,
            &lock,
            PasswordOperation::Enable,
            "",
            "first password"
        ));
        let lock = cx.update(|cx| cx.new(|_| AppLock::load(path.clone())));
        cx.update(|cx| assert!(lock.read(cx).is_locked()));
        assert!(!submit(
            cx,
            &lock,
            PasswordOperation::Unlock,
            "incorrect",
            ""
        ));
        cx.update(|cx| assert!(lock.read(cx).is_locked()));
        // Changing the verifier is impossible until the gate is open.
        assert!(!submit(
            cx,
            &lock,
            PasswordOperation::Change,
            "first password",
            "second password"
        ));
        assert!(submit(
            cx,
            &lock,
            PasswordOperation::Unlock,
            "first password",
            ""
        ));
        assert!(!submit(
            cx,
            &lock,
            PasswordOperation::Change,
            "incorrect",
            "second password"
        ));
        assert!(!submit(
            cx,
            &lock,
            PasswordOperation::Disable,
            "incorrect",
            ""
        ));
        assert!(submit(
            cx,
            &lock,
            PasswordOperation::Change,
            "first password",
            "second password"
        ));
        let lock = cx.update(|cx| cx.new(|_| AppLock::load(path.clone())));
        assert!(!submit(
            cx,
            &lock,
            PasswordOperation::Unlock,
            "first password",
            ""
        ));
        assert!(submit(
            cx,
            &lock,
            PasswordOperation::Unlock,
            "second password",
            ""
        ));
        assert!(submit(
            cx,
            &lock,
            PasswordOperation::Disable,
            "second password",
            ""
        ));
        let restarted = AppLock::load(path);
        assert!(!restarted.is_enabled());
        assert!(!restarted.is_locked());
    }

    #[gpui_kit::test]
    fn failed_writes_keep_the_original_lock_and_corruption_fails_closed(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app-lock.json");
        let lock = cx.update(|cx| cx.new(|_| AppLock::load(path.clone())));
        assert!(submit(
            cx,
            &lock,
            PasswordOperation::Enable,
            "",
            "first password"
        ));
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(!submit(
            cx,
            &lock,
            PasswordOperation::Disable,
            "first password",
            ""
        ));
        assert!(!submit(
            cx,
            &lock,
            PasswordOperation::Change,
            "first password",
            "second password"
        ));
        cx.update(|cx| {
            assert!(lock.read(cx).is_enabled());
            lock.update(cx, |lock, cx| lock.lock(cx));
        });
        assert!(submit(
            cx,
            &lock,
            PasswordOperation::Unlock,
            "first password",
            ""
        ));
        let unreadable = AppLock::load(path);
        assert!(unreadable.is_locked());
        assert!(unreadable.load_error().is_some());
    }
}
