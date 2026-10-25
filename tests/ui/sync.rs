//! 同步: setting it up from 设置 and joining it, edits reaching the other
//! computer, what waits for the user, and turning it off. The folder is one
//! `MemoryStore` shared by every computer of a test.

use shellrs::app::{RemoveSyncDevice, ReviewSync, SetUpSync, SyncNow, TurnOffSync};
use shellrs::sync::{Attention, EDIT_PAUSE, SyncPhase, SyncService, SyncServices};
use shellrs_sync::{DeviceInfo, KdfParams, MemoryStore, SyncStore, layout};
use zeroize::Zeroizing;

use crate::support::*;

const FOLDER: &str = "/Users/me/Dropbox/ShellRS";
const PASSWORD: &str = "correct horse battery";

/// The folder, whatever path is picked, with keys that derive at once.
fn services(folder: &Arc<MemoryStore>, name: &str) -> SyncServices {
    let folder = folder.clone();
    SyncServices {
        open: Arc::new(move |_| folder.clone() as Arc<dyn SyncStore>),
        kdf: KdfParams {
            memory_kib: 8,
            passes: 1,
            parallelism: 1,
        },
        device: DeviceInfo {
            name: name.into(),
            platform: "macos".into(),
            app_version: "0.1.15".into(),
        },
    }
}

/// The computer with the window: the seeded hosts, a keychain in memory.
struct Desk {
    handle: WindowHandle<Root>,
    workspace: Entity<Workspace>,
    secrets: Arc<InMemorySecretStore>,
    _keys: tempfile::TempDir,
}

fn desk(cx: &mut TestAppContext, folder: &Arc<MemoryStore>) -> Desk {
    let secrets = Arc::new(InMemorySecretStore::default());
    let keys = tempfile::tempdir().unwrap();
    let store = HostStore::seed()
        .with_secrets(secrets.clone())
        .with_key_dir(keys.path());
    let (handle, workspace) = open_workspace_with_store(cx, store);
    let services = services(folder, "desk");
    cx.update(|cx| {
        let sync = workspace.read(cx).sync().clone();
        sync.update(cx, |sync, cx| sync.set_services(services, cx));
    });
    Desk {
        handle,
        workspace,
        secrets,
        _keys: keys,
    }
}

impl Desk {
    fn sync(&self, cx: &mut TestAppContext) -> Entity<SyncService> {
        cx.update(|cx| self.workspace.read(cx).sync().clone())
    }

    fn store(&self, cx: &mut TestAppContext) -> Entity<HostStore> {
        cx.update(|cx| self.workspace.read(cx).store().clone())
    }

    fn folder_setting(&self, cx: &mut TestAppContext) -> Option<std::path::PathBuf> {
        cx.update(|cx| {
            self.workspace
                .read(cx)
                .settings()
                .read(cx)
                .settings()
                .sync
                .folder
        })
    }
}

/// Another computer, without a window: its own store, keychain and sync.
struct Laptop {
    store: Entity<HostStore>,
    sync: Entity<SyncService>,
    _keys: tempfile::TempDir,
}

fn laptop(cx: &mut TestAppContext, folder: &Arc<MemoryStore>) -> Laptop {
    let keys = tempfile::tempdir().unwrap();
    let store = HostStore::empty()
        .with_secrets(Arc::new(InMemorySecretStore::default()))
        .with_key_dir(keys.path());
    let store = cx.new(|_| store);
    let services = services(folder, "laptop");
    let sync = cx.new(|cx| {
        let mut sync = SyncService::new(store.clone(), cx);
        sync.set_services(services, cx);
        sync
    });
    Laptop {
        store,
        sync,
        _keys: keys,
    }
}

impl Laptop {
    /// Join the sync in the folder.
    async fn join(&self, cx: &mut TestAppContext) {
        let join = self.sync.update(cx, |sync, cx| {
            sync.join(FOLDER.into(), Zeroizing::new(PASSWORD.into()), true, cx)
        });
        join.await.unwrap();
        cx.run_until_parked();
    }

    fn sync_now(&self, cx: &mut TestAppContext) {
        self.sync.update(cx, |sync, cx| sync.sync_now(cx));
        cx.run_until_parked();
    }
}

fn host_names(store: &Entity<HostStore>, cx: &mut TestAppContext) -> Vec<String> {
    store.read_with(cx, |store, _| {
        let mut names: Vec<String> = store.hosts().iter().map(|h| h.name.to_string()).collect();
        names.sort();
        names
    })
}

