//! 主机管理: the saved hosts, which the code calls hosts. The host
//! model, its store, and the host panel/dialog.

mod credential;
mod data;
mod database;
mod describe;
mod forward;
mod group_dialog;
mod host_dialog;
mod host_panel;
mod import;
mod login;
mod mark;
mod model;
mod node;
mod outline;
mod private_key;
mod protocol;
mod quick_connect;
mod saved_terminal;
mod secret_fields;
pub mod secrets;
mod serial_settings;
mod snippet;
mod ssh_link;
mod state;
mod store;

pub use credential::*;
pub use data::{
    ApplyError, BookmarkMeta, Change, Failure, HostData, HostEvent, KeyFileError, SecretEndpoint,
    SecretError, SecretJob, SecretOutcome, SecretPlace, SecretSource, SecretWork, SecretsRead,
    SecretsWritten, SyncExport, read_secrets,
};
pub use database::{HostDatabase, OpenError, StoredData};
pub use describe::{
    ErrorMessage, KindLabel, credential_summary, matches_credential_query, matches_forward_query,
    secret_error,
};
pub use forward::*;
pub use group_dialog::{GroupForm, confirm_delete_group, open_group_dialog};
pub use host_dialog::{
    DeleteHandler, Dependents, HostForm, confirm_delete_host, join_sentences, open_host_dialog,
    open_temporary_connection_dialog,
};
pub use host_panel::{HostPanel, host_menu, import_menu_items};
pub use import::{ImportHandler, ImportOutcome, ImportSource, open_import_dialog};
pub use login::{
    DEFAULT_KEY_FILES, HostConnection, HostLogin, HostLoginExt, JumpLogin, LoginMethod, LoginRoute,
    ProxyLogin, SerialLine, TelnetAutoLogin, TelnetLogin,
};
pub use mark::mark_color;
pub use model::*;
pub use node::{HostNode, NodeDrop};
pub use outline::{first_matching_host, group_options, host_tree_items, matches_query};
pub use private_key::{GeneratedKey, KeyAlgorithm, PastedKey, PastedKeyError, read_public_key};
pub use protocol::{Protocol, ProtocolKind, TelnetSettings};
pub use quick_connect::{QuickChoice, QuickHost, open_quick_connect_dialog, quick_hosts};
pub use saved_terminal::*;
pub use secret_fields::SecretFields;
pub use serial_settings::*;
pub use snippet::*;
pub use ssh_link::{LinkKind, SshLink, standard_url};
pub use state::ConnectionState;
pub use store::{HostStore, HostStoreEvent};
