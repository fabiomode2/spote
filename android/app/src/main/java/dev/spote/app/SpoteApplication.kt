package dev.spote.app

import android.app.Application
import android.os.Build
import dev.spote.app.logging.ActionLog
import dev.spote.app.remote.RemoteResolver

class SpoteApplication : Application() {
    override fun onCreate() {
        super.onCreate()
        ActionLog.install(this)
        ActionLog.log(
            "app.start",
            "version" to BuildConfig.VERSION_NAME,
            "sdk" to Build.VERSION.SDK_INT,
            "device" to Build.MODEL,
            "mock" to BuildConfig.USE_MOCK_DATA,
        )
        // NewPipeExtractor necesita su Downloader antes del primer search/resolve.
        runCatching { RemoteResolver.init(this) }
    }
}