/// Open 设置 › 同步.
fn open_sync_settings(cx: &mut TestAppContext, handle: WindowHandle<Root>) {
    in_frame(cx, handle, |window, cx| window.click("open-settings", cx));
    in_frame(cx, handle, |window, cx| {
        window.within("settings").click("0-6", cx)
    });
}

/// Set sync up from 设置 › 同步 on the desk: the folder picked, the
/// password typed twice.
async fn set_up_on_the_desk(cx: &mut TestAppContext, desk: &Desk) {
    let handle = desk.handle;
    open_sync_settings(cx, handle);
    in_frame(cx, handle, |window, cx| window.click("sync-set-up", cx));
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|_| Some(vec![FOLDER.into()]));
    cx.run_until_parked();
    in_frame(cx, handle, |window, cx| {
        window.click("sync-password", cx);
        window.input(PASSWORD, cx);
        window.click("sync-password-repeat", cx);
        window.input(PASSWORD, cx);
        window.click("sync-commit", cx);
    });
    cx.wait_for(handle.into(), Duration::from_secs(2), |window, _| {
        window.try_find("sync-commit").is_none()
    })
    .await;
    cx.run_until_parked();
}

#[gpui_kit::test]
async fn sync_is_set_up_on_one_computer_and_joined_on_another(cx: &mut TestAppContext) {
    let folder = Arc::new(MemoryStore::new());
    let desk = desk(cx, &folder);
    set_up_on_the_desk(cx, &desk).await;

    assert_eq!(desk.folder_setting(cx), Some(FOLDER.into()));
    assert!(folder.raw(layout::VAULT).is_some());
    // The keys went to the keychain, never to the folder in the clear.
    assert!(desk.secrets.get(&SecretRef::SyncKeyring).unwrap().is_some());
    let phase = desk
        .sync(cx)
        .read_with(cx, |sync, cx| sync.snapshot(cx).phase);
    assert!(matches!(phase, SyncPhase::Synced { .. }), "{phase:?}");
    in_frame(cx, desk.handle, |window, _| {
        let status = window.find("sync-status").label().unwrap().to_string();
        assert!(status.starts_with("上次同步："), "{status}");
        assert_eq!(window.find("sync-folder").label(), Some(FOLDER));
    });

    let laptop = laptop(cx, &folder);
    laptop.join(cx).await;
    let desk_hosts = host_names(&desk.store(cx), cx);
    assert_eq!(desk_hosts.len(), 6);
    assert_eq!(host_names(&laptop.store, cx), desk_hosts);

    // The desk lists the laptop once it has looked again.
    in_frame(cx, desk.handle, |window, cx| {
        window.dispatch_action(Box::new(SyncNow), cx)
    });
    let devices = desk
        .sync(cx)
        .read_with(cx, |sync, cx| sync.snapshot(cx).devices);
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].info.name, "laptop");
    let remove = ElementId::Name(format!("remove-sync-device-{}", devices[0].id).into());
    in_frame(cx, desk.handle, |window, _| {
        assert!(window.try_find(remove).is_some());
    });
}

#[gpui_kit::test]
async fn a_wrong_password_is_said_over_the_dialog(cx: &mut TestAppContext) {
    let folder = Arc::new(MemoryStore::new());
    let laptop = laptop(cx, &folder);
    let create = laptop.sync.update(cx, |sync, cx| {
        sync.create(FOLDER.into(), Zeroizing::new(PASSWORD.into()), cx)
    });
    create.await.unwrap();
    cx.run_until_parked();

    let desk = desk(cx, &folder);
    let handle = desk.handle;
    open_sync_settings(cx, handle);
    in_frame(cx, handle, |window, cx| window.click("sync-set-up", cx));
    cx.simulate_path_prompt_response(|_| Some(vec![FOLDER.into()]));
    cx.run_until_parked();
    in_frame(cx, handle, |window, cx| {
        // The folder has sync: one field, to join it.
        assert!(window.try_find("sync-password-repeat").is_none());
        window.click("sync-password", cx);
        window.input("battery horse staple", cx);
        window.click("sync-commit", cx);
    });
    cx.wait_for(handle.into(), Duration::from_secs(2), |window, _| {
        window.try_find("form-error").is_some()
    })
    .await;
    in_frame(cx, handle, |window, _| {
        assert_eq!(window.find("form-error").label(), Some("同步密码不对。"));
        assert!(window.try_find("sync-commit").is_some());
    });
    assert_eq!(desk.folder_setting(cx), None);
}

