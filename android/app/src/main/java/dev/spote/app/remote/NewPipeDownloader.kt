package dev.spote.app.remote

/*
 * NewPipeExtractor transport for Spote.
 *
 * NewPipeExtractor v0.27.1 is GPL-3.0-or-later software
 * (https://github.com/TeamNewPipe/NewPipeExtractor). It is used here as an unmodified
 * library dependency. Keep this notice when distributing a binary that links it.
 */

import okhttp3.OkHttpClient
import okhttp3.RequestBody.Companion.toRequestBody
import org.schabi.newpipe.extractor.downloader.Downloader
import org.schabi.newpipe.extractor.downloader.Request
import org.schabi.newpipe.extractor.downloader.Response
import org.schabi.newpipe.extractor.exceptions.ReCaptchaException
import java.util.Locale
import java.util.concurrent.TimeUnit

/**
 * [Downloader] implementation backed by OkHttp 4.
 *
 * OkHttp transparently asks for and decodes gzip (it adds `Accept-Encoding: gzip` itself and
 * strips the encoding headers afterwards), so no manual gzip handling is needed here. The same
 * client follows redirects and reports the final URL through [Response.latestUrl].
 */
class NewPipeDownloader(
    private val client: OkHttpClient = defaultClient(),
) : Downloader() {

    override fun execute(request: Request): Response {
        val method = request.httpMethod().uppercase(Locale.ROOT)

        // OkHttp rejects GET/HEAD with a body; only send a body for methods that carry one.
        val body = if (method == "POST" || method == "PUT" || method == "PATCH") {
            (request.dataToSend() ?: ByteArray(0)).toRequestBody(null)
        } else {
            null
        }

        val builder = okhttp3.Request.Builder()
            .url(request.url())
            .method(method, body)
            // Default UA; headers supplied by the extractor override it below.
            .header("User-Agent", USER_AGENT)

        request.headers().forEach { (name, values) ->
            // Let OkHttp own framing and compression headers.
            if (name.equals("Content-Length", ignoreCase = true) ||
                name.equals("Host", ignoreCase = true) ||
                name.equals("Accept-Encoding", ignoreCase = true)
            ) {
                return@forEach
            }
            builder.removeHeader(name)
            values.forEach { value -> builder.addHeader(name, value) }
        }

        client.newCall(builder.build()).execute().use { response ->
            if (response.code == 429) {
                throw ReCaptchaException("reCAPTCHA challenge requested", request.url())
            }

            return Response(
                response.code,
                response.message,
                response.headers.toMultimap(),
                response.body?.string().orEmpty(),
                response.request.url.toString(),
            )
        }
    }

    companion object {
        /** Browser-like UA for services that reject the OkHttp default. */
        const val USER_AGENT =
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 " +
                "(KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36"

        fun defaultClient(): OkHttpClient = OkHttpClient.Builder()
            .connectTimeout(15, TimeUnit.SECONDS)
            .readTimeout(30, TimeUnit.SECONDS)
            .writeTimeout(30, TimeUnit.SECONDS)
            .callTimeout(60, TimeUnit.SECONDS)
            .retryOnConnectionFailure(true)
            .followRedirects(true)
            .followSslRedirects(true)
            .build()
    }
}
