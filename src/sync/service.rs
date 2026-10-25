//! The sync service: when rounds run and how one goes, and the jobs that
//! set sync up, change its password and turn it off.
//!
//! The blocking calls run on the background executor; a round's two halves
//! there and its middle, applying the changes, on the UI thread with the
//! store. One round runs at a time, and none beside a job: asked for
//! meanwhile, it runs once that ends. A round that would need the user
//! (a new password, a reset, many deletions) stops and says so; automatic
//! rounds then wait until the user has dealt with it.

use std::mem;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use gpui_kit::*;
use shellrs_sync::{
    DeviceId, DeviceInfo, JoinMode, KdfParams, LocalFolderStore, Pulled, Pushed, RemoteDevice,
    RoundOptions, SyncError, SyncState, SyncStore,
};
use zeroize::Zeroizing;

use super::{JobError, worker};
use crate::host::{ApplyError, HostStore};
use crate::secrets::SecretStore;

/// After an edit, how long the user may go on editing before a round runs.
pub const EDIT_PAUSE: Duration = Duration::from_secs(5);
/// A window coming forward runs a round no more often than this.
const ACTIVATION_GAP: Duration = Duration::from_secs(60);
/// Tries a round makes when its changes meet newer ones.
const ATTEMPTS: usize = 3;
/// How long quitting waits for the last round, and how often it looks.
const QUIT_WAIT: Duration = Duration::from_secs(3);
const QUIT_POLL: Duration = Duration::from_millis(50);

/// Opens the storage in a folder.
pub type OpenStore = Arc<dyn Fn(&Path) -> Arc<dyn SyncStore> + Send + Sync>;

/// What the service works with. Production has the real ones
/// ([`SyncServices::system`]); the UI tests share one `MemoryStore`
/// between workspaces.
#[derive(Clone)]
pub struct SyncServices {
    /// The storage in a folder.
    pub open: OpenStore,
    /// What deriving the key of a new sync password costs.
    pub kdf: KdfParams,
    /// This computer, as the other devices list it.
    pub device: DeviceInfo,
}

impl SyncServices {
    pub fn system() -> Self {
        Self {
            open: Arc::new(|folder| Arc::new(LocalFolderStore::new(folder))),
            kdf: KdfParams::DEFAULT,
            device: DeviceInfo {
                name: crate::app::computer_name().unwrap_or_else(|| "ShellRS".into()),
                platform: std::env::consts::OS.into(),
                app_version: crate::update::build_info::VERSION.into(),
            },
        }
    }
}

/// Where sync stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SyncPhase {
    Off,
    /// On, and no round has gone through yet this run.
    Waiting,
    Syncing,
    /// The last round went through, at this time (milliseconds since the
    /// epoch).
    Synced {
        at_ms: u64,
    },
    Failed(JobError),
}

/// What only the user can settle. Automatic rounds wait for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Attention {
    /// The sync password was changed on another device, or the keychain
    /// lost the keys: the password is needed.
    NeedsPassword,
    /// Sync was reset on another device: join it anew, or turn it off.
    VaultReplaced,
    /// This computer was removed from sync on another device: join again,
    /// or turn it off.
    Removed,
    /// A round would delete `count` of the `of` records synced: delete, or
    /// leave sync waiting.
    MassDelete { count: u32, of: u32 },
}

/// Everything the settings page and the dialogs show, as a plain value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncSnapshot {
    /// Whether this copy can sync: it has the services and a keychain.
    pub available: bool,
    pub folder: Option<PathBuf>,
    pub phase: SyncPhase,
    pub attention: Option<Attention>,
    /// This computer, as the others list it.
    pub device: Option<DeviceInfo>,
    /// The other devices, as the last round found them.
    pub devices: Vec<RemoteDevice>,
    /// A round or a job runs.
    pub busy: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SyncEvent {
    /// The user has something to settle, newly.
    NeedsAttention(Attention),
    /// Sync was set up or joined in this folder: keep it in the settings.
    Joined(PathBuf),
    /// Sync was turned off: forget the folder.
    TurnedOff,
}

