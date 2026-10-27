//! ShellRS for the mobile apps: the SSH core (`shellrs-ssh`) behind an
//! interface Swift and Kotlin bindings are generated from (UniFFI). This is
//! the one interface the apps rely on, kept stable; the cores under it
//! change as the desktop app needs.
//!
//! A [`Core`] per data folder holds the runtime everything runs on. A
//! [`Connection`] logs in once and carries any number of terminals
//! ([`Terminal`]) and SFTP sessions ([`SftpSession`]). Each object says
//! what happens through `next_event()`, which the app awaits in a loop:
//! nothing calls back into the app but [`SecretSource`], for saved
//! passwords. Nothing here is in words: failures are typed ([`Failure`]),
//! and the app words them in its own language.

mod connection;
mod core;
mod events;
mod failure;
mod login;
mod prompt;
mod secrets;
mod sftp;
mod task;
mod terminal;

pub use connection::{Connection, ConnectionEvent, ExecOutput};
pub use core::{Core, CoreConfig};
pub use failure::*;
pub use login::{JumpHost, Login, LoginMethod, Proxy, ProxyKind, Route};
pub use prompt::{Prompt, PromptField, PromptKind, PromptReply};
pub use secrets::{SecretRef, SecretSource, account_for};
pub use sftp::{
    DirectoryEntry, DirectoryListing, EntryKind, SftpDisconnect, SftpEvent, SftpSession,
    TransferAnswer, TransferChoice, TransferDetail, TransferDirection, TransferOutcome,
    TransferPhase, TransferProgress, TransferQuestion, TransferQuestionKind,
};
pub use terminal::{Terminal, TerminalEvent, TerminalOptions};

uniffi::setup_scaffolding!();
