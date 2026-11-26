package dev.spote.app.logging

import android.app.Application
import android.content.ClipData
import android.content.Context
import android.content.Intent
import androidx.core.content.FileProvider
import java.io.File
import java.io.FileOutputStream
import java.io.PrintWriter
import java.io.StringWriter
import java.time.Instant
import java.time.LocalDateTime
import java.time.format.DateTimeFormatter
import java.util.Locale
import java.util.UUID
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.channels.BufferOverflow
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeoutOrNull

/**
 * Registro de acciones exportable, pensado para diagnosticar problemas sin
 * depender de `logcat`.
 *
 * Cada línea es un objeto JSON independiente (JSONL):
 *
 * ```json
 * {"timestamp":"2026-11-25T12:34:56.789Z","sessionId":"...","event":"login","data":{"server":"https://..."}}
 * ```
 *
 * - [log] solo formatea y encola: nunca hace I/O ni lanza en el hilo que llama.
 * - Un único consumidor en [Dispatchers.IO] escribe en `filesDir/logs/`.
 * - Rotación: 2 MB por archivo y 7 archivos `spote-YYYYMMDD-HHmmss.log`.
 * - Las claves con `password`, `token`, `secret`, `key` o `invite` se guardan
 *   como `<redacted>`.
 * - Cualquier fallo del writer es silencioso; la app sigue funcionando aunque
 *   el log no se pueda escribir.
 *
 * [install] debe llamarse una sola vez desde `Application.onCreate`.
 */
object ActionLog {

    private const val LOGS_DIR = "logs"
    private const val FILE_PREFIX = "spote-"
    private const val FILE_SUFFIX = ".log"
    private const val MAX_FILE_BYTES = 2L * 1024L * 1024L
    private const val MAX_FILES = 7
    private const val CHANNEL_CAPACITY = 512
    private const val FLUSH_TIMEOUT_MS = 1_500L

    private val installLock = Any()
    private val writeLock = Any()

    private val channel = Channel<Entry>(
        capacity = CHANNEL_CAPACITY,
        onBufferOverflow = BufferOverflow.DROP_OLDEST,
    )
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    @Volatile
    private var installed = false

    @Volatile
    private var appContext: Context? = null

    @Volatile
    private var logDir: File? = null

    @Volatile
    private var sessionId: String = ""

    @Volatile
    private var writerJob: Job? = null

    // Estado del writer; siempre se toca dentro de writeLock.
    private var stream: FileOutputStream? = null
    private var currentFile: File? = null
    private var currentSize = 0L

    /**
     * Prepara la carpeta, arranca el writer y engancha el handler de crashes.
     * Es idempotente y no lanza.
     */
    fun install(app: Application) {
        if (installed) return
        synchronized(installLock) {
            if (installed) return

            val context = app.applicationContext
            appContext = context
            sessionId = UUID.randomUUID().toString()

            val dir = File(context.filesDir, LOGS_DIR)
            val ready = runCatching {
                dir.isDirectory || dir.mkdirs()
            }.getOrDefault(false)
            logDir = if (ready) dir else null
            if (ready) {
                runCatching {
                    synchronized(writeLock) { pruneLocked(dir) }
                }
            }

            installed = true

            runCatching {
                writerJob = scope.launch {
                    runCatching {
                        for (entry in channel) {
                            when (entry) {
                                is EntryLine -> runCatching { write(entry.text) }
                                is FlushAck -> entry.ack.complete(Unit)
                            }
                        }
                    }
                }
            }

            runCatching { installCrashHandler() }
            log("app.start")
        }
    }

    /**
     * Añade una acción al log. Acepta cualquier hilo y nunca lanza.
     *
     * Los valores se convierten a texto/JSON de forma defensiva; las claves
     * sensibles se redactan. Si el writer está saturado se descarta la entrada
     * más antigua para no bloquear nunca al llamador.
     */
    fun log(event: String, vararg data: Pair<String, Any?>) {
        if (!installed) return
        runCatching {
            val values = LinkedHashMap<String, Any?>(data.size)
            for ((key, value) in data) {
                values[key] = value
            }
            val line = encodeLine(
                timestamp = Instant.now().toString(),
                sessionId = sessionId,
                event = event,
                data = values,
            )
            channel.trySend(EntryLine(line))
        }
    }