/// How a round ended.
enum Ended {
    Synced {
        revision: u64,
        devices: Vec<RemoteDevice>,
    },
    Attention(Attention),
    Failed(JobError),
}

pub struct SyncService {
    store: Entity<HostStore>,
    services: Option<SyncServices>,
    folder: Option<PathBuf>,
    phase: SyncPhase,
    attention: Option<Attention>,
    devices: Vec<RemoteDevice>,
    round: Option<Task<()>>,
    /// A job other than a round runs.
    job_running: bool,
    /// A round was asked for while one ran: run it once that ends.
    again: bool,
    /// The next round may delete however many records, as the user agreed.
    allow_mass_delete: bool,
    /// The next round may write this computer's file again, removed
    /// elsewhere as it was.
    accept_removed: bool,
    /// The store's revision the last round that went through had read.
    synced_revision: Option<u64>,
    /// The revision last seen, to tell edits from other notifications.
    seen_revision: u64,
    last_round_at: Option<Instant>,
    pause: Option<Task<()>>,
    _store: Subscription,
}

impl EventEmitter<SyncEvent> for SyncService {}

impl SyncService {
    /// A service with nothing to work with: it never syncs. The workspace
    /// gives production its services.
    pub fn new(store: Entity<HostStore>, cx: &mut Context<Self>) -> Self {
        let seen_revision = store.read(cx).sync_revision();
        let subscription = cx.observe(&store, |this, store, cx| {
            let revision = store.read(cx).sync_revision();
            if revision != this.seen_revision {
                this.seen_revision = revision;
                this.sync_after_pause(cx);
            }
        });
        Self {
            store,
            services: None,
            folder: None,
            phase: SyncPhase::Off,
            attention: None,
            devices: Vec::new(),
            round: None,
            job_running: false,
            again: false,
            allow_mass_delete: false,
            accept_removed: false,
            synced_revision: None,
            seen_revision,
            last_round_at: None,
            pause: None,
            _store: subscription,
        }
    }

    pub fn set_services(&mut self, services: SyncServices, cx: &mut Context<Self>) {
        self.services = Some(services);
        cx.notify();
        self.sync_automatically(cx);
    }

    pub fn snapshot(&self, cx: &App) -> SyncSnapshot {
        SyncSnapshot {
            available: self.available(cx),
            folder: self.folder.clone(),
            phase: self.phase.clone(),
            attention: self.attention,
            device: self
                .services
                .as_ref()
                .map(|services| services.device.clone()),
            devices: self.devices.clone(),
            busy: self.busy(),
        }
    }

    fn available(&self, cx: &App) -> bool {
        self.services.is_some() && self.store.read(cx).secrets().is_available()
    }

    fn busy(&self) -> bool {
        self.round.is_some() || self.job_running
    }

    /// The folder from the settings: sync is on while there is one.
    pub fn set_folder(&mut self, folder: Option<PathBuf>, cx: &mut Context<Self>) {
        if folder == self.folder {
            return;
        }
        self.folder = folder;
        self.devices.clear();
        self.attention = None;
        self.synced_revision = None;
        self.phase = if self.folder.is_some() {
            SyncPhase::Waiting
        } else {
            SyncPhase::Off
        };
        cx.notify();
        self.sync_automatically(cx);
    }

    /// 立即同步: a round now, whatever waited for the user (it asks again
    /// if it still does).
    pub fn sync_now(&mut self, cx: &mut Context<Self>) {
        self.attention = None;
        self.start_round(cx);
    }

    /// The window came forward: a round, unless one ran just now.
    pub fn window_activated(&mut self, cx: &mut Context<Self>) {
        if self
            .last_round_at
            .is_none_or(|at| at.elapsed() >= ACTIVATION_GAP)
        {
            self.sync_automatically(cx);
        }
    }

    /// The user agreed to the deletions a round would make.
    pub fn allow_mass_delete(&mut self, cx: &mut Context<Self>) {
        self.allow_mass_delete = true;
        self.sync_now(cx);
    }