#[gpui_kit::test]
async fn a_new_password_is_at_least_twelve_characters_and_typed_the_same_twice(
    cx: &mut TestAppContext,
) {
    let folder = Arc::new(MemoryStore::new());
    let desk = desk(cx, &folder);
    let handle = desk.handle;
    open_sync_settings(cx, handle);
    in_frame(cx, handle, |window, cx| window.click("sync-set-up", cx));
    cx.simulate_path_prompt_response(|_| Some(vec![FOLDER.into()]));
    cx.run_until_parked();
    in_frame(cx, handle, |window, cx| {
        window.click("sync-password", cx);
        window.input("short", cx);
        window.click("sync-commit", cx);
    });
    in_frame(cx, handle, |window, cx| {
        assert_eq!(
            window.find("form-error").label(),
            Some("同步密码至少要 12 个字符。")
        );
        window.click("sync-generate-password", cx);
    });
    in_frame(cx, handle, |window, cx| {
        window.click("sync-password-repeat", cx);
        window.input("x", cx);
        window.click("sync-commit", cx);
    });
    in_frame(cx, handle, |window, _| {
        assert_eq!(
            window.find("form-error").label(),
            Some("两次输入的同步密码不一样。")
        );
    });
    assert!(folder.raw(layout::VAULT).is_none());
}

#[gpui_kit::test]
async fn an_edit_reaches_the_other_computer_after_a_pause(cx: &mut TestAppContext) {
    let folder = Arc::new(MemoryStore::new());
    let desk = desk(cx, &folder);
    set_up_on_the_desk(cx, &desk).await;
    let laptop = laptop(cx, &folder);
    laptop.join(cx).await;

    let store = desk.store(cx);
    store.update(cx, |store, cx| {
        store.insert(
            HostDraft::new("cache-01", "10.0.3.1", 22, "root", AuthKind::Password, None),
            cx,
        );
    });
    cx.run_until_parked();
    laptop.sync_now(cx);
    assert!(!host_names(&laptop.store, cx).contains(&"cache-01".to_string()));

    // Once the user has stopped editing for a while.
    cx.executor().advance_clock(EDIT_PAUSE);
    cx.run_until_parked();
    laptop.sync_now(cx);
    assert!(host_names(&laptop.store, cx).contains(&"cache-01".to_string()));

    // And back: the laptop's edit shows in the desk's host tree.
    laptop.store.update(cx, |store, cx| {
        store.insert(
            HostDraft::new("queue-01", "10.0.4.1", 22, "root", AuthKind::Password, None),
            cx,
        );
    });
    laptop.sync_now(cx);
    in_frame(cx, desk.handle, |window, cx| {
        window.dispatch_action(Box::new(SyncNow), cx)
    });
    assert!(host_names(&store, cx).contains(&"queue-01".to_string()));
}

#[gpui_kit::test]
async fn passwords_sync_with_the_hosts(cx: &mut TestAppContext) {
    let folder = Arc::new(MemoryStore::new());
    let desk = desk(cx, &folder);
    let store = desk.store(cx);
    let secret = SecretRef::password("root", "10.0.1.12", 22);
    store.update(cx, |store, cx| {
        store.save_secret(secret.clone(), Some("pw-web".into()), cx);
    });
    cx.run_until_parked();
    set_up_on_the_desk(cx, &desk).await;

    let laptop = laptop(cx, &folder);
    laptop.join(cx).await;
    let secrets = laptop.store.read_with(cx, |store, _| store.secrets());
    assert_eq!(
        secrets.get(&secret).unwrap().as_deref().map(String::as_str),
        Some("pw-web")
    );
}

