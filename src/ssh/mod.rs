//! Shared SSH connection service and remote terminal adapter.
mod connection;
mod describe;
mod error;
mod exec;
mod latency;
mod login;
mod probe;
mod prompt;
mod proxy;
#[cfg(test)]
mod route_tests;
mod secret;
mod session;
mod tester;
mod transport;
mod worker;
pub use connection::{
    AgentLocation, ForwardedTcpip, SshConnectionConfig, SshConnector, SshHandle, SshPrompts,
    is_network_error,
};
pub use describe::{
    authentication_fields, authentication_instructions, authentication_title, describe_chain,
    describe_channel_error, describe_error, describe_exec_error, describe_link,
    describe_login_error, describe_missing_credential, latency_label, unknown_host_description,
};
pub use error::{
    AgentProblem, AgentUnreachable, AuthError, ChannelError, ConnectError, ForwardProblem,
    KnownHostsError, MissingCredential, PasswordWanted, ProxyError, RouteFailure, WrapErr, Wrapped,
    find_cause, link_as, wrap, wrap_shared,
};
pub use exec::{ExecError, ExecErrorKind, ExecExit, ExecFailure, ExecStream, run_command};
pub use latency::{LATENCY_INTERVAL, Latency, LatencyLevel, round_trip};
pub use login::{
    DEFAULT_KEY_FILES, HostLogin, JumpLogin, LoginMethod, LoginRoute, ProxyKind, ProxyLogin,
};
pub use probe::HOST_OS_KEYS;
pub use prompt::{
    AuthenticationPrompt, AuthenticationRequest, ConnectionPrompt, ConnectionPromptField,
    ConnectionPromptKind, ConnectionPromptReply, ConnectionSecret, HostKeyChangedPrompt,
    UnknownHostPrompt,
};
pub use secret::{SecretRef, SecretSource, SharedSecretSource, read_secret};
pub use session::{
    ChannelStream, PtySize, SftpChannel, open_probe, open_sftp_channel, open_shell, resize_pty,
    run_exec, write_shell,
};
pub use tester::SshConnectionTester;
pub use transport::SshTerminalTransportProvider;
pub(crate) use worker::run_terminal_worker;
