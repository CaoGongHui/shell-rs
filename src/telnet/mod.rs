//! Telnet hosts: a terminal over a plain TCP connection, reached directly,
//! through SSH jump hosts or through a proxy like an SSH host, with the
//! Telnet protocol and 自动登录 written here rather than taken from a crate.

mod auto_login;
mod local_line;
mod protocol;
mod transport;

pub use transport::TelnetTransportProvider;