#[gpui_kit::test]
async fn many_deletions_wait_for_the_user(cx: &mut TestAppContext) {
    let folder = Arc::new(MemoryStore::new());
    let desk = desk(cx, &folder);
    let store = desk.store(cx);
    store.update(cx, |store, cx| {
        for ix in 0..10 {
            store.insert(
                HostDraft::new(
                    format!("node-{ix:02}"),
                    format!("10.0.9.{ix}"),
                    22,
                    "root",
                    AuthKind::Password,
                    None,
                ),
                cx,
            );
        }
    });
    set_up_on_the_desk(cx, &desk).await;
    let laptop = laptop(cx, &folder);
    laptop.join(cx).await;

    // Every host and group gone at once, as a fault might do it.
    store.update(cx, |store, cx| {
        let groups: Vec<GroupId> = store.groups().iter().map(|group| group.id).collect();
        for group in groups {
            store.remove_group(group, cx);
        }
        let hosts: Vec<HostId> = store.hosts().iter().map(|host| host.id).collect();
        for host in hosts {
            store.remove(host, cx);
        }
    });
    in_frame(cx, desk.handle, |window, cx| {
        window.dispatch_action(Box::new(SyncNow), cx)
    });
    let attention = desk
        .sync(cx)
        .read_with(cx, |sync, cx| sync.snapshot(cx).attention);
    assert!(
        matches!(attention, Some(Attention::MassDelete { count, .. }) if count >= 10),
        "{attention:?}"
    );
    in_frame(cx, desk.handle, |window, cx| {
        assert_eq!(window.notifications(cx).len(), 1);
        assert!(window.try_find("review-sync-notice").is_some());
        assert!(window.within("settings").try_find("review-sync").is_some());
    });
    laptop.sync_now(cx);
    assert_eq!(host_names(&laptop.store, cx).len(), 16);

    // Agreed to, they go everywhere.
    in_frame(cx, desk.handle, |window, cx| {
        window.dispatch_action(Box::new(ReviewSync), cx)
    });
    in_frame(cx, desk.handle, |window, cx| window.click("ok", cx));
    cx.run_until_parked();
    laptop.sync_now(cx);
    assert!(host_names(&laptop.store, cx).is_empty());
}

#[gpui_kit::test]
async fn a_removed_computer_asks_before_it_comes_back(cx: &mut TestAppContext) {
    let folder = Arc::new(MemoryStore::new());
    let desk = desk(cx, &folder);
    set_up_on_the_desk(cx, &desk).await;
    let laptop = laptop(cx, &folder);
    laptop.join(cx).await;
    in_frame(cx, desk.handle, |window, cx| {
        window.dispatch_action(Box::new(SyncNow), cx)
    });

    let device = desk
        .sync(cx)
        .read_with(cx, |sync, cx| sync.snapshot(cx).devices[0].id.clone());
    in_frame(cx, desk.handle, |window, cx| {
        window.dispatch_action(Box::new(RemoveSyncDevice(device.clone())), cx)
    });
    in_frame(cx, desk.handle, |window, cx| window.click("ok", cx));
    cx.run_until_parked();
    assert!(folder.raw(&layout::device_file(&device)).is_none());
    let devices = desk
        .sync(cx)
        .read_with(cx, |sync, cx| sync.snapshot(cx).devices.len());
    assert_eq!(devices, 0);

    laptop.sync_now(cx);
    let attention = laptop
        .sync
        .read_with(cx, |sync, cx| sync.snapshot(cx).attention);
    assert_eq!(attention, Some(Attention::Removed));
    assert!(folder.raw(&layout::device_file(&device)).is_none());

    laptop.sync.update(cx, |sync, cx| sync.accept_removed(cx));
    cx.run_until_parked();
    assert!(folder.raw(&layout::device_file(&device)).is_some());
}

#[gpui_kit::test]
async fn quitting_waits_for_the_last_edit_to_sync(cx: &mut TestAppContext) {
    let folder = Arc::new(MemoryStore::new());
    let desk = desk(cx, &folder);
    set_up_on_the_desk(cx, &desk).await;
    let sync = desk.sync(cx);
    // Nothing new: nothing to wait for.
    assert!(
        sync.update(cx, |sync, cx| sync.sync_before_quit(cx))
            .is_none()
    );

    desk.store(cx).update(cx, |store, cx| {
        store.insert(
            HostDraft::new("cache-01", "10.0.3.1", 22, "root", AuthKind::Password, None),
            cx,
        );
    });
    let wait = sync.update(cx, |sync, cx| sync.sync_before_quit(cx));
    assert!(wait.is_some());
    cx.run_until_parked();
    let laptop = laptop(cx, &folder);
    laptop.join(cx).await;
    assert!(host_names(&laptop.store, cx).contains(&"cache-01".to_string()));
}

