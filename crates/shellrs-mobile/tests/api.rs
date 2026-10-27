//! The interface as the apps use it, against an SSH server in this process.

use std::collections::HashMap;
use std::future::Future;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use shellrs_mobile::{
    ConnectError, Connection, ConnectionEvent, Core, CoreConfig, EntryKind, FailureLink, IoKind,
    Login, LoginMethod, PromptKind, PromptReply, Route, SecretRef, SecretSource, SftpEvent,
    SshError, TerminalEvent, TerminalOptions, TransferPhase,
};
use shellrs_ssh::testing::{TEST_PASSWORD, TEST_USER, TestServer, closed_port, has_sftp_server};

/// What the app's keychain holds.
#[derive(Default)]
struct Keychain(Mutex<HashMap<SecretRef, String>>);

impl SecretSource for Keychain {
    fn secret(&self, secret: SecretRef) -> Option<String> {
        self.0.lock().unwrap().get(&secret).cloned()
    }
}

fn password_ref(port: u16) -> SecretRef {
    SecretRef::Password {
        user: TEST_USER.into(),
        host: "127.0.0.1".into(),
        port,
    }
}

fn login(port: u16) -> Login {
    Login {
        host: "127.0.0.1".into(),
        port,
        user: TEST_USER.into(),
        method: LoginMethod::Password,
        key_path: None,
        password: password_ref(port),
        route: Route::Direct,
    }
}

/// A core on a data folder of its own, with `password` saved for the
/// server at `port`.
fn core(data: &Path, port: u16, password: Option<&str>) -> Arc<Core> {
    let keychain = Keychain::default();
    if let Some(password) = password {
        keychain
            .0
            .lock()
            .unwrap()
            .insert(password_ref(port), password.into());
    }
    Core::new(
        CoreConfig {
            data_dir: data.display().to_string(),
        },
        Arc::new(keychain),
    )
    .unwrap()
}

/// `future`, or a failed test rather than a hung one.
async fn soon<T>(future: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(20), future)
        .await
        .expect("in time")
}

/// The connection's next event other than a latency reading.
async fn next(connection: &Connection) -> ConnectionEvent {
    loop {
        match soon(connection.next_event()).await.expect("an event") {
            ConnectionEvent::Latency { .. } => {}
            event => return event,
        }
    }
}

