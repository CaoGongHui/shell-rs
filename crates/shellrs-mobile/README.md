# shellrs-mobile

The SSH core of the ShellRS iOS and Android apps: the desktop app's own
connection and SFTP code (`crates/shellrs-ssh`) behind one interface, which
Swift and Kotlin bindings are generated from with
[UniFFI](https://mozilla.github.io/uniffi-rs/). This interface is the one the
apps rely on and is kept stable; the cores under it change as the desktop
needs. MIT licensed.

It logs in (passwords, private keys, keyboard-interactive, jump hosts, HTTP
and SOCKS5 proxies), checks host keys against the app's own `known_hosts`,
opens terminals and SFTP sessions on one connection, runs commands and
transfers files with resume. It has no words: questions and failures are
typed, and the app words them in its own language.

## Building

| Platform | Command | Output in `target/mobile/` |
|---|---|---|
| iOS | `scripts/build-ios.sh` | `ios/ShellRSCore.xcframework` (iPhones, and the simulator on Apple silicon), `ios/Sources/ShellRSCore.swift` |
| Android | `scripts/build-android.sh` | `android/jniLibs/{arm64-v8a,x86_64}/libshellrs_mobile.so`, `android/kotlin/` |

- **iOS** needs Xcode and
  `rustup target add aarch64-apple-ios aarch64-apple-ios-sim`. Add the
  xcframework and the Swift file to the app's target; nothing else links
  with it. `IPHONEOS_DEPLOYMENT_TARGET` sets the oldest iOS, 15.0 unless set.
- **Android** needs the NDK r28 or later (`ANDROID_NDK_HOME`), which lays the
  libraries out for 16 KB pages as Google Play asks, `cargo install cargo-ndk`
  and `rustup target add aarch64-linux-android x86_64-linux-android`. Copy
  `jniLibs` to `app/src/main/jniLibs` and the Kotlin sources into the app's.
  They need `net.java.dev.jna:jna:5.x@aar` and
  `org.jetbrains.kotlinx:kotlinx-coroutines-core`. `ANDROID_PLATFORM` sets
  the oldest Android, API level 24 unless set.

`cargo test -p shellrs-mobile` runs its tests against an SSH server in the
test process (`cargo test` leaves this crate out). `tests/swift/run.sh` and
`tests/kotlin/run.sh` build the generated Swift and Kotlin against a build
for the computer and run a short check with them, after the build scripts.

## Using it

```swift
final class Keychain: SecretSource {
    func secret(secret: SecretRef) -> String? {
        savedPassword(account: accountFor(secret: secret))  // The app's own lookup.
    }
}

let core = try Core(config: CoreConfig(dataDir: appSupport.path), secrets: Keychain())
let connection = core.connect(login: Login(
    host: "10.0.0.1", port: 22, user: "root", method: .password, keyPath: nil,
    password: .password(user: "root", host: "10.0.0.1", port: 22), route: .direct))

Task {
    while let event = await connection.nextEvent() {
        switch event {
        case .prompt(let prompt):     // Ask the user, then:
            connection.answer(promptId: prompt.id, reply: .trustAndSave)
        case .connected:              // Open terminals and sessions.
            let terminal = try await connection.openTerminal(options: TerminalOptions(
                term: "xterm-256color", columns: 80, rows: 24, width: 0, height: 0))
        case .latency(let ms): break
        case .closed(let failure): break   // nil: the app disconnected.
        }
    }
}
```

- **One `Core`** per data folder, made when the app starts and kept while it
  runs. The data folder holds the trusted host keys and the transfers'
  resume records; a second core on the same folder fails with
  `DataFolderInUse`. When the core goes, so does everything it opened.
- **Events.** `Connection`, `Terminal` and `SftpSession` each say what
  happens through `nextEvent()`, which the app awaits in a loop until it
  returns nil. Calls answer for themselves; nothing calls back into the app
  except `SecretSource`. Terminal output comes in chunks of up to 64 KiB;
  an app that falls far behind loses the oldest output and gets
  `OutputDropped` with how much.
- **One login, many channels.** Terminals, SFTP sessions and `exec` all open
  on the connection's one login, which bastion hosts allow where a second
  login would not.
- **Secrets.** A connection asks `SecretSource` for a saved password or
  passphrase only when the server wants one. The keychain account names are
  the desktop's (`accountFor`), so synced secrets are found. The core saves
  no secret: the app saves what the user types if it wants to.
- **Failures** are a chain of typed links, outermost first; word each and
  join them with ": ", as the desktop does. `network` says whether trying
  again may help. Links the core adds later arrive as `other(detail:)`.
- **A phone has no agent and no `~/.ssh`.** `noPassword` gets in only
  where the server asks for nothing, `agent` fails, and keys come from files
  the app keeps (`key` with `keyPath`).

## Threads

- Every method may be called from any thread. The ones that are not `async`
  never wait; the `async` ones run on the core's own threads.
- `SecretSource.secret` is called on one of those threads. Return at once:
  never wait for the main thread, Face ID or a dialog there, or the
  connection stalls. Return nil and the connection asks through a prompt.
- **Ending things.** `connection.disconnect()`, `terminal.end()` and
  `session.end()` end them at once, and their `nextEvent()` returns nil.
  Do that rather than counting on the objects being freed: a call waiting
  on an object (such as the event loop's `nextEvent()`) holds it, so letting
  go of it does not end it. (They are not named `close`: Kotlin's
  `AutoCloseable.close()` only frees the handle.)
- **Cancelling.** Cancelling a Kotlin coroutine stops the call it waits on.
  Cancelling a Swift `Task` does not (UniFFI does not pass it on yet): the
  call returns when it is done, so end the object to stop it sooner.

## Foreground and background

iOS suspends an app soon after it goes to the background, and Android may
too; the server then drops its connections after its keepalive gives up.

- When the app comes back to the foreground, or the network changes, call
  `checkAlive()` on each connection. It answers with the round trip, or
  fails at once for a connection that no longer works, which then closes
  with `network` set. Without it, a dead connection is noticed only after
  its keepalive, up to 90 seconds.
- A closed connection stays closed: connect again with a new `Connection`.
  A shell does not survive this; a terminal starts a new one.
- A transfer reconnects its channel on its own while its connection is up.
  When the connection is gone, start the same transfer again on a new
  session: files continue where they stopped, from the resume records in
  the data folder.
- On iOS-only IPv6 networks (NAT64), an IPv4 address is reached through the
  network's NAT64 prefix, as App Review tests.
