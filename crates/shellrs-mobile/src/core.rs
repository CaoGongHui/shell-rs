//! The runtime everything runs on, and what all connections share.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use shellrs_ssh::{self as ssh, AgentLocation, SshConnector};
use tokio::runtime::{Handle, Runtime};

use crate::connection::Connection;
use crate::failure::{Failure, SshError};
use crate::login::Login;
use crate::secrets::{AppSecrets, SecretSource};

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct CoreConfig {
    /// The app's own data folder. The trusted host keys (`known_hosts`)
    /// and the resume records of transfers live in it, as on the desktop.
    pub data_dir: String,
}

/// The core: make one when the app starts and keep it for as long as it
/// runs. Its connections, terminals and sessions stop when it goes.
#[derive(uniffi::Object)]
pub struct Core {
    runtime: Option<Runtime>,
    handle: Handle,
    connector: SshConnector,
    data_dir: PathBuf,
}

/// The data folders a core runs on now: two would both write their
/// `known_hosts` and resume records.
static DATA_DIRS: Mutex<Option<HashSet<PathBuf>>> = Mutex::new(None);

#[uniffi::export]
impl Core {
    /// Fails when another core runs on `config.data_dir`, or the folder
    /// cannot be made.
    #[uniffi::constructor]
    pub fn new(config: CoreConfig, secrets: Arc<dyn SecretSource>) -> Result<Arc<Self>, SshError> {
        let data_dir = PathBuf::from(&config.data_dir);
        std::fs::create_dir_all(&data_dir).map_err(|error| SshError::Failed {
            failure: Failure::ssh(&error.into()),
        })?;
        if !data_dirs()
            .get_or_insert_with(HashSet::new)
            .insert(data_dir.clone())
        {
            return Err(SshError::DataFolderInUse);
        }
        let runtime = match tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("shellrs-core")
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(error) => {
                release(&data_dir);
                return Err(SshError::Internal {
                    detail: error.to_string(),
                });
            }
        };
        // A phone has no agent and no `~/.ssh`; iOS asks for NAT64.
        let connector =
            SshConnector::new(data_dir.join("known_hosts"), Arc::new(AppSecrets(secrets)))
                .with_agent(AgentLocation::None)
                .with_default_key_dir(None)
                .with_nat64(true);
        Ok(Arc::new(Self {
            handle: runtime.handle().clone(),
            runtime: Some(runtime),
            connector,
            data_dir,
        }))
    }

    /// Start connecting to `login`'s host, and return at once: what
    /// happens comes as the connection's events.
    pub fn connect(&self, login: Login) -> Arc<Connection> {
        Connection::start(
            self.handle.clone(),
            self.connector.clone(),
            login.into(),
            self.data_dir.clone(),
        )
    }

    /// How many files of a transfer move at once, 1 to 10, in every SFTP
    /// session; 3 until told.
    pub fn set_parallel_transfers(&self, count: u32) {
        ssh::sftp::set_parallel_transfers(count as usize);
    }
}

impl Drop for Core {
    fn drop(&mut self) {
        // Never waits: the app may drop it on its main thread.
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
        release(&self.data_dir);
    }
}

fn data_dirs() -> std::sync::MutexGuard<'static, Option<HashSet<PathBuf>>> {
    DATA_DIRS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn release(data_dir: &PathBuf) {
    if let Some(dirs) = data_dirs().as_mut() {
        dirs.remove(data_dir);
    }
}