/// Log in, trusting the server's key when asked.
async fn connected(core: &Core, port: u16) -> Arc<Connection> {
    let connection = core.connect(login(port));
    loop {
        match next(&connection).await {
            ConnectionEvent::Prompt { prompt } => {
                assert!(matches!(prompt.kind, PromptKind::UnknownHost { .. }));
                connection.answer(prompt.id, PromptReply::TrustAndSave);
            }
            ConnectionEvent::Connected { .. } => return connection,
            event => panic!("{event:?}"),
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_first_login_asks_about_the_key_and_reads_the_saved_password() {
    let data = tempfile::tempdir().unwrap();
    let server = TestServer::start(data.path().into()).await.unwrap();
    let core = core(data.path(), server.port, Some(TEST_PASSWORD));

    let connection = core.connect(login(server.port));
    let ConnectionEvent::Prompt { prompt } = next(&connection).await else {
        panic!("asked about the key first");
    };
    let PromptKind::UnknownHost {
        host,
        port,
        fingerprint,
        ..
    } = &prompt.kind
    else {
        panic!("{prompt:?}");
    };
    assert_eq!((host.as_str(), *port), ("127.0.0.1", server.port));
    assert_eq!(prompt.jump_host, None);
    let trusted = fingerprint.clone();
    connection.answer(prompt.id, PromptReply::TrustAndSave);
    assert_eq!(
        next(&connection).await,
        ConnectionEvent::Connected {
            fingerprint: trusted.clone()
        }
    );
    assert!(data.path().join("known_hosts").exists());

    // Trusted now: straight in.
    let again = core.connect(login(server.port));
    assert_eq!(
        next(&again).await,
        ConnectionEvent::Connected {
            fingerprint: trusted
        }
    );

    connection.disconnect();
    assert_eq!(
        next(&connection).await,
        ConnectionEvent::Closed { failure: None }
    );
    assert_eq!(soon(connection.next_event()).await, None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_rejected_password_is_asked_for() {
    let data = tempfile::tempdir().unwrap();
    let server = TestServer::start(data.path().into()).await.unwrap();
    let core = core(data.path(), server.port, Some("wrong"));

    let connection = core.connect(login(server.port));
    let mut asked = false;
    loop {
        match next(&connection).await {
            ConnectionEvent::Prompt { prompt } => match prompt.kind {
                PromptKind::UnknownHost { .. } => {
                    connection.answer(prompt.id, PromptReply::TrustAndSave)
                }
                PromptKind::Password { saved_rejected } => {
                    assert!(saved_rejected);
                    asked = true;
                    connection.answer(
                        prompt.id,
                        PromptReply::Answers {
                            answers: vec![TEST_PASSWORD.into()],
                        },
                    );
                }
                kind => panic!("{kind:?}"),
            },
            ConnectionEvent::Connected { .. } => break,
            event => panic!("{event:?}"),
        }
    }
    assert!(asked);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_closed_port_fails_typed_as_the_network() {
    let data = tempfile::tempdir().unwrap();
    let port = closed_port();
    let core = core(data.path(), port, None);

    let connection = core.connect(login(port));
    let ConnectionEvent::Closed {
        failure: Some(failure),
    } = next(&connection).await
    else {
        panic!("closed with a failure");
    };
    assert!(failure.network);
    assert_eq!(
        failure.links[0],
        FailureLink::Connect {
            error: ConnectError::Unreachable {
                host: "127.0.0.1".into(),
                port,
            }
        }
    );
    assert!(failure.links.iter().any(|link| matches!(
        link,
        FailureLink::Io {
            kind: IoKind::ConnectionRefused,
            ..
        }
    )));
    assert!(matches!(
        soon(connection.open_terminal(options())).await,
        Err(SshError::NotConnected)
    ));
}

fn options() -> TerminalOptions {
    TerminalOptions {
        term: "xterm-256color".into(),
        columns: 80,
        rows: 24,
        width: 0,
        height: 0,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_terminal_echoes_resizes_and_ends_with_its_shell() {
    let data = tempfile::tempdir().unwrap();
    let server = TestServer::start(data.path().into()).await.unwrap();
    let core = core(data.path(), server.port, Some(TEST_PASSWORD));
    let connection = connected(&core, server.port).await;

    let terminal = soon(connection.open_terminal(options())).await.unwrap();
    terminal.write(b"hello ".to_vec()).unwrap();
    terminal.write("世界".as_bytes().to_vec()).unwrap();
    let mut echoed = Vec::new();
    while !echoed.ends_with("世界".as_bytes()) {
        match soon(terminal.next_event()).await {
            Some(TerminalEvent::Output { bytes }) => echoed.extend(bytes),
            event => panic!("{event:?}"),
        }
    }
    assert_eq!(echoed, "hello 世界".as_bytes());
    // The shell started on a terminal of the size asked for.
    assert_eq!(server.seen().size, Some((80, 24)));

    terminal.resize(120, 40, 0, 0).unwrap();
    soon(async {
        while server.seen().size != Some((120, 40)) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;

    terminal.write(b"exit\n".to_vec()).unwrap();
    assert_eq!(
        soon(terminal.next_event()).await,
        Some(TerminalEvent::Exited {
            code: Some(3),
            signal: None
        })
    );
    assert_eq!(soon(terminal.next_event()).await, None);
    assert!(matches!(
        terminal.write(b"late".to_vec()),
        Err(SshError::Closed)
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn closing_the_connection_ends_its_terminals_quietly() {
    let data = tempfile::tempdir().unwrap();
    let server = TestServer::start(data.path().into()).await.unwrap();
    let core = core(data.path(), server.port, Some(TEST_PASSWORD));
    let connection = connected(&core, server.port).await;
    let terminal = soon(connection.open_terminal(options())).await.unwrap();

    connection.disconnect();
    assert_eq!(soon(terminal.next_event()).await, None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ending_a_terminal_leaves_its_connection() {
    let data = tempfile::tempdir().unwrap();
    let server = TestServer::start(data.path().into()).await.unwrap();
    let core = core(data.path(), server.port, Some(TEST_PASSWORD));
    let connection = connected(&core, server.port).await;
    let terminal = soon(connection.open_terminal(options())).await.unwrap();

    // A call waiting on it holds it, as one from Swift or Kotlin does:
    // ending it gets through all the same.
    let waiting = tokio::spawn({
        let terminal = terminal.clone();
        async move { terminal.next_event().await }
    });
    terminal.end();
    assert_eq!(soon(waiting).await.unwrap(), None);

    let output = soon(connection.exec("uptime".into())).await.unwrap();
    assert_eq!(output.stdout, b"uptime\n");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn commands_run_beside_and_the_connection_answers_a_check() {
    let data = tempfile::tempdir().unwrap();
    let server = TestServer::start(data.path().into()).await.unwrap();
    let core = core(data.path(), server.port, Some(TEST_PASSWORD));
    let connection = connected(&core, server.port).await;

    let output = soon(connection.exec("uptime".into())).await.unwrap();
    assert_eq!(output.stdout, b"uptime\n");
    assert_eq!(server.seen().commands, ["uptime"]);
    soon(connection.check_alive()).await.unwrap();

    connection.disconnect();
    assert_eq!(
        next(&connection).await,
        ConnectionEvent::Closed { failure: None }
    );
    assert!(matches!(
        soon(connection.check_alive()).await,
        Err(SshError::NotConnected)
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn one_data_folder_has_one_core() {
    let data = tempfile::tempdir().unwrap();
    let first = core(data.path(), 22, None);
    let config = CoreConfig {
        data_dir: data.path().display().to_string(),
    };
    assert!(matches!(
        Core::new(config.clone(), Arc::new(Keychain::default())),
        Err(SshError::DataFolderInUse)
    ));
    drop(first);
    Core::new(config, Arc::new(Keychain::default())).unwrap();
}

/// The session's events until the transfer is over.
async fn transferred(session: &shellrs_mobile::SftpSession) -> Vec<SftpEvent> {
    let mut events = Vec::new();
    loop {
        let event = soon(session.next_event()).await.expect("an event");
        let idle = event == SftpEvent::Idle;
        events.push(event);
        if idle {
            return events;
        }
    }
}

fn completed(events: &[SftpEvent]) -> bool {
    events.iter().any(|event| {
        matches!(event, SftpEvent::Progress { progress }
            if progress.phase == TransferPhase::Completed && progress.failed == 0)
    })
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sftp_lists_changes_and_transfers_on_the_same_connection() {
    if !has_sftp_server() {
        eprintln!("skipped: no OpenSSH sftp-server on this machine");
        return;
    }
    let data = tempfile::tempdir().unwrap();
    let remote = data.path().join("remote");
    let local = data.path().join("local");
    std::fs::create_dir_all(&remote).unwrap();
    std::fs::create_dir_all(&local).unwrap();
    let server = TestServer::start(remote.clone()).await.unwrap();
    let core = core(data.path(), server.port, Some(TEST_PASSWORD));
    let connection = connected(&core, server.port).await;

    let session = soon(connection.open_sftp()).await.unwrap();
    let home = session.home();
    assert_eq!(
        Path::new(&home).canonicalize().unwrap(),
        remote.canonicalize().unwrap()
    );

    let source = local.join("文件.txt");
    std::fs::write(&source, b"contents").unwrap();
    session
        .upload(vec![source.display().to_string()], home.clone())
        .unwrap();
    assert!(completed(&transferred(&session).await));
    assert_eq!(std::fs::read(remote.join("文件.txt")).unwrap(), b"contents");

    soon(session.create_directory(format!("{home}/目录")))
        .await
        .unwrap();
    soon(session.rename(format!("{home}/文件.txt"), format!("{home}/目录/改名.txt")))
        .await
        .unwrap();
    let listing = soon(session.list(format!("{home}/目录"))).await.unwrap();
    let names: Vec<_> = listing
        .entries
        .iter()
        .map(|entry| (entry.name.as_str(), entry.kind, entry.size))
        .collect();
    assert_eq!(names, [("改名.txt", EntryKind::File, 8)]);

    let back = local.join("back");
    std::fs::create_dir(&back).unwrap();
    session
        .download(
            vec![format!("{home}/目录/改名.txt")],
            back.display().to_string(),
        )
        .unwrap();
    assert!(completed(&transferred(&session).await));
    assert_eq!(std::fs::read(back.join("改名.txt")).unwrap(), b"contents");

    soon(session.delete(vec![format!("{home}/目录")]))
        .await
        .unwrap();
    assert!(!remote.join("目录").exists());
    let Err(SshError::Failed { failure }) = soon(session.list(format!("{home}/目录"))).await
    else {
        panic!("listing a deleted folder fails");
    };
    assert!(!failure.network);

    // The terminal shares the connection the session is on.
    soon(connection.open_terminal(options())).await.unwrap();
    session.end();
    assert_eq!(soon(session.next_event()).await, None);
}