#[gpui_kit::test]
async fn turning_sync_off_keeps_the_data_and_can_take_the_file_too(cx: &mut TestAppContext) {
    let folder = Arc::new(MemoryStore::new());
    let desk = desk(cx, &folder);
    set_up_on_the_desk(cx, &desk).await;
    let files = || {
        folder
            .paths()
            .into_iter()
            .filter(|path| path.starts_with(layout::DEVICES))
            .count()
    };
    assert_eq!(files(), 1);

    in_frame(cx, desk.handle, |window, cx| {
        window.dispatch_action(Box::new(TurnOffSync), cx)
    });
    in_frame(cx, desk.handle, |window, cx| {
        window.click("sync-delete-own-file", cx);
        window.click("sync-turn-off", cx);
    });
    cx.wait_for(desk.handle.into(), Duration::from_secs(2), |window, _| {
        window.try_find("sync-turn-off").is_none()
    })
    .await;
    cx.run_until_parked();

    assert_eq!(desk.folder_setting(cx), None);
    assert_eq!(files(), 0);
    assert!(desk.secrets.get(&SecretRef::SyncKeyring).unwrap().is_none());
    assert_eq!(host_names(&desk.store(cx), cx).len(), 6);
    assert_eq!(
        desk.store(cx).read_with(cx, |store, _| store.sync_state()),
        None
    );
    in_frame(cx, desk.handle, |window, _| {
        assert!(window.try_find("sync-set-up").is_some());
    });
}

#[gpui_kit::test]
async fn without_a_keychain_sync_cannot_be_turned_on(cx: &mut TestAppContext) {
    let (handle, workspace) = open_workspace(cx);
    let folder = Arc::new(MemoryStore::new());
    cx.update(|cx| {
        let sync = workspace.read(cx).sync().clone();
        sync.update(cx, |sync, cx| {
            sync.set_services(services(&folder, "desk"), cx)
        });
    });
    open_sync_settings(cx, handle);
    // The button does nothing, and the command says why.
    in_frame(cx, handle, |window, cx| window.click("sync-set-up", cx));
    assert!(!cx.did_prompt_for_paths());
    in_frame(cx, handle, |window, cx| {
        window.dispatch_action(Box::new(SetUpSync), cx)
    });
    cx.simulate_path_prompt_response(|_| Some(vec![FOLDER.into()]));
    cx.run_until_parked();
    in_frame(cx, handle, |window, cx| {
        assert!(window.try_find("sync-password").is_none());
        assert_eq!(window.notifications(cx).len(), 1);
    });
    assert!(folder.raw(layout::VAULT).is_none());
}

#[gpui_kit::test]
async fn a_new_password_and_key_are_asked_for_on_the_other_computer(cx: &mut TestAppContext) {
    let folder = Arc::new(MemoryStore::new());
    let desk = desk(cx, &folder);
    set_up_on_the_desk(cx, &desk).await;
    let laptop = laptop(cx, &folder);
    laptop.join(cx).await;

    let new_password = "staple battery horse";
    in_frame(cx, desk.handle, |window, cx| {
        window.click("rotate-sync-key", cx)
    });
    in_frame(cx, desk.handle, |window, cx| {
        window.click("sync-password", cx);
        window.input(new_password, cx);
        window.click("sync-password-repeat", cx);
        window.input(new_password, cx);
        window.click("sync-commit", cx);
    });
    cx.wait_for(desk.handle.into(), Duration::from_secs(2), |window, _| {
        window.try_find("sync-commit").is_none()
    })
    .await;
    cx.run_until_parked();

    desk.store(cx).update(cx, |store, cx| {
        store.insert(
            HostDraft::new("cache-01", "10.0.3.1", 22, "root", AuthKind::Password, None),
            cx,
        );
    });
    in_frame(cx, desk.handle, |window, cx| {
        window.dispatch_action(Box::new(SyncNow), cx)
    });
    laptop.sync_now(cx);
    let attention = laptop
        .sync
        .read_with(cx, |sync, cx| sync.snapshot(cx).attention);
    assert_eq!(attention, Some(Attention::NeedsPassword));
    assert!(!host_names(&laptop.store, cx).contains(&"cache-01".to_string()));

    // With the new password it goes on where it was.
    let join = laptop.sync.update(cx, |sync, cx| {
        sync.join(
            FOLDER.into(),
            Zeroizing::new(new_password.into()),
            false,
            cx,
        )
    });
    join.await.unwrap();
    cx.run_until_parked();
    assert_eq!(
        laptop
            .sync
            .read_with(cx, |sync, cx| sync.snapshot(cx).attention),
        None
    );
    assert!(host_names(&laptop.store, cx).contains(&"cache-01".to_string()));
}
