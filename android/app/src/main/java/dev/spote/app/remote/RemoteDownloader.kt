package dev.spote.app.remote

/*
 * Streaming downloader for resolved remote audio.
 *
 * NewPipeExtractor v0.27.1 is GPL-3.0-or-later software
 * (https://github.com/TeamNewPipe/NewPipeExtractor); keep its license in mind when
 * distributing binaries that link it.
 */

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.withContext
import okhttp3.OkHttpClient
import okhttp3.Request
import java.io.File
import java.io.IOException
import java.net.URLDecoder
import java.nio.charset.StandardCharsets
import java.util.Locale
import java.util.concurrent.TimeUnit

/**
 * Downloads a [ResolvedAudio] to a file, streaming the response to `<safe-key>.part` and
 * renaming it to the right extension on success.
 *
 * The method is idempotent: if the final file already exists and is non-empty, it is returned
 * without touching the network. On any error or cancellation the partial file is deleted.
 * Progress is reported on the IO dispatcher with values in `0f..1f`.
 */
object RemoteDownloader {

    private const val BUFFER_SIZE = 64 * 1024
    private const val PROGRESS_STEP = 0.005f
    private const val MAX_BASE_LENGTH = 80

    private val client: OkHttpClient by lazy {
        OkHttpClient.Builder()
            .connectTimeout(15, TimeUnit.SECONDS)
            .readTimeout(30, TimeUnit.SECONDS)
            .writeTimeout(30, TimeUnit.SECONDS)
            .retryOnConnectionFailure(true)
            .followRedirects(true)
            .followSslRedirects(true)
            .build()
    }

    private val illegalFileNameChars = Regex("""[\\/:*?"<>|\p{Cntrl}]""")
    private val whitespace = Regex("\\s+")

    suspend fun download(
        stream: ResolvedAudio,
        destinationDir: File,
        onProgress: (Float) -> Unit = {},
    ): Result<File> = withContext(Dispatchers.IO) {
        val mediaUrl = stream.url.trim()
        if (!mediaUrl.startsWith("http://", ignoreCase = true) &&
            !mediaUrl.startsWith("https://", ignoreCase = true)
        ) {
            return@withContext Result.failure(
                IOException("Unsupported audio URL: ${stream.url}"),
            )
        }
        if (stream.isHls) {
            return@withContext Result.failure(
                IOException("HLS se reproduce en streaming y no se descarga como archivo único"),
            )
        }

        try {
            if (!destinationDir.isDirectory) {
                if (!destinationDir.mkdirs() && !destinationDir.isDirectory) {
                    throw IOException(
                        "Cannot create download directory: ${destinationDir.absolutePath}",
                    )
                }
            }

            val extension = extensionFor(stream.mimeType, mediaUrl)
            val base = fileBase(stream)
            val target = File(destinationDir, "$base.$extension")
            if (target.isFile && target.length() > 0L) {
                onProgress(1f)
                return@withContext Result.success(target)
            }

            val request = Request.Builder()
                .url(mediaUrl)
                .header("User-Agent", NewPipeDownloader.USER_AGENT)
                .header("Accept", "*/*")
                .build()

            val call = client.newCall(request)
            val cancelHandle = currentCoroutineContext()[Job]?.invokeOnCompletion { cause ->
                if (cause is CancellationException) call.cancel()
            }

            val part = File(destinationDir, "$base.part")
            var createdFile: File? = null
            try {
                part.delete() // stale partial from a previous attempt

                call.execute().use { response ->
                    if (response.code == 403) {
                        throw IOException(
                            "HTTP 403: the direct audio URL expired or is IP-locked; " +
                                "resolve the track again",
                        )
                    }
                    if (!response.isSuccessful) {
                        throw IOException("HTTP ${response.code} ${response.message}")
                    }

                    val body = response.body
                        ?: throw IOException("HTTP ${response.code} returned no body")
                    val totalBytes = body.contentLength()
                    var written = 0L
                    var lastFraction = -1f

                    fun reportProgress(fraction: Float, force: Boolean = false) {
                        val clamped = fraction.coerceIn(0f, 1f)
                        if (force || clamped - lastFraction >= PROGRESS_STEP) {
                            lastFraction = clamped
                            onProgress(clamped)
                        }
                    }

                    reportProgress(0f, force = true)

                    body.byteStream().use { input ->
                        part.outputStream().buffered().use { output ->
                            val buffer = ByteArray(BUFFER_SIZE)
                            while (true) {
                                currentCoroutineContext().ensureActive()
                                val read = input.read(buffer)
                                if (read == -1) break
                                output.write(buffer, 0, read)
                                written += read
                                if (totalBytes > 0L) {
                                    reportProgress(written.toFloat() / totalBytes.toFloat())
                                }
                            }
                        }
                    }

                    if (totalBytes > 0L && written < totalBytes) {
                        throw IOException("Incomplete download ($written of $totalBytes bytes)")
                    }
                    if (written <= 0L) {
                        throw IOException("The server returned an empty audio file")
                    }

                    reportProgress(1f, force = true)

                    createdFile = target
                    if (!part.renameTo(target)) {
                        part.copyTo(target, overwrite = true)
                        part.delete()
                    }
                    if (!target.isFile || target.length() <= 0L) {
                        throw IOException("Could not finalize downloaded file ${target.name}")
                    }
                }

                Result.success(target)
            } catch (t: Throwable) {
                part.delete()
                createdFile?.delete()
                currentCoroutineContext().ensureActive()
                if (t is CancellationException) throw t
                Result.failure(
                    IOException(
                        "Download of \"${stream.title}\" failed: " +
                            (t.message ?: t.javaClass.simpleName),
                        t,
                    ),
                )
            } finally {
                cancelHandle?.dispose()
            }
        } catch (cancellation: CancellationException) {
            throw cancellation
        } catch (t: Throwable) {
            Result.failure(
                IOException(
                    "Download of \"${stream.title}\" failed: " +
                        (t.message ?: t.javaClass.simpleName),
                    t,
                ),
            )
        }
    }