    /// The user wants this computer back in sync, removed elsewhere as it
    /// was.
    pub fn accept_removed(&mut self, cx: &mut Context<Self>) {
        self.accept_removed = true;
        self.sync_now(cx);
    }

    fn sync_automatically(&mut self, cx: &mut Context<Self>) {
        if self.attention.is_none() {
            self.start_round(cx);
        }
    }

    /// A round once the user has stopped editing for a while.
    fn sync_after_pause(&mut self, cx: &mut Context<Self>) {
        if self.folder.is_none() {
            return;
        }
        self.pause = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(EDIT_PAUSE).await;
            this.update(cx, |this, cx| this.sync_automatically(cx)).ok();
        }));
    }

    fn start_round(&mut self, cx: &mut Context<Self>) {
        if self.busy() {
            self.again = true;
            return;
        }
        let (Some(services), Some(folder)) = (self.services.clone(), self.folder.clone()) else {
            return;
        };
        if !self.available(cx) {
            self.phase = SyncPhase::Failed(JobError::Unavailable);
            cx.notify();
            return;
        }
        self.pause = None;
        let remote = (services.open)(&folder);
        let opts = RoundOptions {
            allow_mass_delete: mem::take(&mut self.allow_mass_delete),
            join: JoinMode::Merge,
        };
        let accept_removed = mem::take(&mut self.accept_removed);
        let store = self.store.clone();
        self.phase = SyncPhase::Syncing;
        cx.notify();
        self.round = Some(cx.spawn(async move |this, cx| {
            let ended = round(store, remote, services.device, opts, accept_removed, cx).await;
            this.update(cx, |this, cx| this.round_ended(ended, cx)).ok();
        }));
    }

    fn round_ended(&mut self, ended: Ended, cx: &mut Context<Self>) {
        self.round = None;
        self.last_round_at = Some(Instant::now());
        match ended {
            Ended::Synced { revision, devices } => {
                self.phase = SyncPhase::Synced { at_ms: now_ms() };
                self.devices = devices;
                self.synced_revision = Some(revision);
            }
            Ended::Attention(attention) => {
                // What the last round that went through found still stands.
                self.phase = match &self.phase {
                    SyncPhase::Syncing => SyncPhase::Waiting,
                    phase => phase.clone(),
                };
                if self.attention != Some(attention) {
                    self.attention = Some(attention);
                    cx.emit(SyncEvent::NeedsAttention(attention));
                }
            }
            Ended::Failed(error) => self.phase = SyncPhase::Failed(error),
        }
        cx.notify();
        if mem::take(&mut self.again) {
            self.sync_automatically(cx);
        }
    }

    /// Before quitting: a round if anything is not synced yet, and a task
    /// that ends with it, or after [`QUIT_WAIT`]. `None` when there is
    /// nothing to wait for.
    pub fn sync_before_quit(&mut self, cx: &mut Context<Self>) -> Option<Task<()>> {
        if self.folder.is_none() || self.attention.is_some() || !self.available(cx) {
            return None;
        }
        let revision = self.store.read(cx).sync_revision();
        if self.round.is_none() {
            if self.job_running || self.synced_revision == Some(revision) {
                return None;
            }
            self.start_round(cx);
        }
        Some(cx.spawn(async move |this, cx| {
            let mut waited = Duration::ZERO;
            while waited < QUIT_WAIT {
                cx.background_executor().timer(QUIT_POLL).await;
                waited += QUIT_POLL;
                let running = this.read_with(cx, |this, _| this.round.is_some());
                if !matches!(running, Ok(true)) {
                    break;
                }
            }
        }))
    }

    /// Whether sync is set up in `folder` already: to join it, or to make
    /// it.
    pub fn inspect(
        &mut self,
        folder: &Path,
        cx: &mut Context<Self>,
    ) -> Task<Result<bool, JobError>> {
        self.job(folder, cx, |remote, _, _| worker::inspect(remote))
    }

    /// Set sync up in `folder` with a new password.
    pub fn create(
        &mut self,
        folder: PathBuf,
        password: Zeroizing<String>,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), JobError>> {
        let job = self.job(&folder, cx, move |remote, secrets, services| {
            worker::create(remote, secrets, &password, services.kdf)
        });
        cx.spawn(async move |this, cx| {
            job.await?;
            this.update(cx, |this, cx| this.joined(folder, true, cx))
                .ok();
            Ok(())
        })
    }

    /// Join the sync in `folder` with its password. `fresh` forgets what
    /// this computer synced before: sync was reset, or this is its first
    /// time. The password changed elsewhere keeps it.
    pub fn join(
        &mut self,
        folder: PathBuf,
        password: Zeroizing<String>,
        fresh: bool,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), JobError>> {
        let job = self.job(&folder, cx, move |remote, secrets, _| {
            worker::join(remote, secrets, &password)
        });
        cx.spawn(async move |this, cx| {
            job.await?;
            this.update(cx, |this, cx| this.joined(folder, fresh, cx))
                .ok();
            Ok(())
        })
    }

    fn joined(&mut self, folder: PathBuf, fresh: bool, cx: &mut Context<Self>) {
        if fresh {
            self.store.update(cx, |store, _| store.clear_sync_state());
            self.synced_revision = None;
            self.devices.clear();
        }
        self.attention = None;
        self.folder = Some(folder.clone());
        if self.phase == SyncPhase::Off {
            self.phase = SyncPhase::Waiting;
        }
        cx.emit(SyncEvent::Joined(folder));
        self.start_round(cx);
    }

    /// 更换同步密码, and with `rotate` 更换密钥 after it: what the other
    /// devices hold no longer opens anything new.
    pub fn change_password(
        &mut self,
        password: Zeroizing<String>,
        rotate: bool,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), JobError>> {
        let Some(folder) = self.folder.clone() else {
            return Task::ready(Err(JobError::Unavailable));
        };
        let job = self.job(&folder, cx, move |remote, secrets, services| {
            worker::new_password(remote, secrets, &password, rotate, services.kdf)
        });
        cx.spawn(async move |this, cx| {
            job.await?;
            // Written again with the new key.
            this.update(cx, |this, cx| this.start_round(cx)).ok();
            Ok(())
        })
    }

    /// 移除 a device: its file goes, and it leaves the list.
    pub fn remove_device(
        &mut self,
        device: DeviceId,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), JobError>> {
        let Some(folder) = self.folder.clone() else {
            return Task::ready(Err(JobError::Unavailable));
        };
        let gone = device.clone();
        let job = self.job(&folder, cx, move |remote, _, _| {
            worker::remove_device(remote, &device)
        });
        cx.spawn(async move |this, cx| {
            job.await?;
            this.update(cx, |this, cx| {
                this.devices.retain(|device| device.id != gone);
                cx.notify();
            })
            .ok();
            Ok(())
        })
    }

    /// 关闭同步: the data here stays as it is; the keys and the state go,
    /// and with `delete_file` this computer's file in the folder too.
    pub fn turn_off(
        &mut self,
        delete_file: bool,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), JobError>> {
        let Some(folder) = self.folder.clone() else {
            return Task::ready(Ok(()));
        };
        let state = self.store.read(cx).sync_state();
        let job = self.job(&folder, cx, move |remote, secrets, _| {
            if delete_file
                && let Some(state) = state.and_then(|bytes| SyncState::from_bytes(&bytes).ok())
            {
                worker::remove_device(remote, state.device())?;
            }
            // Left in the keychain, they open nothing new once the next
            // set-up writes over them.
            let _ = worker::forget_keyring(secrets);
            Ok(())
        });
        cx.spawn(async move |this, cx| {
            job.await?;
            this.update(cx, |this, cx| this.turned_off(cx)).ok();
            Ok(())
        })
    }

    fn turned_off(&mut self, cx: &mut Context<Self>) {
        self.store.update(cx, |store, _| store.clear_sync_state());
        self.folder = None;
        self.phase = SyncPhase::Off;
        self.attention = None;
        self.devices.clear();
        self.synced_revision = None;
        self.again = false;
        cx.emit(SyncEvent::TurnedOff);
        cx.notify();
    }

    /// Run `work` on the background executor, alone: no round beside it.
    fn job<R: Send + 'static>(
        &mut self,
        folder: &Path,
        cx: &mut Context<Self>,
        work: impl FnOnce(&dyn SyncStore, &dyn SecretStore, &SyncServices) -> Result<R, JobError>
        + Send
        + 'static,
    ) -> Task<Result<R, JobError>> {
        let Some(services) = self.services.clone().filter(|_| self.available(cx)) else {
            return Task::ready(Err(JobError::Unavailable));
        };
        if self.busy() {
            return Task::ready(Err(JobError::Busy));
        }
        self.job_running = true;
        cx.notify();
        let remote = (services.open)(folder);
        let secrets = self.store.read(cx).secrets();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { work(&*remote, &*secrets, &services) })
                .await;
            this.update(cx, |this, cx| {
                this.job_running = false;
                cx.notify();
                if mem::take(&mut this.again) {
                    this.sync_automatically(cx);
                }
            })
            .ok();
            result
        })
    }
}

