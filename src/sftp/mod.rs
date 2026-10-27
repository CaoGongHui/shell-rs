//! Real file browsing and resumable SFTP uploads. No GPUI or workspace dependencies.
//!
//! The engine (the worker, its transfers and the protocol under them) says
//! what happens in typed values and depends on nothing of the app's but
//! the SSH layer. The app's side words it (`describe`), keeps this
//! computer's files (`local`), and runs workers on their own threads
//! (`transport`).
mod client;
mod control;
mod describe;
mod download;
mod edit;
mod engine;
mod error;
mod journal;
mod local;
mod meter;
mod model;
mod operations;
mod parallel;
mod speed;
mod sync;
mod transport;
mod upload;
mod worker;
pub use client::TerminalNotConnected;
pub use control::Cancelled;
pub use describe::*;
pub use edit::{EDIT_LIMIT, FileBytes, FileStamp, TextFile, TextFormat};
pub use engine::{
    Disconnect, EngineEvent, EventSink, EventsClosed, RemoteOperation, SftpChannelOpener,
    SftpCommand,
};
pub use error::*;
pub use local::*;
pub use model::*;
pub use parallel::{DEFAULT_PARALLEL_TRANSFERS, PARALLEL_TRANSFERS_RANGE, set_parallel_transfers};
pub use transport::*;
pub use worker::SftpWorker;

/// Why the editor could not open a file, as the app tells it.
pub type ReadFailure = edit::ReadFailure<String>;
/// Why the editor's save did not happen or finish, as the app tells it.
pub type SaveFailure = edit::SaveFailure<String>;

#[cfg(all(test, unix))]
mod protocol_tests;
#[cfg(test)]
mod tests;