    /** Chooses the file extension from the mime type, with the URL as fallback. */
    internal fun extensionFor(mimeType: String?, url: String): String {
        val mime = (mimeType?.takeIf { it.isNotBlank() } ?: mimeFromUrl(url))
            ?.substringBefore(';')
            ?.trim()
            ?.lowercase(Locale.ROOT)
            .orEmpty()

        return when (mime) {
            "audio/mp4", "audio/m4a", "audio/x-m4a", "audio/aac" -> "m4a"
            "audio/webm" -> "webm"
            "audio/opus" -> "opus"
            "audio/mpeg", "audio/mp3" -> "mp3"
            "audio/ogg", "audio/vorbis" -> "ogg"
            "audio/flac", "audio/x-flac" -> "flac"
            "audio/wav", "audio/x-wav" -> "wav"
            else -> extensionFromPath(url) ?: "m4a"
        }
    }

    private fun extensionFromPath(url: String): String? {
        val path = url.substringBefore('?').lowercase(Locale.ROOT)
        return when {
            path.endsWith(".m4a") || path.endsWith(".mp4") || path.endsWith(".aac") -> "m4a"
            path.endsWith(".webm") -> "webm"
            path.endsWith(".opus") -> "opus"
            path.endsWith(".mp3") -> "mp3"
            path.endsWith(".ogg") || path.endsWith(".oga") -> "ogg"
            path.endsWith(".flac") -> "flac"
            path.endsWith(".wav") -> "wav"
            else -> null
        }
    }

    private fun mimeFromUrl(url: String): String? {
        val query = url.substringAfter('?', missingDelimiterValue = "")
        if (query.isEmpty()) return null
        for (param in query.split('&')) {
            if (!param.startsWith("mime=")) continue
            return runCatching {
                URLDecoder.decode(param.substringAfter('='), StandardCharsets.UTF_8.name())
            }.getOrNull()
        }
        return null
    }

    private fun fileBase(stream: ResolvedAudio): String {
        val artist = stream.artist.trim().takeIf {
            it.isNotEmpty() && !it.equals("Unknown artist", ignoreCase = true)
        }
        val title = stream.title.trim().ifEmpty { "audio" }
        val readable = listOfNotNull(artist, title).joinToString(" - ")

        // Stable identity across resolves, because direct media URLs expire.
        val identity = stream.webUrl.trim().ifEmpty { stream.url }
        val id = identity.hashCode().toUInt().toString(16).padStart(8, '0')

        return "${sanitizeFileName(readable)}-$id"
    }

    private fun sanitizeFileName(raw: String): String {
        val replaced = illegalFileNameChars.replace(raw, "_")
        val collapsed = whitespace.replace(replaced, " ").trim().trim('.')
        val limited = if (collapsed.length > MAX_BASE_LENGTH) {
            collapsed.take(MAX_BASE_LENGTH).trim().trim('.')
        } else {
            collapsed
        }
        return limited.ifBlank { "audio" }
    }
}