/// One round, tried again while its changes meet newer ones.
async fn round(
    store: Entity<HostStore>,
    remote: Arc<dyn SyncStore>,
    device: DeviceInfo,
    opts: RoundOptions,
    accept_removed: bool,
    cx: &mut AsyncApp,
) -> Ended {
    for _ in 0..ATTEMPTS {
        let (export, state, secrets, key_dir) = store.read_with(cx, |store, _| {
            (
                store.sync_export(),
                store.sync_state(),
                store.secrets(),
                store.key_dir().map(Path::to_path_buf),
            )
        });
        let pulled =
            cx.background_executor()
                .spawn({
                    let (remote, secrets, export) =
                        (remote.clone(), secrets.clone(), export.clone());
                    async move {
                        worker::pull_round(&*remote, &*secrets, &export, state.as_deref(), opts)
                    }
                })
                .await;
        let (pulled, keyring) = match pulled {
            Ok(pulled) => pulled,
            Err(error) => return stopped(error),
        };
        if pulled.report.removed && !accept_removed {
            return Ended::Attention(Attention::Removed);
        }
        let state_bytes = pulled.state.to_bytes();
        let applied = store.update(cx, |store, cx| {
            store.apply_sync(&pulled.changes, &state_bytes, &export, cx)
        });
        let work = match applied {
            Ok(work) => work,
            Err(ApplyError::Stale) => continue,
            Err(ApplyError::NotSaved(message)) => {
                return Ended::Failed(JobError::NotSaved(message));
            }
        };
        let Pulled { state, report, .. } = pulled;
        let (written, pushed) = cx
            .background_executor()
            .spawn({
                let (remote, device) = (remote.clone(), device.clone());
                async move {
                    worker::push_round(
                        &*remote,
                        &*secrets,
                        key_dir.as_deref(),
                        work,
                        &keyring,
                        &device,
                        &state,
                    )
                }
            })
            .await;
        store.update(cx, |store, cx| store.adopt_synced_secrets(&written, cx));
        match pushed {
            Ok(Pushed::Uploaded { state }) => {
                let saved = store.update(cx, |store, _| store.save_sync_state(&state.to_bytes()));
                if let Err(message) = saved {
                    return Ended::Failed(JobError::NotSaved(message));
                }
            }
            Ok(Pushed::Unchanged) => {}
            Ok(Pushed::Conflict) => continue,
            Err(error) => return stopped(error),
        }
        return Ended::Synced {
            revision: export.revision,
            devices: report.devices,
        };
    }
    Ended::Failed(JobError::Busy)
}

/// A round's error, or what the user is to settle when only they can.
fn stopped(error: JobError) -> Ended {
    match error {
        JobError::NoKeyring | JobError::Sync(SyncError::NeedsPassword) => {
            Ended::Attention(Attention::NeedsPassword)
        }
        JobError::Sync(SyncError::VaultReplaced) => Ended::Attention(Attention::VaultReplaced),
        JobError::Sync(SyncError::MassDelete { count, of }) => {
            Ended::Attention(Attention::MassDelete { count, of })
        }
        error => Ended::Failed(error),
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64)
}
