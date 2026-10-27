// The Swift interface builds, in Swift 6, and works: a core connects to a
// closed port and fails, typed as the network. `run.sh` runs it.
import Foundation
import ShellRSCore

final class NoSecrets: SecretSource {
    func secret(secret: SecretRef) -> String? { nil }
}

func check(_ condition: Bool, _ message: @autoclosure () -> String) {
    if !condition {
        FileHandle.standardError.write(Data("FAILED: \(message())\n".utf8))
        exit(1)
    }
}

let folder = FileManager.default.temporaryDirectory
    .appendingPathComponent("shellrs-swift-\(UUID().uuidString)")
let config = CoreConfig(dataDir: folder.path)
let core = try Core(config: config, secrets: NoSecrets())

do {
    _ = try Core(config: config, secrets: NoSecrets())
    check(false, "a second core ran on the same folder")
} catch SshError.DataFolderInUse {}

// Nothing listens on port 1.
let connection = core.connect(
    login: Login(
        host: "127.0.0.1", port: 1, user: "nobody", method: .noPassword,
        keyPath: nil, password: .temporary(host: 1), route: .direct))
var failure: Failure?
while let event = await connection.nextEvent() {
    if case .closed(let closed) = event {
        failure = closed
    }
}
check(failure?.network == true, "not a network failure: \(String(describing: failure))")
check(
    failure?.links.first == .connect(error: .unreachable(host: "127.0.0.1", port: 1)),
    "not unreachable: \(String(describing: failure))")

do {
    _ = try await connection.exec(command: "true")
    check(false, "a closed connection ran a command")
} catch SshError.NotConnected {}

check(
    accountFor(secret: .password(user: "root", host: "10.0.0.1", port: 22))
        == "password:root@10.0.0.1:22",
    "the keychain account is not the desktop's")

try? FileManager.default.removeItem(at: folder)
print("ok: \(failure!.links)")
