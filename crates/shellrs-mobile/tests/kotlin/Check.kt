// The Kotlin interface builds and works: a core connects to a closed port
// and fails, typed as the network. `run.sh` runs it on this computer's JVM.
import com.shellrs.core.*
import java.nio.file.Files
import kotlin.system.exitProcess
import kotlinx.coroutines.runBlocking

class NoSecrets : SecretSource {
    override fun secret(secret: SecretRef): String? = null
}

fun check(condition: Boolean, message: () -> String) {
    if (!condition) {
        System.err.println("FAILED: ${message()}")
        exitProcess(1)
    }
}

fun main() = runBlocking {
    val folder = Files.createTempDirectory("shellrs-kotlin")
    val config = CoreConfig(folder.toString())
    val core = Core(config, NoSecrets())

    try {
        Core(config, NoSecrets())
        check(false) { "a second core ran on the same folder" }
    } catch (_: SshException.DataFolderInUse) {}

    // Nothing listens on port 1.
    val connection = core.connect(
        Login("127.0.0.1", 1u, "nobody", LoginMethod.NO_PASSWORD, null, SecretRef.Temporary(1u), Route.Direct)
    )
    var failure: Failure? = null
    while (true) {
        val event = connection.nextEvent() ?: break
        if (event is ConnectionEvent.Closed) failure = event.failure
    }
    check(failure?.network == true) { "not a network failure: $failure" }
    check(failure?.links?.first() == FailureLink.Connect(ConnectError.Unreachable("127.0.0.1", 1u))) {
        "not unreachable: $failure"
    }

    try {
        connection.exec("true")
        check(false) { "a closed connection ran a command" }
    } catch (_: SshException.NotConnected) {}

    check(accountFor(SecretRef.Password("root", "10.0.0.1", 22u)) == "password:root@10.0.0.1:22") {
        "the keychain account is not the desktop's"
    }

    // Kotlin's own close() frees the core at once.
    core.close()
    folder.toFile().deleteRecursively()
    println("ok: ${failure!!.links}")
}
