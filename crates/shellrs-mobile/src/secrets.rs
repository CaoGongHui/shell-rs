//! Saved passwords and passphrases, which the app keeps in its platform's
//! keychain and hands over when a connection asks.

use std::sync::Arc;

use shellrs_ssh as ssh;
use zeroize::Zeroizing;

/// Which saved secret a connection asks for. The same secret has the same
/// keychain account name as on the desktop ([`account_for`]), so synced
/// secrets are found under it.
#[derive(Clone, Debug, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum SecretRef {
    /// The login password of `user@host:port`.
    Password {
        user: String,
        host: String,
        port: u16,
    },
    /// The passphrase of the private key file at `key_path`.
    Passphrase { key_path: String },
    /// A password credential's password, by the credential's `keychain_id`.
    Credential { keychain_id: String },
    /// The password of an HTTP or SOCKS5 proxy.
    Proxy {
        user: String,
        host: String,
        port: u16,
    },
    /// The password of a connection that is not saved, by an id the app
    /// gave it. Keep it in memory only.
    Temporary { host: u64 },
}

impl SecretRef {
    /// The core's secrets a login reads; `None` for those it never does.
    fn from_core(secret: &ssh::SecretRef) -> Option<Self> {
        Some(match secret {
            ssh::SecretRef::Password { user, host, port } => Self::Password {
                user: user.clone(),
                host: host.clone(),
                port: *port,
            },
            ssh::SecretRef::Passphrase { key_path } => Self::Passphrase {
                key_path: key_path.clone(),
            },
            ssh::SecretRef::Credential { keychain_id } => Self::Credential {
                keychain_id: keychain_id.clone(),
            },
            ssh::SecretRef::Proxy { user, host, port } => Self::Proxy {
                user: user.clone(),
                host: host.clone(),
                port: *port,
            },
            ssh::SecretRef::Temporary { host } => Self::Temporary { host: *host },
            ssh::SecretRef::SyncKeyring | ssh::SecretRef::SyncLogin => return None,
        })
    }
}

impl From<SecretRef> for ssh::SecretRef {
    fn from(secret: SecretRef) -> Self {
        match secret {
            SecretRef::Password { user, host, port } => Self::password(user, host, port),
            SecretRef::Passphrase { key_path } => Self::Passphrase { key_path },
            SecretRef::Credential { keychain_id } => Self::credential(keychain_id),
            SecretRef::Proxy { user, host, port } => Self::proxy(user, host, port),
            SecretRef::Temporary { host } => Self::temporary(host),
        }
    }
}

/// The keychain account name `secret` is saved under, the same as on the
/// desktop.
#[uniffi::export]
pub fn account_for(secret: SecretRef) -> String {
    ssh::SecretRef::from(secret).account()
}

/// Where the app keeps saved secrets: implemented in Swift or Kotlin.
#[uniffi::export(with_foreign)]
pub trait SecretSource: Send + Sync {
    /// The saved secret, or nothing when none is saved or it cannot be
    /// read now (a locked keychain): the connection then asks the user.
    ///
    /// Called on a background thread, and only when a server wants the
    /// secret. Return at once: never wait for the main thread, such as for
    /// Face ID or a dialog. Ask through the connection's prompt instead.
    fn secret(&self, secret: SecretRef) -> Option<String>;
}

/// The app's secrets as the core reads them.
pub(crate) struct AppSecrets(pub(crate) Arc<dyn SecretSource>);

impl ssh::SecretSource for AppSecrets {
    fn secret(&self, secret: &ssh::SecretRef) -> anyhow::Result<Option<Zeroizing<String>>> {
        Ok(SecretRef::from_core(secret)
            .and_then(|secret| self.0.secret(secret))
            .map(Zeroizing::new))
    }
}

#[cfg(test)]
mod tests {
    use super::{SecretRef, account_for};

    /// Secrets synced from the desktop are found under the same names.
    #[test]
    fn accounts_are_the_desktops() {
        assert_eq!(
            account_for(SecretRef::Password {
                user: "root".into(),
                host: "10.0.0.1".into(),
                port: 22,
            }),
            "password:root@10.0.0.1:22"
        );
        assert_eq!(
            account_for(SecretRef::Passphrase {
                key_path: "/keys/id".into()
            }),
            "passphrase:/keys/id"
        );
        assert_eq!(
            account_for(SecretRef::Credential {
                keychain_id: "abc".into()
            }),
            "credential:abc"
        );
        assert_eq!(
            account_for(SecretRef::Proxy {
                user: "me".into(),
                host: "proxy".into(),
                port: 3128,
            }),
            "proxy:me@proxy:3128"
        );
        assert_eq!(account_for(SecretRef::Temporary { host: 7 }), "temporary:7");
    }
}