    /**
     * Fuerza el volcado de lo pendiente y devuelve un `ACTION_SEND` con el log
     * más reciente a través de [FileProvider].
     *
     * Devuelve `null` si todavía no hay archivos o si el provider falla. Puede
     * bloquearse como máximo [FLUSH_TIMEOUT_MS] esperando al writer.
     */
    fun shareIntent(): Intent? {
        val context = appContext ?: return null
        return runCatching {
            flushBlocking()
            val file = currentFiles().lastOrNull() ?: return@runCatching null
            val uri = FileProvider.getUriForFile(
                context,
                "${context.packageName}.fileprovider",
                file,
            )
            Intent(Intent.ACTION_SEND).apply {
                type = "text/plain"
                putExtra(Intent.EXTRA_STREAM, uri)
                putExtra(Intent.EXTRA_SUBJECT, "Spote · registro de acciones")
                clipData = ClipData.newUri(context.contentResolver, "spote-log", uri)
                addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            }
        }.getOrNull()
    }

    /** Borra todos los archivos de log. El writer sigue activo para los nuevos. */
    fun clear() {
        val context = appContext ?: return
        runCatching {
            flushBlocking()
            synchronized(writeLock) {
                closeLocked()
                val dir = logDir ?: File(context.filesDir, LOGS_DIR)
                logFiles(dir).forEach { file ->
                    runCatching { file.delete() }
                }
            }
        }
    }

    /** Archivos de log actuales, ordenados del más viejo al más reciente. */
    fun currentFiles(): List<File> {
        val dir = logDir ?: return emptyList()
        return runCatching { logFiles(dir) }.getOrDefault(emptyList())
    }

    // -----------------------------------------------------------------------
    // Writer de un solo hilo
    // -----------------------------------------------------------------------

    private fun write(text: String) {
        val dir = logDir ?: return
        val bytes = (text + "\n").toByteArray(Charsets.UTF_8)
        if (bytes.size > MAX_FILE_BYTES) return

        synchronized(writeLock) {
            if (!dir.isDirectory && !dir.mkdirs()) return

            if (stream == null || currentSize + bytes.size > MAX_FILE_BYTES) {
                closeLocked()
                openLocked(dir)
            }

            val output = stream ?: return
            output.write(bytes)
            output.flush()
            currentSize += bytes.size
        }
    }

    private fun openLocked(dir: File) {
        runCatching {
            val stamp = LocalDateTime.now().format(fileNameFormatter)
            val file = File(dir, "$FILE_PREFIX$stamp$FILE_SUFFIX")
            stream = FileOutputStream(file, true)
            currentFile = file
            currentSize = file.length()
        }
        pruneLocked(dir)
    }

    private fun closeLocked() {
        runCatching { stream?.flush() }
        runCatching { stream?.close() }
        stream = null
        currentFile = null
        currentSize = 0L
    }

    /** Deja como máximo [MAX_FILES] archivos, descartando los más viejos. */
    private fun pruneLocked(dir: File) {
        val files = logFiles(dir)
        var excess = files.size - MAX_FILES
        if (excess <= 0) return
        for (file in files) {
            if (excess <= 0) break
            if (file == currentFile) continue
            if (runCatching { file.delete() }.getOrDefault(false)) excess--
        }
    }

    private fun logFiles(dir: File): List<File> =
        dir.listFiles { file ->
            file.isFile && file.name.startsWith(FILE_PREFIX) && file.name.endsWith(FILE_SUFFIX)
        }
            ?.sortedBy { it.name }
            ?: emptyList()

    // -----------------------------------------------------------------------
    // Flush y crash handler
    // -----------------------------------------------------------------------

    private fun flushBlocking() {
        if (!installed) return
        val job = writerJob ?: return
        if (!job.isActive) return
        runCatching {
            runBlocking {
                withTimeoutOrNull(FLUSH_TIMEOUT_MS) {
                    val ack = CompletableDeferred<Unit>()
                    if (channel.trySend(FlushAck(ack)).isSuccess) ack.await()
                }
            }
        }
    }

    private fun installCrashHandler() {
        val previous = Thread.getDefaultUncaughtExceptionHandler()
        val handler = Thread.UncaughtExceptionHandler { thread, throwable ->
            runCatching {
                val stackTrace = StringWriter().also { buffer ->
                    PrintWriter(buffer).use { writer ->
                        throwable.printStackTrace(writer)
                    }
                }.toString()
                log(
                    "crash",
                    "thread" to thread.name,
                    "exception" to throwable.javaClass.name,
                    "message" to throwable.message,
                    "stacktrace" to stackTrace,
                )
                flushBlocking()
            }
            previous?.uncaughtException(thread, throwable)
        }
        Thread.setDefaultUncaughtExceptionHandler(handler)
    }

    private val fileNameFormatter: DateTimeFormatter =
        DateTimeFormatter.ofPattern("yyyyMMdd-HHmmss", Locale.US)
}

private sealed interface Entry

private data class EntryLine(val text: String) : Entry

private class FlushAck(val ack: CompletableDeferred<Unit>) : Entry
